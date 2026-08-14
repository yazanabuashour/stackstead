use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::{
    command, compose,
    config::{CommandConfig, HealthCheckConfig, PortExposure, PostgresConfig, StacksteadConfig},
    discovery::{self, Discovery},
    git, paths,
    slug::sanitize_slug,
    state::ProjectPaths,
};

use super::{
    types::{CurrentIdentity, ProjectRuntime},
    validation::{
        validate_durable_manifest_binding, validate_pointer_binding, validate_source_binding,
    },
};

pub fn init_with_compose_file(cwd: &Path, compose_file: Option<&Path>) -> anyhow::Result<PathBuf> {
    let repo_root = git::repo_root(cwd)?;
    let path = repo_root.join("stackstead.yaml");
    if path.exists() {
        anyhow::bail!(
            "{} already exists; refusing to overwrite it",
            path.display()
        );
    }
    let project = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("repository path is not valid UTF-8"))?;
    let project = sanitize_slug(project)?;
    let base = current_base(&repo_root)?;
    let plan = compose::plan_at(&repo_root, compose_file)?;
    let yaml = default_config(&project, &base, &plan)?;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    file.write_all(yaml.as_bytes())?;
    file.sync_all()?;
    Ok(path)
}

pub fn compose_plan(cwd: &Path) -> anyhow::Result<compose::ComposePlan> {
    compose_plan_with_file(cwd, None)
}

pub fn compose_plan_with_file(
    cwd: &Path,
    compose_file: Option<&Path>,
) -> anyhow::Result<compose::ComposePlan> {
    let repo_root = git::repo_root(cwd)?;
    let compose_file = configured_compose_file(&repo_root, compose_file)?;
    compose::plan_at(&repo_root, compose_file.as_deref())
}

pub fn compose_apply_with_file(
    cwd: &Path,
    compose_file: Option<&Path>,
) -> anyhow::Result<compose::ComposeApplyOutput> {
    let repo_root = git::repo_root(cwd)?;
    let compose_file = configured_compose_file(&repo_root, compose_file)?;
    compose::apply_at(&repo_root, compose_file.as_deref())
}

fn configured_compose_file(
    repo_root: &Path,
    requested: Option<&Path>,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(requested) = requested {
        return Ok(Some(requested.to_owned()));
    }
    let config_path = repo_root.join(crate::config::CONFIG_FILE);
    if !config_path.is_file() {
        return Ok(None);
    }
    let config = StacksteadConfig::load(&config_path)?;
    match config.runtime.files.as_slice() {
        [file] => Ok(Some(file.clone())),
        _ => Ok(None),
    }
}

pub fn current(cwd: &Path) -> anyhow::Result<CurrentIdentity> {
    let manifest = discovery::discover_stackstead(cwd)?;
    validate_durable_manifest_binding(&manifest)?;
    validate_pointer_binding(&manifest)?;
    let repo_root = git::primary_worktree(&manifest.worktree)?;
    let config = StacksteadConfig::load(&repo_root.join(crate::config::CONFIG_FILE))?;
    let state_root = config.validated_state_root(&repo_root)?;
    if manifest.repo_root != repo_root
        || manifest.project_state_root != state_root
        || manifest.project != config.project.name
    {
        anyhow::bail!(
            "current worktree identity does not match the primary project's configured state root"
        );
    }
    validate_source_binding(&manifest)?;
    Ok(CurrentIdentity {
        stackstead_id: manifest.stackstead_id.clone(),
        source_ownership: manifest.source_ownership,
        repo_root: manifest.repo_root.clone(),
        worktree: manifest.worktree.clone(),
        pointer: manifest.pointer_file.clone(),
    })
}

