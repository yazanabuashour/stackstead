use std::{collections::BTreeMap, path::PathBuf};

use serde::Serialize;

use super::{
    LIST_VERSION, VERSION,
    contract::StacksteadOutput,
    runtime::{LiveServiceOutput, ReadinessOutput},
};
use crate::{lifecycle, manifest::StacksteadManifest};

#[derive(Debug, Serialize)]
pub struct StacksteadChangeOutput {
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
pub struct StacksteadListOutput {
    kind: &'static str,
    version: &'static str,
    stacksteads: Vec<StacksteadSummaryOutput>,
}

#[derive(Debug, Serialize)]
pub struct StacksteadSummaryOutput {
    stackstead_id: String,
    branch: String,
    ports: BTreeMap<String, u16>,
    runtime: &'static str,
    readiness: ReadinessOutput,
    services: Option<Vec<LiveServiceOutput>>,
    issues: Vec<String>,
    worktree: PathBuf,
}

impl StacksteadSummaryOutput {
    pub(crate) fn new(
        manifest: StacksteadManifest,
        observation: &lifecycle::RuntimeObservation,
    ) -> Self {
        Self {
            stackstead_id: manifest.stackstead_id,
            branch: manifest.branch,
            ports: manifest.ports,
            runtime: observation.activity(),
            readiness: (&observation.readiness).into(),
            services: observation
                .services
                .as_ref()
                .map(|services| services.iter().map(Into::into).collect()),
            issues: observation.issues.clone(),
            worktree: manifest.worktree,
        }
    }
}

impl StacksteadListOutput {
    pub(crate) const fn new(stacksteads: Vec<StacksteadSummaryOutput>) -> Self {
        Self {
            kind: "StacksteadList",
            version: LIST_VERSION,
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

    pub(crate) const fn ports(&self) -> &BTreeMap<String, u16> {
        &self.ports
    }

    pub(crate) const fn runtime(&self) -> &'static str {
        self.runtime
    }

    pub(crate) const fn readiness(&self) -> &'static str {
        self.readiness.status()
    }

    pub(crate) fn issues(&self) -> impl Iterator<Item = &str> {
        self.issues
            .iter()
            .chain(self.readiness.issues())
            .map(String::as_str)
    }

    pub(crate) fn service_statuses(&self) -> impl Iterator<Item = String> + '_ {
        self.services
            .iter()
            .flatten()
            .map(LiveServiceOutput::summary)
    }
}

#[derive(Debug, Serialize)]
pub struct StacksteadCurrentOutput {
    kind: &'static str,
    version: &'static str,
    stackstead_id: String,
    source_ownership: crate::manifest::SourceOwnership,
    repo_root: PathBuf,
    worktree: PathBuf,
    pointer: PathBuf,
}

impl From<&lifecycle::CurrentIdentity> for StacksteadCurrentOutput {
    fn from(current: &lifecycle::CurrentIdentity) -> Self {
        Self {
            kind: "StacksteadCurrent",
            version: VERSION,
            stackstead_id: current.stackstead_id.clone(),
            source_ownership: current.source_ownership,
            repo_root: current.repo_root.clone(),
            worktree: current.worktree.clone(),
            pointer: current.pointer.clone(),
        }
    }
}
