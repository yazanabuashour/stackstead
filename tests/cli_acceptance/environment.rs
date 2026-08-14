use super::*;

#[test]
fn env_outputs_redact_credentials_and_generation_is_deterministic() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    project.replace_config(
        "    DATABASE_URL: postgres://app:app@127.0.0.1:{{ ports.postgres }}/app\n",
        "    Z_LAST: \"value#hash\"\n    SERVICE_DSN: postgresql://worker:dnspass@127.0.0.1:{{ ports.postgres }}/app\n    DATABASE_URL: postgres://app:app@127.0.0.1:{{ ports.postgres }}/app\n    A_FIRST: \"hello world\"\n",
    )?;
    let manifest = project.create("feature-a")?;

    let env = fs::read_to_string(&manifest.env_file).test_context("read generated env")?;
    assert!(
        env.contains("A_FIRST=\"hello world\""),
        "test contract condition failed"
    );
    assert!(
        env.contains("Z_LAST=\"value#hash\""),
        "test contract condition failed"
    );
    let keys = env
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            line.split_once('=')
                .test_context("env assignment")
                .map(|assignment| assignment.0)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted, "generated env assignments are not sorted");

    for args in [
        vec!["env", "feature-a"],
        vec!["env", "feature-a", "--json"],
        vec!["env", "feature-a", "--print"],
    ] {
        let assert = stackstead(&project.repo).args(args).assert().success();
        let stdout = output_text(&assert.get_output().stdout)?;
        assert!(
            !stdout.contains("postgres://app:app@"),
            "DATABASE_URL leaked: {stdout}"
        );
        assert!(
            !stdout.contains("worker:dnspass@"),
            "SERVICE_DSN leaked: {stdout}"
        );
        assert!(
            stdout.contains("DATABASE_URL"),
            "test contract condition failed"
        );
        assert!(
            stdout.contains("SERVICE_DSN"),
            "test contract condition failed"
        );
        assert!(
            stdout.contains("[REDACTED]"),
            "test contract condition failed"
        );
    }
    Ok(())
}
