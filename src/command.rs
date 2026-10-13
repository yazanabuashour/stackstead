use std::{
    collections::BTreeMap,
    path::Path,
    process::{Command, ExitStatus},
};
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use std::{process::Output, time::Instant};

mod configured;
pub use configured::ConfiguredCommand;

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

#[cfg(test)]
mod tests;
