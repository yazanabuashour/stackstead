use std::{
    collections::BTreeMap,
    path::Path,
    process::{Command, ExitStatus, Output, Stdio},
    time::{Duration, Instant},
};

mod runner;
pub use runner::{run, run_sanitized, run_sanitized_until};

#[cfg(unix)]
mod process;
#[cfg(target_os = "macos")]
pub use process::leader_is_alone;
#[cfg(unix)]
pub use process::{observe as observe_child_exit, require_waitable_children};

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod captured;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use captured::{output as captured_until, status as status_until};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn captured_until(_command: &mut Command, _deadline: Instant) -> anyhow::Result<Output> {
    anyhow::bail!("bounded captured commands require Linux or macOS")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn status_until(_command: &mut Command, _deadline: Instant) -> anyhow::Result<Option<ExitStatus>> {
    anyhow::bail!("bounded command status requires Linux or macOS")
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
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow::anyhow!("command timeout exceeds the supported Instant range"))?;
    status_until(&mut configured, deadline)
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

mod redaction;
pub use redaction::{redact, redact_with_env};

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
mod tests;

#[cfg(windows)]
fn empty_success() -> Output {
    use std::os::windows::process::ExitStatusExt;
    Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: vec![],
        stderr: vec![],
    }
}
