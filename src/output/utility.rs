use std::{collections::BTreeMap, path::PathBuf};

use serde::Serialize;

use super::VERSION;
use crate::{database, doctor};

#[derive(Debug, Serialize)]
pub struct EnvironmentOutput {
    kind: &'static str,
    version: &'static str,
    stackstead_id: String,
    path: PathBuf,
    values: BTreeMap<String, String>,
}

impl EnvironmentOutput {
    pub(crate) const fn new(
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
pub struct ContextOutput {
    kind: &'static str,
    version: &'static str,
    stackstead_id: String,
    path: PathBuf,
    content: Option<String>,
}

impl ContextOutput {
    pub(crate) const fn new(stackstead_id: String, path: PathBuf, content: Option<String>) -> Self {
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
pub struct LogsOutput {
    kind: &'static str,
    version: &'static str,
    stackstead_id: String,
    service: Option<String>,
    tail: usize,
    content: String,
}

impl LogsOutput {
    pub(crate) const fn new(
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
pub struct OpenOutput {
    kind: &'static str,
    version: &'static str,
    stackstead_id: String,
    url: String,
    opened: bool,
}

impl OpenOutput {
    pub(crate) const fn new(stackstead_id: String, url: String) -> Self {
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
pub struct DatabaseStatusOutput {
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
pub struct DoctorOutput {
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

    pub(crate) const fn has_errors(&self) -> bool {
        self.error_count != 0
    }
}
