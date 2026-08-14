use std::{collections::BTreeMap, path::PathBuf};

use serde::Serialize;

use super::{VERSION, contract::StacksteadOutput};
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
    pub(crate) const fn new(stacksteads: Vec<StacksteadSummaryOutput>) -> Self {
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

    pub(crate) const fn ports(&self) -> &BTreeMap<String, u16> {
        &self.ports
    }

    pub(crate) fn runtime(&self) -> &str {
        &self.runtime
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
