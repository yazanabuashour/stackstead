use super::*;

#[test]
fn compose_arguments_use_manifest_contract() -> anyhow::Result<()> {
    let manifest = manifest()?;
    let args = base_args(&manifest);
    assert_eq!(args[0], "compose");
    assert!(args.contains(&"demo-a-b123".to_string()));
    assert!(args.contains(&"/state/demo/a-b123/source/compose.yml".to_string()));
    assert!(
        args.contains(&"/state/demo/a-b123/source/.stackstead/compose-ownership.yaml".to_string())
    );
    Ok(())
}

#[test]
fn observation_and_normalization_withhold_invalid_generated_environment() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let mut manifest = manifest()?;
    manifest.env_file = directory.path().join("generated.env");
    std::fs::write(&manifest.env_file, "private-malformed-file-value").test()?;
    let required = BTreeMap::from([("web".into(), crate::readiness::Role::LongRunning)]);
    for error in [
        service_observations(&manifest, None).test_err()?,
        resolve_requirements(&manifest, &required, None, None).test_err()?,
    ] {
        assert!(!format!("{error:#}").contains("private-malformed-file-value"));
        assert!(
            error
                .to_string()
                .contains("cannot validate generated Compose environment")
        );
    }
    Ok(())
}

#[test]
fn selected_service_running_check_is_manifest_scoped() -> anyhow::Result<()> {
    let manifest = manifest()?;
    let args = service_running_args(&manifest, "frontend");
    assert_eq!(
        &args[args.len() - 5..],
        ["ps", "--status", "running", "--quiet", "frontend"]
    );
    assert!(args.windows(2).any(|args| args == ["-p", "demo-a-b123"]));
    assert!(
        args.windows(2)
            .any(|args| { args == ["-f", "/state/demo/a-b123/source/compose.yml"] })
    );
    assert!(running_service_output(b"frontend-container\n"));
    assert!(!running_service_output(b" \n\t"));
    Ok(())
}

#[test]
fn parses_compose_port_endpoints() -> anyhow::Result<()> {
    assert!(endpoint_matches("127.0.0.1:39000", "127.0.0.1", 39000));
    assert!(!endpoint_matches("0.0.0.0:39000", "127.0.0.1", 39000));
    assert!(!endpoint_matches("127.0.0.2:39000", "127.0.0.1", 39000));
    assert!(!endpoint_matches("[::]:39000", "127.0.0.1", 39000));
    assert!(endpoint_matches("127.0.0.1:39000", "localhost", 39000));
    assert!(endpoint_matches("[::1]:39000", "localhost", 39000));
    assert!(!endpoint_matches("127.0.0.2:39000", "localhost", 39000));
    assert!(!endpoint_matches("127.0.0.1:39001", "localhost", 39000));
    Ok(())
}
