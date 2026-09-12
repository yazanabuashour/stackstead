use std::path::Path;

use crate::{
    compose, events,
    manifest::{ComponentStatus, StacksteadManifest},
};

use super::{
    HeldEnvironment,
    lease::verify_port_leases,
    project::load_project,
    teardown::ensure_no_teardown,
    validation::{validate_manifest_binding, validate_pointer_binding, validate_source_binding},
};

pub fn stop(cwd: &Path, name: &str) -> anyhow::Result<StacksteadManifest> {
    let runtime = load_project(cwd)?;
    let mut held = HeldEnvironment::exclusive(runtime.resolve(name)?)?;
    let manifest = held.manifest_mut();
    validate_manifest_binding(&runtime, manifest)?;
    ensure_no_teardown(manifest)?;
    validate_pointer_binding(manifest)?;
    validate_source_binding(manifest)?;
    verify_port_leases(manifest)?;
    compose::stop(manifest)?;
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
    Ok(held.into_manifest())
}
