use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::{
    compose,
    config::StacksteadConfig,
    git,
    manifest::{StacksteadManifest, StacksteadPointer},
    paths,
    slug::make_stackstead_id,
    template::render_template,
};

use super::{contract::template_context, ensure_no_teardown, types::ProjectRuntime};

pub fn validate_manifest_binding(
    runtime: &ProjectRuntime,
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    if manifest.repo_root != runtime.paths.repo_root
        || manifest.project_state_root != runtime.paths.state_root
        || manifest.project != runtime.config.project.name
    {
        anyhow::bail!("manifest project identity does not match the discovered project");
    }
    validate_durable_manifest_binding(manifest)
}

pub(super) fn validate_durable_manifest_binding(
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    paths::validate_destroy_target(manifest, &manifest.project_state_root)?;
    validate_compose_project(&manifest.compose_project)?;
    let expected_id = make_stackstead_id(&manifest.slug, &manifest.short_id)?;
    if manifest.stackstead_id != expected_id {
        anyhow::bail!("manifest stackstead ID does not match its slug and short ID");
    }
    let expected_compose_project = format!("{}-{}", manifest.project, manifest.stackstead_id);
    if manifest.compose_project != expected_compose_project {
        anyhow::bail!(
            "manifest Compose project does not match the durable stackstead identity; refusing to target `{}`",
            manifest.compose_project
        );
    }
    if manifest.compose_files.is_empty() {
        anyhow::bail!("manifest has no Compose files");
    }
    if manifest.ports.keys().ne(manifest.container_ports.keys()) {
        anyhow::bail!("manifest host and container port service sets differ");
    }
    for file in &manifest.compose_files {
        validate_worktree_path(&manifest.worktree, file, "Compose")?;
    }
    validate_worktree_path(&manifest.worktree, &manifest.env_file, "environment")?;
    validate_worktree_path(&manifest.worktree, &manifest.agent_context, "agent context")?;
    let expected_pointer =
        paths::safe_generated_path(&manifest.worktree, Path::new(".stackstead/stackstead.json"))?;
    if manifest.pointer_file != expected_pointer
        || manifest.event_log != manifest.state_dir.join("events.jsonl")
    {
        anyhow::bail!(
            "manifest contract paths for {} do not match its durable layout",
            manifest.stackstead_id
        );
    }
    Ok(())
}

pub fn validate_pointer_binding(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    let pointer = StacksteadPointer::read(&manifest.pointer_file)?;
    if pointer.stackstead_id != manifest.stackstead_id
        || paths::normalize_absolute(&pointer.manifest)?
            != paths::normalize_absolute(&manifest.manifest_path())?
        || pointer.project != manifest.project
        || pointer.repo_root != manifest.repo_root
        || pointer.project_state_root != manifest.project_state_root
        || pointer.stackstead_root != manifest.stackstead_root
    {
        anyhow::bail!(
            "reciprocal pointer {} does not match manifest identity {}; refusing to use or delete either stackstead",
            manifest.pointer_file.display(),
            manifest.stackstead_id
        );
    }
    Ok(())
}

pub fn validate_current_contract(
    runtime: &ProjectRuntime,
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    validate_manifest_binding(runtime, manifest)?;
    ensure_no_teardown(manifest)?;
    let expected_compose_files = configured_compose_files(&runtime.config, &manifest.worktree)?;
    let expected_env = paths::safe_generated_path(&manifest.worktree, &runtime.config.env.file)?;
    let expected_context =
        paths::safe_generated_path(&manifest.worktree, &runtime.config.agent.context_file)?;
    let rendered_compose_project = render_template(
        &runtime.config.runtime.project_name_template,
        &template_context(manifest),
    )?;
    if manifest.compose_files != expected_compose_files
        || manifest.env_file != expected_env
        || manifest.agent_context != expected_context
        || rendered_compose_project != manifest.compose_project
    {
        anyhow::bail!(
            "current stackstead.yaml contract differs from {}; restore it or recreate the stackstead before regeneration",
            manifest.stackstead_id
        );
    }
    validate_contract_binding(&runtime.config, manifest)?;
    compose::validate_port_contract(
        &manifest.compose_files,
        &manifest.container_ports,
        &runtime.config.env.generate,
    )
}

pub fn validate_source_binding(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    let branch = git::registered_worktree_branch(&manifest.repo_root, &manifest.worktree)?;
    if branch != manifest.branch {
        anyhow::bail!(
            "worktree {} has checked-out branch `{branch}`, expected `{}` for {}; refusing to use the wrong source",
            manifest.worktree.display(),
            manifest.branch,
            manifest.stackstead_id
        );
    }
    git::ensure_revision_ancestor(&manifest.worktree, &manifest.base)?;
    Ok(())
}

pub(super) fn validate_configured_ports(
    config: &StacksteadConfig,
    worktree: &Path,
) -> anyhow::Result<()> {
    compose::validate_port_contract(
        &configured_compose_files(config, worktree)?,
        &configured_container_ports(config),
        &config.env.generate,
    )
}

pub(super) fn configured_compose_files(
    config: &StacksteadConfig,
    worktree: &Path,
) -> anyhow::Result<Vec<PathBuf>> {
    config
        .runtime
        .files
        .iter()
        .map(|file| paths::safe_generated_path(worktree, file))
        .collect()
}

pub(super) fn configured_container_ports(config: &StacksteadConfig) -> BTreeMap<String, u16> {
    config
        .resources
        .ports
        .expose
        .iter()
        .map(|(name, exposure)| (name.clone(), exposure.container))
        .collect()
}

pub(super) fn validate_compose_project(name: &str) -> anyhow::Result<()> {
    let mut characters = name.chars();
    if !characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        || !characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
    {
        anyhow::bail!(
            "Compose project name `{name}` must start with a lowercase letter or digit and contain only lowercase letters, digits, `-`, or `_`"
        );
    }
    Ok(())
}

pub(super) fn validate_contract_binding(
    config: &StacksteadConfig,
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    let configured_ports = configured_container_ports(config);
    if manifest.container_ports != configured_ports
        || manifest.ports.keys().ne(configured_ports.keys())
    {
        anyhow::bail!(
            "configured service/port contract differs from {}; recreate the stackstead to allocate a new durable contract",
            manifest.stackstead_id
        );
    }
    match (
        config.database.postgres.as_ref(),
        manifest.database.as_ref(),
    ) {
        (None, None) => {}
        (Some(config), Some(database))
            if config.service == database.service && config.database == database.database => {}
        _ => anyhow::bail!(
            "configured database contract differs from {}; recreate the stackstead",
            manifest.stackstead_id
        ),
    }
    Ok(())
}

fn validate_worktree_path(worktree: &Path, path: &Path, label: &str) -> anyhow::Result<()> {
    let relative = path.strip_prefix(worktree).map_err(|error| {
        anyhow::anyhow!(
            "manifest {label} path {} escapes worktree {}: {error}",
            path.display(),
            worktree.display()
        )
    })?;
    if paths::safe_generated_path(worktree, relative)? != path {
        anyhow::bail!(
            "manifest {label} path is not normalized: {}",
            path.display()
        );
    }
    Ok(())
}
