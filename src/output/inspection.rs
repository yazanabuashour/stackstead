use serde::Serialize;

use super::{
    INSPECTION_VERSION,
    contract::StacksteadOutput,
    runtime::{LiveServiceOutput, ReadinessOutput},
};
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
    services: Option<Vec<LiveServiceOutput>>,
    readiness: ReadinessOutput,
    database: Option<LiveDatabaseOutput>,
    health: Option<LiveHealthOutput>,
}

#[derive(Debug, Serialize)]
struct LiveComponentOutput {
    running: Option<bool>,
    status: String,
    activity: &'static str,
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
                    running: inspection.live.runtime.running(),
                    status: inspection.live.runtime.status().to_string(),
                    activity: inspection.live.runtime.activity(),
                },
                services: inspection
                    .live
                    .runtime
                    .evidence()
                    .map(|services| services.iter().map(Into::into).collect()),
                readiness: (&inspection.live.runtime.readiness).into(),
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
