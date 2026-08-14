use std::{path::Path, time::Duration};

use chrono::Utc;

use crate::{
    compose,
    config::StacksteadConfig,
    database,
    manifest::{ComponentStatus, StacksteadManifest},
};

use super::{
    EffectiveComponent, EffectiveStatus, InspectOutput, LiveStatus, StatusBasis,
    inspect_health::observed_passive_health,
    project::load_project,
    teardown::{TeardownState, read_teardown},
    validation::{validate_manifest_binding, validate_pointer_binding, validate_source_binding},
};

pub fn inspect(cwd: &Path, name: &str) -> anyhow::Result<InspectOutput> {
    let runtime = load_project(cwd)?;
    let manifest = runtime.paths.resolve(name)?;
    validate_manifest_binding(&runtime, &manifest)?;
    let teardown = read_teardown(&manifest)?;
    let mut warnings = binding_warnings(&manifest);
    let live = observe_live(&runtime.config, &manifest, &mut warnings);
    append_filesystem_warnings(&manifest, &mut warnings);
    let effective = effective_status(
        &runtime.config,
        &manifest,
        teardown.as_ref(),
        &live,
        &mut warnings,
    );
    Ok(InspectOutput {
        manifest,
        live,
        effective,
        warnings,
    })
}

fn binding_warnings(manifest: &StacksteadManifest) -> Vec<String> {
    let mut warnings = vec![];
    if let Err(error) = validate_source_binding(manifest) {
        warnings.push(format!("source binding is invalid: {error}"));
    }
    if let Err(error) = validate_pointer_binding(manifest) {
        warnings.push(format!("generated pointer is invalid: {error}"));
    }
    warnings
}

fn observe_live(
    config: &StacksteadConfig,
    manifest: &StacksteadManifest,
    warnings: &mut Vec<String>,
) -> LiveStatus {
    let (runtime_status, services) = observe_runtime(manifest, warnings);
    let database_status = manifest
        .database
        .as_ref()
        .map(|_| database::live_status(manifest, runtime_status));
    let database_reachable = manifest.database.as_ref().map(|database| {
        database::reachable(&database.host, database.port, Duration::from_millis(250))
    });
    let health_healthy = if runtime_status == ComponentStatus::Running {
        match observed_passive_health(config, manifest, &services) {
            Ok(status) => status,
            Err(error) => {
                warnings.push(format!(
                    "could not inspect configured health targets: {error}"
                ));
                None
            }
        }
    } else {
        None
    };
    LiveStatus {
        runtime_status,
        services,
        database_reachable,
        database_status,
        health_healthy,
    }
}

fn observe_runtime(
    manifest: &StacksteadManifest,
    warnings: &mut Vec<String>,
) -> (ComponentStatus, Vec<compose::ServiceObservation>) {
    match compose::service_observations(manifest) {
        Ok(services) => {
            let status = if services.iter().any(|service| service.state == "running") {
                ComponentStatus::Running
            } else {
                ComponentStatus::Stopped
            };
            (status, services)
        }
        Err(error) => {
            warnings.push(format!("could not inspect Docker runtime: {error}"));
            (ComponentStatus::Unknown, vec![])
        }
    }
}

fn append_filesystem_warnings(manifest: &StacksteadManifest, warnings: &mut Vec<String>) {
    if !manifest.worktree.is_dir() {
        warnings.push("worktree is missing; run `stackstead doctor`".into());
    }
    for file in &manifest.compose_files {
        match compose::fixed_ports_in_file(file) {
            Ok(fixed_ports) => warnings.extend(fixed_ports.into_iter().map(|fixed| {
                format!(
                    "fixed host port {} in {}:{}",
                    fixed.host_port,
                    file.display(),
                    fixed.file_line
                )
            })),
            Err(error) => warnings.push(format!(
                "could not inspect fixed ports in {}: {error}",
                file.display()
            )),
        }
    }
}

fn effective_status(
    config: &StacksteadConfig,
    manifest: &StacksteadManifest,
    teardown: Option<&TeardownState>,
    live: &LiveStatus,
    warnings: &mut Vec<String>,
) -> EffectiveStatus {
    let runtime = EffectiveComponent {
        status: live.runtime_status,
        basis: StatusBasis::Live,
    };
    let database = live.database_status.map(|status| EffectiveComponent {
        status,
        basis: StatusBasis::Live,
    });
    let health = effective_health(config, manifest, live.health_healthy);
    push_status_divergence(warnings, "runtime", manifest.status.runtime, runtime);
    if let Some(database) = database {
        push_status_divergence(warnings, "database", manifest.status.database, database);
    }
    if health.basis == StatusBasis::Live {
        push_status_divergence(warnings, "health", manifest.status.health, health);
    }
    let phase = match teardown {
        None => "normal",
        Some(state) if state.has_error() => "teardown_failed",
        Some(_) => "teardown_incomplete",
    };
    if phase != "normal" {
        warnings.push(format!(
            "lifecycle phase is {phase}; retry `stackstead destroy {} --yes`",
            manifest.stackstead_id
        ));
    }
    EffectiveStatus {
        phase,
        recorded_at: manifest.updated_at,
        observed_at: Utc::now(),
        runtime,
        database,
        health,
    }
}

fn effective_health(
    config: &StacksteadConfig,
    manifest: &StacksteadManifest,
    health_healthy: Option<bool>,
) -> EffectiveComponent {
    match health_healthy {
        Some(healthy) => EffectiveComponent {
            status: if healthy {
                ComponentStatus::Ready
            } else {
                ComponentStatus::Failed
            },
            basis: StatusBasis::Live,
        },
        None if config.health.checks.is_empty()
            || config.health.checks.iter().any(|check| check.url.is_none()) =>
        {
            EffectiveComponent {
                status: manifest.status.health,
                basis: StatusBasis::Recorded,
            }
        }
        None => EffectiveComponent {
            status: ComponentStatus::Unknown,
            basis: StatusBasis::Lifecycle,
        },
    }
}

fn push_status_divergence(
    warnings: &mut Vec<String>,
    component: &str,
    recorded: ComponentStatus,
    effective: EffectiveComponent,
) {
    if recorded != ComponentStatus::Unknown
        && effective.status != ComponentStatus::Unknown
        && recorded != effective.status
    {
        warnings.push(format!(
            "recorded/live divergence: {component} recorded={recorded} effective={} ({})",
            effective.status, effective.basis
        ));
    }
}
