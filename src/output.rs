use std::{collections::BTreeMap, path::PathBuf};

use serde::Serialize;

use crate::{compose, database, doctor, lifecycle, manifest::StacksteadManifest};

const VERSION: &str = "1";

mod private {
    pub trait Sealed {}
}

pub(crate) trait CliOutput: private::Sealed + Serialize {}

macro_rules! cli_output {
    ($($type:ty),+ $(,)?) => {$(
        impl private::Sealed for $type {}
        impl CliOutput for $type {}
    )+};
}

#[derive(Debug, Serialize)]
pub(crate) struct PathOutput {
    kind: &'static str,
    version: &'static str,
    path: PathBuf,
}

impl PathOutput {
    pub(crate) fn initialized(path: PathBuf) -> Self {
        Self {
            kind: "StacksteadInit",
            version: VERSION,
            path,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ComposePlanOutput {
    kind: &'static str,
    version: &'static str,
    file: PathBuf,
    ports: Vec<ComposePortOutput>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ComposePortOutput {
    name: String,
    service: String,
    container_port: u16,
    env: String,
    current_host_port: Option<u16>,
    replacement: String,
    url: Option<String>,
}

impl From<&compose::ComposePlan> for ComposePlanOutput {
    fn from(plan: &compose::ComposePlan) -> Self {
        Self {
            kind: "ComposePlan",
            version: VERSION,
            file: plan.file.clone(),
            ports: plan
                .ports
                .iter()
                .map(|port| ComposePortOutput {
                    name: port.name.clone(),
                    service: port.service.clone(),
                    container_port: port.container_port,
                    env: port.env.clone(),
                    current_host_port: port.current_host_port,
                    replacement: port.replacement.clone(),
                    url: port.url.clone(),
                })
                .collect(),
            warnings: plan.warnings.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ComposeApplyOutput {
    kind: &'static str,
    version: &'static str,
    file: PathBuf,
    changed_lines: usize,
}

impl From<&compose::ComposeApplyOutput> for ComposeApplyOutput {
    fn from(output: &compose::ComposeApplyOutput) -> Self {
        Self {
            kind: "ComposeApply",
            version: VERSION,
            file: output.file.clone(),
            changed_lines: output.changed_lines,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct StacksteadChangeOutput {
    kind: &'static str,
    version: &'static str,
    action: &'static str,
    stackstead: StacksteadOutput,
}

impl StacksteadChangeOutput {
    pub(crate) fn new(action: &'static str, manifest: &StacksteadManifest) -> Self {
        Self {
            kind: "StacksteadChange",
            version: VERSION,
            action,
            stackstead: manifest.into(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct StacksteadListOutput {
    kind: &'static str,
    version: &'static str,
    stacksteads: Vec<StacksteadSummaryOutput>,
}

#[derive(Debug, Serialize)]
pub(crate) struct StacksteadSummaryOutput {
    stackstead_id: String,
    branch: String,
    ports: BTreeMap<String, u16>,
    runtime: String,
    worktree: PathBuf,
}

impl StacksteadSummaryOutput {
    pub(crate) fn new(manifest: StacksteadManifest, runtime: String) -> Self {
        Self {
            stackstead_id: manifest.stackstead_id,
            branch: manifest.branch,
            ports: manifest.ports,
            runtime,
            worktree: manifest.worktree,
        }
    }
}

impl StacksteadListOutput {
    pub(crate) fn new(stacksteads: Vec<StacksteadSummaryOutput>) -> Self {
        Self {
            kind: "StacksteadList",
            version: VERSION,
            stacksteads,
        }
    }

    pub(crate) fn stacksteads(&self) -> &[StacksteadSummaryOutput] {
        &self.stacksteads
    }
}

impl StacksteadSummaryOutput {
    pub(crate) fn stackstead_id(&self) -> &str {
        &self.stackstead_id
    }

    pub(crate) fn branch(&self) -> &str {
        &self.branch
    }

    pub(crate) fn ports(&self) -> &BTreeMap<String, u16> {
        &self.ports
    }

    pub(crate) fn runtime(&self) -> &str {
        &self.runtime
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct StacksteadInspectionOutput {
    kind: &'static str,
    version: &'static str,
    stackstead: StacksteadOutput,
    live: LiveOutput,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct LiveOutput {
    runtime: LiveComponentOutput,
    database: Option<LiveDatabaseOutput>,
    health: Option<LiveHealthOutput>,
}

#[derive(Debug, Serialize)]
struct LiveComponentOutput {
    running: bool,
    status: String,
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

impl StacksteadInspectionOutput {
    pub(crate) fn new(inspection: &lifecycle::InspectOutput) -> Self {
        let database = inspection
            .manifest
            .database
            .as_ref()
            .map(|_| LiveDatabaseOutput {
                reachable: inspection.live.database_reachable.unwrap_or(false),
                status: database::live_status(&inspection.manifest, inspection.live.runtime_status)
                    .to_string(),
            });
        Self {
            kind: "StacksteadInspection",
            version: VERSION,
            stackstead: (&inspection.manifest).into(),
            live: LiveOutput {
                runtime: LiveComponentOutput {
                    running: inspection.live.runtime_running,
                    status: inspection.live.runtime_status.to_string(),
                },
                database,
                health: inspection
                    .live
                    .health_healthy
                    .map(|healthy| LiveHealthOutput { healthy }),
            },
            warnings: inspection.warnings.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct EnvironmentOutput {
    kind: &'static str,
    version: &'static str,
    stackstead_id: String,
    path: PathBuf,
    values: BTreeMap<String, String>,
}

impl EnvironmentOutput {
    pub(crate) fn new(
        stackstead_id: String,
        path: PathBuf,
        values: BTreeMap<String, String>,
    ) -> Self {
        Self {
            kind: "StacksteadEnvironment",
            version: VERSION,
            stackstead_id,
            path,
            values,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ContextOutput {
    kind: &'static str,
    version: &'static str,
    stackstead_id: String,
    path: PathBuf,
    content: Option<String>,
}

impl ContextOutput {
    pub(crate) fn new(stackstead_id: String, path: PathBuf, content: Option<String>) -> Self {
        Self {
            kind: "StacksteadContext",
            version: VERSION,
            stackstead_id,
            path,
            content,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct LogsOutput {
    kind: &'static str,
    version: &'static str,
    stackstead_id: String,
    service: Option<String>,
    tail: usize,
    content: String,
}

impl LogsOutput {
    pub(crate) fn new(
        stackstead_id: String,
        service: Option<String>,
        tail: usize,
        content: String,
    ) -> Self {
        Self {
            kind: "StacksteadLogs",
            version: VERSION,
            stackstead_id,
            service,
            tail,
            content,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct OpenOutput {
    kind: &'static str,
    version: &'static str,
    stackstead_id: String,
    url: String,
    opened: bool,
}

impl OpenOutput {
    pub(crate) fn new(stackstead_id: String, url: String) -> Self {
        Self {
            kind: "StacksteadOpen",
            version: VERSION,
            stackstead_id,
            url,
            opened: false,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct DatabaseStatusOutput {
    kind: &'static str,
    version: &'static str,
    stackstead_id: String,
    strategy: String,
    service: String,
    host: String,
    port: u16,
    database: String,
    reachable: bool,
    identity_status: String,
    seed_status: String,
    last_seed_at: Option<String>,
}

impl DatabaseStatusOutput {
    pub(crate) fn new(status: database::DatabaseStatusOutput, identity_status: String) -> Self {
        Self {
            kind: "DatabaseStatus",
            version: VERSION,
            stackstead_id: status.stackstead_id,
            strategy: status.strategy,
            service: status.service,
            host: status.host,
            port: status.port,
            database: status.database,
            reachable: status.reachable,
            identity_status,
            seed_status: status.seed_status.to_string(),
            last_seed_at: status.last_seed_at.map(|value| value.to_rfc3339()),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct DoctorOutput {
    kind: &'static str,
    version: &'static str,
    ok: bool,
    error_count: usize,
    warning_count: usize,
    diagnostics: Vec<DiagnosticOutput>,
}

#[derive(Debug, Serialize)]
struct DiagnosticOutput {
    code: String,
    severity: String,
    message: String,
    suggestion: Option<String>,
}

impl DoctorOutput {
    pub(crate) fn new(diagnostics: &[doctor::Diagnostic]) -> Self {
        let error_count = diagnostics
            .iter()
            .filter(|item| item.severity == doctor::DiagnosticSeverity::Error)
            .count();
        let warning_count = diagnostics
            .iter()
            .filter(|item| item.severity == doctor::DiagnosticSeverity::Warning)
            .count();
        Self {
            kind: "DoctorReport",
            version: VERSION,
            ok: error_count == 0,
            error_count,
            warning_count,
            diagnostics: diagnostics
                .iter()
                .map(|item| DiagnosticOutput {
                    code: item.code.clone(),
                    severity: item.severity.to_string(),
                    message: item.message.clone(),
                    suggestion: item.suggestion.clone(),
                })
                .collect(),
        }
    }

    pub(crate) fn has_errors(&self) -> bool {
        self.error_count != 0
    }
}

#[derive(Debug, Serialize)]
struct StacksteadOutput {
    stackstead_id: String,
    slug: String,
    project: String,
    branch: String,
    base: String,
    source_ownership: String,
    repo_root: PathBuf,
    worktree: PathBuf,
    compose_project: String,
    compose_files: Vec<PathBuf>,
    ports: BTreeMap<String, u16>,
    container_ports: BTreeMap<String, u16>,
    urls: BTreeMap<String, String>,
    files: StacksteadFilesOutput,
    status: StacksteadStatusOutput,
    database: Option<StacksteadDatabaseOutput>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
struct StacksteadFilesOutput {
    manifest: PathBuf,
    environment: PathBuf,
    context: PathBuf,
    events: PathBuf,
    pointer: PathBuf,
}

#[derive(Debug, Serialize)]
struct StacksteadStatusOutput {
    source: String,
    dependencies: String,
    runtime: String,
    database: String,
    health: String,
}

#[derive(Debug, Serialize)]
struct StacksteadDatabaseOutput {
    strategy: String,
    service: String,
    host: String,
    port: u16,
    database: String,
    seed_status: String,
    last_seed_at: Option<String>,
}

impl From<&StacksteadManifest> for StacksteadOutput {
    fn from(manifest: &StacksteadManifest) -> Self {
        let mut compose_files = manifest.compose_files.clone();
        compose_files.push(manifest.worktree.join(".stackstead/compose-ownership.yaml"));
        Self {
            stackstead_id: manifest.stackstead_id.clone(),
            slug: manifest.slug.clone(),
            project: manifest.project.clone(),
            branch: manifest.branch.clone(),
            base: manifest.base.clone(),
            source_ownership: match manifest.source_ownership {
                crate::manifest::SourceOwnership::Stackstead => "stackstead",
                crate::manifest::SourceOwnership::External => "external",
            }
            .into(),
            repo_root: manifest.repo_root.clone(),
            worktree: manifest.worktree.clone(),
            compose_project: manifest.compose_project.clone(),
            compose_files,
            ports: manifest.ports.clone(),
            container_ports: manifest.container_ports.clone(),
            urls: manifest.urls.clone(),
            files: StacksteadFilesOutput {
                manifest: manifest.manifest_path(),
                environment: manifest.env_file.clone(),
                context: manifest.agent_context.clone(),
                events: manifest.event_log.clone(),
                pointer: manifest.pointer_file.clone(),
            },
            status: StacksteadStatusOutput {
                source: manifest.status.source.to_string(),
                dependencies: manifest.status.dependencies.to_string(),
                runtime: manifest.status.runtime.to_string(),
                database: manifest.status.database.to_string(),
                health: manifest.status.health.to_string(),
            },
            database: manifest
                .database
                .as_ref()
                .map(|database| StacksteadDatabaseOutput {
                    strategy: database.strategy.clone(),
                    service: database.service.clone(),
                    host: database.host.clone(),
                    port: database.port,
                    database: database.database.clone(),
                    seed_status: database.seed_status.to_string(),
                    last_seed_at: database.last_seed_at.map(|value| value.to_rfc3339()),
                }),
            created_at: manifest.created_at.to_rfc3339(),
            updated_at: manifest.updated_at.to_rfc3339(),
        }
    }
}

cli_output!(
    PathOutput,
    ComposePlanOutput,
    ComposeApplyOutput,
    StacksteadChangeOutput,
    StacksteadListOutput,
    StacksteadInspectionOutput,
    EnvironmentOutput,
    ContextOutput,
    LogsOutput,
    OpenOutput,
    DatabaseStatusOutput,
    DoctorOutput,
);
