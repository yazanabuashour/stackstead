use super::*;
use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};

#[cfg(unix)]
#[test]
fn command_failures_redact_secret_assignments() -> anyhow::Result<()> {
    let args = [
        "-c".into(),
        "printf '%s\\n' AUTH_TOKEN=stderr-secret >&2; exit 7".into(),
    ];
    let error = run("sh", &args, Path::new("/"), &BTreeMap::new())
        .test_err()?
        .to_string();

    assert!(
        error.contains("command failed: sh"),
        "test contract condition failed"
    );
    assert!(
        error.contains("AUTH_TOKEN=[REDACTED]"),
        "test contract condition failed"
    );
    assert!(
        !error.contains("stderr-secret"),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn redacts_quoted_and_multiline_assignments_without_reformatting_diagnostics() -> anyhow::Result<()>
{
    let input = "before  AUTH_TOKEN=\"alpha beta\nsecond line\"  after\n'API_KEY=quoted value'\nAPI_TOKEN+=appended\nAPI_TOKEN[0]=array-value\nPUBLIC_NAME=visible\n";

    assert_eq!(
        redact(input),
        "before  AUTH_TOKEN=[REDACTED]  after\n'API_KEY=[REDACTED]'\nAPI_TOKEN+=[REDACTED]\nAPI_TOKEN[0]=[REDACTED]\nPUBLIC_NAME=visible\n",
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn redacts_supported_sensitive_headers_case_insensitively() -> anyhow::Result<()> {
    for header in [
        "Authorization",
        "proxy-AUTHORIZATION",
        "Cookie",
        "SET-cookie",
        "X-Api-Key",
    ] {
        let diagnostic = format!("prefix {header} \t: Bearer private-value\nnext");
        let redacted = redact(&diagnostic);
        assert_eq!(
            redacted,
            format!("prefix {header} \t: [REDACTED]\nnext"),
            "test contract values differ"
        );
        assert!(
            !redacted.contains("private-value"),
            "test contract condition failed"
        );
    }
    Ok(())
}

#[test]
fn redacts_credential_url_userinfo_but_preserves_the_endpoint() -> anyhow::Result<()> {
    assert_eq!(
        redact("fatal: https://alice:password@example.invalid/repo?retry=1"),
        "fatal: https://[REDACTED]@example.invalid/repo?retry=1",
        "test contract values differ"
    );
    assert_eq!(
        redact("fetch https://access-token@example.invalid/repo"),
        "fetch https://[REDACTED]@example.invalid/repo",
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn environment_aware_redaction_masks_only_nonempty_known_secret_values() -> anyhow::Result<()> {
    let env = BTreeMap::from([
        ("API_TOKEN".into(), "secret".into()),
        ("AUTH_PASSWORD".into(), "secret-suffix".into()),
        ("EMPTY_SECRET".into(), String::new()),
        ("WEB_PORT".into(), "39000".into()),
    ]);

    assert_eq!(
        redact_with_env("long=secret-suffix short=secret port=39000", &env),
        "long=[REDACTED] short=[REDACTED] port=39000",
        "test contract values differ"
    );
    assert_eq!(
        redact_with_env("ordinary text", &env),
        "ordinary text",
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn harmless_diagnostics_remain_byte_for_byte_intact() -> anyhow::Result<()> {
    let input =
        "ready  PUBLIC_URL=https://example.invalid/path\r\nX-Request-Id: abc\nCookieJar: enabled\n";
    assert_eq!(redact(input), input, "test contract values differ");
    Ok(())
}

#[cfg(unix)]
#[test]
fn command_failures_use_header_and_environment_aware_redaction() -> anyhow::Result<()> {
    let args = [
        "-c".into(),
        "printf '%s\\n' 'Authorization: Bearer header-secret' 'known-value' >&2; exit 7".into(),
    ];
    let env = BTreeMap::from([("API_TOKEN".into(), "known-value".into())]);
    let error = run("sh", &args, Path::new("/"), &env)
        .test_err()?
        .to_string();

    assert!(
        error.contains("Authorization: [REDACTED]"),
        "test contract condition failed"
    );
    assert!(
        error.contains(">&2; exit"),
        "test contract condition failed"
    );
    assert!(
        !error.contains("header-secret"),
        "test contract condition failed"
    );
    assert!(
        !error.contains("known-value"),
        "test contract condition failed"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn configured_status_kills_a_command_at_its_deadline() -> anyhow::Result<()> {
    let status = configured_status_with_timeout(
        "sh -c 'while :; do :; done'",
        false,
        Path::new("/"),
        &BTreeMap::new(),
        Duration::from_millis(30),
    )
    .test()?;
    assert!(status.is_none(), "test contract condition failed");
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn configured_timeout_kills_descendants_in_the_process_group() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let pid_file = directory.path().join("descendant.pid");
    let command = format!("sh -c 'sleep 30 & echo $! > {} ; wait'", pid_file.display());
    assert!(
        configured_status_with_timeout(
            &command,
            false,
            Path::new("/"),
            &BTreeMap::new(),
            Duration::from_millis(100),
        )
        .test()?
        .is_none(),
        "test contract condition failed"
    );
    let pid = std::fs::read_to_string(pid_file)
        .test()?
        .trim()
        .parse::<i32>()
        .test()?;
    for _ in 0..50 {
        if rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).test()?).is_err()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    anyhow::bail!("timed command descendant {pid} survived process-group termination")
}

#[cfg(target_os = "linux")]
#[test]
fn configured_success_also_kills_background_descendants() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let pid_file = directory.path().join("descendant.pid");
    let command = format!("sh -c 'sleep 30 & echo $! > {}'", pid_file.display());
    assert!(
        configured_status_with_timeout(
            &command,
            false,
            Path::new("/"),
            &BTreeMap::new(),
            Duration::from_secs(1),
        )
        .test()?
        .test()?
        .success(),
        "test contract condition failed"
    );
    let pid = std::fs::read_to_string(pid_file)
        .test()?
        .trim()
        .parse::<i32>()
        .test()?;
    for _ in 0..50 {
        if rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).test()?).is_err()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    anyhow::bail!("background descendant {pid} survived successful configured command")
}
