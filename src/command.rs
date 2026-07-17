use std::{
    collections::BTreeMap,
    path::Path,
    process::{Command, ExitStatus, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::error::StacksteadError;

pub fn run(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
) -> anyhow::Result<Output> {
    run_sanitized(program, args, cwd, env, std::iter::empty::<&str>())
}

pub fn run_sanitized<'a>(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    removed: impl IntoIterator<Item = &'a str>,
) -> anyhow::Result<Output> {
    tracing::debug!(program, arg_count = args.len(), cwd = %cwd.display(), "running external command");
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    let mut redaction_env = std::env::vars().collect::<BTreeMap<_, _>>();
    for key in removed {
        command.env_remove(key);
        redaction_env.remove(key);
    }
    redaction_env.extend(env.clone());
    let output = command
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| anyhow::anyhow!("could not run {program}: {error}"))?;
    if !output.status.success() {
        return Err(StacksteadError::CommandFailed {
            command: redact_with_env(&display_command(program, args), &redaction_env),
            stderr: redact_with_env(&String::from_utf8_lossy(&output.stderr), &redaction_env),
        }
        .into());
    }
    Ok(output)
}

pub fn status_sanitized<'a>(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    removed: impl IntoIterator<Item = &'a str>,
) -> anyhow::Result<ExitStatus> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    for key in removed {
        command.env_remove(key);
    }
    command
        .envs(env)
        .status()
        .map_err(|error| anyhow::anyhow!("could not run {program}: {error}"))
}

pub fn run_configured(
    command: &str,
    shell: bool,
    cwd: &Path,
    env: &BTreeMap<String, String>,
) -> anyhow::Result<Output> {
    let Some((program, args)) = configured_parts(command, shell)? else {
        return Ok(empty_success());
    };
    run(&program, &args, cwd, env)
}

pub fn configured_status_with_timeout(
    command: &str,
    shell: bool,
    cwd: &Path,
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> anyhow::Result<Option<ExitStatus>> {
    let Some((program, args)) = configured_parts(command, shell)? else {
        return Ok(Some(empty_success().status));
    };
    let mut configured = Command::new(&program);
    configured
        .args(&args)
        .current_dir(cwd)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        configured.process_group(0);
    }
    let mut child = configured
        .spawn()
        .map_err(|error| anyhow::anyhow!("could not run {program}: {error}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            terminate_descendants_after_exit(&mut child)?;
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            terminate_process_tree(&mut child)?;
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(25).min(timeout));
    }
}

#[cfg(unix)]
pub(crate) fn terminate_descendants_after_exit(
    child: &mut std::process::Child,
) -> std::io::Result<()> {
    kill_process_group(child)
}

#[cfg(windows)]
pub(crate) fn terminate_descendants_after_exit(
    child: &mut std::process::Child,
) -> std::io::Result<()> {
    let _ = Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Ok(())
}

#[cfg(unix)]
fn terminate_process_tree(child: &mut std::process::Child) -> std::io::Result<()> {
    kill_process_group(child)?;
    child.wait().map(|_| ())
}

#[cfg(unix)]
fn kill_process_group(child: &std::process::Child) -> std::io::Result<()> {
    // The child was spawned as its own process-group leader above, so
    // signaling its group targets only this command and its descendants.
    match rustix::process::kill_process_group(
        rustix::process::Pid::from_child(child),
        rustix::process::Signal::KILL,
    ) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
fn terminate_process_tree(child: &mut std::process::Child) -> std::io::Result<()> {
    terminate_windows_process_tree(child)
}

#[cfg(windows)]
fn terminate_windows_process_tree(child: &mut std::process::Child) -> std::io::Result<()> {
    let killed = Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !killed {
        child.kill()?;
    }
    child.wait().map(|_| ())
}

fn configured_parts(command: &str, shell: bool) -> anyhow::Result<Option<(String, Vec<String>)>> {
    if command.trim().is_empty() {
        return Ok(None);
    }
    if shell {
        #[cfg(windows)]
        let parts = ("cmd".to_string(), vec!["/C".into(), command.into()]);
        #[cfg(not(windows))]
        let parts = ("sh".to_string(), vec!["-c".into(), command.into()]);
        return Ok(Some(parts));
    }
    let words = shell_words::split(command)
        .map_err(|error| anyhow::anyhow!("cannot parse configured command: {error}"))?;
    let (program, args) = words
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("configured command is empty"))?;
    Ok(Some((program.clone(), args.to_vec())))
}

