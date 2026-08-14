use super::*;

#[test]
fn applies_only_unambiguous_fixed_port_edits() -> anyhow::Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().test()?;
    let file = directory.path().join("compose.yaml");
    std::fs::write(
        &file,
        "services:\n  web:\n    ports:\n      - \"127.0.0.1:3000:80/tcp\"\n  postgres:\n    ports:\n      - target: 5432\n        published: \"5432\"\n",
    )
    .test()?;
    #[cfg(unix)]
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).test()?;

    let output = apply(directory.path()).test()?;
    assert_eq!(output.changed_lines, 2, "test contract values differ");
    let updated = std::fs::read_to_string(file).test()?;
    assert!(
        updated.contains("\"127.0.0.1:${WEB_PORT}:80/tcp\""),
        "test contract condition failed"
    );
    assert!(
        updated.contains("published: \"${POSTGRES_PORT}\""),
        "test contract condition failed"
    );
    assert!(
        updated.contains("host_ip: \"127.0.0.1\""),
        "test contract condition failed"
    );
    assert!(
        plan(directory.path()).test()?.warnings.is_empty(),
        "test contract condition failed"
    );
    let document: serde_yaml::Value = serde_yaml::from_str(&updated).test()?;
    let postgres = yaml_field(&document, "services")
        .and_then(serde_yaml::Value::as_mapping)
        .and_then(|services| services.get(serde_yaml::Value::String("postgres".into())))
        .and_then(|service| yaml_field(service, "ports"))
        .and_then(serde_yaml::Value::as_sequence)
        .and_then(|ports| ports.first())
        .and_then(serde_yaml::Value::as_mapping)
        .test()?;
    assert_eq!(
        postgres
            .get(serde_yaml::Value::String("host_ip".into()))
            .and_then(serde_yaml::Value::as_str),
        Some("127.0.0.1"),
        "test contract values differ"
    );
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(directory.path().join("compose.yaml"))
            .test()?
            .permissions()
            .mode()
            & 0o777,
        0o640,
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn apply_rejects_an_explicit_all_interface_binding() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let file = directory.path().join("compose.yaml");
    std::fs::write(
        &file,
        "services:\n  web:\n    ports:\n      - \"0.0.0.0:3000:80\"\n",
    )
    .test()?;
    (plan(directory.path())).test_err()?;
    (apply(directory.path())).test_err()?;
    assert!(
        std::fs::read_to_string(file)
            .test()?
            .contains("0.0.0.0:3000:80"),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn duplicate_fixed_host_ports_fail_before_the_file_is_written() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let file = directory.path().join("compose.yaml");
    let original = "services:\n  web:\n    ports:\n      - \"3000:80\"\n  api:\n    ports:\n      - \"3000:8080\"\n";
    std::fs::write(&file, original).test()?;
    let error = apply(directory.path()).test_err()?.to_string();
    assert!(
        error.contains("cannot safely rewrite host port 3000"),
        "test contract condition failed"
    );
    assert_eq!(
        std::fs::read_to_string(file).test()?,
        original,
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn inline_fixed_mapping_is_never_rewritten() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let file = directory.path().join("compose.yaml");
    let original = "services:\n  web:\n    ports: [\"3000:80\"]\n";
    std::fs::write(&file, original).test()?;
    let error = apply(directory.path()).test_err()?.to_string();
    assert!(
        error.contains("one port mapping per YAML line"),
        "test contract condition failed"
    );
    assert_eq!(
        std::fs::read_to_string(file).test()?,
        original,
        "test contract values differ"
    );
    Ok(())
}
