use std::path::PathBuf;

use serde::Serialize;

use super::VERSION;
use crate::compose;

#[derive(Debug, Serialize)]
pub struct PathOutput {
    kind: &'static str,
    version: &'static str,
    path: PathBuf,
}

impl PathOutput {
    pub(crate) const fn initialized(path: PathBuf) -> Self {
        Self {
            kind: "StacksteadInit",
            version: VERSION,
            path,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ComposePlanOutput {
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
pub struct ComposeApplyOutput {
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
