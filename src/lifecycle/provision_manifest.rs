use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use chrono::Utc;

use crate::{
    config::StacksteadConfig,
    manifest::{
        ComponentStatus, DatabaseState, ManifestStatus, SourceOwnership, StacksteadManifest,
        new_runtime_token,
    },
    paths,
    readiness::Contract,
    slug::{make_stackstead_id, new_short_id},
    template::{TemplateContext, render_template},
};

use super::{
    provision_plan::PreparedProvision,
    types::ProjectRuntime,
    validation::{configured_compose_files, configured_container_ports, validate_compose_project},
};

pub(super) struct ProvisionIdentity {
    slug: String,
    short_id: String,
    stackstead_id: String,
    runtime_token: String,
    worktree: PathBuf,
    branch: String,
    source_ownership: SourceOwnership,
}

pub(super) fn prepare_identity(
    prepared: &PreparedProvision,
    slug: String,
    existing: &[StacksteadManifest],
) -> anyhow::Result<ProvisionIdentity> {
    let (short_id, stackstead_id) = unique_id(&slug, existing)?;
    let stackstead_root = prepared
        .runtime
        .paths
        .project_state_dir
        .join(&stackstead_id);
    let (worktree, branch, source_ownership) = match &prepared.external_worktree {
        Some((worktree, branch)) => (worktree.clone(), branch.clone(), SourceOwnership::External),
        None => (
            stackstead_root.join("source"),
            slug.clone(),
            SourceOwnership::Stackstead,
        ),
    };
    Ok(ProvisionIdentity {
        slug,
        short_id,
        stackstead_id,
        runtime_token: new_runtime_token()?,
        worktree,
        branch,
        source_ownership,
    })
}

pub(super) fn build(
    prepared: &PreparedProvision,
    identity: ProvisionIdentity,
    ports: BTreeMap<String, u16>,
    port_lease_state_dir: Option<PathBuf>,
    existing: &[StacksteadManifest],
) -> anyhow::Result<(StacksteadManifest, TemplateContext)> {
    let stackstead_root = prepared
        .runtime
        .paths
        .project_state_dir
        .join(&identity.stackstead_id);
    if stackstead_root.exists() {
        anyhow::bail!("target already exists: {}", stackstead_root.display());
    }
    let state_dir = stackstead_root.join("state");
    let mut context = build_template_context(
        &prepared.runtime,
        &identity,
        &stackstead_root,
        &state_dir,
        &ports,
    );
    let (urls, compose_project) =
        resolve_urls_and_compose(prepared, &identity, &mut context, existing)?;
    let worktree = &identity.worktree;
    let compose_files = configured_compose_files(&prepared.runtime.config, worktree)?;
    let env_file = paths::safe_generated_path(worktree, &prepared.runtime.config.env.file)?;
    let agent_context =
        paths::safe_generated_path(worktree, &prepared.runtime.config.agent.context_file)?;
    let now = Utc::now();
    let database = database_manifest(&prepared.runtime.config, &ports)?;
    let manifest = StacksteadManifest {
        kind: "StacksteadManifest".into(),
        version: crate::manifest::MANIFEST_VERSION.into(),
        stackstead_id: identity.stackstead_id,
        slug: identity.slug,
        short_id: identity.short_id,
        runtime_token: identity.runtime_token,
        project: prepared.runtime.config.project.name.clone(),
        branch: identity.branch,
        base: prepared.base_commit.clone(),
        source_ownership: identity.source_ownership,
        repo_root: prepared.runtime.paths.repo_root.clone(),
        project_state_root: prepared.runtime.paths.state_root.clone(),
        stackstead_root,
        worktree: worktree.clone(),
        state_dir: state_dir.clone(),
        port_lease_state_dir,
        compose_project,
        compose_files,
        readiness: prepared.runtime.config.runtime.readiness.as_ref().map_or(
            Contract::Unconfigured {},
            |readiness| Contract::Declared {
                required: readiness.required.clone(),
                resolved: None,
            },
        ),
        ports,
        container_ports: configured_container_ports(&prepared.runtime.config),
        urls,
        env_file,
        agent_context,
        pointer_file: worktree.join(".stackstead/stackstead.json"),
        event_log: state_dir.join("events.jsonl"),
        env_keys: vec![],
        status: ManifestStatus::default(),
        database,
        created_at: now,
        updated_at: now,
    };
    Ok((manifest, context))
}