fn display_command(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(|part| {
            if part.contains(char::is_whitespace) {
                format!("{part:?}")
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn redact(value: &str) -> String {
    redact_with_env(value, &BTreeMap::new())
}

pub fn redact_with_env(value: &str, env: &BTreeMap<String, String>) -> String {
    let mut ranges = sensitive_assignment_ranges(value);
    ranges.extend(sensitive_header_ranges(value));
    ranges.extend(credential_url_ranges(value));
    for secret in known_secret_values(env) {
        ranges.extend(
            value
                .match_indices(secret)
                .map(|(start, matched)| (start, start + matched.len())),
        );
    }
    replace_ranges(value, ranges)
}

fn sensitive_assignment_ranges(value: &str) -> Vec<(usize, usize)> {
    let bytes = value.as_bytes();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !is_name_start(bytes[index]) || index > 0 && is_name_character(bytes[index - 1]) {
            index += 1;
            continue;
        }
        let name_start = index;
        index += 1;
        while index < bytes.len() && is_name_character(bytes[index]) {
            index += 1;
        }
        let name_end = index;
        if index < bytes.len() && bytes[index] == b'[' {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if index >= bytes.len() || bytes[index] != b']' {
                continue;
            }
            index += 1;
        }
        if index < bytes.len() && bytes[index] == b'+' {
            index += 1;
        }
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            continue;
        }
        index += 1;
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
            index += 1;
        }
        if !crate::envfile::is_secret_name(&value[name_start..name_end]) {
            continue;
        }

        let start = index;
        let enclosing_quote = name_start
            .checked_sub(1)
            .and_then(|before| matches!(bytes[before], b'\'' | b'"').then_some(bytes[before]));
        let end = if index < bytes.len() && matches!(bytes[index], b'\'' | b'"') {
            quoted_end(bytes, index + 1, bytes[index], true)
        } else if let Some(quote) = enclosing_quote {
            quoted_end(bytes, index, quote, false)
        } else {
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            index
        };
        ranges.push((start, end));
        index = end.max(index);
    }
    ranges
}

fn sensitive_header_ranges(value: &str) -> Vec<(usize, usize)> {
    const HEADERS: [&str; 5] = [
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "x-api-key",
    ];
    let bytes = value.as_bytes();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_alphabetic()
            || index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'-')
        {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'-' | b'_'))
        {
            index += 1;
        }
        let mut separator = index;
        while separator < bytes.len() && matches!(bytes[separator], b' ' | b'\t') {
            separator += 1;
        }
        if separator >= bytes.len()
            || bytes[separator] != b':'
            || !HEADERS
                .iter()
                .any(|header| value[start..index].eq_ignore_ascii_case(header))
        {
            continue;
        }
        index = separator + 1;
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
            index += 1;
        }
        let value_start = index;
        let enclosing_quote = start
            .checked_sub(1)
            .and_then(|before| matches!(bytes[before], b'\'' | b'"').then_some(bytes[before]));
        let value_end = if let Some(quote) = enclosing_quote {
            quoted_end(bytes, index, quote, false)
        } else {
            while index < bytes.len() && !matches!(bytes[index], b'\r' | b'\n') {
                index += 1;
            }
            index
        };
        ranges.push((value_start, value_end));
        index = value_end.max(index);
    }
    ranges
}

fn credential_url_ranges(value: &str) -> Vec<(usize, usize)> {
    let bytes = value.as_bytes();
    let mut ranges = Vec::new();
    let mut index = 0;
    while let Some(relative) = value[index..].find("://") {
        let separator = index + relative;
        let mut scheme_start = separator;
        while scheme_start > 0 && is_scheme_character(bytes[scheme_start - 1]) {
            scheme_start -= 1;
        }
        let valid_scheme = scheme_start < separator
            && bytes[scheme_start].is_ascii_alphabetic()
            && (scheme_start == 0 || !is_scheme_character(bytes[scheme_start - 1]));
        let authority_start = separator + 3;
        let mut authority_end = authority_start;
        while authority_end < bytes.len()
            && !bytes[authority_end].is_ascii_whitespace()
            && !matches!(
                bytes[authority_end],
                b'/' | b'?' | b'#' | b'\'' | b'"' | b'<' | b'>' | b')' | b']' | b'}'
            )
        {
            authority_end += 1;
        }
        if valid_scheme && let Some(at) = value[authority_start..authority_end].rfind('@') {
            let userinfo_end = authority_start + at;
            if userinfo_end > authority_start {
                ranges.push((authority_start, userinfo_end));
            }
        }
        index = authority_end.max(separator + 3);
    }
    ranges
}

