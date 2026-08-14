use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

impl std::fmt::Display for DiagnosticSeverity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub suggestion: Option<String>,
}

impl Diagnostic {
    fn new(
        code: impl Into<String>,
        severity: DiagnosticSeverity,
        message: impl Into<String>,
        suggestion: Option<impl Into<String>>,
    ) -> Self {
        Self {
            code: code.into(),
            severity,
            message: message.into(),
            suggestion: suggestion.map(Into::into),
        }
    }

    fn info(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, DiagnosticSeverity::Info, message, None::<String>)
    }

    fn warning(
        code: impl Into<String>,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self::new(code, DiagnosticSeverity::Warning, message, Some(suggestion))
    }

    fn error(
        code: impl Into<String>,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self::new(code, DiagnosticSeverity::Error, message, Some(suggestion))
    }
}

#[derive(Debug, Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each boolean is an independent external-tool readiness receipt"
)]
struct ToolStatus {
    git: bool,
    docker: bool,
    compose: bool,
    docker_daemon: bool,
}

mod manifest;
mod project;
mod run;
mod state;
mod tools;

pub fn run(cwd: &Path) -> Vec<Diagnostic> {
    run::run(cwd)
}

#[cfg(test)]
mod tests;
