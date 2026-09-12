use std::{collections::BTreeMap, path::Path, time::Instant};

use crate::{
    compose, events,
    manifest::{ComponentStatus, StacksteadManifest},
    readiness::Requirements,
};

use super::{
    CreateOutcome, HeldEnvironment,
    contract::{install_dependencies, run_commands, template_context, write_contract},
    lease::verify_port_leases,
    project::load_project,
    types::{ProjectRuntime, UpOutcome, UpTimings},
    up_database, up_readiness,
    validation::{
        validate_configured_ports, validate_current_contract, validate_pointer_binding,
        validate_source_binding,
    },
};

struct UpTransaction {
    runtime: ProjectRuntime,
    held: HeldEnvironment,
    environment: BTreeMap<String, String>,
    profiles: Option<String>,
    requirements: Option<Requirements>,
    timings: UpTimings,
}

pub fn up(cwd: &Path, name: &str) -> anyhow::Result<UpOutcome> {
    start(cwd, name, None)
}

pub fn up_after_create(cwd: &Path, created: CreateOutcome) -> anyhow::Result<UpOutcome> {
    let name = created.manifest().stackstead_id.clone();
    start(cwd, &name, Some(created))
}

fn start(cwd: &Path, name: &str, created: Option<CreateOutcome>) -> anyhow::Result<UpOutcome> {
    let total_started = Instant::now();
    let mut transaction = begin(cwd, name, created)?;
    install_dependencies_phase(&mut transaction)?;
    hooks_phase(&mut transaction, true)?;
    runtime_phase(&mut transaction)?;
    database_phase(&mut transaction)?;
    hooks_phase(&mut transaction, false)?;
    health_phase(&mut transaction)?;
    transaction.held.manifest_mut().save_atomic()?;
    transaction.timings.total = total_started.elapsed();
    Ok(UpOutcome {
        environment: transaction.held,
        timings: transaction.timings,
    })
}

fn begin(cwd: &Path, name: &str, created: Option<CreateOutcome>) -> anyhow::Result<UpTransaction> {
    // Reject non-UTF-8 profiles before the command runner enumerates its environment.
    let profiles = up_readiness::capture_profiles()?;
    let runtime = load_project(cwd)?;
    let resolved = runtime.resolve(name)?;
    let mut held = match created {
        Some(created) => HeldEnvironment::after_create(created, &resolved)?,
        None => HeldEnvironment::exclusive(resolved)?,
    };
    let manifest = held.manifest_mut();
    validate_current_contract(&runtime, manifest)?;
    validate_pointer_binding(manifest)?;
    validate_source_binding(manifest)?;
    verify_port_leases(manifest)?;
    manifest.readiness.invalidate();
    manifest.status.health = ComponentStatus::Unknown;
    manifest.status.database = ComponentStatus::Unknown;
    manifest.save_atomic()?;
    let context = template_context(manifest);
    write_contract(&runtime.config, manifest, &context)?;
    let environment = manifest.trusted_environment(&manifest.validated_environment()?);
    Ok(UpTransaction {
        runtime,
        held,
        environment,
        profiles,
        requirements: None,
        timings: UpTimings::default(),
    })
}

fn install_dependencies_phase(transaction: &mut UpTransaction) -> anyhow::Result<()> {
    let started = Instant::now();
    let manifest = transaction.held.manifest_mut();
    append_event(
        manifest,
        events::EventType::DependenciesInstall,
        events::EventStatus::Started,
        None,
    )?;
    if let Err(error) = install_dependencies(
        &transaction.runtime.config,
        manifest,
        &transaction.environment,
    ) {
        manifest.status.dependencies = ComponentStatus::Failed;
        manifest.save_atomic()?;
        append_event(
            manifest,
            events::EventType::DependenciesInstall,
            events::EventStatus::Failed,
            Some(&error),
        )?;
        return Err(error);
    }
    manifest.status.dependencies = ComponentStatus::Ready;
    transaction.timings.dependencies = started.elapsed();
    append_event(
        manifest,
        events::EventType::DependenciesInstall,
        events::EventStatus::Succeeded,
        None,
    )?;
    manifest.save_atomic()?;
    Ok(())
}

