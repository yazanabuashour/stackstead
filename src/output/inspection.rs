use serde::Serialize;

use super::{INSPECTION_VERSION, contract::StacksteadOutput};
use crate::lifecycle;

#[derive(Debug, Serialize)]
pub struct StacksteadInspectionOutput {
    kind: &'static str,
    version: &'static str,
    stackstead: StacksteadOutput,
    live: LiveOutput,
    effective: EffectiveOutput,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct LiveOutput {
    runtime: LiveComponentOutput,
    services: Vec<LiveServiceOutput>,
    database: Option<LiveDatabaseOutput>,
    health: Option<LiveHealthOutput>,
}

#[derive(Debug, Serialize)]
struct LiveComponentOutput {
    running: bool,
    status: String,
}

#[derive(Debug, Serialize)]
struct LiveServiceOutput {
    service: String,
    container: String,
    status: String,
    exit_code: Option<i64>,
}

#[derive(Debug, Serialize)]
struct LiveDatabaseOutput {
    reachable: bool,
    status: String,
}

#[derive(Debug, Serialize)]
struct LiveHealthOutput {
    healthy: bool,
}

#[derive(Debug, Serialize)]
struct EffectiveOutput {
    phase: &'static str,
    recorded_at: chrono::DateTime<chrono::Utc>,
    observed_at: chrono::DateTime<chrono::Utc>,
    runtime: EffectiveComponentOutput,
    database: Option<EffectiveComponentOutput>,
    health: EffectiveComponentOutput,
}

#[derive(Debug, Serialize)]
struct EffectiveComponentOutput {
    status: String,
    basis: String,
}

impl From<lifecycle::EffectiveComponent> for EffectiveComponentOutput {
    fn from(component: lifecycle::EffectiveComponent) -> Self {
        Self {
            status: component.status.to_string(),
            basis: component.basis.to_string(),
        }
    }
}

impl StacksteadInspectionOutput {
    pub(crate) fn new(inspection: &lifecycle::InspectOutput) -> Self {
        let database = inspection
            .live
            .database_status
            .map(|status| LiveDatabaseOutput {
                reachable: inspection.live.database_reachable.unwrap_or(false),
                status: status.to_string(),
            });
        Self {
            kind: "StacksteadInspection",
            version: INSPECTION_VERSION,
            stackstead: (&inspection.manifest).into(),
            live: LiveOutput {
                runtime: LiveComponentOutput {
                    running: inspection.live.runtime_status
                        == crate::manifest::ComponentStatus::Running,
                    status: inspection.live.runtime_status.to_string(),
                },
                services: inspection
                    .live
                    .services
                    .iter()
                    .map(|service| LiveServiceOutput {
                        service: service.service.clone(),
                        container: service.container.clone(),
                        status: service.status(),
                        exit_code: service.exit_code,
                    })
                    .collect(),
                database,
                health: inspection
                    .live
                    .health_healthy
                    .map(|healthy| LiveHealthOutput { healthy }),
            },
            effective: EffectiveOutput {
                phase: inspection.effective.phase,
                recorded_at: inspection.effective.recorded_at,
                observed_at: inspection.effective.observed_at,
                runtime: inspection.effective.runtime.into(),
                database: inspection.effective.database.map(Into::into),
                health: inspection.effective.health.into(),
            },
            warnings: inspection.warnings.clone(),
        }
    }
}
