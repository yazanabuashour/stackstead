use super::*;

#[test]
fn compose_discovery_generates_config_and_rewrites_only_after_confirmation() -> anyhow::Result<()> {
    let project = Project::git_repo()?;
    let compose = project.repo.join("docker-compose.yml");
    fs::write(
        &compose,
        r#"services:
  web:
    image: nginx:alpine
    ports:
      - "3000:80"
  postgres:
    image: postgres:16-alpine
    ports:
      - "5432:5432"
"#,
    )
    .test_context("write fixed-port Compose fixture")?;
    git(&project.repo, &["add", "docker-compose.yml"])?;
    git(&project.repo, &["commit", "-m", "use fixed fixture ports"])?;

    stackstead(&project.repo).arg("init").assert().success();
    let config = load_config(&project.repo.join("stackstead.yaml"))?;
    assert_eq!(
        config["resources"]["ports"]["expose"]["web"]["container"],
        80
    );
    assert_eq!(
        config["resources"]["ports"]["expose"]["postgres"]["container"],
        5432
    );
    assert_eq!(config["health"]["checks"].as_sequence().test()?.len(), 1);
    assert_eq!(config["health"]["checks"][0]["name"], "web");

    let plan = stackstead(&project.repo)
        .args(["--json", "compose", "plan"])
        .assert()
        .success();
    let plan: Value =
        serde_json::from_slice(&plan.get_output().stdout).test_context("parse Compose plan")?;
    assert_eq!(plan["kind"], "ComposePlan");
    assert_eq!(plan["version"], "1");
    assert_eq!(plan["file"], "docker-compose.yml");
    let ports = plan["ports"]
        .as_array()
        .test_context("Compose plan ports")?;
    assert_eq!(
        ports
            .iter()
            .find(|port| port["name"] == "web")
            .test_context("web plan")?["current_host_port"],
        3000
    );
    assert_eq!(
        ports
            .iter()
            .find(|port| port["name"] == "postgres")
            .test_context("Postgres plan")?["current_host_port"],
        5432
    );

    let original = fs::read(&compose).test_context("read original Compose fixture")?;
    stackstead(&project.repo)
        .args(["compose", "apply"])
        .assert()
        .failure();
    assert_eq!(
        fs::read(&compose).test_context("reread Compose fixture")?,
        original
    );

    stackstead(&project.repo)
        .args(["compose", "apply", "--yes"])
        .assert()
        .success();
    let rewritten = fs::read_to_string(&compose).test_context("read rewritten Compose fixture")?;
    assert!(rewritten.contains("127.0.0.1:${WEB_PORT}:80"));
    assert!(rewritten.contains("127.0.0.1:${POSTGRES_PORT}:5432"));
    Ok(())
}

#[test]
fn explicit_nested_compose_file_drives_init_plan_and_apply() -> anyhow::Result<()> {
    let project = Project::git_repo()?;
    git(&project.repo, &["rm", "docker-compose.yml"])?;
    let nested = project.repo.join("infra/docker/compose.yml");
    fs::create_dir_all(nested.parent().test()?).test_context("create nested Compose directory")?;
    fs::write(
        &nested,
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"3000:80\"\n",
    )
    .test_context("write nested Compose file")?;
    git(&project.repo, &["add", "infra/docker/compose.yml"])?;
    git(&project.repo, &["commit", "-m", "add nested Compose file"])?;

    let missing = stackstead(&project.repo).arg("init").assert().failure();
    let error = output_text(&missing.get_output().stderr)?;
    assert!(error.contains("--compose-file"));
    assert!(error.contains("infra/docker/compose.yml"));

    stackstead(&project.repo)
        .args(["init", "--compose-file", "infra/docker/compose.yml"])
        .assert()
        .success();
    let config = load_config(&project.repo.join("stackstead.yaml"))?;
    assert_eq!(
        config["runtime"]["files"],
        serde_yaml::Value::Sequence(vec!["infra/docker/compose.yml".into()])
    );

    let plan = stackstead(&project.repo)
        .args(["compose", "plan", "--json"])
        .assert()
        .success();
    let plan: Value =
        serde_json::from_slice(&plan.get_output().stdout).test_context("parse plan")?;
    assert_eq!(plan["file"], "infra/docker/compose.yml");

    let second = project.repo.join("infra/docker/admin-compose.yml");
    fs::write(
        &second,
        "services:\n  admin:\n    image: nginx:alpine\n    ports:\n      - \"4000:81\"\n",
    )
    .test_context("write second Compose file")?;
    let second_before = fs::read(&second).test()?;
    let mut config = config;
    config["runtime"]["files"]
        .as_sequence_mut()
        .test()?
        .push("infra/docker/admin-compose.yml".into());
    fs::write(
        project.repo.join("stackstead.yaml"),
        serde_yaml::to_string(&config).test()?,
    )
    .test()?;

    stackstead(&project.repo)
        .args(["compose", "plan"])
        .assert()
        .failure();
    let explicit = stackstead(&project.repo)
        .args([
            "compose",
            "plan",
            "--compose-file",
            "infra/docker/compose.yml",
            "--json",
        ])
        .assert()
        .success();
    let explicit: Value = serde_json::from_slice(&explicit.get_output().stdout).test()?;
    assert_eq!(explicit["file"], "infra/docker/compose.yml");

    stackstead(&project.repo)
        .args([
            "compose",
            "apply",
            "--compose-file",
            "infra/docker/compose.yml",
            "--yes",
        ])
        .assert()
        .success();
    assert!(
        fs::read_to_string(&nested)
            .test_context("read rewritten nested Compose file")?
            .contains("127.0.0.1:${WEB_PORT}:80")
    );
    assert_eq!(fs::read(&second).test()?, second_before);
    Ok(())
}

#[test]
fn multi_file_config_keeps_the_conventional_root_plan_fallback() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    fs::write(
        project.repo.join("compose.override.yml"),
        "services:\n  web:\n    environment:\n      TRIAL: yes\n",
    )
    .test()?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["runtime"]["files"]
        .as_sequence_mut()
        .test()?
        .push("compose.override.yml".into());
    fs::write(
        project.repo.join("stackstead.yaml"),
        serde_yaml::to_string(&config).test()?,
    )
    .test()?;

    let plan = stackstead(&project.repo)
        .args(["compose", "plan", "--json"])
        .assert()
        .success();
    let plan: Value = serde_json::from_slice(&plan.get_output().stdout).test()?;
    assert_eq!(plan["file"], "docker-compose.yml");
    Ok(())
}
