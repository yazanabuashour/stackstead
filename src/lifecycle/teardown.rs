use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{
    command, events, git,
    manifest::{SourceOwnership, StacksteadManifest, write_json_atomic},
    paths,
};

use super::validation::{validate_pointer_binding, validate_source_binding};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum TeardownPhase {
    RuntimeRemove,
    SourceRemove,
    Finalize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TeardownState {
    kind: String,
    version: String,
    stackstead_id: String,
    runtime_token: String,
    phase: TeardownPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

impl TeardownState {
    pub(super) const fn phase(&self) -> TeardownPhase {
        self.phase
    }

    pub(super) const fn has_error(&self) -> bool {
        self.last_error.is_some()
    }
}

pub(super) fn read_teardown(
    manifest: &StacksteadManifest,
) -> anyhow::Result<Option<TeardownState>> {
    let path = teardown_path(manifest);
    let state: TeardownState = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("parse teardown state {}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if state.kind != "StacksteadTeardown"
        || state.version != "1"
        || state.stackstead_id != manifest.stackstead_id
        || state.runtime_token != manifest.runtime_token
    {
        anyhow::bail!(
            "teardown state {} is not bound to stackstead `{}` and its runtime token",
            path.display(),
            manifest.stackstead_id
        );
    }
    Ok(Some(state))
}

pub fn ensure_no_teardown(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if read_teardown(manifest)?.is_some() {
        anyhow::bail!(
            "stackstead `{}` has an incomplete teardown; retry `stackstead destroy {} --yes`",
            manifest.stackstead_id,
            manifest.stackstead_id
        );
    }
    Ok(())
}

pub(super) fn write_teardown(
    manifest: &StacksteadManifest,
    phase: TeardownPhase,
    last_error: Option<&str>,
) -> anyhow::Result<()> {
    write_json_atomic(
        &teardown_path(manifest),
        &TeardownState {
            kind: "StacksteadTeardown".into(),
            version: "1".into(),
            stackstead_id: manifest.stackstead_id.clone(),
            runtime_token: manifest.runtime_token.clone(),
            phase,
            last_error: last_error.map(command::redact),
        },
    )
}

pub(super) fn validate_recovery_source(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if source_cleanup_complete(manifest) {
        return Ok(());
    }
    if manifest.source_ownership == SourceOwnership::Stackstead
        && !git::is_registered_worktree(&manifest.repo_root, &manifest.worktree)?
    {
        return Ok(());
    }
    validate_pointer_binding(manifest)?;
    validate_source_binding(manifest)?;
    git::ensure_worktree_clean(&manifest.worktree)
}

pub(super) fn finish_source_cleanup(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if !source_cleanup_complete(manifest)
        && let Err(error) = remove_bound_source(manifest)
    {
        events::append(
            &manifest.event_log,
            events::EventType::SourceRemove,
            events::EventStatus::Failed,
            Some(&error.to_string()),
        )?;
        return Err(error);
    }
    if !source_cleanup_complete(manifest) {
        anyhow::bail!("source cleanup did not reach the manifest-owned final state");
    }
    events::append(
        &manifest.event_log,
        events::EventType::SourceRemove,
        events::EventStatus::Succeeded,
        None,
    )?;
    Ok(())
}

pub(super) fn source_cleanup_complete(manifest: &StacksteadManifest) -> bool {
    if manifest.pointer_file.exists() {
        return false;
    }
    match manifest.source_ownership {
        SourceOwnership::Stackstead => !manifest.worktree.exists(),
        SourceOwnership::External => {
            manifest.worktree.is_dir() && !manifest.worktree.join(".stackstead").exists()
        }
    }
}

fn remove_bound_source(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    match manifest.source_ownership {
        SourceOwnership::Stackstead => remove_owned_source(manifest)?,
        SourceOwnership::External => {
            paths::remove_generated_dir(&manifest.worktree, Path::new(".stackstead"))?;
        }
    }
    Ok(())
}

fn remove_owned_source(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if let Err(error) = git::remove_worktree(&manifest.repo_root, &manifest.worktree) {
        if git::is_registered_worktree(&manifest.repo_root, &manifest.worktree)? {
            return Err(error);
        }
        if manifest.worktree.exists() {
            std::fs::remove_dir_all(&manifest.worktree).map_err(|cleanup| {
                anyhow::anyhow!(
                    "Git unregistered worktree {} but could not remove its remaining files: {cleanup}; container-created files may need their ownership restored before rerunning destroy (initial Git error: {error})",
                    manifest.worktree.display()
                )
            })?;
        }
    }
    Ok(())
}

fn teardown_path(manifest: &StacksteadManifest) -> PathBuf {
    manifest.state_dir.join("teardown.json")
}

#[cfg(test)]
pub(super) fn validate_completed_source_cleanup(
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    if !read_teardown(manifest)?.is_some_and(|state| state.phase() == TeardownPhase::Finalize)
        || !source_cleanup_complete(manifest)
    {
        anyhow::bail!("destroy has not recorded completed source cleanup");
    }
    Ok(())
}
