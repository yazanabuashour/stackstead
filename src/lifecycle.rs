use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{
    command, compose,
    config::{
        CommandConfig, DependencyProvider, HealthCheckConfig, PortExposure, PostgresConfig,
        StacksteadConfig,
    },
    context, database,
    discovery::{self, Discovery},
    envfile, events, git, health,
    lease::{LeaseIdentity, PortLeaseStore},
    lock::{LockGuard, project_lock_path},
    manifest::{
        ComponentStatus, DatabaseState, ManifestStatus, POINTER_VERSION, SourceOwnership,
        StacksteadManifest, StacksteadPointer, new_runtime_token, write_json_atomic, write_pointer,
    },
    paths, ports,
    slug::{make_stackstead_id, new_short_id, sanitize_slug},
    state::ProjectPaths,
    template::{TemplateContext, render_template},
};
use anyhow::Context;
use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ProjectRuntime {
    pub config: StacksteadConfig,
    pub paths: ProjectPaths,
}

impl ProjectRuntime {
    pub fn resolve(&self, name: &str) -> anyhow::Result<StacksteadManifest> {
        let manifest = self.paths.resolve(name)?;
        validate_manifest_binding(self, &manifest)?;
        validate_pointer_binding(&manifest)?;
        ensure_no_teardown(&manifest)?;
        Ok(manifest)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TeardownPhase {
    RuntimeRemove,
    SourceRemove,
    Finalize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TeardownState {
    kind: String,
    version: String,
    stackstead_id: String,
    runtime_token: String,
    phase: TeardownPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InspectOutput {
    pub manifest: StacksteadManifest,
    pub live: LiveStatus,
    pub effective: EffectiveStatus,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LiveStatus {
    pub runtime_status: ComponentStatus,
    pub services: Vec<compose::ServiceObservation>,
    pub database_reachable: Option<bool>,
    pub database_status: Option<ComponentStatus>,
    pub health_healthy: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusBasis {
    Live,
    Recorded,
    Lifecycle,
}

impl std::fmt::Display for StatusBasis {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Live => "live",
            Self::Recorded => "recorded",
            Self::Lifecycle => "lifecycle",
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EffectiveComponent {
    pub status: ComponentStatus,
    pub basis: StatusBasis,
}

#[derive(Debug, Clone)]
pub struct EffectiveStatus {
    pub phase: &'static str,
    pub recorded_at: chrono::DateTime<Utc>,
    pub observed_at: chrono::DateTime<Utc>,
    pub runtime: EffectiveComponent,
    pub database: Option<EffectiveComponent>,
    pub health: EffectiveComponent,
}

#[derive(Default)]
pub(crate) struct UpTimings {
    pub dependencies: Duration,
    pub runtime: Duration,
    pub database: Option<Duration>,
    pub seed: Option<Duration>,
    pub hooks: Option<Duration>,
    pub health: Option<Duration>,
    pub total: Duration,
}

pub(crate) struct UpOutcome {
    pub manifest: StacksteadManifest,
    pub timings: UpTimings,
    pub mutation_lock: LockGuard,
    pub run_lease: LockGuard,
}

pub(crate) struct CreateOutcome {
    pub manifest: StacksteadManifest,
    pub mutation_lock: LockGuard,
}

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

pub fn load_project(cwd: &Path) -> anyhow::Result<ProjectRuntime> {
    let discovered = discovery::discover(cwd)?;
    let (repo_root, state_root, project) = match &discovered {
        Discovery::Project { repo_root, .. } => {
            let config = StacksteadConfig::load(&repo_root.join("stackstead.yaml"))?;
            let state_root = config.validated_state_root(repo_root)?;
            let project = config.project.name.clone();
            return finish_project(config, repo_root.clone(), state_root, project);
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
    finish_project(config, repo_root, state_root, project)
}

fn finish_project(
    config: StacksteadConfig,
    repo_root: PathBuf,
    state_root: PathBuf,
    project: String,
) -> anyhow::Result<ProjectRuntime> {
    config.validate()?;
    let repo_root = paths::normalize_absolute(&repo_root)?;
    let state_root = paths::normalize_absolute(&state_root)?;
    if state_root.parent().is_none() {
        anyhow::bail!("state.root must not resolve to the filesystem root");
    }
    if state_root == repo_root {
        anyhow::bail!("state.root must not resolve to the repository root");
    }
    let paths = ProjectPaths::new(repo_root, state_root, &project);
    Ok(ProjectRuntime { config, paths })
}

pub fn create(cwd: &Path, name: &str) -> anyhow::Result<StacksteadManifest> {
    Ok(provision(cwd, name, None)?.manifest)
}

pub(crate) fn create_for_launch(cwd: &Path, name: &str) -> anyhow::Result<CreateOutcome> {
    provision(cwd, name, None)
}

pub fn adopt(cwd: &Path, name: &str, worktree: &Path) -> anyhow::Result<StacksteadManifest> {
    Ok(provision(cwd, name, Some(worktree))?.manifest)
}

fn provision(
    cwd: &Path,
    name: &str,
    external_worktree: Option<&Path>,
) -> anyhow::Result<CreateOutcome> {
    let runtime = load_project(cwd)?;
    runtime.config.validate_for_repo(&runtime.paths.repo_root)?;
    let canonical_git_root = git::repo_root(&runtime.paths.repo_root)?;
    if canonical_git_root != std::fs::canonicalize(&runtime.paths.repo_root)? {
        anyhow::bail!(
            "stackstead.yaml must be at the canonical Git repository root ({})",
            canonical_git_root.display()
        );
    }
    let external_worktree = match external_worktree {
        Some(worktree) => {
            let worktree = std::fs::canonicalize(worktree)?;
            let branch = git::registered_worktree_branch(&runtime.paths.repo_root, &worktree)?;
            Some((worktree, branch))
        }
        None => None,
    };
    let base_commit =
        git::ensure_repository_ready(&runtime.paths.repo_root, &runtime.config.source.base)?;
    git::ensure_contract_on_revision(
        &runtime.paths.repo_root,
        &base_commit,
        &runtime.config.runtime.files,
    )?;
    validate_configured_ports(&runtime.config, &runtime.paths.repo_root)?;
    if let Some((worktree, _)) = &external_worktree {
        let external_config = StacksteadConfig::load(&worktree.join("stackstead.yaml"))?;
        external_config.validate_for_repo(worktree)?;
        if external_config != runtime.config {
            anyhow::bail!(
                "external worktree {} has a different stackstead.yaml; merge the reviewed runtime contract before adoption",
                worktree.display()
            );
        }
        git::ensure_revision_ancestor(worktree, &base_commit)?;
        git::ensure_contract_on_revision(worktree, &base_commit, &runtime.config.runtime.files)?;
        validate_configured_ports(&runtime.config, worktree)?;
        let generated = paths::safe_generated_path(worktree, Path::new(".stackstead"))?;
        if generated.exists() {
            anyhow::bail!(
                "external worktree already contains {}; refusing to overwrite or remove existing tool state",
                generated.display()
            );
        }
    }
    std::fs::create_dir_all(&runtime.paths.project_state_dir)?;
    let _project_lock = LockGuard::acquire(
        &project_lock_path(&runtime.paths.project_state_dir),
        "project",
    )?;

    let slug = sanitize_slug(name)?;
    let existing = runtime.paths.manifests()?;
    if existing
        .iter()
        .any(|manifest| manifest.slug == slug || manifest.stackstead_id == slug)
    {
        anyhow::bail!(
            "stackstead identifier `{slug}` already exists as a slug or full ID; destroy it before recreating it"
        );
    }
    if let Some((worktree, _)) = &external_worktree
        && existing
            .iter()
            .any(|manifest| &manifest.worktree == worktree)
    {
        anyhow::bail!(
            "external worktree {} is already bound to a Stackstead manifest",
            worktree.display()
        );
    }
    let (short_id, stackstead_id) = unique_id(&slug, &existing)?;
    let runtime_token = new_runtime_token()?;
    let service_names = runtime.config.service_names();
    let port_lease_store = if service_names.is_empty() {
        None
    } else {
        Some(PortLeaseStore::for_current_user()?)
    };
    let port_lease_state_dir = port_lease_store
        .as_ref()
        .map(|store| store.state_dir().to_path_buf());
    let mut port_leases = port_lease_store
        .as_ref()
        .map(PortLeaseStore::transaction)
        .transpose()?;
    let mut used_ports = existing
        .iter()
        .flat_map(|manifest| manifest.ports.values().copied())
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(leases) = &port_leases {
        used_ports.extend(leases.used_ports());
    }
    let allocation = ports::allocate_ports(
        runtime.config.resources.ports.base,
        runtime.config.resources.ports.stride,
        &service_names,
        &used_ports,
    )?;

    let stackstead_root = runtime.paths.project_state_dir.join(&stackstead_id);
    let (worktree, branch, source_ownership) = match external_worktree {
        Some((worktree, branch)) => (worktree, branch, SourceOwnership::External),
        None => (
            stackstead_root.join("source"),
            slug.clone(),
            SourceOwnership::Stackstead,
        ),
    };
    let state_dir = stackstead_root.join("state");
    if stackstead_root.exists() {
        anyhow::bail!("target already exists: {}", stackstead_root.display());
    }
    let mut template_context = TemplateContext::from([
        ("project.name".into(), runtime.config.project.name.clone()),
        ("stackstead.id".into(), stackstead_id.clone()),
        ("stackstead.slug".into(), slug.clone()),
        ("stackstead.short_id".into(), short_id.clone()),
        (
            "paths.repo_root".into(),
            runtime.paths.repo_root.display().to_string(),
        ),
        (
            "paths.stackstead_root".into(),
            stackstead_root.display().to_string(),
        ),
        ("paths.worktree".into(), worktree.display().to_string()),
        ("paths.state_dir".into(), state_dir.display().to_string()),
    ]);
    for (service, port) in &allocation.ports {
        template_context.insert(format!("ports.{service}"), port.to_string());
    }
    let mut urls = BTreeMap::new();
    for (service, exposure) in &runtime.config.resources.ports.expose {
        if let Some(template) = &exposure.url {
            let url = render_template(template, &template_context)?;
            template_context.insert(format!("urls.{service}"), url.clone());
            urls.insert(service.clone(), url);
        }
    }
    let configured_compose_project = render_template(
        &runtime.config.runtime.project_name_template,
        &template_context,
    )?;
    let compose_project = format!("{}-{stackstead_id}", runtime.config.project.name);
    if configured_compose_project != compose_project {
        anyhow::bail!(
            "runtime.project_name_template must render the durable identity `{compose_project}`; use `{{{{ project.name }}}}-{{{{ stackstead.id }}}}`"
        );
    }
    validate_compose_project(&compose_project)?;
    if let Some(owner) = existing
        .iter()
        .find(|manifest| manifest.compose_project == compose_project)
    {
        anyhow::bail!(
            "Compose project `{compose_project}` is already owned by {}; make runtime.project_name_template include a stackstead-unique value",
            owner.stackstead_id
        );
    }
    let compose_files = configured_compose_files(&runtime.config, &worktree)?;
    let env_file = paths::safe_generated_path(&worktree, &runtime.config.env.file)?;
    let agent_context = paths::safe_generated_path(&worktree, &runtime.config.agent.context_file)?;
    let now = Utc::now();
    let database = database_manifest(&runtime.config, &allocation.ports)?;
    std::fs::create_dir_all(state_dir.join("logs"))?;
    let cell_lock = LockGuard::acquire(&state_dir.join("lock"), "stackstead")?;
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state_dir.join("run.lock"))?;
    let mut manifest = StacksteadManifest {
        kind: "StacksteadManifest".into(),
        version: crate::manifest::MANIFEST_VERSION.into(),
        stackstead_id: stackstead_id.clone(),
        slug: slug.clone(),
        short_id,
        runtime_token,
        project: runtime.config.project.name.clone(),
        branch,
        base: base_commit,
        source_ownership,
        repo_root: runtime.paths.repo_root.clone(),
        project_state_root: runtime.paths.state_root.clone(),
        stackstead_root: stackstead_root.clone(),
        worktree: worktree.clone(),
        state_dir: state_dir.clone(),
        port_lease_state_dir,
        compose_project,
        compose_files,
        ports: allocation.ports,
        container_ports: configured_container_ports(&runtime.config),
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
    if let Err(error) = manifest.save_atomic() {
        let cleanup = paths::remove_stackstead_root(&manifest, &runtime.paths.state_root);
        drop(cell_lock);
        return match cleanup {
            Ok(()) => Err(error),
            Err(cleanup) => Err(anyhow::anyhow!(
                "{error}; failed to clean state after initial manifest persistence failed: {cleanup}"
            )),
        };
    }
    let leased_ports = manifest.ports.values().copied().collect();
    if let Some(leases) = &mut port_leases
        && let Err(error) = leases.reserve(
            &manifest.runtime_token,
            &LeaseIdentity::new(&manifest.stackstead_id, &manifest.project),
            &leased_ports,
        )
    {
        drop(port_leases);
        let lease_cleanup = release_port_leases_after_destroy(&manifest);
        let cleanup = if lease_cleanup.is_ok() {
            cleanup_failed_create(&runtime, &manifest)
        } else {
            Err(anyhow::anyhow!(
                "skipped to retain recovery state after ambiguous port lease reservation"
            ))
        };
        return match (lease_cleanup, cleanup) {
            (Ok(()), Ok(())) => Err(error),
            (lease_cleanup, cleanup) => Err(anyhow::anyhow!(
                "{error}; failed to reconcile partial create: port lease cleanup={}; source/state cleanup={}",
                lease_cleanup.map_or_else(|error| error.to_string(), |_| "ok".into()),
                cleanup.map_or_else(|error| error.to_string(), |_| "ok".into())
            )),
        };
    }
    drop(port_leases);
    if let Err(error) =
        create_source_and_contract(&runtime.config, &mut manifest, &template_context)
    {
        if manifest.pointer_file.is_file() {
            let event_type = if manifest.source_ownership == SourceOwnership::Stackstead {
                events::EventType::Create
            } else {
                events::EventType::Adopt
            };
            let _ = events::append(
                &manifest.event_log,
                event_type,
                events::EventStatus::Failed,
                Some(&error.to_string()),
            );
            drop(cell_lock);
            return Err(error);
        }
        let lease_cleanup = release_port_leases(&manifest);
        let cleanup = if lease_cleanup.is_ok() {
            cleanup_failed_create(&runtime, &manifest)
        } else {
            Err(anyhow::anyhow!(
                "skipped to retain recovery state after port lease cleanup failed"
            ))
        };
        return match (cleanup, lease_cleanup) {
            (Ok(()), Ok(())) => Err(error),
            (cleanup, lease_cleanup) => Err(anyhow::anyhow!(
                "{error}; failed to roll back partial create: source/state cleanup={}; port lease cleanup={}",
                cleanup.map_or_else(|error| error.to_string(), |_| "ok".into()),
                lease_cleanup.map_or_else(|error| error.to_string(), |_| "ok".into())
            )),
        };
    }
    Ok(CreateOutcome {
        manifest,
        mutation_lock: cell_lock,
    })
}

fn create_source_and_contract(
    config: &StacksteadConfig,
    manifest: &mut StacksteadManifest,
    template_context: &TemplateContext,
) -> anyhow::Result<()> {
    match manifest.source_ownership {
        SourceOwnership::Stackstead => git::create_worktree(
            &manifest.repo_root,
            &manifest.worktree,
            &manifest.branch,
            &manifest.base,
        )?,
        SourceOwnership::External => {
            let branch = git::registered_worktree_branch(&manifest.repo_root, &manifest.worktree)?;
            if branch != manifest.branch {
                anyhow::bail!(
                    "external worktree branch changed from `{}` to `{branch}` during adoption",
                    manifest.branch
                );
            }
        }
    }
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
    std::fs::create_dir_all(generated_dir)?;
    let result = (|| {
        write_contract(config, manifest, template_context)?;
        let event_type = if manifest.source_ownership == SourceOwnership::Stackstead {
            events::EventType::Create
        } else {
            events::EventType::Adopt
        };
        events::append(
            &manifest.event_log,
            event_type,
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
    })();
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

fn cleanup_failed_create(
    runtime: &ProjectRuntime,
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    if manifest.worktree.exists() {
        match manifest.source_ownership {
            SourceOwnership::Stackstead => {
                git::remove_worktree(&manifest.repo_root, &manifest.worktree)?
            }
            SourceOwnership::External => {
                paths::remove_generated_dir(&manifest.worktree, Path::new(".stackstead"))?
            }
        }
    }
    paths::remove_stackstead_root(manifest, &runtime.paths.state_root)
}

pub fn up(cwd: &Path, name: &str) -> anyhow::Result<UpOutcome> {
    up_with_lock(cwd, name, None)
}

pub(crate) fn up_after_create(
    cwd: &Path,
    name: &str,
    mutation_lock: LockGuard,
) -> anyhow::Result<UpOutcome> {
    up_with_lock(cwd, name, Some(mutation_lock))
}

fn up_with_lock(
    cwd: &Path,
    name: &str,
    mutation_lock: Option<LockGuard>,
) -> anyhow::Result<UpOutcome> {
    let total_started = Instant::now();
    let mut timings = UpTimings::default();
    let runtime = load_project(cwd)?;
    let mut manifest = runtime.resolve(name)?;
    let mutation_lock = match mutation_lock {
        Some(lock) => lock,
        None => LockGuard::acquire_existing(&manifest.state_dir.join("lock"), "stackstead")?,
    };
    let run_lease = LockGuard::acquire_existing(
        &manifest.state_dir.join("run.lock"),
        "active stackstead agent",
    )?;
    manifest = StacksteadManifest::read(&manifest.manifest_path())?;
    validate_current_contract(&runtime, &manifest)?;
    validate_pointer_binding(&manifest)?;
    validate_source_binding(&manifest)?;
    verify_port_leases(&manifest)?;
    manifest.status.health = ComponentStatus::Unknown;
    manifest.status.database = ComponentStatus::Unknown;
    manifest.save_atomic()?;
    let template_context = template_context(&manifest);
    write_contract(&runtime.config, &mut manifest, &template_context)?;
    let environment = manifest.trusted_environment(&manifest.validated_environment()?);

    let phase_started = Instant::now();
    events::append(
        &manifest.event_log,
        events::EventType::DependenciesInstall,
        events::EventStatus::Started,
        None,
    )?;
    if let Err(error) = install_dependencies(&runtime.config, &manifest, &environment) {
        manifest.status.dependencies = ComponentStatus::Failed;
        manifest.save_atomic()?;
        events::append(
            &manifest.event_log,
            events::EventType::DependenciesInstall,
            events::EventStatus::Failed,
            Some(&error.to_string()),
        )?;
        return Err(error);
    }
    manifest.status.dependencies = ComponentStatus::Ready;
    timings.dependencies = phase_started.elapsed();
    events::append(
        &manifest.event_log,
        events::EventType::DependenciesInstall,
        events::EventStatus::Succeeded,
        None,
    )?;
    manifest.save_atomic()?;
    let phase_started = Instant::now();
    run_commands(
        &runtime.config.hooks.pre_up,
        &manifest.worktree,
        &environment,
    )?;
    if !runtime.config.hooks.pre_up.is_empty() {
        timings.hooks = Some(phase_started.elapsed());
    }
    validate_source_binding(&manifest)?;
    validate_pointer_binding(&manifest)?;
    validate_configured_ports(&runtime.config, &manifest.worktree)?;

    let phase_started = Instant::now();
    events::append(
        &manifest.event_log,
        events::EventType::RuntimeStart,
        events::EventStatus::Started,
        None,
    )?;
    if let Err(error) = compose::up(&manifest) {
        manifest.status.runtime = ComponentStatus::Failed;
        manifest.save_atomic()?;
        events::append(
            &manifest.event_log,
            events::EventType::RuntimeStart,
            events::EventStatus::Failed,
            Some(&error.to_string()),
        )?;
        return Err(error);
    }
    manifest.status.runtime = ComponentStatus::Running;
    timings.runtime = phase_started.elapsed();
    events::append(
        &manifest.event_log,
        events::EventType::RuntimeStart,
        events::EventStatus::Succeeded,
        None,
    )?;
    manifest.save_atomic()?;

    if let Some(config) = runtime.config.database.postgres.as_ref() {
        let phase_started = Instant::now();
        events::append(
            &manifest.event_log,
            events::EventType::DatabaseWait,
            events::EventStatus::Started,
            None,
        )?;
        let readiness = {
            let database_state = manifest
                .database
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("validated database contract has no state"))?;
            let container_port = manifest
                .container_ports
                .get(&config.service)
                .copied()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "validated database contract has no container port for `{}`",
                        config.service
                    )
                })?;
            compose::ensure_endpoint_published(
                &manifest,
                &config.service,
                container_port,
                &database_state.host,
                database_state.port,
            )
            .and_then(|()| {
                database::wait_until_postgres_ready(
                    &database_state.host,
                    database_state.port,
                    Duration::from_secs(30),
                    || {
                        compose::postgres_is_ready(
                            &manifest,
                            &config.service,
                            &config.user,
                            &config.database,
                        )
                    },
                )
            })
        };
        if let Err(error) = readiness {
            manifest.status.database = ComponentStatus::Unreachable;
            manifest.save_atomic()?;
            events::append(
                &manifest.event_log,
                events::EventType::DatabaseWait,
                events::EventStatus::Failed,
                Some(&error.to_string()),
            )?;
            return Err(error);
        }
        manifest.status.database = ComponentStatus::Reachable;
        timings.database = Some(phase_started.elapsed());
        events::append(
            &manifest.event_log,
            events::EventType::DatabaseWait,
            events::EventStatus::Succeeded,
            None,
        )?;
        manifest.save_atomic()?;
        if !config.seed.command.trim().is_empty() {
            let phase_started = Instant::now();
            events::append(
                &manifest.event_log,
                events::EventType::DatabaseSeed,
                events::EventStatus::Started,
                None,
            )?;
            if let Err(error) = command::run_configured(
                &config.seed.command,
                config.seed.shell,
                &manifest.worktree,
                &environment,
            ) {
                manifest
                    .database
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("validated database contract has no state"))?
                    .seed_status = ComponentStatus::Failed;
                manifest.save_atomic()?;
                events::append(
                    &manifest.event_log,
                    events::EventType::DatabaseSeed,
                    events::EventStatus::Failed,
                    Some(&error.to_string()),
                )?;
                return Err(error);
            }
            let database_state = manifest
                .database
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("validated database contract has no state"))?;
            database_state.seed_status = ComponentStatus::Ready;
            database_state.last_seed_at = Some(Utc::now());
            timings.seed = Some(phase_started.elapsed());
            events::append(
                &manifest.event_log,
                events::EventType::DatabaseSeed,
                events::EventStatus::Succeeded,
                None,
            )?;
            manifest.save_atomic()?;
        }
    }
    let phase_started = Instant::now();
    run_commands(
        &runtime.config.hooks.post_up,
        &manifest.worktree,
        &environment,
    )?;
    if !runtime.config.hooks.post_up.is_empty() {
        timings.hooks = Some(timings.hooks.unwrap_or_default() + phase_started.elapsed());
    }
    validate_source_binding(&manifest)?;
    validate_pointer_binding(&manifest)?;
    validate_configured_ports(&runtime.config, &manifest.worktree)?;
    if !runtime.config.health.checks.is_empty() {
        let phase_started = Instant::now();
        events::append(
            &manifest.event_log,
            events::EventType::HealthWait,
            events::EventStatus::Started,
            None,
        )?;
        if let Err(error) = health::wait(&runtime.config.health, &manifest, &environment) {
            manifest.status.health = ComponentStatus::Failed;
            manifest.save_atomic()?;
            events::append(
                &manifest.event_log,
                events::EventType::HealthWait,
                events::EventStatus::Failed,
                Some(&error.to_string()),
            )?;
            return Err(error);
        }
        manifest.status.health = ComponentStatus::Ready;
        timings.health = Some(phase_started.elapsed());
        events::append(
            &manifest.event_log,
            events::EventType::HealthWait,
            events::EventStatus::Succeeded,
            None,
        )?;
    } else {
        manifest.status.health = ComponentStatus::Unknown;
    }
    manifest.save_atomic()?;
    timings.total = total_started.elapsed();
    Ok(UpOutcome {
        manifest,
        timings,
        mutation_lock,
        run_lease,
    })
}

pub fn stop(cwd: &Path, name: &str) -> anyhow::Result<StacksteadManifest> {
    let runtime = load_project(cwd)?;
    let mut manifest = runtime.resolve(name)?;
    let _lock = LockGuard::acquire_existing(&manifest.state_dir.join("lock"), "stackstead")?;
    let _run_lease = LockGuard::acquire_existing(
        &manifest.state_dir.join("run.lock"),
        "active stackstead agent",
    )?;
    manifest = StacksteadManifest::read(&manifest.manifest_path())?;
    validate_manifest_binding(&runtime, &manifest)?;
    ensure_no_teardown(&manifest)?;
    validate_pointer_binding(&manifest)?;
    validate_source_binding(&manifest)?;
    verify_port_leases(&manifest)?;
    compose::stop(&manifest)?;
    manifest.status.runtime = ComponentStatus::Stopped;
    manifest.status.database = ComponentStatus::Unknown;
    manifest.status.health = ComponentStatus::Unknown;
    manifest.save_atomic()?;
    events::append(
        &manifest.event_log,
        events::EventType::RuntimeStop,
        events::EventStatus::Succeeded,
        None,
    )?;
    Ok(manifest)
}

pub fn destroy(cwd: &Path, name: &str) -> anyhow::Result<StacksteadManifest> {
    let runtime = load_project(cwd)?;
    let mut manifest = resolve_destroy_manifest(&runtime, name)?;
    let lock = LockGuard::acquire_existing(&manifest.state_dir.join("lock"), "stackstead")?;
    let _run_lease = LockGuard::acquire_existing(
        &manifest.state_dir.join("run.lock"),
        "active stackstead agent",
    )?;
    manifest = StacksteadManifest::read(&manifest.manifest_path())?;
    validate_manifest_binding(&runtime, &manifest)?;
    let pending = pending_create(&manifest);
    if pending && !manifest.pointer_file.exists() {
        release_port_leases_after_destroy(&manifest)?;
        cleanup_failed_create(&runtime, &manifest)?;
        drop(_run_lease);
        drop(lock);
        return Ok(manifest);
    }
    let teardown = read_teardown(&manifest)?;
    if teardown
        .as_ref()
        .is_none_or(|state| state.phase != TeardownPhase::Finalize)
    {
        verify_port_leases(&manifest)?;
    }
    let mut phase = if let Some(teardown) = teardown {
        teardown.phase
    } else {
        validate_pointer_binding(&manifest)?;
        validate_source_binding(&manifest)?;
        git::ensure_worktree_clean(&manifest.worktree)?;
        events::append(
            &manifest.event_log,
            events::EventType::Destroy,
            events::EventStatus::Started,
            None,
        )?;
        let environment = manifest.trusted_environment(&manifest.validated_environment()?);
        run_commands(
            &runtime.config.hooks.pre_destroy,
            &manifest.worktree,
            &environment,
        )?;
        write_teardown(&manifest, TeardownPhase::RuntimeRemove, None)?;
        TeardownPhase::RuntimeRemove
    };
    if phase == TeardownPhase::RuntimeRemove {
        validate_source_binding(&manifest)?;
        validate_pointer_binding(&manifest)?;
        git::ensure_worktree_clean(&manifest.worktree)?;
        events::append(
            &manifest.event_log,
            events::EventType::RuntimeRemove,
            events::EventStatus::Started,
            None,
        )?;
        let removal = (|| {
            compose::stop(&manifest)?;
            compose::down_volumes(&manifest)
        })();
        if let Err(error) = removal {
            write_teardown(
                &manifest,
                TeardownPhase::RuntimeRemove,
                Some(&error.to_string()),
            )?;
            events::append(
                &manifest.event_log,
                events::EventType::RuntimeRemove,
                events::EventStatus::Failed,
                Some(&error.to_string()),
            )?;
            return Err(error);
        }
        events::append(
            &manifest.event_log,
            events::EventType::RuntimeRemove,
            events::EventStatus::Succeeded,
            None,
        )?;
        write_teardown(&manifest, TeardownPhase::SourceRemove, None)?;
        phase = TeardownPhase::SourceRemove;
    }
    if phase == TeardownPhase::SourceRemove {
        validate_recovery_source(&manifest)?;
        events::append(
            &manifest.event_log,
            events::EventType::SourceRemove,
            events::EventStatus::Started,
            None,
        )?;
        let cleanup = match finish_source_cleanup(&manifest) {
            Ok(()) => Ok(()),
            Err(initial) if manifest.source_ownership == SourceOwnership::Stackstead => {
                compose::prepare_owned_source_removal(&manifest)
                    .with_context(|| {
                        format!("source cleanup failed before ownership repair: {initial}")
                    })
                    .and_then(|()| finish_source_cleanup(&manifest))
            }
            Err(error) => Err(error),
        };
        if let Err(error) = cleanup {
            write_teardown(
                &manifest,
                TeardownPhase::SourceRemove,
                Some(&error.to_string()),
            )?;
            return Err(error);
        }
        write_teardown(&manifest, TeardownPhase::Finalize, None)?;
        phase = TeardownPhase::Finalize;
    }
    debug_assert_eq!(phase, TeardownPhase::Finalize);
    if !source_cleanup_complete(&manifest) {
        anyhow::bail!("teardown reached finalize before source cleanup completed");
    }
    finalize_destroy(&runtime, manifest, lock)
}

fn finalize_destroy(
    runtime: &ProjectRuntime,
    manifest: StacksteadManifest,
    lock: LockGuard,
) -> anyhow::Result<StacksteadManifest> {
    compose::remove_runtime_claim(&manifest).context("remove Compose runtime ownership claim")?;
    events::append(
        &manifest.event_log,
        events::EventType::Destroy,
        events::EventStatus::Succeeded,
        None,
    )
    .context("record completed destroy before final cleanup")?;
    release_port_leases_after_destroy(&manifest).context("release global port leases")?;
    paths::remove_stackstead_root(&manifest, &runtime.paths.state_root)
        .context("remove final Stackstead state root")?;
    drop(lock);
    Ok(manifest)
}

pub(crate) fn verify_port_leases(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if manifest.ports.is_empty() {
        return Ok(());
    }
    manifest_port_lease_store(manifest)?.transaction()?.verify(
        &manifest.runtime_token,
        &LeaseIdentity::new(&manifest.stackstead_id, &manifest.project),
        &manifest.ports.values().copied().collect(),
    )
}

fn release_port_leases(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if manifest.ports.is_empty() {
        return Ok(());
    }
    manifest_port_lease_store(manifest)?.transaction()?.release(
        &manifest.runtime_token,
        &LeaseIdentity::new(&manifest.stackstead_id, &manifest.project),
        &manifest.ports.values().copied().collect(),
    )
}

fn release_port_leases_after_destroy(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if manifest.ports.is_empty() {
        return Ok(());
    }
    manifest_port_lease_store(manifest)?
        .transaction()?
        .release_if_owned_or_absent(
            &manifest.runtime_token,
            &LeaseIdentity::new(&manifest.stackstead_id, &manifest.project),
            &manifest.ports.values().copied().collect(),
        )
}

fn manifest_port_lease_store(manifest: &StacksteadManifest) -> anyhow::Result<PortLeaseStore> {
    manifest
        .port_lease_state_dir
        .as_ref()
        .map(|path| PortLeaseStore::at(path.clone()))
        .ok_or_else(|| anyhow::anyhow!("manifest is missing its durable port lease registry path"))
}

fn remove_bound_source(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    match manifest.source_ownership {
        SourceOwnership::Stackstead => {
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
        }
        SourceOwnership::External => {
            paths::remove_generated_dir(&manifest.worktree, Path::new(".stackstead"))?
        }
    }
    Ok(())
}

pub fn resolve_destroy(cwd: &Path, name: &str) -> anyhow::Result<StacksteadManifest> {
    resolve_destroy_manifest(&load_project(cwd)?, name)
}

fn resolve_destroy_manifest(
    runtime: &ProjectRuntime,
    name: &str,
) -> anyhow::Result<StacksteadManifest> {
    let manifest = runtime.paths.resolve(name)?;
    validate_manifest_binding(runtime, &manifest)?;
    let pending = pending_create(&manifest);
    if pending {
        if manifest.pointer_file.exists() {
            validate_pointer_binding(&manifest)?;
        }
        return Ok(manifest);
    }
    if read_teardown(&manifest)?.is_some() {
        return Ok(manifest);
    }
    validate_pointer_binding(&manifest)?;
    Ok(manifest)
}

fn pending_create(manifest: &StacksteadManifest) -> bool {
    !manifest.event_log.exists() && manifest.status.source == ComponentStatus::Created
}

fn teardown_path(manifest: &StacksteadManifest) -> PathBuf {
    manifest.state_dir.join("teardown.json")
}

fn read_teardown(manifest: &StacksteadManifest) -> anyhow::Result<Option<TeardownState>> {
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

pub(crate) fn ensure_no_teardown(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if read_teardown(manifest)?.is_some() {
        anyhow::bail!(
            "stackstead `{}` has an incomplete teardown; retry `stackstead destroy {} --yes`",
            manifest.stackstead_id,
            manifest.stackstead_id
        );
    }
    Ok(())
}

fn write_teardown(
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

fn source_cleanup_complete(manifest: &StacksteadManifest) -> bool {
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

fn validate_recovery_source(manifest: &StacksteadManifest) -> anyhow::Result<()> {
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

fn finish_source_cleanup(manifest: &StacksteadManifest) -> anyhow::Result<()> {
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

#[cfg(test)]
fn validate_completed_source_cleanup(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if !read_teardown(manifest)?.is_some_and(|state| state.phase == TeardownPhase::Finalize)
        || !source_cleanup_complete(manifest)
    {
        anyhow::bail!("destroy has not recorded completed source cleanup");
    }
    Ok(())
}

pub fn inspect(cwd: &Path, name: &str) -> anyhow::Result<InspectOutput> {
    let runtime = load_project(cwd)?;
    let manifest = runtime.paths.resolve(name)?;
    validate_manifest_binding(&runtime, &manifest)?;
    let teardown = read_teardown(&manifest)?;
    let mut warnings = vec![];
    if let Err(error) = validate_source_binding(&manifest) {
        warnings.push(format!("source binding is invalid: {error}"));
    }
    if let Err(error) = validate_pointer_binding(&manifest) {
        warnings.push(format!("generated pointer is invalid: {error}"));
    }
    let (runtime_status, services) = match compose::service_observations(&manifest) {
        Ok(services) => {
            let running = services.iter().any(|service| service.state == "running");
            (
                if running {
                    ComponentStatus::Running
                } else {
                    ComponentStatus::Stopped
                },
                services,
            )
        }
        Err(error) => {
            warnings.push(format!("could not inspect Docker runtime: {error}"));
            (ComponentStatus::Unknown, vec![])
        }
    };
    let database_status = manifest
        .database
        .as_ref()
        .map(|_| database::live_status(&manifest, runtime_status));
    let database_reachable = manifest.database.as_ref().map(|database| {
        database::reachable(&database.host, database.port, Duration::from_millis(250))
    });
    let health_healthy = if runtime_status == ComponentStatus::Running {
        match observed_passive_health(&runtime.config, &manifest, &services) {
            Ok(status) => status,
            Err(error) => {
                warnings.push(format!(
                    "could not inspect configured health targets: {error}"
                ));
                None
            }
        }
    } else {
        None
    };
    if !manifest.worktree.is_dir() {
        warnings.push("worktree is missing; run `stackstead doctor`".into());
    }
    for file in &manifest.compose_files {
        match compose::fixed_ports_in_file(file) {
            Ok(fixed_ports) => {
                for fixed in fixed_ports {
                    warnings.push(format!(
                        "fixed host port {} in {}:{}",
                        fixed.host_port,
                        file.display(),
                        fixed.file_line
                    ));
                }
            }
            Err(error) => warnings.push(format!(
                "could not inspect fixed ports in {}: {error}",
                file.display()
            )),
        }
    }
    let effective_runtime = EffectiveComponent {
        status: runtime_status,
        basis: StatusBasis::Live,
    };
    let effective_database = database_status.map(|status| EffectiveComponent {
        status,
        basis: StatusBasis::Live,
    });
    let effective_health = match health_healthy {
        Some(healthy) => EffectiveComponent {
            status: if healthy {
                ComponentStatus::Ready
            } else {
                ComponentStatus::Failed
            },
            basis: StatusBasis::Live,
        },
        None if runtime.config.health.checks.is_empty()
            || runtime
                .config
                .health
                .checks
                .iter()
                .any(|check| check.url.is_none()) =>
        {
            EffectiveComponent {
                status: manifest.status.health,
                basis: StatusBasis::Recorded,
            }
        }
        None => EffectiveComponent {
            status: ComponentStatus::Unknown,
            basis: StatusBasis::Lifecycle,
        },
    };
    push_status_divergence(
        &mut warnings,
        "runtime",
        manifest.status.runtime,
        effective_runtime,
    );
    if let Some(database) = effective_database {
        push_status_divergence(
            &mut warnings,
            "database",
            manifest.status.database,
            database,
        );
    }
    if effective_health.basis == StatusBasis::Live {
        push_status_divergence(
            &mut warnings,
            "health",
            manifest.status.health,
            effective_health,
        );
    }
    let phase = match teardown.as_ref() {
        None => "normal",
        Some(state) if state.last_error.is_some() => "teardown_failed",
        Some(_) => "teardown_incomplete",
    };
    if phase != "normal" {
        warnings.push(format!(
            "lifecycle phase is {phase}; retry `stackstead destroy {} --yes`",
            manifest.stackstead_id
        ));
    }
    let effective = EffectiveStatus {
        phase,
        recorded_at: manifest.updated_at,
        observed_at: Utc::now(),
        runtime: effective_runtime,
        database: effective_database,
        health: effective_health,
    };
    Ok(InspectOutput {
        manifest,
        live: LiveStatus {
            runtime_status,
            services,
            database_reachable,
            database_status,
            health_healthy,
        },
        effective,
        warnings,
    })
}

fn observed_passive_health(
    config: &StacksteadConfig,
    manifest: &StacksteadManifest,
    services: &[compose::ServiceObservation],
) -> anyhow::Result<Option<bool>> {
    if config.health.checks.is_empty()
        || config.health.checks.iter().any(|check| check.url.is_none())
    {
        return Ok(None);
    }
    for check in &config.health.checks {
        let Some(template) = check.url.as_deref() else {
            return Ok(None);
        };
        let correlation = (|| {
            let url = render_template(template, &template_context(manifest))?;
            let endpoint = crate::open::manifest_endpoint(&url, manifest)?;
            let target = compose::resolve_port_target(
                &manifest.compose_files,
                &manifest.container_ports,
                &config.env.generate,
                &endpoint.contract_key,
            )?;
            Ok::<_, anyhow::Error>((endpoint, target))
        })();
        let Ok((endpoint, target)) = correlation else {
            return Ok(health::healthy_passive(
                &config.health,
                manifest,
                &BTreeMap::new(),
            ));
        };
        if !services
            .iter()
            .any(|service| service.service == target.service && service.state == "running")
            || !compose::endpoint_is_published(
                manifest,
                &target.service,
                target.container_port,
                &endpoint.endpoint.host,
                endpoint.endpoint.port,
            )?
        {
            return Ok(Some(false));
        }
    }
    Ok(health::healthy_passive(
        &config.health,
        manifest,
        &BTreeMap::new(),
    ))
}

fn push_status_divergence(
    warnings: &mut Vec<String>,
    component: &str,
    recorded: ComponentStatus,
    effective: EffectiveComponent,
) {
    if recorded != ComponentStatus::Unknown
        && effective.status != ComponentStatus::Unknown
        && recorded != effective.status
    {
        warnings.push(format!(
            "recorded/live divergence: {component} recorded={recorded} effective={} ({})",
            effective.status, effective.basis
        ));
    }
}

pub fn regenerate_contract(
    config: &StacksteadConfig,
    manifest: &mut StacksteadManifest,
) -> anyhow::Result<()> {
    validate_contract_binding(config, manifest)?;
    let values = template_context(manifest);
    write_contract(config, manifest, &values)
}

pub fn install_dependencies(
    config: &StacksteadConfig,
    manifest: &StacksteadManifest,
    environment: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    if !config.dependencies.install.command.trim().is_empty() {
        let output = command::run_configured(
            &config.dependencies.install.command,
            config.dependencies.install.shell,
            &manifest.worktree,
            environment,
        )?;
        write_command_log(
            &manifest.state_dir.join("logs/dependencies.log"),
            &output,
            environment,
        )?;
    }
    if config.dependencies.provider == DependencyProvider::YarnClassic
        && let Some(link) = config
            .dependencies
            .link
            .as_ref()
            .filter(|link| link.enabled)
    {
        let folder = paths::safe_generated_path(&manifest.worktree, &link.link_folder)?;
        std::fs::create_dir_all(&folder)?;
        let output =
            command::run_configured(&link.command, link.shell, &manifest.worktree, environment)?;
        write_command_log(
            &manifest.state_dir.join("logs/yarn-link.log"),
            &output,
            environment,
        )?;
        write_json_atomic(
            &manifest.state_dir.join("link-state.json"),
            &serde_json::json!({
                "kind": "StacksteadYarnLinkState",
                "version": "1",
                "link_folder": folder,
                "status": "ready",
                "updated_at": Utc::now()
            }),
        )?;
    }
    Ok(())
}

fn write_contract(
    config: &StacksteadConfig,
    manifest: &mut StacksteadManifest,
    context_values: &TemplateContext,
) -> anyhow::Result<()> {
    let expected_env = paths::safe_generated_path(&manifest.worktree, &config.env.file)?;
    let expected_context =
        paths::safe_generated_path(&manifest.worktree, &config.agent.context_file)?;
    let expected_pointer =
        paths::safe_generated_path(&manifest.worktree, Path::new(".stackstead/stackstead.json"))?;
    if manifest.env_file != expected_env
        || manifest.agent_context != expected_context
        || manifest.pointer_file != expected_pointer
    {
        anyhow::bail!("generated contract paths do not match the current validated configuration");
    }
    let mut generated = config
        .env
        .generate
        .iter()
        .map(|(key, template)| Ok((key.clone(), render_template(template, context_values)?)))
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    if config.dependencies.provider == DependencyProvider::YarnClassic
        && let Some(link) = config
            .dependencies
            .link
            .as_ref()
            .filter(|link| link.enabled)
    {
        generated.insert(
            "YARN_LINK_FOLDER".into(),
            paths::safe_generated_path(&manifest.worktree, &link.link_folder)?
                .display()
                .to_string(),
        );
    }
    manifest.env_keys = generated.keys().cloned().collect();
    envfile::write_generated(manifest, &generated)?;
    context::write_agent_context(manifest, &config.agent.rules)?;
    compose::write_ownership_override(manifest)?;
    let pointer = StacksteadPointer {
        kind: "StacksteadPointer".into(),
        version: POINTER_VERSION.into(),
        stackstead_id: manifest.stackstead_id.clone(),
        manifest: manifest.manifest_path(),
        project: manifest.project.clone(),
        repo_root: manifest.repo_root.clone(),
        project_state_root: manifest.project_state_root.clone(),
        stackstead_root: manifest.stackstead_root.clone(),
    };
    write_pointer(&manifest.pointer_file, &pointer)?;
    manifest.save_atomic()?;
    Ok(())
}

pub(crate) fn template_context(manifest: &StacksteadManifest) -> TemplateContext {
    let mut context = TemplateContext::from([
        ("project.name".into(), manifest.project.clone()),
        ("stackstead.id".into(), manifest.stackstead_id.clone()),
        ("stackstead.slug".into(), manifest.slug.clone()),
        ("stackstead.short_id".into(), manifest.short_id.clone()),
        (
            "paths.repo_root".into(),
            manifest.repo_root.display().to_string(),
        ),
        (
            "paths.stackstead_root".into(),
            manifest.stackstead_root.display().to_string(),
        ),
        (
            "paths.worktree".into(),
            manifest.worktree.display().to_string(),
        ),
        (
            "paths.state_dir".into(),
            manifest.state_dir.display().to_string(),
        ),
    ]);
    for (service, port) in &manifest.ports {
        context.insert(format!("ports.{service}"), port.to_string());
    }
    for (service, url) in &manifest.urls {
        context.insert(format!("urls.{service}"), url.clone());
    }
    context
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

fn validate_compose_project(name: &str) -> anyhow::Result<()> {
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

fn validate_contract_binding(
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

fn validate_configured_ports(config: &StacksteadConfig, worktree: &Path) -> anyhow::Result<()> {
    compose::validate_port_contract(
        &configured_compose_files(config, worktree)?,
        &configured_container_ports(config),
        &config.env.generate,
    )
}

fn configured_compose_files(
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

fn configured_container_ports(config: &StacksteadConfig) -> BTreeMap<String, u16> {
    config
        .resources
        .ports
        .expose
        .iter()
        .map(|(name, exposure)| (name.clone(), exposure.container))
        .collect()
}

pub(crate) fn validate_manifest_binding(
    runtime: &ProjectRuntime,
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    if manifest.repo_root != runtime.paths.repo_root
        || manifest.project_state_root != runtime.paths.state_root
        || manifest.project != runtime.config.project.name
    {
        anyhow::bail!("manifest project identity does not match the discovered project");
    }
    paths::validate_destroy_target(manifest, &runtime.paths.state_root)?;
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

pub(crate) fn validate_pointer_binding(manifest: &StacksteadManifest) -> anyhow::Result<()> {
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

pub(crate) fn validate_current_contract(
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

fn validate_worktree_path(worktree: &Path, path: &Path, label: &str) -> anyhow::Result<()> {
    let relative = path.strip_prefix(worktree).map_err(|_| {
        anyhow::anyhow!(
            "manifest {label} path {} escapes worktree {}",
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

pub(crate) fn validate_source_binding(manifest: &StacksteadManifest) -> anyhow::Result<()> {
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

fn run_commands(
    commands: &[CommandConfig],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    for configured in commands {
        command::run_configured(&configured.command, configured.shell, cwd, environment)?;
    }
    Ok(())
}

fn write_command_log(
    path: &Path,
    output: &std::process::Output,
    environment: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::write(path, command::redact_with_env(&data, environment))?;
    Ok(())
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

fn default_config(
    project: &str,
    base: &str,
    plan: &compose::ComposePlan,
) -> anyhow::Result<String> {
    let mut config = StacksteadConfig::default();
    config.project.name = project.into();
    config.source.base = base.into();
    config.runtime.files = vec![plan.file.clone()];
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

    if let Some(port) = plan
        .ports
        .iter()
        .find(|port| port.container_port == 5432 && port.name == port.service)
    {
        config.database.postgres = Some(PostgresConfig {
            strategy: Default::default(),
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
    config.validate()?;
    Ok(serde_yaml::to_string(&config)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cleanup_manifest(root: &Path, ownership: SourceOwnership) -> StacksteadManifest {
        let short_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let stackstead_id = format!("feature-a-{short_id}");
        let stackstead_root = root.join("state/demo").join(&stackstead_id);
        let worktree = match ownership {
            SourceOwnership::Stackstead => stackstead_root.join("source"),
            SourceOwnership::External => root.join("manager-source"),
        };
        let state_dir = stackstead_root.join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        if ownership == SourceOwnership::External {
            std::fs::create_dir_all(&worktree).unwrap();
        }
        let event_log = state_dir.join("events.jsonl");
        for (event_type, status) in [
            (events::EventType::Destroy, events::EventStatus::Started),
            (
                events::EventType::RuntimeRemove,
                events::EventStatus::Succeeded,
            ),
            (
                events::EventType::SourceRemove,
                events::EventStatus::Started,
            ),
            (
                events::EventType::SourceRemove,
                events::EventStatus::Succeeded,
            ),
        ] {
            events::append(&event_log, event_type, status, None).unwrap();
        }
        StacksteadManifest {
            kind: "StacksteadManifest".into(),
            version: crate::manifest::MANIFEST_VERSION.into(),
            stackstead_id: stackstead_id.clone(),
            slug: "feature-a".into(),
            short_id: short_id.into(),
            runtime_token: "0123456789abcdef0123456789abcdef".into(),
            project: "demo".into(),
            branch: "feature-a".into(),
            base: "base".into(),
            source_ownership: ownership,
            repo_root: root.join("repo"),
            project_state_root: root.join("state"),
            stackstead_root,
            worktree: worktree.clone(),
            state_dir,
            port_lease_state_dir: None,
            compose_project: format!("demo-{stackstead_id}"),
            compose_files: vec![worktree.join("compose.yaml")],
            ports: BTreeMap::new(),
            container_ports: BTreeMap::new(),
            urls: BTreeMap::new(),
            env_file: worktree.join(".stackstead/.env"),
            agent_context: worktree.join(".stackstead/AGENT_CONTEXT.md"),
            pointer_file: worktree.join(".stackstead/stackstead.json"),
            event_log,
            env_keys: vec![],
            status: ManifestStatus::default(),
            database: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn generated_config_parses() {
        let plan = compose::ComposePlan {
            file: "compose.yaml".into(),
            ports: vec![
                compose::ComposePortPlan {
                    name: "web".into(),
                    service: "web".into(),
                    container_port: 3000,
                    env: "WEB_PORT".into(),
                    current_host_port: Some(3000),
                    replacement: "127.0.0.1:${WEB_PORT}:3000".into(),
                    url: Some("http://127.0.0.1:{{ ports.web }}".into()),
                },
                compose::ComposePortPlan {
                    name: "postgres".into(),
                    service: "postgres".into(),
                    container_port: 5432,
                    env: "POSTGRES_PORT".into(),
                    current_host_port: Some(5432),
                    replacement: "127.0.0.1:${POSTGRES_PORT}:5432".into(),
                    url: None,
                },
            ],
            warnings: vec![],
        };
        let yaml = default_config("demo", "main", &plan).unwrap();
        let config = StacksteadConfig::from_yaml(&yaml).unwrap();
        assert_eq!(config.project.name, "demo");
        assert_eq!(config.resources.ports.expose.len(), 2);
    }

    #[test]
    fn compose_project_identity_is_docker_safe() {
        assert!(validate_compose_project("demo-feature-a17c").is_ok());
        assert!(validate_compose_project("Demo-feature").is_err());
        assert!(validate_compose_project("../demo").is_err());
    }

    #[test]
    fn manifest_binding_rejects_mismatched_port_service_sets() {
        let directory = tempfile::tempdir().unwrap();
        let mut manifest = cleanup_manifest(directory.path(), SourceOwnership::Stackstead);
        manifest.ports.insert("web".into(), 39000);
        let mut config = StacksteadConfig::default();
        config.project.name = "demo".into();
        let runtime = ProjectRuntime {
            config,
            paths: ProjectPaths::new(
                directory.path().join("repo"),
                directory.path().join("state"),
                "demo",
            ),
        };

        let error = validate_manifest_binding(&runtime, &manifest)
            .unwrap_err()
            .to_string();

        assert_eq!(
            error,
            "manifest host and container port service sets differ"
        );
    }

    #[test]
    fn partial_destroy_retry_requires_truthful_source_cleanup_state() {
        let directory = tempfile::tempdir().unwrap();
        let external = cleanup_manifest(directory.path(), SourceOwnership::External);
        assert!(validate_completed_source_cleanup(&external).is_err());
        write_teardown(&external, TeardownPhase::Finalize, None).unwrap();
        validate_completed_source_cleanup(&external).unwrap();
        std::fs::create_dir(external.worktree.join(".stackstead")).unwrap();
        assert!(validate_completed_source_cleanup(&external).is_err());

        std::fs::remove_dir(external.worktree.join(".stackstead")).unwrap();
        let owned = cleanup_manifest(directory.path(), SourceOwnership::Stackstead);
        write_teardown(&owned, TeardownPhase::Finalize, None).unwrap();
        validate_completed_source_cleanup(&owned).unwrap();
        std::fs::create_dir_all(&owned.worktree).unwrap();
        assert!(validate_completed_source_cleanup(&owned).is_err());
    }
}
