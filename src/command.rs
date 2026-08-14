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
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow::anyhow!("command timeout exceeds the supported Instant range"))?;
    loop {
        if let Some(status) = child.try_wait()? {
            terminate_descendants_after_exit(&child)?;
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
pub fn terminate_descendants_after_exit(child: &std::process::Child) -> std::io::Result<()> {
    kill_process_group(child)
}

#[cfg(windows)]
pub(crate) fn terminate_descendants_after_exit(
    child: &mut std::process::Child,
) -> std::io::Result<()> {
    drop(
        Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    );
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
