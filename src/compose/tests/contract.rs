use super::*;

#[test]
fn resolves_contract_key_to_its_actual_compose_service() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let file = directory.path().join("compose.yaml");
    std::fs::write(
        &file,
        "services:\n  frontend:\n    image: nginx\n    ports: [\"127.0.0.1:${WEB_PORT}:3000\"]\n",
    )
    .test()?;
    let target = resolve_port_target(
        &[file],
        &BTreeMap::from([("dashboard".into(), 3000)]),
        &BTreeMap::from([("WEB_PORT".into(), "{{ ports.dashboard }}".into())]),
        "dashboard",
    )
    .test()?;
    assert_eq!(
        target,
        ComposePortTarget {
            service: "frontend".into(),
            container_port: 3000
        }
    );
    Ok(())
}

#[test]
fn validates_the_exact_structural_port_environment_contract() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let file = directory.path().join("compose.yaml");
    std::fs::write(
        &file,
        "services:\n  web:\n    image: nginx\n    ports: [\"127.0.0.1:${APP_PORT:-3000}:80\"]\n",
    )
    .test()?;
    let containers = BTreeMap::from([("web".into(), 80)]);
    let environment = BTreeMap::from([("APP_PORT".into(), "{{ ports.web }}".into())]);
    validate_port_contract(std::slice::from_ref(&file), &containers, &environment).test()?;

    let wrong_environment = BTreeMap::from([("WORKER_PORT".into(), "{{ ports.web }}".into())]);
    assert!(
        validate_port_contract(std::slice::from_ref(&file), &containers, &wrong_environment)
            .test_err()?
            .to_string()
            .contains("APP_PORT")
    );

    std::fs::write(
        &file,
        "services:\n  web:\n    image: nginx\n    ports: [\"3000:80\"]\n",
    )
    .test()?;
    assert!(
        validate_port_contract(&[file], &containers, &environment)
            .test_err()?
            .to_string()
            .contains("fixed host port")
    );
    Ok(())
}

#[test]
fn validates_port_names_from_generated_contract_across_override_files() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let primary = directory.path().join("compose.yaml");
    let override_file = directory.path().join("compose.override.yaml");
    std::fs::write(
        &primary,
        "services:\n  frontend:\n    ports: [\"127.0.0.1:${WEB_PORT}:80\"]\n",
    )
    .test()?;
    std::fs::write(&override_file, "volumes:\n  cache: {}\n").test()?;
    validate_port_contract(
        &[primary, override_file],
        &BTreeMap::from([("web".into(), 80)]),
        &BTreeMap::from([("WEB_PORT".into(), "{{ ports.web }}".into())]),
    )
    .test()?;
    Ok(())
}

#[test]
fn rejects_direct_and_merge_hidden_compose_includes() -> anyhow::Result<()> {
    let file = Path::new("compose.yaml");
    for contents in [
        "include: compose.shared.yaml\nservices: {}\n",
        "x-root: &root\n  include: compose.shared.yaml\n<<: *root\nservices: {}\n",
    ] {
        let document: serde_yaml::Value = serde_yaml::from_str(contents).test()?;
        let error = port_declarations(&document, file).test_err()?.to_string();
        assert!(error.contains("`include`"), "unexpected error: {error}");
        assert!(error.contains("explicitly"), "unexpected error: {error}");
    }
    Ok(())
}

#[test]
fn rejects_direct_and_merge_hidden_compose_extends() -> anyhow::Result<()> {
    let file = Path::new("compose.yaml");
    for contents in [
        "services:\n  web:\n    extends:\n      file: compose.shared.yaml\n      service: web\n",
        "x-service: &base\n  extends:\n    file: compose.shared.yaml\n    service: web\nservices:\n  web:\n    <<: *base\n",
    ] {
        let document: serde_yaml::Value = serde_yaml::from_str(contents).test()?;
        let error = port_declarations(&document, file).test_err()?.to_string();
        assert!(error.contains("`extends`"), "unexpected error: {error}");
        assert!(error.contains("web"), "unexpected error: {error}");
    }
    Ok(())
}

#[test]
fn explicit_runtime_file_overlays_remain_supported() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let primary = directory.path().join("compose.yaml");
    let overlay = directory.path().join("compose.stackstead.yaml");
    std::fs::write(
        &primary,
        "services:\n  web:\n    image: nginx\n    ports: [\"127.0.0.1:${WEB_PORT}:80\"]\n",
    )
    .test()?;
    std::fs::write(
        &overlay,
        "services:\n  web:\n    environment:\n      STACKSTEAD: \"true\"\n",
    )
    .test()?;

    validate_port_contract(
        &[primary, overlay],
        &BTreeMap::from([("web".into(), 80)]),
        &BTreeMap::from([("WEB_PORT".into(), "{{ ports.web }}".into())]),
    )
    .test()?;
    Ok(())
}