pub fn load_project(cwd: &Path) -> anyhow::Result<ProjectRuntime> {
    let discovered = discovery::discover(cwd)?;
    let (repo_root, state_root, project) = match &discovered {
        Discovery::Project { repo_root, .. } => {
            let config = StacksteadConfig::load(&repo_root.join("stackstead.yaml"))?;
            let state_root = config.validated_state_root(repo_root)?;
            let project = config.project.name.clone();
            return finish_project(config, repo_root, &state_root, &project);
        }
        Discovery::Stackstead {
            pointer, manifest, ..
        } => (
            pointer.repo_root.clone(),
            pointer.project_state_root.clone(),
            manifest.project.clone(),
        ),
    };
    let config = StacksteadConfig::load(&repo_root.join("stackstead.yaml"))?;
    finish_project(config, &repo_root, &state_root, &project)
}

fn finish_project(
    config: StacksteadConfig,
    repo_root: &Path,
    state_root: &Path,
    project: &str,
) -> anyhow::Result<ProjectRuntime> {
    let repo_root = paths::normalize_absolute(repo_root)?;
    let state_root = paths::normalize_absolute(state_root)?;
    if state_root.parent().is_none() {
        anyhow::bail!("state.root must not resolve to the filesystem root");
    }
    if state_root == repo_root {
        anyhow::bail!("state.root must not resolve to the repository root");
    }
    let paths = ProjectPaths::new(repo_root, state_root, project);
    Ok(ProjectRuntime { config, paths })
}

fn current_base(repo_root: &Path) -> anyhow::Result<String> {
    let branch = command::run(
        "git",
        &[
            "symbolic-ref".into(),
            "--quiet".into(),
            "--short".into(),
            "HEAD".into(),
        ],
        repo_root,
        &BTreeMap::new(),
    );
    let output = match branch {
        Ok(output) => output,
        Err(branch_error) => command::run(
            "git",
            &["rev-parse".into(), "--verify".into(), "HEAD".into()],
            repo_root,
            &BTreeMap::new(),
        )
        .with_context(|| {
            format!("cannot determine the current branch or commit: {branch_error}")
        })?,
    };
    Ok(String::from_utf8(output.stdout)?.trim().into())
}

pub(super) fn default_config(
    project: &str,
    base: &str,
    plan: &compose::ComposePlan,
) -> anyhow::Result<String> {
    let mut config = StacksteadConfig::default();
    config.project.name = project.into();
    config.source.base = base.into();
    config.runtime.files = vec![plan.file.clone()];
    add_ports_to_default_config(&mut config, plan);
    add_postgres_to_default_config(&mut config, plan);
    config.validate()?;
    Ok(serde_yaml::to_string(&config)?)
}

fn add_ports_to_default_config(config: &mut StacksteadConfig, plan: &compose::ComposePlan) {
    for port in &plan.ports {
        config.resources.ports.expose.insert(
            port.name.clone(),
            PortExposure {
                container: port.container_port,
                url: port.url.clone(),
            },
        );
        config
            .env
            .generate
            .insert(port.env.clone(), format!("{{{{ ports.{} }}}}", port.name));
        if let Some(url) = &port.url {
            config.health.checks.push(HealthCheckConfig {
                name: port.name.clone(),
                url: Some(url.clone()),
                expect_status: 200,
                command: CommandConfig::default(),
            });
        }
    }
    config
        .env
        .generate
        .insert("STACKSTEAD_ID".into(), "{{ stackstead.id }}".into());
}

fn add_postgres_to_default_config(config: &mut StacksteadConfig, plan: &compose::ComposePlan) {
    let Some(port) = plan
        .ports
        .iter()
        .find(|port| port.container_port == 5432 && port.name == port.service)
    else {
        return;
    };
    config.database.postgres = Some(PostgresConfig {
        strategy: crate::config::PostgresStrategy::default(),
        database: "app".into(),
        user: "app".into(),
        password: "app".into(),
        service: port.service.clone(),
        seed: CommandConfig::default(),
    });
    config.env.generate.insert(
        "DATABASE_URL".into(),
        format!(
            "postgres://app:app@127.0.0.1:{{{{ ports.{} }}}}/app",
            port.name
        ),
    );
}
