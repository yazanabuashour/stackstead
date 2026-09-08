use std::{path::PathBuf, time::Duration};

use chrono::Utc;

use crate::{
    config::StacksteadConfig,
    lock::LockGuard,
    manifest::{ComponentStatus, SourceOwnership, StacksteadManifest},
    state::ProjectPaths,
};

use super::{ensure_no_teardown, validation};

#[derive(Debug, Clone)]
pub struct ProjectRuntime {
    pub config: StacksteadConfig,
    pub paths: ProjectPaths,
}

impl ProjectRuntime {
    pub fn resolve(&self, name: &str) -> anyhow::Result<StacksteadManifest> {
        let manifest = self.paths.resolve(name)?;
        validation::validate_manifest_binding(self, &manifest)?;
        validation::validate_pointer_binding(&manifest)?;
        ensure_no_teardown(&manifest)?;
        Ok(manifest)
    }
}

#[derive(Debug, Clone)]
pub struct CurrentIdentity {
    pub stackstead_id: String,
    pub source_ownership: SourceOwnership,
    pub repo_root: PathBuf,
    pub worktree: PathBuf,
    pub pointer: PathBuf,
}

#[derive(Debug, Clone)]
pub struct InspectOutput {
    pub manifest: StacksteadManifest,
    pub live: LiveStatus,
    pub effective: EffectiveStatus,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LiveStatus {
    pub runtime: super::RuntimeObservation,
    pub database_reachable: Option<bool>,
    pub database_status: Option<ComponentStatus>,
    pub health_healthy: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusBasis {
    Live,
    Recorded,
    Lifecycle,
    Unconfigured,
}

impl std::fmt::Display for StatusBasis {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Live => "live",
            Self::Recorded => "recorded",
            Self::Lifecycle => "lifecycle",
            Self::Unconfigured => "unconfigured",
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EffectiveComponent {
    pub status: ComponentStatus,
    pub basis: StatusBasis,
}

#[derive(Debug, Clone)]
pub struct EffectiveStatus {
    pub phase: &'static str,
    pub recorded_at: chrono::DateTime<Utc>,
    pub observed_at: chrono::DateTime<Utc>,
    pub runtime: EffectiveComponent,
    pub database: Option<EffectiveComponent>,
    pub health: EffectiveComponent,
}

#[derive(Default)]
pub struct UpTimings {
    pub dependencies: Duration,
    pub runtime: Duration,
    pub database: Option<Duration>,
    pub seed: Option<Duration>,
    pub hooks: Option<Duration>,
    pub health: Option<Duration>,
    pub total: Duration,
}

pub struct UpOutcome {
    pub manifest: StacksteadManifest,
    pub timings: UpTimings,
    pub mutation_lock: LockGuard,
    pub run_lease: LockGuard,
}

pub struct CreateOutcome {
    pub manifest: StacksteadManifest,
    pub mutation_lock: LockGuard,
}
