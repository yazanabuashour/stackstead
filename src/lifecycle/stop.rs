use std::path::Path;

use crate::{
    compose, events,
    lock::LockGuard,
    manifest::{ComponentStatus, StacksteadManifest},
};

use super::{
    lease::verify_port_leases,
    project::load_project,
    teardown::ensure_no_teardown,
    validation::{validate_manifest_binding, validate_pointer_binding, validate_source_binding},
};

pub fn stop(cwd: &Path, name: &str) -> anyhow::Result<StacksteadManifest> {
    let runtime = load_project(cwd)?;
    let mut manifest = runtime.resolve(name)?;
    let _lock = LockGuard::acquire_existing(&manifest.state_dir.join("lock"), "stackstead")?;
    let _run_lease = LockGuard::acquire_existing(
        &manifest.state_dir.join("run.lock"),
        "active stackstead agent",
    )?;
    manifest = StacksteadManifest::read(&manifest.manifest_path())?;
    validate_manifest_binding(&runtime, &manifest)?;
    ensure_no_teardown(&manifest)?;
    validate_pointer_binding(&manifest)?;
    validate_source_binding(&manifest)?;
    verify_port_leases(&manifest)?;
    compose::stop(&manifest)?;
    manifest.status.runtime = ComponentStatus::Stopped;
    manifest.status.database = ComponentStatus::Unknown;
    manifest.status.health = ComponentStatus::Unknown;
    manifest.save_atomic()?;
    events::append(
        &manifest.event_log,
        events::EventType::RuntimeStop,
        events::EventStatus::Succeeded,
        None,
    )?;
    Ok(manifest)
}
