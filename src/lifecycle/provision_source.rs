use std::path::Path;

use crate::{
    events, git,
    manifest::{ComponentStatus, SourceOwnership, StacksteadManifest},
    paths,
    template::TemplateContext,
};

use super::{
    contract::{run_commands, write_contract},
    types::ProjectRuntime,
    validation::{validate_configured_ports, validate_pointer_binding, validate_source_binding},
};

pub(super) fn create_source_and_contract(
    config: &crate::config::StacksteadConfig,
    manifest: &mut StacksteadManifest,
    template_context: &TemplateContext,
) -> anyhow::Result<()> {
    create_or_verify_source(manifest)?;
    git::ensure_contract_on_revision(&manifest.worktree, &manifest.base, &config.runtime.files)?;
    validate_source_binding(manifest)?;
    validate_configured_ports(config, &manifest.worktree)?;
    let generated_dir = paths::safe_generated_path(&manifest.worktree, Path::new(".stackstead"))?;
    if generated_dir.exists() {
        anyhow::bail!(
            "source checkout already contains {}; Stackstead will not overwrite it",
            generated_dir.display()
        );
    }
    git::ensure_stackstead_excluded(&manifest.worktree)?;
    std::fs::create_dir_all(&generated_dir)?;
    let result = write_source_contract(config, manifest, template_context);
    if let Err(error) = result {
        if !manifest.pointer_file.is_file() {
            return match paths::remove_generated_dir(&manifest.worktree, Path::new(".stackstead")) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(anyhow::anyhow!(
                    "{error}; failed to remove partial generated contract: {cleanup}"
                )),
            };
        }
        return Err(error);
    }
    Ok(())
}

fn create_or_verify_source(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    match manifest.source_ownership {
        SourceOwnership::Stackstead => git::create_worktree(
            &manifest.repo_root,
            &manifest.worktree,
            &manifest.branch,
            &manifest.base,
        ),
        SourceOwnership::External => {
            let branch = git::registered_worktree_branch(&manifest.repo_root, &manifest.worktree)?;
            if branch != manifest.branch {
                anyhow::bail!(
                    "external worktree branch changed from `{}` to `{branch}` during adoption",
                    manifest.branch
                );
            }
            Ok(())
        }
    }
}

fn write_source_contract(
    config: &crate::config::StacksteadConfig,
    manifest: &mut StacksteadManifest,
    template_context: &TemplateContext,
) -> anyhow::Result<()> {
    write_contract(config, manifest, template_context)?;
    events::append(
        &manifest.event_log,
        create_event_type(manifest),
        events::EventStatus::Succeeded,
        None,
    )?;
    for event_type in [
        events::EventType::PointerGenerate,
        events::EventType::EnvironmentGenerate,
        events::EventType::ContextGenerate,
    ] {
        events::append(
            &manifest.event_log,
            event_type,
            events::EventStatus::Succeeded,
            None,
        )?;
    }
    let environment = manifest.trusted_environment(&manifest.validated_environment()?);
    run_commands(&config.hooks.post_create, &manifest.worktree, &environment)?;
    validate_source_binding(manifest)?;
    validate_pointer_binding(manifest)?;
    validate_configured_ports(config, &manifest.worktree)?;
    manifest.status.source = ComponentStatus::Ready;
    manifest.save_atomic()?;
    Ok(())
}

pub(super) fn create_event_type(manifest: &StacksteadManifest) -> events::EventType {
    if manifest.source_ownership == SourceOwnership::Stackstead {
        events::EventType::Create
    } else {
        events::EventType::Adopt
    }
}

pub(super) fn cleanup_failed_create(
    runtime: &ProjectRuntime,
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    if manifest.worktree.exists() {
        match manifest.source_ownership {
            SourceOwnership::Stackstead => {
                git::remove_worktree(&manifest.repo_root, &manifest.worktree)?;
            }
            SourceOwnership::External => {
                paths::remove_generated_dir(&manifest.worktree, Path::new(".stackstead"))?;
            }
        }
    }
    paths::remove_stackstead_root(manifest, &runtime.paths.state_root)
}
