use std::{
    ffi::{OsStr, OsString},
    path::Path,
    process::{Command, ExitStatus},
};

use crate::{compose, lifecycle, lock::LockGuard, manifest::StacksteadManifest};

/// Run a command inside a named stackstead with its generated runtime contract.
///
/// The command is executed directly, without a shell, and inherits the caller's
/// terminal streams. Generated environment values and stable Stackstead metadata
/// are added to the inherited process environment without being logged.
pub fn run(
    cwd: &Path,
    name: &str,
    program: &OsStr,
    args: &[OsString],
) -> anyhow::Result<ExitStatus> {
    run_with_locks(cwd, name, program, args, None)
}

pub fn run_after_up(
    cwd: &Path,
    name: &str,
    program: &OsStr,
    args: &[OsString],
    mutation_lock: LockGuard,
    run_lease: LockGuard,
) -> anyhow::Result<ExitStatus> {
    run_with_locks(cwd, name, program, args, Some((mutation_lock, run_lease)))
}

/// Run a command inside one running service of a named stackstead.
pub fn exec(
    cwd: &Path,
    name: &str,
    service: &str,
    program: &OsStr,
    args: &[OsString],
) -> anyhow::Result<ExitStatus> {
    use std::io::IsTerminal as _;

    if program.is_empty() {
        anyhow::bail!("a program is required after `--`");
    }

    let runtime = lifecycle::load_project(cwd)?;
    let mut resolved = runtime.resolve(name)?;
    let mutation_lock =
        LockGuard::acquire_existing(&resolved.state_dir.join("lock"), "stackstead")?;
    let run_lease = LockGuard::acquire_existing_shared(
        &resolved.state_dir.join("run.lock"),
        "stackstead service exec",
    )?;
    resolved = StacksteadManifest::read(&resolved.manifest_path())?;
    validate_contract(&runtime, &resolved)?;
    lifecycle::validate_pointer_binding(&resolved)?;
    lifecycle::verify_port_leases(&resolved)?;
    let (removed, environment) = compose::docker_environment(&resolved).map_err(|error| {
        anyhow::anyhow!(
            "cannot read generated environment for {} at {}: {error}",
            resolved.stackstead_id,
            resolved.env_file.display()
        )
    })?;
    compose::verify_owned_runtime(&resolved)?;
    compose::ensure_service_configured(&resolved, service)?;
    if !compose::service_is_running(&resolved, service)? {
        anyhow::bail!(
            "Compose service `{service}` is not running for {}; run `stackstead inspect {}`",
            resolved.stackstead_id,
            resolved.stackstead_id
        );
    }
    compose::verify_ownership_override(&resolved)?;
    drop(mutation_lock);

    let mut docker_args = compose::base_args(&resolved)
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    docker_args.push("exec".into());
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        docker_args.push("-T".into());
    }
    docker_args.push(service.into());
    docker_args.push(program.to_os_string());
    docker_args.extend(args.iter().cloned());
    foreground_status(
        &resolved,
        OsStr::new("docker"),
        &docker_args,
        &environment,
        &removed,
        run_lease,
        &format!(
            "could not execute command in Compose service `{service}` for {}",
            resolved.stackstead_id
        ),
    )
}

fn foreground_status(
    manifest: &StacksteadManifest,
    program: &OsStr,
    args: &[OsString],
    environment: &std::collections::BTreeMap<String, String>,
    removed: &[String],
    run_lease: LockGuard,
    start_error: &str,
) -> anyhow::Result<ExitStatus> {
    let mut command = command(manifest, program, args, environment, removed);
    run_lease.inherit_on_exec()?;
    let mut child = command
        .spawn()
        .map_err(|error| anyhow::anyhow!("{start_error}: {error}"))?;
    #[cfg(unix)]
    run_lease.close_after_handoff();
    let status = child.wait()?;
    #[cfg(not(unix))]
    drop(run_lease);
    Ok(status)
}

