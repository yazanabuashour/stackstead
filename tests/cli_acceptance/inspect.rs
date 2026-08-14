use super::*;

#[test]
fn runtime_probe_failure_is_reported_without_breaking_inspect_json() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    project.create("feature-a")?;

    let ps = stackstead(&project.repo)
        .env("PATH", "")
        .args(["ps", "--json"])
        .assert()
        .success();
    let listed: Value =
        serde_json::from_slice(&ps.get_output().stdout).test_context("parse ps output")?;
    assert_eq!(
        listed["stacksteads"][0]["runtime"], "unknown",
        "test contract values differ"
    );

    let inspect = stackstead(&project.repo)
        .env("PATH", "")
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspect.get_output().stdout)
        .test_context("parse inspect output")?;
    assert_eq!(inspected["version"], "3", "test contract values differ");
    assert_eq!(
        inspected["live"]["runtime"]["running"], false,
        "test contract values differ"
    );
    assert_eq!(
        inspected["live"]["runtime"]["status"], "unknown",
        "test contract values differ"
    );
    assert_eq!(
        inspected["live"]["database"]["status"], "unknown",
        "test contract values differ"
    );
    assert_eq!(
        inspected["effective"]["runtime"]["basis"], "live",
        "test contract values differ"
    );
    assert!(
        inspected["warnings"]
            .as_array()
            .is_some_and(|warnings| !warnings.is_empty()),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn inspect_reports_unreadable_compose_files_instead_of_hiding_them() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    fs::remove_file(&manifest.compose_files[0]).test_context("remove generated Compose fixture")?;

    let inspected = stackstead(&project.repo)
        .env("PATH", "")
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout)
        .test_context("parse inspect output")?;

    assert!(
        inspected["warnings"].as_array().is_some_and(|warnings| {
            warnings.iter().any(|warning| {
                warning.as_str().is_some_and(|warning| {
                    warning.contains("could not inspect fixed ports")
                        && warning.contains(manifest.compose_files[0].to_string_lossy().as_ref())
                })
            })
        }),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn inspect_rejects_mismatched_manifest_port_service_sets() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let mut manifest = project.create("feature-a")?;
    manifest.container_ports.remove("web");
    manifest
        .save_atomic()
        .test_context("write malformed manifest fixture")?;

    let rejected = stackstead(&project.repo)
        .args(["inspect", "feature-a"])
        .assert()
        .failure();

    assert!(
        output_text(&rejected.get_output().stderr)?
            .contains("manifest host and container port service sets differ"),
        "test contract condition failed"
    );
    assert!(
        !output_text(&rejected.get_output().stdout)?.contains("-> 0"),
        "test contract condition failed"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn inspect_distinguishes_completed_and_failed_compose_services() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    project.create("feature-a")?;
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "inspect-services-bin",
        r#"#!/bin/sh
case " $* " in
  *" ps --all --format json "*)
    printf '%s\n' '[{"Name":"demo-web-1","Service":"web","State":"running","ExitCode":0},{"Name":"demo-init-1","Service":"init","State":"exited","ExitCode":0},{"Name":"demo-migrate-1","Service":"migrate","State":"exited","ExitCode":7}]'
    exit 0
    ;;
esac
exit 97
"#,
    )?;

    let json = stackstead(&project.repo)
        .env("PATH", &path)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&json.get_output().stdout).test()?;
    assert_eq!(inspected["version"], "3", "test contract values differ");
    assert_eq!(
        inspected["live"]["runtime"]["status"], "running",
        "test contract values differ"
    );
    assert_eq!(
        inspected["live"]["services"][0]["status"], "completed (0)",
        "test contract values differ"
    );
    assert_eq!(
        inspected["live"]["services"][1]["status"], "exited (7)",
        "test contract values differ"
    );
    assert_eq!(
        inspected["live"]["services"][2]["status"], "running",
        "test contract values differ"
    );

    let human = stackstead(&project.repo)
        .env("PATH", path)
        .args(["inspect", "feature-a"])
        .assert()
        .success();
    let stdout = output_text(&human.get_output().stdout)?;
    assert!(stdout.contains("init           completed (0)"), "{stdout}");
    assert!(stdout.contains("migrate        exited (7)"), "{stdout}");
    assert!(stdout.contains("web            running"), "{stdout}");
    Ok(())
}

