use std::path::Path;

use anyhow::Context;

use crate::{
    compose, events, git,
    lock::LockGuard,
    manifest::{ComponentStatus, SourceOwnership, StacksteadManifest},
    paths,
};

use super::{
    contract::run_commands,
    lease::{release_port_leases_after_destroy, verify_port_leases},
    project::load_project,
    provision_source::cleanup_failed_create,
    teardown::{
        TeardownPhase, finish_source_cleanup, read_teardown, source_cleanup_complete,
        validate_recovery_source, write_teardown,
    },
    types::ProjectRuntime,
    validation::{validate_manifest_binding, validate_pointer_binding, validate_source_binding},
};

pub fn destroy(cwd: &Path, name: &str) -> anyhow::Result<StacksteadManifest> {
    let runtime = load_project(cwd)?;
    let mut manifest = resolve_destroy_manifest(&runtime, name)?;
    let lock = LockGuard::acquire_existing(&manifest.state_dir.join("lock"), "stackstead")?;
    let run_lease = LockGuard::acquire_existing(
        &manifest.state_dir.join("run.lock"),
        "active stackstead agent",
    )?;
    manifest = StacksteadManifest::read(&manifest.manifest_path())?;
    validate_manifest_binding(&runtime, &manifest)?;
    if pending_create(&manifest) && !manifest.pointer_file.exists() {
        release_port_leases_after_destroy(&manifest)?;
        cleanup_failed_create(&runtime, &manifest)?;
        drop(run_lease);
        drop(lock);
        return Ok(manifest);
    }
    let teardown = read_teardown(&manifest)?;
    if teardown
        .as_ref()
        .is_none_or(|state| state.phase() != TeardownPhase::Finalize)
    {
        verify_port_leases(&manifest)?;
    }
    let mut phase = match teardown {
        Some(state) => state.phase(),
        None => begin_teardown(&runtime, &manifest)?,
    };
    if phase == TeardownPhase::RuntimeRemove {
        remove_runtime(&manifest)?;
        phase = TeardownPhase::SourceRemove;
    }
    if phase == TeardownPhase::SourceRemove {
        remove_source(&manifest)?;
        phase = TeardownPhase::Finalize;
    }
    debug_assert_eq!(
        phase,
        TeardownPhase::Finalize,
        "the teardown transaction must reach finalize"
    );
    if !source_cleanup_complete(&manifest) {
        anyhow::bail!("teardown reached finalize before source cleanup completed");
    }
    finalize_destroy(&runtime, manifest, lock)
}

fn begin_teardown(
    runtime: &ProjectRuntime,
    manifest: &StacksteadManifest,
) -> anyhow::Result<TeardownPhase> {
    validate_pointer_binding(manifest)?;
    validate_source_binding(manifest)?;
    git::ensure_worktree_clean(&manifest.worktree)?;
    events::append(
        &manifest.event_log,
        events::EventType::Destroy,
        events::EventStatus::Started,
        None,
    )?;
    let environment = manifest.trusted_environment(&manifest.validated_environment()?);
    run_commands(
        &runtime.config.hooks.pre_destroy,
        &manifest.worktree,
        &environment,
    )?;
    write_teardown(manifest, TeardownPhase::RuntimeRemove, None)?;
    Ok(TeardownPhase::RuntimeRemove)
}

fn remove_runtime(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    validate_source_binding(manifest)?;
    validate_pointer_binding(manifest)?;
    git::ensure_worktree_clean(&manifest.worktree)?;
    events::append(
        &manifest.event_log,
        events::EventType::RuntimeRemove,
        events::EventStatus::Started,
        None,
    )?;
    let removal = compose::stop(manifest).and_then(|()| compose::down_volumes(manifest));
    if let Err(error) = removal {
        write_teardown(
            manifest,
            TeardownPhase::RuntimeRemove,
            Some(&error.to_string()),
        )?;
        events::append(
            &manifest.event_log,
            events::EventType::RuntimeRemove,
            events::EventStatus::Failed,
            Some(&error.to_string()),
        )?;
        return Err(error);
    }
    events::append(
        &manifest.event_log,
        events::EventType::RuntimeRemove,
        events::EventStatus::Succeeded,
        None,
    )?;
    write_teardown(manifest, TeardownPhase::SourceRemove, None)
}

fn remove_source(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    validate_recovery_source(manifest)?;
    events::append(
        &manifest.event_log,
        events::EventType::SourceRemove,
        events::EventStatus::Started,
        None,
    )?;
    let cleanup = match finish_source_cleanup(manifest) {
        Ok(()) => Ok(()),
        Err(initial) if manifest.source_ownership == SourceOwnership::Stackstead => {
            compose::prepare_owned_source_removal(manifest)
                .with_context(|| {
                    format!("source cleanup failed before ownership repair: {initial}")
                })
                .and_then(|()| finish_source_cleanup(manifest))
        }
        Err(error) => Err(error),
    };
    if let Err(error) = cleanup {
        write_teardown(
            manifest,
            TeardownPhase::SourceRemove,
            Some(&error.to_string()),
        )?;
        return Err(error);
    }
    write_teardown(manifest, TeardownPhase::Finalize, None)
}

fn finalize_destroy(
    runtime: &ProjectRuntime,
    manifest: StacksteadManifest,
    lock: LockGuard,
) -> anyhow::Result<StacksteadManifest> {
    compose::remove_runtime_claim(&manifest).context("remove Compose runtime ownership claim")?;
    events::append(
        &manifest.event_log,
        events::EventType::Destroy,
        events::EventStatus::Succeeded,
        None,
    )
    .context("record completed destroy before final cleanup")?;
    release_port_leases_after_destroy(&manifest).context("release global port leases")?;
    paths::remove_stackstead_root(&manifest, &runtime.paths.state_root)
        .context("remove final Stackstead state root")?;
    drop(lock);
    Ok(manifest)
}

pub fn resolve_destroy(cwd: &Path, name: &str) -> anyhow::Result<StacksteadManifest> {
    resolve_destroy_manifest(&load_project(cwd)?, name)
}

fn resolve_destroy_manifest(
    runtime: &ProjectRuntime,
    name: &str,
) -> anyhow::Result<StacksteadManifest> {
    let manifest = runtime.paths.resolve(name)?;
    validate_manifest_binding(runtime, &manifest)?;
    if pending_create(&manifest) {
        if manifest.pointer_file.exists() {
            validate_pointer_binding(&manifest)?;
        }
        return Ok(manifest);
    }
    if read_teardown(&manifest)?.is_some() {
        return Ok(manifest);
    }
    validate_pointer_binding(&manifest)?;
    Ok(manifest)
}

fn pending_create(manifest: &StacksteadManifest) -> bool {
    !manifest.event_log.exists() && manifest.status.source == ComponentStatus::Created
}