fn known_secret_values(env: &BTreeMap<String, String>) -> Vec<&str> {
    let mut secrets = env
        .iter()
        .filter(|(name, value)| !value.is_empty() && crate::envfile::should_redact(name, value))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    secrets
        .sort_unstable_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    secrets.dedup();
    secrets
}

fn replace_ranges(value: &str, mut ranges: Vec<(usize, usize)>) -> String {
    ranges.sort_unstable_by_key(|&(start, end)| (start, std::cmp::Reverse(end)));
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some((_, previous_end)) = merged.last_mut()
            && start <= *previous_end
        {
            *previous_end = (*previous_end).max(end);
        } else {
            merged.push((start, end));
        }
    }
    let mut redacted = String::with_capacity(value.len());
    let mut previous_end = 0;
    for (start, end) in merged {
        redacted.push_str(&value[previous_end..start]);
        redacted.push_str("[REDACTED]");
        previous_end = end;
    }
    redacted.push_str(&value[previous_end..]);
    redacted
}

fn quoted_end(bytes: &[u8], mut index: usize, quote: u8, include_quote: bool) -> usize {
    let mut escaped = false;
    while index < bytes.len() {
        if bytes[index] == quote && !escaped {
            return index + usize::from(include_quote);
        }
        escaped = bytes[index] == b'\\' && !escaped;
        if bytes[index] != b'\\' {
            escaped = false;
        }
        index += 1;
    }
    bytes.len()
}

fn is_name_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_name_character(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn is_scheme_character(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
}

#[cfg(unix)]
fn empty_success() -> Output {
    use std::os::unix::process::ExitStatusExt;
    Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: vec![],
        stderr: vec![],
    }
}

#[cfg(test)]
mod tests {
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

        assert!(error.contains("command failed: sh"));
        assert!(error.contains("AUTH_TOKEN=[REDACTED]"));
        assert!(!error.contains("stderr-secret"));
        Ok(())
    }

    #[test]
    fn redacts_quoted_and_multiline_assignments_without_reformatting_diagnostics()
    -> anyhow::Result<()> {
        let input = "before  AUTH_TOKEN=\"alpha beta\nsecond line\"  after\n'API_KEY=quoted value'\nAPI_TOKEN+=appended\nAPI_TOKEN[0]=array-value\nPUBLIC_NAME=visible\n";

        assert_eq!(
            redact(input),
            "before  AUTH_TOKEN=[REDACTED]  after\n'API_KEY=[REDACTED]'\nAPI_TOKEN+=[REDACTED]\nAPI_TOKEN[0]=[REDACTED]\nPUBLIC_NAME=visible\n"
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
            assert_eq!(redacted, format!("prefix {header} \t: [REDACTED]\nnext"));
            assert!(!redacted.contains("private-value"));
        }
        Ok(())
    }

    #[test]
    fn redacts_credential_url_userinfo_but_preserves_the_endpoint() -> anyhow::Result<()> {
        assert_eq!(
            redact("fatal: https://alice:password@example.invalid/repo?retry=1"),
            "fatal: https://[REDACTED]@example.invalid/repo?retry=1"
        );
        assert_eq!(
            redact("fetch https://access-token@example.invalid/repo"),
            "fetch https://[REDACTED]@example.invalid/repo"
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
            "long=[REDACTED] short=[REDACTED] port=39000"
        );
        assert_eq!(redact_with_env("ordinary text", &env), "ordinary text");
        Ok(())
    }

    #[test]
    fn harmless_diagnostics_remain_byte_for_byte_intact() -> anyhow::Result<()> {
        let input = "ready  PUBLIC_URL=https://example.invalid/path\r\nX-Request-Id: abc\nCookieJar: enabled\n";
        assert_eq!(redact(input), input);
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

        assert!(error.contains("Authorization: [REDACTED]"));
        assert!(error.contains(">&2; exit"));
        assert!(!error.contains("header-secret"));
        assert!(!error.contains("known-value"));
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
        assert!(status.is_none());
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
            .is_none()
        );
        let pid = std::fs::read_to_string(pid_file)
            .test()?
            .trim()
            .parse::<i32>()
            .test()?;
        for _ in 0..50 {
            if rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).test()?)
                .is_err()
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
            .success()
        );
        let pid = std::fs::read_to_string(pid_file)
            .test()?
            .trim()
            .parse::<i32>()
            .test()?;
        for _ in 0..50 {
            if rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).test()?)
                .is_err()
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(10));
        }
        anyhow::bail!("background descendant {pid} survived successful configured command")
    }
}

#[cfg(windows)]
fn empty_success() -> Output {
    use std::os::windows::process::ExitStatusExt;
    Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: vec![],
        stderr: vec![],
    }
}