fn hooks_phase(transaction: &mut UpTransaction, before_runtime: bool) -> anyhow::Result<()> {
    let commands = if before_runtime {
        &transaction.runtime.config.hooks.pre_up
    } else {
        &transaction.runtime.config.hooks.post_up
    };
    let started = Instant::now();
    let manifest = transaction.held.manifest();
    run_commands(commands, &manifest.worktree, &transaction.environment)?;
    if !commands.is_empty() {
        let elapsed = started.elapsed();
        transaction.timings.hooks = Some(if before_runtime {
            elapsed
        } else {
            transaction
                .timings
                .hooks
                .unwrap_or_default()
                .checked_add(elapsed)
                .ok_or_else(|| anyhow::anyhow!("hook timing exceeds the supported duration"))?
        });
    }
    validate_source_binding(manifest)?;
    validate_pointer_binding(manifest)?;
    validate_configured_ports(&transaction.runtime.config, &manifest.worktree)
}

fn runtime_phase(transaction: &mut UpTransaction) -> anyhow::Result<()> {
    let started = Instant::now();
    let manifest = transaction.held.manifest_mut();
    append_event(
        manifest,
        events::EventType::RuntimeStart,
        events::EventStatus::Started,
        None,
    )?;
    let result = (|| {
        transaction.requirements =
            up_readiness::resolve(manifest, transaction.profiles.as_deref())?;
        compose::up(manifest)
    })();
    if let Err(error) = result {
        manifest.status.runtime = ComponentStatus::Failed;
        manifest.save_atomic()?;
        append_event(
            manifest,
            events::EventType::RuntimeStart,
            events::EventStatus::Failed,
            Some(&error),
        )?;
        return Err(error);
    }
    manifest.status.runtime = ComponentStatus::Running;
    transaction.timings.runtime = started.elapsed();
    append_event(
        manifest,
        events::EventType::RuntimeStart,
        events::EventStatus::Succeeded,
        None,
    )?;
    manifest.save_atomic()?;
    Ok(())
}

fn database_phase(transaction: &mut UpTransaction) -> anyhow::Result<()> {
    up_database::run(
        transaction.runtime.config.database.postgres.clone(),
        transaction.held.manifest_mut(),
        &transaction.environment,
        &mut transaction.timings,
    )
}

fn health_phase(transaction: &mut UpTransaction) -> anyhow::Result<()> {
    let manifest = transaction.held.manifest_mut();
    if transaction.runtime.config.health.checks.is_empty()
        && manifest.readiness.required().is_none()
    {
        return Ok(());
    }
    let started = Instant::now();
    append_event(
        manifest,
        events::EventType::HealthWait,
        events::EventStatus::Started,
        None,
    )?;
    if let Err(error) = up_readiness::wait(
        &transaction.runtime.config.health,
        manifest,
        &transaction.environment,
        transaction.requirements.as_ref(),
    ) {
        manifest.readiness.invalidate();
        manifest.save_atomic()?;
        append_event(
            manifest,
            events::EventType::HealthWait,
            events::EventStatus::Failed,
            Some(&error),
        )?;
        return Err(error);
    }
    transaction.timings.health = Some(started.elapsed());
    append_event(
        manifest,
        events::EventType::HealthWait,
        events::EventStatus::Succeeded,
        None,
    )
}

fn append_event(
    manifest: &StacksteadManifest,
    event_type: events::EventType,
    status: events::EventStatus,
    error: Option<&anyhow::Error>,
) -> anyhow::Result<()> {
    events::append(
        &manifest.event_log,
        event_type,
        status,
        error.map(ToString::to_string).as_deref(),
    )
}
