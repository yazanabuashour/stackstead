use std::path::Path;

use crate::{
    compose, database, events, git,
    lifecycle::{self},
    lock::LockGuard,
    manifest::{ComponentStatus, StacksteadManifest},
};

pub fn run(cwd: &Path, name: &str) -> anyhow::Result<StacksteadManifest> {
    let runtime = lifecycle::load_project(cwd)?;
    let mut manifest = runtime.paths.resolve(name)?;
    lifecycle::validate_manifest_binding(&runtime, &manifest)?;
    if manifest.pointer_file.exists() {
        lifecycle::validate_pointer_binding(&manifest)?;
    }
    let _lock = LockGuard::acquire_existing(&manifest.state_dir.join("lock"), "stackstead")?;
    let _run_lease = LockGuard::acquire_existing(
        &manifest.state_dir.join("run.lock"),
        "active stackstead agent",
    )?;
    manifest = StacksteadManifest::read(&manifest.manifest_path())?;
    lifecycle::validate_manifest_binding(&runtime, &manifest)?;
    if manifest.pointer_file.exists() {
        lifecycle::validate_pointer_binding(&manifest)?;
    }
    lifecycle::verify_port_leases(&manifest)?;
    lifecycle::validate_current_contract(&runtime, &manifest)?;
    lifecycle::validate_source_binding(&manifest)?;
    if !manifest.worktree.is_dir() {
        anyhow::bail!(
            "worktree {} is missing; repair will not recreate source or overwrite data",
            manifest.worktree.display()
        );
    }
    std::fs::create_dir_all(manifest.state_dir.join("logs"))?;
    git::ensure_stackstead_excluded(&manifest.worktree)?;
    lifecycle::regenerate_contract(&runtime.config, &mut manifest)?;
    let environment = manifest.trusted_environment(&manifest.validated_environment()?);
    lifecycle::install_dependencies(&runtime.config, &manifest, &environment)?;
    lifecycle::validate_current_contract(&runtime, &manifest)?;
    lifecycle::validate_pointer_binding(&manifest)?;
    lifecycle::validate_source_binding(&manifest)?;
    manifest.status.source = ComponentStatus::Ready;
    manifest.status.dependencies = ComponentStatus::Ready;
    let runtime_status = match compose::is_running(&manifest) {
        Ok(true) => ComponentStatus::Running,
        Ok(false) => ComponentStatus::Stopped,
        Err(_) => ComponentStatus::Unknown,
    };
    manifest.status.runtime = runtime_status;
    manifest.status.database = database::live_status(&manifest, runtime_status);
    manifest.save_atomic()?;
    events::append(
        &manifest.event_log,
        events::EventType::Repair,
        events::EventStatus::Succeeded,
        None,
    )?;
    Ok(manifest)
}