fn run_with_locks(
    cwd: &Path,
    name: &str,
    program: &OsStr,
    args: &[OsString],
    locks: Option<(LockGuard, LockGuard)>,
) -> anyhow::Result<ExitStatus> {
    if program.is_empty() {
        anyhow::bail!("a program is required after `--`");
    }

    let runtime = lifecycle::load_project(cwd)?;
    let mut resolved = runtime.resolve(name)?;
    let (mutation_lock, run_lease) = match locks {
        Some((mutation_lock, run_lease)) => (mutation_lock, run_lease.downgrade_to_shared()?),
        None => (
            LockGuard::acquire_existing(&resolved.state_dir.join("lock"), "stackstead")?,
            LockGuard::acquire_existing_shared(
                &resolved.state_dir.join("run.lock"),
                "stackstead agent run",
            )?,
        ),
    };
    resolved = StacksteadManifest::read(&resolved.manifest_path())?;
    validate_contract(&runtime, &resolved)?;
    lifecycle::verify_port_leases(&resolved)?;
    drop(mutation_lock);
    let generated = resolved.validated_environment().map_err(|error| {
        anyhow::anyhow!(
            "cannot read generated environment for {} at {}: {error}",
            resolved.stackstead_id,
            resolved.env_file.display()
        )
    })?;
    let environment = resolved.trusted_environment(&generated);
    supervised_status(
        &resolved,
        program,
        args,
        &environment,
        &[],
        run_lease,
        &format!(
            "could not start command in stackstead {}",
            resolved.stackstead_id
        ),
    )
}

fn supervised_status(
    manifest: &StacksteadManifest,
    program: &OsStr,
    args: &[OsString],
    environment: &std::collections::BTreeMap<String, String>,
    removed: &[String],
    run_lease: LockGuard,
    start_error: &str,
) -> anyhow::Result<ExitStatus> {
    #[cfg(unix)]
    {
        use std::os::{fd::AsRawFd, unix::process::CommandExt};

        let (control, supervisor_control) = std::os::unix::net::UnixStream::pair()?;
        crate::supervisor::set_cloexec(&supervisor_control, false)?;
        let (lease_fd, lease_dev, lease_ino) = run_lease.inherited_identity()?;
        run_lease.inherit_on_exec()?;
        let executable = std::env::current_exe()?;
        let supervisor_args = [
            crate::supervisor::ARGUMENT.into(),
            supervisor_control.as_raw_fd().to_string().into(),
            lease_fd.to_string().into(),
            lease_dev.to_string().into(),
            lease_ino.to_string().into(),
            "--".into(),
        ]
        .into_iter()
        .chain(std::iter::once(program.to_os_string()))
        .chain(args.iter().cloned())
        .collect::<Vec<_>>();
        let mut supervisor = command(
            manifest,
            executable.as_os_str(),
            &supervisor_args,
            environment,
            removed,
        );
        supervisor.process_group(0);
        let mut child = supervisor
            .spawn()
            .map_err(|error| anyhow::anyhow!("{start_error}: {error}"))?;
        drop(supervisor_control);
        run_lease.close_after_handoff();
        let status = child.wait()?;
        drop(control);
        Ok(status)
    }
    #[cfg(not(unix))]
    {
        let mut command = command(manifest, program, args, environment, removed);
        run_lease.inherit_on_exec()?;
        let mut child = command
            .spawn()
            .map_err(|error| anyhow::anyhow!("{start_error}: {error}"))?;
        Ok(child.wait()?)
    }
}

/// Convert a child status to the process code the Stackstead CLI should return.
pub fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        128_i32.saturating_add(status.signal().unwrap_or(1))
    }

    #[cfg(not(unix))]
    1
}

fn validate_contract(
    runtime: &lifecycle::ProjectRuntime,
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    lifecycle::validate_current_contract(runtime, manifest)?;
    lifecycle::validate_source_binding(manifest)?;
    if !manifest.worktree.is_dir() {
        anyhow::bail!(
            "worktree for {} is missing at {}; run `stackstead doctor`",
            manifest.stackstead_id,
            manifest.worktree.display()
        );
    }
    if !manifest.agent_context.is_file() {
        anyhow::bail!(
            "agent context for {} is missing at {}; run `stackstead repair {}`",
            manifest.stackstead_id,
            manifest.agent_context.display(),
            manifest.stackstead_id
        );
    }
    Ok(())
}

fn command(
    manifest: &StacksteadManifest,
    program: &OsStr,
    args: &[OsString],
    environment: &std::collections::BTreeMap<String, String>,
    removed: &[String],
) -> Command {
    let mut command = Command::new(program);
    command.args(args).current_dir(&manifest.worktree);
    for key in removed {
        command.env_remove(key);
    }
    command.envs(environment);
    command
}

#[cfg(test)]
mod tests;
