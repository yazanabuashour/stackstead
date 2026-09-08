use std::{os::unix::ffi::OsStringExt as _, time::Duration};

use super::*;
use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn bounded_failures_redact_stderr_and_arguments_without_disclosing_stdout() -> anyhow::Result<()> {
    let env = BTreeMap::from([
        ("API_TOKEN".into(), "known-private-value".into()),
        ("PUBLIC_OUTPUT".into(), "stdout-only-private".into()),
    ]);
    let args = ["-c".into(), "printf '%s' \"$PUBLIC_OUTPUT\"; printf '%s\\n' 'known-private-value' 'Authorization: Bearer header-private' >&2; exit 7".into()];
    let error = run_sanitized_until("sh", &args, Path::new("/"), &env, [], Some(deadline()?))
        .test_err()?
        .to_string();
    assert!(error.contains("command failed"));
    assert!(error.contains("Authorization: [REDACTED]"));
    assert!(!error.contains("known-private-value"));
    assert!(!error.contains("header-private"));
    assert!(!error.contains("stdout-only-private"));
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn deadline_errors_withhold_partial_output_and_secret_arguments() -> anyhow::Result<()> {
    let args = ["-c".into(), "printf output-private; printf stderr-private >&2; while :; do :; done # AUTH_TOKEN=argument-private".into()];
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(100))
        .test()?;
    let error = run_sanitized_until(
        "sh",
        &args,
        Path::new("/"),
        &BTreeMap::new(),
        [],
        Some(deadline),
    )
    .test_err()?
    .to_string();
    assert!(error.contains("deadline expired"));
    for secret in ["output-private", "stderr-private", "argument-private"] {
        assert!(!error.contains(secret));
    }
    Ok(())
}

#[test]
fn non_utf8_environment_is_lossless_for_execution_and_safe_for_diagnostics() -> anyhow::Result<()> {
    let output = Command::new(std::env::current_exe().test()?)
        .args([
            "--exact",
            "command::runner::tests::non_utf8_environment_fixture",
            "--nocapture",
        ])
        .env("STACKSTEAD_NON_UTF8_FIXTURE", "1")
        .env("COMPOSE_PROFILES", OsString::from_vec(vec![0xff]))
        .env(
            "API_TOKEN",
            OsString::from_vec(b"private-\xff-value".to_vec()),
        )
        .output()
        .test()?;
    assert!(
        output.status.success(),
        "non-UTF-8 subprocess fixture failed"
    );
    Ok(())
}

#[test]
fn non_utf8_environment_fixture() -> anyhow::Result<()> {
    if std::env::var_os("STACKSTEAD_NON_UTF8_FIXTURE").is_none() {
        return Ok(());
    }
    let env = BTreeMap::new();
    let args = ["-c".into(), "printf '%s' \"$COMPOSE_PROFILES\"".into()];
    let output = run("sh", &args, Path::new("/"), &env).test()?;
    assert_eq!(output.stdout, [0xff]);
    let args = ["-c".into(), "printf '%s' \"$API_TOKEN\" >&2; exit 7".into()];
    let error = run("sh", &args, Path::new("/"), &env)
        .test_err()?
        .to_string();
    assert!(error.contains("diagnostics withheld: non-UTF-8 environment"));
    assert!(!error.contains("private-"));
    let args = ["-c".into(), "printf 'useful diagnostic' >&2; exit 7".into()];
    let env = BTreeMap::from([("API_TOKEN".into(), "replacement-secret".into())]);
    let error = run_sanitized("sh", &args, Path::new("/"), &env, ["COMPOSE_PROFILES"])
        .test_err()?
        .to_string();
    assert!(error.contains("useful diagnostic"));
    Ok(())
}

#[test]
fn non_utf8_environment_names_cannot_collide_in_redaction_preparation() -> anyhow::Result<()> {
    let environment = BTreeMap::from([
        (OsString::from_vec(b"API_TOKEN_\xff".to_vec()), "one".into()),
        (OsString::from("API_TOKEN_\u{fffd}"), "two".into()),
    ]);
    assert!(diagnostic_environment(environment).is_none());
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn deadline() -> anyhow::Result<Instant> {
    Instant::now().checked_add(Duration::from_secs(10)).test()
}
