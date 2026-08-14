use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use chrono::Utc;

use crate::{
    command, compose,
    config::PostgresConfig,
    database, events,
    manifest::{ComponentStatus, StacksteadManifest},
};

use super::types::UpTimings;

pub(super) fn run(
    config: Option<PostgresConfig>,
    manifest: &mut StacksteadManifest,
    environment: &BTreeMap<String, String>,
    timings: &mut UpTimings,
) -> anyhow::Result<()> {
    let Some(config) = config else {
        return Ok(());
    };
    wait_for_database(&config, manifest, timings)?;
    if !config.seed.command.trim().is_empty() {
        seed_database(&config, manifest, environment, timings)?;
    }
    Ok(())
}

fn wait_for_database(
    config: &PostgresConfig,
    manifest: &mut StacksteadManifest,
    timings: &mut UpTimings,
) -> anyhow::Result<()> {
    let started = Instant::now();
    append_event(
        manifest,
        events::EventType::DatabaseWait,
        events::EventStatus::Started,
        None,
    )?;
    if let Err(error) = database_readiness(config, manifest) {
        manifest.status.database = ComponentStatus::Unreachable;
        manifest.save_atomic()?;
        append_event(
            manifest,
            events::EventType::DatabaseWait,
            events::EventStatus::Failed,
            Some(&error),
        )?;
        return Err(error);
    }
    manifest.status.database = ComponentStatus::Reachable;
    timings.database = Some(started.elapsed());
    append_event(
        manifest,
        events::EventType::DatabaseWait,
        events::EventStatus::Succeeded,
        None,
    )?;
    manifest.save_atomic()?;
    Ok(())
}

fn database_readiness(
    config: &PostgresConfig,
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    let state = manifest
        .database
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("validated database contract has no state"))?;
    let container_port = manifest
        .container_ports
        .get(&config.service)
        .copied()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "validated database contract has no container port for `{}`",
                config.service
            )
        })?;
    compose::ensure_endpoint_published(
        manifest,
        &config.service,
        container_port,
        &state.host,
        state.port,
    )?;
    database::wait_until_postgres_ready(&state.host, state.port, Duration::from_secs(30), || {
        compose::postgres_is_ready(manifest, &config.service, &config.user, &config.database)
    })
}

fn seed_database(
    config: &PostgresConfig,
    manifest: &mut StacksteadManifest,
    environment: &BTreeMap<String, String>,
    timings: &mut UpTimings,
) -> anyhow::Result<()> {
    let started = Instant::now();
    append_event(
        manifest,
        events::EventType::DatabaseSeed,
        events::EventStatus::Started,
        None,
    )?;
    if let Err(error) = command::run_configured(
        &config.seed.command,
        config.seed.shell,
        &manifest.worktree,
        environment,
    ) {
        manifest
            .database
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("validated database contract has no state"))?
            .seed_status = ComponentStatus::Failed;
        manifest.save_atomic()?;
        append_event(
            manifest,
            events::EventType::DatabaseSeed,
            events::EventStatus::Failed,
            Some(&error),
        )?;
        return Err(error);
    }
    let state = manifest
        .database
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("validated database contract has no state"))?;
    state.seed_status = ComponentStatus::Ready;
    state.last_seed_at = Some(Utc::now());
    timings.seed = Some(started.elapsed());
    append_event(
        manifest,
        events::EventType::DatabaseSeed,
        events::EventStatus::Succeeded,
        None,
    )?;
    manifest.save_atomic()?;
    Ok(())
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
