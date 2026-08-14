use super::*;

#[test]
fn finds_common_fixed_port_forms_but_not_variables() -> anyhow::Result<()> {
    let ports = detect_fixed_host_ports(
        r#"
          - "3000:3000"
          - '127.0.0.1:4000:4000'
          - 5000:5000
          - "${WEB_PORT}:3000"
          - "127.0.0.1:${API_PORT}:4000"
          - target: 6000
            published: 6000
        "#,
    );
    assert_eq!(
        ports.iter().map(|port| port.host_port).collect::<Vec<_>>(),
        [3000, 4000, 5000, 6000],
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn reports_ports_exposed_on_all_host_interfaces() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let file = directory.path().join("compose.yaml");
    std::fs::write(
        &file,
        "services:\n  web:\n    ports: [\"${WEB_PORT}:80\", \"127.0.0.1:${ADMIN_PORT}:81\"]\n  db:\n    ports:\n      - target: 5432\n        published: ${DB_PORT}\n        host_ip: 0.0.0.0\n",
    )
    .test()?;
    assert_eq!(
        all_interface_ports_in_file(&file).test()?,
        [("web".into(), 80), ("db".into(), 5432)],
        "test contract values differ"
    );
    Ok(())
}
