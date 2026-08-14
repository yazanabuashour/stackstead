use std::{collections::BTreeSet, path::Path};

use crate::{
    events,
    lease::{LeaseIdentity, PortLeaseStore},
    lock::{LockGuard, project_lock_path},
    manifest::StacksteadManifest,
    paths, ports,
};

use super::{
    CreateOutcome, ProjectRuntime,
    lease::{release_port_leases, release_port_leases_after_destroy},
    provision_manifest,
    provision_plan::{self, PreparedProvision},
    provision_source::{cleanup_failed_create, create_event_type, create_source_and_contract},
};

pub fn create(cwd: &Path, name: &str) -> anyhow::Result<StacksteadManifest> {
    Ok(provision(cwd, name, None)?.manifest)
}

pub fn create_for_launch(cwd: &Path, name: &str) -> anyhow::Result<CreateOutcome> {
    provision(cwd, name, None)
}

pub fn adopt(cwd: &Path, name: &str, worktree: &Path) -> anyhow::Result<StacksteadManifest> {
    Ok(provision(cwd, name, Some(worktree))?.manifest)
}

fn provision(
    cwd: &Path,
    name: &str,
    external_worktree: Option<&Path>,
) -> anyhow::Result<CreateOutcome> {
    let prepared = provision_plan::prepare(cwd, external_worktree)?;
    std::fs::create_dir_all(&prepared.runtime.paths.project_state_dir)?;
    let _project_lock = LockGuard::acquire(
        &project_lock_path(&prepared.runtime.paths.project_state_dir),
        "project",
    )?;
    provision_locked(&prepared, name)
}

fn provision_locked(prepared: &PreparedProvision, name: &str) -> anyhow::Result<CreateOutcome> {
    let existing = prepared.runtime.paths.manifests()?;
    let slug = provision_plan::validate_name(prepared, name, &existing)?;
    let identity = provision_manifest::prepare_identity(prepared, slug, &existing)?;
    let service_names = prepared.runtime.config.service_names();
    let port_lease_store = if service_names.is_empty() {
        None
    } else {
        Some(PortLeaseStore::for_current_user()?)
    };
    let port_lease_state_dir = port_lease_store
        .as_ref()
        .map(|store| store.state_dir().to_path_buf());
    let mut port_leases = port_lease_store
        .as_ref()
        .map(PortLeaseStore::transaction)
        .transpose()?;
    let mut used_ports = existing
        .iter()
        .flat_map(|manifest| manifest.ports.values().copied())
        .collect::<BTreeSet<_>>();
    if let Some(leases) = &port_leases {
        used_ports.extend(leases.used_ports());
    }
    let allocation = ports::allocate_ports(
        prepared.runtime.config.resources.ports.base,
        prepared.runtime.config.resources.ports.stride,
        &service_names,
        &used_ports,
    )?;
    let (mut manifest, context) = provision_manifest::build(
        prepared,
        identity,
        allocation.ports,
        port_lease_state_dir,
        &existing,
    )?;
    let cell_lock = initialize_manifest(&manifest)?;
    let cell_lock = persist_initial_manifest(&prepared.runtime, &mut manifest, cell_lock)?;
    let leased_ports = manifest.ports.values().copied().collect();
    let reservation = port_leases.as_mut().map_or(Ok(()), |leases| {
        leases.reserve(
            &manifest.runtime_token,
            &LeaseIdentity::new(&manifest.stackstead_id, &manifest.project),
            &leased_ports,
        )
    });
    if let Err(error) = reservation {
        drop(port_leases);
        return Err(rollback_failed_reservation(
            &prepared.runtime,
            &manifest,
            error,
        ));
    }
    drop(port_leases);
    if let Err(error) =
        create_source_and_contract(&prepared.runtime.config, &mut manifest, &context)
    {
        return Err(rollback_failed_source(&prepared.runtime, &manifest, error));
    }
    Ok(CreateOutcome {
        manifest,
        mutation_lock: cell_lock,
    })
}

fn initialize_manifest(manifest: &StacksteadManifest) -> anyhow::Result<LockGuard> {
    std::fs::create_dir_all(manifest.state_dir.join("logs"))?;
    let cell_lock = LockGuard::acquire(&manifest.state_dir.join("lock"), "stackstead")?;
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(manifest.state_dir.join("run.lock"))?;
    Ok(cell_lock)
}

fn persist_initial_manifest(
    runtime: &ProjectRuntime,
    manifest: &mut StacksteadManifest,
    cell_lock: LockGuard,
) -> anyhow::Result<LockGuard> {
    if let Err(error) = manifest.save_atomic() {
        let cleanup = paths::remove_stackstead_root(manifest, &runtime.paths.state_root);
        drop(cell_lock);
        return match cleanup {
            Ok(()) => Err(error),
            Err(cleanup) => Err(anyhow::anyhow!(
                "{error}; failed to clean state after initial manifest persistence failed: {cleanup}"
            )),
        };
    }
    Ok(cell_lock)
}

fn rollback_failed_reservation(
    runtime: &ProjectRuntime,
    manifest: &StacksteadManifest,
    error: anyhow::Error,
) -> anyhow::Error {
    let lease_cleanup = release_port_leases_after_destroy(manifest);
    let cleanup = if lease_cleanup.is_ok() {
        cleanup_failed_create(runtime, manifest)
    } else {
        Err(anyhow::anyhow!(
            "skipped to retain recovery state after ambiguous port lease reservation"
        ))
    };
    match (lease_cleanup, cleanup) {
        (Ok(()), Ok(())) => error,
        (lease_cleanup, cleanup) => anyhow::anyhow!(
            "{error}; failed to reconcile partial create: port lease cleanup={}; source/state cleanup={}",
            lease_cleanup.map_or_else(|error| error.to_string(), |()| "ok".into()),
            cleanup.map_or_else(|error| error.to_string(), |()| "ok".into())
        ),
    }
}

fn rollback_failed_source(
    runtime: &ProjectRuntime,
    manifest: &StacksteadManifest,
    error: anyhow::Error,
) -> anyhow::Error {
    if manifest.pointer_file.is_file() {
        let event_type = create_event_type(manifest);
        drop(events::append(
            &manifest.event_log,
            event_type,
            events::EventStatus::Failed,
            Some(&error.to_string()),
        ));
        return error;
    }
    let lease_cleanup = release_port_leases(manifest);
    let cleanup = if lease_cleanup.is_ok() {
        cleanup_failed_create(runtime, manifest)
    } else {
        Err(anyhow::anyhow!(
            "skipped to retain recovery state after port lease cleanup failed"
        ))
    };
    match (cleanup, lease_cleanup) {
        (Ok(()), Ok(())) => error,
        (cleanup, lease_cleanup) => anyhow::anyhow!(
            "{error}; failed to roll back partial create: source/state cleanup={}; port lease cleanup={}",
            cleanup.map_or_else(|error| error.to_string(), |()| "ok".into()),
            lease_cleanup.map_or_else(|error| error.to_string(), |()| "ok".into())
        ),
    }
}
