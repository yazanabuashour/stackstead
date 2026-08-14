use std::path::{Path, PathBuf};

use crate::{
    config::StacksteadConfig, git, manifest::StacksteadManifest, paths, slug::sanitize_slug,
};

use super::{project::load_project, types::ProjectRuntime, validation::validate_configured_ports};

pub(super) struct PreparedProvision {
    pub runtime: ProjectRuntime,
    pub external_worktree: Option<(PathBuf, String)>,
    pub base_commit: String,
}

pub(super) fn prepare(
    cwd: &Path,
    external_worktree: Option<&Path>,
) -> anyhow::Result<PreparedProvision> {
    let runtime = load_project(cwd)?;
    runtime.config.validate_for_repo(&runtime.paths.repo_root)?;
    let canonical_git_root = git::repo_root(&runtime.paths.repo_root)?;
    if canonical_git_root != std::fs::canonicalize(&runtime.paths.repo_root)? {
        anyhow::bail!(
            "stackstead.yaml must be at the canonical Git repository root ({})",
            canonical_git_root.display()
        );
    }
    let external_worktree = canonicalize_external_worktree(&runtime, external_worktree)?;
    let base_commit =
        git::ensure_repository_ready(&runtime.paths.repo_root, &runtime.config.source.base)?;
    git::ensure_contract_on_revision(
        &runtime.paths.repo_root,
        &base_commit,
        &runtime.config.runtime.files,
    )?;
    validate_configured_ports(&runtime.config, &runtime.paths.repo_root)?;
    if let Some((worktree, _)) = &external_worktree {
        validate_external_worktree(&runtime, worktree, &base_commit)?;
    }
    Ok(PreparedProvision {
        runtime,
        external_worktree,
        base_commit,
    })
}

fn canonicalize_external_worktree(
    runtime: &ProjectRuntime,
    worktree: Option<&Path>,
) -> anyhow::Result<Option<(PathBuf, String)>> {
    worktree
        .map(|worktree| {
            let worktree = std::fs::canonicalize(worktree)?;
            let branch = git::registered_worktree_branch(&runtime.paths.repo_root, &worktree)?;
            Ok((worktree, branch))
        })
        .transpose()
}

fn validate_external_worktree(
    runtime: &ProjectRuntime,
    worktree: &Path,
    base_commit: &str,
) -> anyhow::Result<()> {
    let external_config = StacksteadConfig::load(&worktree.join("stackstead.yaml"))?;
    external_config.validate_for_repo(worktree)?;
    if external_config != runtime.config {
        anyhow::bail!(
            "external worktree {} has a different stackstead.yaml; merge the reviewed runtime contract before adoption",
            worktree.display()
        );
    }
    git::ensure_revision_ancestor(worktree, base_commit)?;
    git::ensure_contract_on_revision(worktree, base_commit, &runtime.config.runtime.files)?;
    validate_configured_ports(&runtime.config, worktree)?;
    let generated = paths::safe_generated_path(worktree, Path::new(".stackstead"))?;
    if generated.exists() {
        anyhow::bail!(
            "external worktree already contains {}; refusing to overwrite or remove existing tool state",
            generated.display()
        );
    }
    Ok(())
}

pub(super) fn validate_name(
    prepared: &PreparedProvision,
    name: &str,
    existing: &[StacksteadManifest],
) -> anyhow::Result<String> {
    let slug = sanitize_slug(name)?;
    if existing
        .iter()
        .any(|manifest| manifest.slug == slug || manifest.stackstead_id == slug)
    {
        anyhow::bail!(
            "stackstead identifier `{slug}` already exists as a slug or full ID; destroy it before recreating it"
        );
    }
    if let Some((worktree, _)) = &prepared.external_worktree
        && existing
            .iter()
            .any(|manifest| &manifest.worktree == worktree)
    {
        anyhow::bail!(
            "external worktree {} is already bound to a Stackstead manifest",
            worktree.display()
        );
    }
    Ok(slug)
}
