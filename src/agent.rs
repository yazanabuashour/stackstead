use std::{
    ffi::{OsStr, OsString},
    path::Path,
    process::{Command, ExitStatus},
};

use crate::{lifecycle, lock::LockGuard, manifest::StacksteadManifest};

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

pub(crate) fn run_after_up(
    cwd: &Path,
    name: &str,
    program: &OsStr,
    args: &[OsString],
    mutation_lock: LockGuard,
    run_lease: LockGuard,
) -> anyhow::Result<ExitStatus> {
    run_with_locks(cwd, name, program, args, Some((mutation_lock, run_lease)))
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
    #[cfg(unix)]
    let status = {
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
            &resolved,
            executable.as_os_str(),
            &supervisor_args,
            &generated,
        );
        supervisor.process_group(0);
        let mut child = supervisor.spawn().map_err(|error| {
            anyhow::anyhow!(
                "could not start command in stackstead {}: {error}",
                resolved.stackstead_id
            )
        })?;
        drop(supervisor_control);
        run_lease.close_after_handoff();
        let status = child.wait()?;
        drop(control);
        status
    };
    #[cfg(not(unix))]
    let status = {
        let mut command = command(&resolved, program, args, &generated);
        run_lease.inherit_on_exec()?;
        let mut child = command.spawn().map_err(|error| {
            anyhow::anyhow!(
                "could not start command in stackstead {}: {error}",
                resolved.stackstead_id
            )
        })?;
        child.wait()?
    };
    Ok(status)
}

/// Convert a child status to the process code the Stackstead CLI should return.
pub fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        128 + status.signal().unwrap_or(1)
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
) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(&manifest.worktree)
        // Apply trusted identity after generated values, so inherited values
        // cannot redirect the child.
        .envs(manifest.trusted_environment(environment));
    command
}

#[cfg(test)]
mod tests {
    use crate::test_support::TestResultExt as _;
    use std::{collections::BTreeMap, fs, process::ExitStatus};

    use chrono::Utc;

    use super::*;
    use crate::manifest::{ManifestStatus, SourceOwnership, StacksteadManifest};
    use crate::{config::StacksteadConfig, state::ProjectPaths};

    fn manifest(root: &Path) -> anyhow::Result<StacksteadManifest> {
        let short_id = "a17ca17ca17ca17ca17ca17ca17ca17c";
        let stackstead_id = format!("feature-a-{short_id}");
        let stackstead_root = root.join("demo").join(&stackstead_id);
        let worktree = stackstead_root.join("source");
        let state_dir = stackstead_root.join("state");
        fs::create_dir_all(worktree.join(".stackstead")).test()?;
        fs::create_dir_all(&state_dir).test()?;
        Ok(StacksteadManifest {
            kind: "StacksteadManifest".into(),
            version: crate::manifest::MANIFEST_VERSION.into(),
            stackstead_id: stackstead_id.clone(),
            slug: "feature-a".into(),
            short_id: short_id.into(),
            runtime_token: "0123456789abcdef0123456789abcdef".into(),
            project: "demo".into(),
            branch: "feature-a".into(),
            base: "main".into(),
            source_ownership: SourceOwnership::Stackstead,
            repo_root: root.join("repo"),
            project_state_root: root.to_path_buf(),
            stackstead_root,
            worktree: worktree.clone(),
            state_dir: state_dir.clone(),
            port_lease_state_dir: None,
            compose_project: format!("demo-{stackstead_id}"),
            compose_files: vec![],
            ports: BTreeMap::new(),
            container_ports: BTreeMap::new(),
            urls: BTreeMap::new(),
            env_file: worktree.join(".stackstead/.env"),
            agent_context: worktree.join(".stackstead/AGENT_CONTEXT.md"),
            pointer_file: worktree.join(".stackstead/stackstead.json"),
            event_log: state_dir.join("events.jsonl"),
            env_keys: vec![],
            status: ManifestStatus::default(),
            database: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    #[cfg(unix)]
    #[test]
    fn command_preserves_arguments_cwd_environment_and_exit_status() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().test()?;
        let root = directory.path().canonicalize().test()?;
        let manifest = manifest(&root)?;
        let script = root.join("probe");
        let script_body = format!(
            r#"#!/bin/sh
test "$API_TOKEN" = "private" || exit 90
test "$STACKSTEAD_ID" = "{}" || exit 91
test "$COMPOSE_PROJECT_NAME" = "{}" || exit 92
printf '%s\n' "$PWD" "$1" "$2" "$STACKSTEAD_MANIFEST" "$STACKSTEAD_CONTEXT"
exit "$3"
"#,
            manifest.stackstead_id, manifest.compose_project
        );
        fs::write(&script, script_body).test()?;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).test()?;
        let environment = BTreeMap::from([
            ("API_TOKEN".into(), "private".into()),
            ("STACKSTEAD_ID".into(), "spoofed".into()),
            ("COMPOSE_PROJECT_NAME".into(), "shared".into()),
        ]);
        let args = ["one argument".into(), "; touch nowhere".into(), "23".into()];

        let output = command(&manifest, script.as_os_str(), &args, &environment)
            .output()
            .test()?;

        assert_eq!(exit_code(output.status), 23);
        assert_eq!(
            String::from_utf8(output.stdout).test()?,
            format!(
                "{}\none argument\n; touch nowhere\n{}\n{}\n",
                manifest.worktree.display(),
                manifest.manifest_path().display(),
                manifest.agent_context.display()
            )
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn signal_status_uses_conventional_shell_exit_code() -> anyhow::Result<()> {
        use std::os::unix::process::ExitStatusExt;

        let status = ExitStatus::from_raw(9);
        assert_eq!(exit_code(status), 137);
        Ok(())
    }

    #[test]
    fn validation_rejects_contract_files_outside_the_worktree() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let mut manifest = manifest(directory.path())?;
        let compose = manifest.worktree.join("docker-compose.yml");
        fs::write(&compose, "services: {}\n").test()?;
        manifest.compose_files = vec![compose];
        fs::write(&manifest.agent_context, "# context\n").test()?;
        let mut config = StacksteadConfig::default();
        config.project.name = "demo".into();
        let runtime = lifecycle::ProjectRuntime {
            config,
            paths: ProjectPaths::new(
                manifest.repo_root.clone(),
                directory.path().to_path_buf(),
                "demo",
            ),
        };
        lifecycle::validate_manifest_binding(&runtime, &manifest).test()?;

        manifest.env_file = directory.path().join("shared.env");
        assert!(lifecycle::validate_manifest_binding(&runtime, &manifest).is_err());
        Ok(())
    }
}