#[test]
fn inspect_human_output_ends_with_full_id_actions() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;

    let inspect = stackstead(&project.repo)
        .env("PATH", "")
        .args(["inspect", "feature-a"])
        .assert()
        .success();
    let stdout = output_text(&inspect.get_output().stdout)?;

    assert!(
        stdout.contains(&format!(
            "\nNext:\n  stackstead doctor\n  stackstead context {} --print\n",
            manifest.stackstead_id
        )),
        "test contract condition failed"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn database_status_requires_the_exact_compose_port_publication() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let port = manifest.ports["postgres"];
    let _listener =
        TcpListener::bind(("127.0.0.1", port)).test_context("bind unrelated listener")?;

    let wrong_path = fake_docker_path(
        project.repo.parent().test()?,
        "wrong-database-publication-fake-bin",
        &format!(
            "#!/bin/sh\ncase \" $* \" in *\" ps --all --format json \"*) printf '%s\\n' '[{{\"Name\":\"demo-postgres-1\",\"Service\":\"postgres\",\"State\":\"running\",\"ExitCode\":0}}]'; exit 0;; esac\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    ps) printf 'container-id\\n'; exit 0 ;;\n    port) printf '127.0.0.2:{port}\\n'; exit 0 ;;\n  esac\ndone\nexit 0\n"
        ),
    )?;
    let status = stackstead(&project.repo)
        .env("PATH", &wrong_path)
        .args(["db", "status", "feature-a", "--json"])
        .assert()
        .success();
    let status: Value = serde_json::from_slice(&status.get_output().stdout).test()?;
    for key in [
        "stackstead_id",
        "strategy",
        "service",
        "host",
        "port",
        "database",
        "reachable",
        "identity_status",
        "seed_status",
        "last_seed_at",
    ] {
        assert!(status.get(key).is_some(), "db status JSON omitted {key}");
    }
    assert_eq!(status["reachable"], true, "test contract values differ");
    assert_eq!(
        status["identity_status"], "unreachable",
        "test contract values differ"
    );

    let inspected = stackstead(&project.repo)
        .env("PATH", &wrong_path)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout).test()?;
    assert_eq!(
        inspected["live"]["database"]["reachable"], true,
        "test contract values differ"
    );
    assert_eq!(
        inspected["live"]["database"]["status"], "unreachable",
        "test contract values differ"
    );

    let exact_path = fake_docker_path(
        project.repo.parent().test()?,
        "exact-database-publication-fake-bin",
        &format!(
            "#!/bin/sh\ncase \" $* \" in *\" ps --all --format json \"*) printf '%s\\n' '[{{\"Name\":\"demo-postgres-1\",\"Service\":\"postgres\",\"State\":\"running\",\"ExitCode\":0}}]'; exit 0;; esac\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    ps) printf 'container-id\\n'; exit 0 ;;\n    port) printf '127.0.0.1:{port}\\n'; exit 0 ;;\n  esac\ndone\nexit 0\n"
        ),
    )?;
    let status = stackstead(&project.repo)
        .env("PATH", &exact_path)
        .args(["db", "status", "feature-a", "--json"])
        .assert()
        .success();
    let status: Value = serde_json::from_slice(&status.get_output().stdout).test()?;
    assert_eq!(status["reachable"], true, "test contract values differ");
    assert_eq!(
        status["identity_status"], "reachable",
        "test contract values differ"
    );

    let inspected = stackstead(&project.repo)
        .env("PATH", exact_path)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout).test()?;
    assert_eq!(
        inspected["live"]["database"]["reachable"], true,
        "test contract values differ"
    );
    assert_eq!(
        inspected["live"]["database"]["status"], "reachable",
        "test contract values differ"
    );
    Ok(())
}
