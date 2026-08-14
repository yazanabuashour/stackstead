use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HostBinding {
    Fixed(u16),
    Variable(String),
    Missing,
}

pub(super) const OWNERSHIP_OVERRIDE: &str = ".stackstead/compose-ownership.yaml";
pub(super) const OWNERSHIP_HELPER_IMAGE: &str =
    "alpine@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc";
pub(super) const RUNTIME_TOKEN_LABEL: &str = "io.stackstead.runtime-token";
pub(super) const COMPOSE_PROJECT_LABEL: &str = "com.docker.compose.project";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposePlan {
    pub file: PathBuf,
    pub ports: Vec<ComposePortPlan>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposePortPlan {
    pub name: String,
    pub service: String,
    pub container_port: u16,
    pub env: String,
    pub current_host_port: Option<u16>,
    pub replacement: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeApplyOutput {
    pub file: PathBuf,
    pub changed_lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposePortTarget {
    pub service: String,
    pub container_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceObservation {
    pub service: String,
    pub container: String,
    pub state: String,
    pub exit_code: Option<i64>,
}

impl ServiceObservation {
    pub fn status(&self) -> String {
        match (self.state.as_str(), self.exit_code) {
            ("exited", Some(0)) => "completed (0)".into(),
            ("exited", Some(code)) => format!("exited ({code})"),
            _ => self.state.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedPort {
    pub file_line: usize,
    pub host_port: u16,
    pub mapping: String,
}
