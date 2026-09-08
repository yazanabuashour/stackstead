use super::*;

#[test]
fn plans_isolation_from_common_compose_ports() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let file = directory.path().join("compose.yaml");
    std::fs::write(
        &file,
        r#"
services:
  web:
    image: nginx
    ports:
      - "3000:80"
      - target: 8080
        published: 4000
  postgres:
    image: postgres:16
    ports:
      - "127.0.0.1:${POSTGRES_PORT}:5432"
"#,
    )
    .test()?;

    let plan = plan(directory.path()).test()?;
    assert_eq!(plan.file, Path::new("compose.yaml"));
    assert_eq!(plan.ports.len(), 3);
    assert_eq!(plan.ports[0].env, "WEB_PORT");
    assert_eq!(plan.ports[0].current_host_port, Some(3000));
    assert_eq!(plan.ports[0].replacement, "127.0.0.1:${WEB_PORT}:80");
    assert_eq!(plan.ports[1].name, "web-8080");
    assert_eq!(plan.ports[1].env, "WEB_8080_PORT");
    assert_eq!(plan.ports[2].current_host_port, None);
    assert_eq!(plan.ports[2].env, "POSTGRES_PORT");
    assert_eq!(plan.warnings.len(), 2);
    assert!(
        plan.warnings
            .iter()
            .all(|warning| warning.contains("compose apply"))
    );
    Ok(())
}

#[test]
fn explicit_compose_paths_cannot_escape_the_repository() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let repo = directory.path().join("repo");
    std::fs::create_dir(&repo).test()?;
    let contents = "services:\n  web:\n    ports:\n      - \"3000:80\"\n";
    std::fs::write(repo.join("inside.yml"), contents).test()?;
    let outside = directory.path().join("outside-compose.yml");
    std::fs::write(&outside, contents).test()?;

    assert_eq!(
        plan_at(&repo.join("."), Some(Path::new("inside.yml")))
            .test()?
            .file,
        Path::new("inside.yml")
    );

    (plan_at(&repo, Some(&outside))).test_err()?;
    (plan_at(&repo, Some(Path::new("../outside-compose.yml")))).test_err()?;
    (apply_at(&repo, Some(Path::new("../outside-compose.yml")))).test_err()?;
    assert_eq!(std::fs::read_to_string(&outside).test()?, contents);

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, repo.join("compose.yml")).test()?;
        (apply_at(&repo, Some(Path::new("compose.yml")))).test_err()?;
        assert_eq!(std::fs::read_to_string(&outside).test()?, contents);
    }
    Ok(())
}

#[test]
fn reuses_the_variable_already_consumed_by_compose() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    std::fs::write(
        directory.path().join("compose.yaml"),
        "services:\n  web:\n    ports: [\"127.0.0.1:${APP_PORT:-3000}:80\", \"127.0.0.1:$ADMIN_PORT:81\"]\n",
    )
    .test()?;
    let plan = plan(directory.path()).test()?;
    assert_eq!(plan.ports[0].env, "APP_PORT");
    assert_eq!(plan.ports[0].current_host_port, None);
    assert_eq!(plan.ports[1].env, "ADMIN_PORT");
    Ok(())
}

#[test]
fn rejects_generated_ports_on_non_loopback_interfaces() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let file = directory.path().join("compose.yaml");
    for mapping in [
        "${MISSING_PORT}:80",
        "0.0.0.0:${IPV4_PORT}:81",
        "[::]:${IPV6_PORT}:82",
    ] {
        std::fs::write(
            &file,
            format!("services:\n  web:\n    ports: [\"{mapping}\"]\n"),
        )
        .test()?;
        (plan(directory.path())).test_err()?;
    }
    std::fs::write(
        &file,
        "services:\n  web:\n    ports: [\"127.0.0.1:${WEB_PORT}:80\"]\n",
    )
    .test()?;
    assert!(plan(directory.path()).test()?.warnings.is_empty());
    Ok(())
}

#[test]
fn rejects_container_only_ports_instead_of_inventing_a_host_url() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    std::fs::write(
        directory.path().join("compose.yaml"),
        "services:\n  web:\n    image: nginx\n    ports: [\"80\"]\n",
    )
    .test()?;
    let error = plan(directory.path()).test_err()?.to_string();
    assert!(error.contains("without a deterministic host binding"));
    assert!(error.contains("${WEB_PORT}:80"));
    Ok(())
}

#[test]
fn rejects_unsupported_ports_and_generated_environment_collisions() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let file = directory.path().join("compose.yaml");
    std::fs::write(
        &file,
        "services:\n  web:\n    image: nginx\n    ports: [3000]\n",
    )
    .test()?;
    assert!(
        plan_file(directory.path(), &file)
            .test_err()?
            .to_string()
            .contains("unsupported")
    );

    std::fs::write(
        &file,
        "services:\n  web:\n    image: nginx\n    ports: [\"3000-3002:80-82\"]\n",
    )
    .test()?;
    assert!(
        plan_file(directory.path(), &file)
            .test_err()?
            .to_string()
            .contains("unsupported")
    );

    std::fs::write(
        &file,
        "services:\n  foo-bar:\n    image: nginx\n    ports: [\"3000:80\"]\n  foo_bar:\n    image: nginx\n    ports: [\"4000:80\"]\n",
    )
    .test()?;
    assert!(
        plan_file(directory.path(), &file)
            .test_err()?
            .to_string()
            .contains("FOO_BAR_PORT")
    );

    for compose in [
        "services:\n  web:\n    ports: [\"192.168.1.8:3000:80\"]\n",
        "services:\n  web:\n    ports:\n      - target: 80\n        published: 3000\n        host_ip: 192.168.1.8\n",
        "services:\n  web:\n    ports: [\"${WEB_PORT}:80/udp\"]\n",
        "services:\n  web:\n    ports:\n      - target: 80\n        published: ${WEB_PORT}\n        protocol: udp\n",
        "services:\n  web:\n    ports: [\"${WEB_PORT:+3000}:80\"]\n",
    ] {
        std::fs::write(&file, compose).test()?;
        (plan_file(directory.path(), &file)).test_err()?;
    }
    Ok(())
}
