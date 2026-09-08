use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::Path,
    process::{Command, Output, Stdio},
    time::Instant,
};

use crate::error::StacksteadError;

use super::{captured_until, display_command, redact_with_env};

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
    run_sanitized_until(program, args, cwd, env, removed, None)
}

/// With a deadline, terminate the command's private process group before reaping,
/// including on success. Use this for read-only clients, not rollback of external work.
/// Without a deadline, preserve ordinary `Command::output` execution.
pub fn run_sanitized_until<'a>(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    removed: impl IntoIterator<Item = &'a str>,
    deadline: Option<Instant>,
) -> anyhow::Result<Output> {
    tracing::debug!(program, arg_count = args.len(), cwd = %cwd.display(), "running external command");
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    let mut inherited = std::env::vars_os().collect::<BTreeMap<_, _>>();
    for key in removed {
        command.env_remove(key);
        inherited.remove(OsStr::new(key));
    }
    inherited.extend(
        env.iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value))),
    );
    let redaction_env = diagnostic_environment(inherited);
    let sanitize = |text: &str| {
        redaction_env.as_ref().map_or_else(
            || "external command diagnostics withheld: non-UTF-8 environment".into(),
            |environment| redact_with_env(text, environment),
        )
    };
    command
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = match deadline {
        Some(deadline) => captured_until(&mut command, deadline),
        None => command.output().map_err(anyhow::Error::from),
    }
    .map_err(|error| anyhow::anyhow!(sanitize(&format!("could not run {program}: {error:#}"))))?;
    if !output.status.success() {
        return Err(StacksteadError::CommandFailed {
            command: sanitize(&display_command(program, args)),
            stderr: sanitize(&String::from_utf8_lossy(&output.stderr)),
        }
        .into());
    }
    Ok(output)
}

fn diagnostic_environment(
    environment: BTreeMap<OsString, OsString>,
) -> Option<BTreeMap<String, String>> {
    // Keep removals and overrides lossless. If any remaining entry cannot be represented,
    // withhold diagnostics rather than matching secrets against a lossy approximation.
    environment
        .into_iter()
        .map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

#[cfg(all(test, unix))]
#[path = "runner_tests.rs"]
mod tests;
