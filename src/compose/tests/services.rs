use super::*;

#[test]
fn compose_arguments_use_manifest_contract() -> anyhow::Result<()> {
    let manifest = manifest()?;
    let args = base_args(&manifest);
    assert_eq!(args[0], "compose", "test contract values differ");
    assert!(
        args.contains(&"demo-a-b123".to_string()),
        "test contract condition failed"
    );
    assert!(
        args.contains(&"/state/demo/a-b123/source/compose.yml".to_string()),
        "test contract condition failed"
    );
    assert!(
        args.contains(&"/state/demo/a-b123/source/.stackstead/compose-ownership.yaml".to_string()),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn parses_array_and_line_delimited_service_observations() -> anyhow::Result<()> {
    let array = br#"[
      {"Name":"demo-web-1","Service":"web","State":"running","ExitCode":0},
      {"Name":"demo-init-1","Service":"init","State":"exited","ExitCode":0},
      {"Name":"demo-migrate-1","Service":"migrate","State":"exited","ExitCode":7}
    ]"#;
    let observations = parse_service_observations(array).test()?;
    assert_eq!(
        observations
            .iter()
            .map(|service| (service.service.as_str(), service.status()))
            .collect::<Vec<_>>(),
        [
            ("init", "completed (0)".into()),
            ("migrate", "exited (7)".into()),
            ("web", "running".into()),
        ],
        "test contract values differ"
    );

    let lines = br#"{"Name":"demo-web-1","Service":"web","State":"running","ExitCode":0}
{"Name":"demo-init-1","Service":"init","State":"exited","ExitCode":0}"#;
    assert_eq!(
        parse_service_observations(lines).test()?.len(),
        2,
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn selected_service_running_check_is_manifest_scoped() -> anyhow::Result<()> {
    let manifest = manifest()?;
    let args = service_running_args(&manifest, "frontend");
    assert_eq!(
        &args[args.len() - 5..],
        ["ps", "--status", "running", "--quiet", "frontend"],
        "test contract values differ"
    );
    assert!(
        args.windows(2).any(|args| args == ["-p", "demo-a-b123"]),
        "test contract condition failed"
    );
    assert!(
        args.windows(2)
            .any(|args| { args == ["-f", "/state/demo/a-b123/source/compose.yml"] }),
        "test contract condition failed"
    );
    assert!(
        running_service_output(b"frontend-container\n"),
        "test contract condition failed"
    );
    assert!(
        !running_service_output(b" \n\t"),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn parses_compose_port_endpoints() -> anyhow::Result<()> {
    assert_eq!(
        endpoint_port("0.0.0.0:39000"),
        Some(39000),
        "test contract values differ"
    );
    assert_eq!(
        endpoint_port("[::]:39001"),
        Some(39001),
        "test contract values differ"
    );
    assert_eq!(
        endpoint_port("not-an-endpoint"),
        None,
        "test contract values differ"
    );
    assert!(
        endpoint_matches("127.0.0.1:39000", "127.0.0.1", 39000),
        "test contract condition failed"
    );
    assert!(
        !endpoint_matches("0.0.0.0:39000", "127.0.0.1", 39000),
        "test contract condition failed"
    );
    assert!(
        !endpoint_matches("127.0.0.2:39000", "127.0.0.1", 39000),
        "test contract condition failed"
    );
    assert!(
        !endpoint_matches("[::]:39000", "127.0.0.1", 39000),
        "test contract condition failed"
    );
    assert!(
        endpoint_matches("127.0.0.1:39000", "localhost", 39000),
        "test contract condition failed"
    );
    assert!(
        endpoint_matches("[::1]:39000", "localhost", 39000),
        "test contract condition failed"
    );
    assert!(
        !endpoint_matches("127.0.0.2:39000", "localhost", 39000),
        "test contract condition failed"
    );
    assert!(
        !endpoint_matches("127.0.0.1:39001", "localhost", 39000),
        "test contract condition failed"
    );
    Ok(())
}
