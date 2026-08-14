use super::*;

#[test]
fn rejects_invalid_url_templates() -> anyhow::Result<()> {
    let mut config = StacksteadConfig::from_yaml(SAMPLE).test()?;
    config.resources.ports.expose.get_mut("web").test()?.url =
        Some("127.0.0.1:{{ ports.web }}".into());
    (config.validate()).test_err()?;

    let remote = SAMPLE.replace(
        "http://127.0.0.1:{{ ports.web }}",
        "https://example.com/{{ ports.web }}",
    );
    (StacksteadConfig::from_yaml(&remote)).test_err()?;
    Ok(())
}

#[test]
fn checks_compose_files_against_repo() -> anyhow::Result<()> {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .test()?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("stackstead-config-{suffix}"));
    fs::create_dir_all(&root).test()?;
    let config = StacksteadConfig::from_yaml(SAMPLE).test()?;
    (config.validate_for_repo(&root)).test_err()?;
    fs::write(root.join("docker-compose.yml"), "services: {}").test()?;
    (config.validate_for_repo(&root)).test()?;
    fs::remove_dir_all(root).test()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn rejects_state_root_that_normalizes_to_filesystem_root() -> anyhow::Result<()> {
    let root = tempfile::tempdir().test()?;
    fs::write(root.path().join("docker-compose.yml"), "services: {}").test()?;
    let mut config = StacksteadConfig::from_yaml(SAMPLE).test()?;
    config.state.root = PathBuf::from("/tmp/..");
    (config.validate_for_repo(root.path())).test_err()?;

    config.state.root = PathBuf::from(".");
    (config.validate_for_repo(root.path())).test_err()?;

    let inside = root.path().join("inside-state");
    let alias = root.path().parent().test()?.join(format!(
        "{}-state-link",
        root.path().file_name().test()?.to_string_lossy()
    ));
    fs::create_dir(&inside).test()?;
    std::os::unix::fs::symlink(&inside, &alias).test()?;
    config.state.root = alias;
    (config.validate_for_repo(root.path())).test_err()?;
    fs::remove_file(&config.state.root).test()?;
    Ok(())
}

#[test]
fn validates_yarn_link_shape() -> anyhow::Result<()> {
    let mut config = StacksteadConfig::from_yaml(SAMPLE).test()?;
    config.dependencies.provider = DependencyProvider::YarnClassic;
    config.dependencies.link = Some(LinkConfig {
        enabled: true,
        ..LinkConfig::default()
    });
    (config.validate()).test_err()?;
    config.dependencies.link.as_mut().test()?.command = "./scripts/link-packages.sh".into();
    (config.validate()).test()?;
    Ok(())
}

#[test]
fn validates_http_and_command_health_checks() -> anyhow::Result<()> {
    let mut config = StacksteadConfig::from_yaml(SAMPLE).test()?;
    config.health.checks = vec![
        HealthCheckConfig {
            name: "web".into(),
            url: Some("{{ urls.web }}/health".into()),
            expect_status: 200,
            command: CommandConfig::default(),
        },
        HealthCheckConfig {
            name: "worker".into(),
            url: None,
            expect_status: 200,
            command: CommandConfig {
                command: "true".into(),
                shell: false,
            },
        },
    ];
    (config.validate()).test()?;

    config.health.checks[0].command.command = "true".into();
    (config.validate()).test_err()?;

    config.health.checks[0].command.command.clear();
    config.health.checks[0].url = Some(" \t".into());
    (config.validate()).test_err()?;

    config.health.checks[0].url = Some("{{ urls.web }}/health".into());
    config.health.timeout_seconds = u64::MAX;
    (config.validate()).test_err()?;
    config.health.timeout_seconds = 60;
    config.health.interval_millis = 60_001;
    (config.validate()).test_err()?;
    Ok(())
}