fn build_template_context(
    runtime: &ProjectRuntime,
    identity: &ProvisionIdentity,
    stackstead_root: &Path,
    state_dir: &Path,
    ports: &BTreeMap<String, u16>,
) -> TemplateContext {
    let mut context = TemplateContext::from([
        ("project.name".into(), runtime.config.project.name.clone()),
        ("stackstead.id".into(), identity.stackstead_id.clone()),
        ("stackstead.slug".into(), identity.slug.clone()),
        ("stackstead.short_id".into(), identity.short_id.clone()),
        (
            "paths.repo_root".into(),
            runtime.paths.repo_root.display().to_string(),
        ),
        (
            "paths.stackstead_root".into(),
            stackstead_root.display().to_string(),
        ),
        (
            "paths.worktree".into(),
            identity.worktree.display().to_string(),
        ),
        ("paths.state_dir".into(), state_dir.display().to_string()),
    ]);
    for (service, port) in ports {
        context.insert(format!("ports.{service}"), port.to_string());
    }
    context
}

fn resolve_urls_and_compose(
    prepared: &PreparedProvision,
    identity: &ProvisionIdentity,
    context: &mut TemplateContext,
    existing: &[StacksteadManifest],
) -> anyhow::Result<(BTreeMap<String, String>, String)> {
    let mut urls = BTreeMap::new();
    for (service, exposure) in &prepared.runtime.config.resources.ports.expose {
        if let Some(template) = &exposure.url {
            let url = render_template(template, context)?;
            context.insert(format!("urls.{service}"), url.clone());
            urls.insert(service.clone(), url);
        }
    }
    let compose_project = format!(
        "{}-{}",
        prepared.runtime.config.project.name, identity.stackstead_id
    );
    validate_compose_project(&compose_project)?;
    if let Some(owner) = existing
        .iter()
        .find(|manifest| manifest.compose_project == compose_project)
    {
        anyhow::bail!(
            "Compose project `{compose_project}` is already owned by {}; refusing to reuse its runtime identity",
            owner.stackstead_id
        );
    }
    Ok((urls, compose_project))
}

fn unique_id(slug: &str, existing: &[StacksteadManifest]) -> anyhow::Result<(String, String)> {
    for _ in 0..32 {
        let short_id = new_short_id()?;
        let stackstead_id = make_stackstead_id(slug, &short_id)?;
        if !existing.iter().any(|manifest| {
            manifest.stackstead_id == stackstead_id || manifest.slug == stackstead_id
        }) {
            return Ok((short_id, stackstead_id));
        }
    }
    anyhow::bail!("could not generate a unique stackstead id after 32 attempts")
}

fn database_manifest(
    config: &StacksteadConfig,
    allocated_ports: &BTreeMap<String, u16>,
) -> anyhow::Result<Option<DatabaseState>> {
    let Some(postgres) = &config.database.postgres else {
        return Ok(None);
    };
    let port = allocated_ports
        .get(&postgres.service)
        .copied()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "database.postgres.service `{}` must be present under resources.ports.expose",
                postgres.service
            )
        })?;
    Ok(Some(DatabaseState {
        strategy: "compose-volume".into(),
        service: postgres.service.clone(),
        host: "127.0.0.1".into(),
        port,
        database: postgres.database.clone(),
        seed_status: ComponentStatus::Unknown,
        last_seed_at: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::ReadinessConfig, readiness::Role, state::ProjectPaths,
        test_support::TestResultExt as _,
    };

    #[test]
    fn provisioning_copies_roles_without_resolving_or_inferring_them() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        for readiness in [
            None,
            Some(ReadinessConfig {
                required: BTreeMap::from([("Worker.api_1".into(), Role::Job)]),
            }),
        ] {
            let mut config = StacksteadConfig::default();
            config.project.name = "demo".into();
            config.runtime.readiness = readiness.clone();
            let prepared = PreparedProvision {
                runtime: ProjectRuntime {
                    config,
                    paths: ProjectPaths::new(
                        directory.path().join("repo"),
                        directory.path().join("state"),
                        "demo",
                    ),
                },
                external_worktree: None,
                base_commit: "base".into(),
            };
            let identity = prepare_identity(&prepared, "feature".into(), &[]).test()?;
            let (manifest, _) = build(&prepared, identity, BTreeMap::new(), None, &[]).test()?;
            assert_eq!(manifest.version, "3");
            assert_eq!(
                manifest.readiness.required(),
                readiness.as_ref().map(|config| &config.required)
            );
            assert!(manifest.readiness.resolved().is_none());
            manifest.readiness.validate().test()?;
        }
        Ok(())
    }
}
