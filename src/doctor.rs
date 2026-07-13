use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::config::StacksteadConfig;
use crate::discovery::{self, Discovery};
use crate::manifest::StacksteadManifest;
use crate::{command, compose, events, git, lock, paths, repository_policy, state};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

impl std::fmt::Display for DiagnosticSeverity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub suggestion: Option<String>,
}

impl Diagnostic {
    fn new(
        code: impl Into<String>,
        severity: DiagnosticSeverity,
        message: impl Into<String>,
        suggestion: Option<impl Into<String>>,
    ) -> Self {
        Self {
            code: code.into(),
            severity,
            message: message.into(),
            suggestion: suggestion.map(Into::into),
        }
    }

    fn info(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, DiagnosticSeverity::Info, message, None::<String>)
    }

    fn warning(
        code: impl Into<String>,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self::new(code, DiagnosticSeverity::Warning, message, Some(suggestion))
    }

    fn error(
        code: impl Into<String>,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self::new(code, DiagnosticSeverity::Error, message, Some(suggestion))
    }
}

#[derive(Debug, Clone, Copy)]
struct ToolStatus {
    git: bool,
    docker: bool,
    compose: bool,
    docker_daemon: bool,
}

pub fn run(cwd: &Path) -> anyhow::Result<Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let tools = diagnose_tools(&mut diagnostics);

    let discovery = match discovery::discover(cwd) {
        Ok(discovery) => discovery,
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                "discovery.project_not_found",
                error.to_string(),
                "run `stackstead init` from the root of a Git repository",
            ));
            return Ok(diagnostics);
        }
    };

    diagnose_initial_discovery(&discovery, &mut diagnostics);
    let repo_root = discovery::project_root(&discovery).to_path_buf();
    diagnose_repository_policy(&repo_root, &mut diagnostics);
    let config_path = repo_root.join(crate::config::CONFIG_FILE);
    let config = match StacksteadConfig::load(&config_path) {
        Ok(config) => config,
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                "config.invalid",
                error.to_string(),
                format!(
                    "fix {} and rerun `stackstead doctor`",
                    config_path.display()
                ),
            ));
            return Ok(diagnostics);
        }
    };

    match config.validate_for_repo(&repo_root) {
        Ok(()) => diagnostics.push(Diagnostic::info(
            "config.valid",
            format!("configuration is valid: {}", config_path.display()),
        )),
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                "config.invalid",
                error.to_string(),
                format!(
                    "fix {} and rerun `stackstead doctor`",
                    config_path.display()
                ),
            ));
            return Ok(diagnostics);
        }
    }

    diagnose_repository(&repo_root, tools.git, &mut diagnostics);
    diagnose_compose_files(&repo_root, &config, &mut diagnostics);

    let configured_state_root = match paths::absolute_from(&repo_root, &config.state.root) {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                "state.root.invalid",
                format!("cannot resolve state.root: {error}"),
                "choose a state.root that resolves to a safe absolute path",
            ));
            return Ok(diagnostics);
        }
    };
    let state_root = match &discovery {
        Discovery::Stackstead { pointer, .. } => {
            let pointer_root = match paths::absolute_from(&repo_root, &pointer.project_state_root) {
                Ok(path) => path,
                Err(error) => {
                    diagnostics.push(Diagnostic::error(
                        "state.pointer_root.invalid",
                        format!("cannot resolve pointer project state root: {error}"),
                        "repair the generated pointer before operating on this stackstead",
                    ));
                    return Ok(diagnostics);
                }
            };
            if pointer_root != configured_state_root {
                diagnostics.push(Diagnostic::warning(
                    "state.pointer_config_mismatch",
                    format!(
                        "pointer state root {} differs from current config {}",
                        pointer_root.display(),
                        configured_state_root.display()
                    ),
                    "use the pointer state root for this stackstead and review intentional config migrations",
                ));
            }
            pointer_root
        }
        Discovery::Project { .. } => configured_state_root,
    };

    diagnose_state_root(&state_root, &mut diagnostics);
    let project_paths =
        state::ProjectPaths::new(repo_root.clone(), state_root.clone(), &config.project.name);
    let project_state_dir = project_paths.project_state_dir;
    let manifests = read_manifests(&project_state_dir, &mut diagnostics);
    diagnose_duplicate_ports(&manifests, &mut diagnostics);
    diagnose_duplicate_compose_projects(&manifests, &mut diagnostics);
    diagnose_project_lock(&project_state_dir, &mut diagnostics);

    for manifest in &manifests {
        diagnose_manifest(
            manifest,
            &config,
            &config.project.name,
            &state_root,
            tools,
            &mut diagnostics,
        );
    }

    Ok(diagnostics)
}

fn diagnose_tools(diagnostics: &mut Vec<Diagnostic>) -> ToolStatus {
    let git = command_succeeds("git", &["--version"]);
    diagnostics.push(tool_diagnostic(
        "git",
        git,
        "install Git and ensure it is available on PATH",
    ));

    let docker = command_succeeds("docker", &["--version"]);
    diagnostics.push(tool_diagnostic(
        "docker",
        docker,
        "install Docker and ensure it is available on PATH",
    ));

    let compose = docker && command_succeeds("docker", &["compose", "version"]);
    diagnostics.push(tool_diagnostic(
        "docker_compose",
        compose,
        "install the Docker Compose plugin (`docker compose`)",
    ));

    let docker_daemon =
        docker && command_succeeds("docker", &["info", "--format", "{{.ServerVersion}}"]);
    diagnostics.push(if docker_daemon {
        Diagnostic::info("tool.docker_daemon.available", "Docker daemon is reachable")
    } else {
        Diagnostic::warning(
            "tool.docker_daemon.unavailable",
            "Docker daemon is not reachable; runtime project checks were skipped",
            "start Docker and rerun `stackstead doctor`",
        )
    });

    ToolStatus {
        git,
        docker,
        compose,
        docker_daemon,
    }
}

fn command_succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn tool_diagnostic(tool: &str, available: bool, suggestion: &str) -> Diagnostic {
    if available {
        Diagnostic::info(
            format!("tool.{tool}.available"),
            format!("{} is available", tool.replace('_', " ")),
        )
    } else {
        Diagnostic::error(
            format!("tool.{tool}.missing"),
            format!("{} is not available", tool.replace('_', " ")),
            suggestion,
        )
    }
}

fn diagnose_initial_discovery(discovery: &Discovery, diagnostics: &mut Vec<Diagnostic>) {
    match discovery {
        Discovery::Project {
            repo_root,
            config_path,
        } => diagnostics.push(Diagnostic::info(
            "discovery.project",
            format!(
                "project root {} discovered through {}",
                repo_root.display(),
                config_path.display()
            ),
        )),
        Discovery::Stackstead {
            pointer_path,
            manifest,
            ..
        } => diagnostics.push(Diagnostic::info(
            "discovery.stackstead",
            format!(
                "stackstead {} discovered through {}",
                manifest.stackstead_id,
                pointer_path.display()
            ),
        )),
    }
}

fn diagnose_repository(repo_root: &Path, git_available: bool, diagnostics: &mut Vec<Diagnostic>) {
    if !git_available {
        return;
    }
    match git::repo_root(repo_root) {
        Ok(detected) if detected == repo_root => diagnostics.push(Diagnostic::info(
            "git.repo_root.valid",
            format!("Git repository root is {}", repo_root.display()),
        )),
        Ok(detected) => diagnostics.push(Diagnostic::error(
            "git.repo_root.mismatch",
            format!(
                "Stackstead project root {} differs from Git root {}",
                repo_root.display(),
                detected.display()
            ),
            "move stackstead.yaml to the canonical Git repository root",
        )),
        Err(error) => diagnostics.push(Diagnostic::error(
            "git.repo_root.unavailable",
            format!("cannot resolve Git repository root: {error}"),
            "run Stackstead from a valid Git repository",
        )),
    }
}

fn diagnose_repository_policy(repo_root: &Path, diagnostics: &mut Vec<Diagnostic>) {
    let mut found = false;
    for name in repository_policy::FILE_NAMES {
        let path = repo_root.join(name);
        if !path.exists() {
            continue;
        }
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) => {
                found = true;
                diagnostics.push(Diagnostic::warning(
                    "repository_policy.unreadable",
                    format!(
                        "cannot read repository policy file {}: {error}",
                        path.display()
                    ),
                    "make the instruction file readable and rerun `stackstead doctor`",
                ));
                continue;
            }
        };
        match policy_marker_version(&contents) {
            Some(Ok(version)) => {
                found = true;
                if version == repository_policy::VERSION {
                    diagnostics.push(Diagnostic::info(
                        "repository_policy.current",
                        format!("repository policy is current in {}", path.display()),
                    ));
                } else if version < repository_policy::VERSION {
                    diagnostics.push(Diagnostic::warning(
                        "repository_policy.outdated",
                        format!(
                            "repository policy version {version} in {} is older than version {} required by this Stackstead binary",
                            path.display(),
                            repository_policy::VERSION
                        ),
                        format!("update the policy from {}", repository_policy::GUIDE_URL),
                    ));
                } else {
                    diagnostics.push(Diagnostic::warning(
                        "repository_policy.binary_outdated",
                        format!(
                            "repository policy version {version} in {} is newer than version {} understood by this Stackstead binary",
                            path.display(),
                            repository_policy::VERSION
                        ),
                        "upgrade Stackstead before relying on this repository policy",
                    ));
                }
            }
            Some(Err(())) => {
                found = true;
                diagnostics.push(Diagnostic::warning(
                    "repository_policy.invalid",
                    format!("repository policy marker is invalid in {}", path.display()),
                    format!(
                        "replace it with the current policy from {}",
                        repository_policy::GUIDE_URL
                    ),
                ));
            }
            None if looks_like_repository_policy(&contents) => {
                found = true;
                diagnostics.push(Diagnostic::warning(
                    "repository_policy.unversioned",
                    format!(
                        "Stackstead repository policy in {} has no version marker",
                        path.display()
                    ),
                    format!("update the policy from {}", repository_policy::GUIDE_URL),
                ));
            }
            None => {}
        }
    }
    if !found {
        diagnostics.push(Diagnostic::warning(
            "repository_policy.missing",
            "no current Stackstead repository policy was found in AGENTS.md or CLAUDE.md",
            format!("add the policy from {}", repository_policy::GUIDE_URL),
        ));
    }
}

fn policy_marker_version(contents: &str) -> Option<Result<u64, ()>> {
    contents.lines().find_map(|line| {
        line.trim()
            .strip_prefix(repository_policy::MARKER_PREFIX)
            .map(|value| {
                value
                    .strip_suffix(repository_policy::MARKER_SUFFIX)
                    .ok_or(())?
                    .trim()
                    .parse()
                    .map_err(|_| ())
            })
    })
}

fn looks_like_repository_policy(contents: &str) -> bool {
    contents.contains("## Stackstead") && contents.contains("$STACKSTEAD_CONTEXT")
}

fn diagnose_compose_files(
    repo_root: &Path,
    config: &StacksteadConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let files = config
        .runtime
        .files
        .iter()
        .map(|relative| repo_root.join(relative))
        .collect::<Vec<_>>();
    for path in &files {
        match compose::fixed_ports_in_file(path) {
            Ok(fixed_ports) => {
                diagnostics.push(Diagnostic::info(
                    "compose.file.readable",
                    format!("Compose file is readable: {}", path.display()),
                ));
                for fixed in fixed_ports {
                    diagnostics.push(Diagnostic::error(
                        "compose.fixed_host_port",
                        format!(
                            "fixed host port {} found in {}:{} (`{}`)",
                            fixed.host_port,
                            path.display(),
                            fixed.file_line,
                            fixed.mapping
                        ),
                        "replace the fixed host port with a generated Stackstead env variable",
                    ));
                }
                if let Ok(unbound) = compose::unbound_ports_in_file(path) {
                    for (service, container) in unbound {
                        diagnostics.push(Diagnostic::error(
                            "compose.unbound_host_port",
                            format!(
                                "service `{service}` publishes container port {container} without a deterministic host port in {}",
                                path.display()
                            ),
                            "add a generated host-port mapping such as `127.0.0.1:${WEB_PORT}:80`",
                        ));
                    }
                }
                if let Ok(exposed) = compose::all_interface_ports_in_file(path) {
                    for (service, container) in exposed {
                        diagnostics.push(Diagnostic::error(
                            "compose.all_interfaces_host_port",
                            format!(
                                "Compose port `{service}` ({container}/tcp) binds all host interfaces in {}",
                                path.display()
                            ),
                            "bind `127.0.0.1:${PORT}:<container-port>`",
                        ));
                    }
                }
            }
            Err(error) => diagnostics.push(Diagnostic::error(
                "compose.file.unreadable",
                format!("cannot inspect {}: {error}", path.display()),
                "make the configured Compose file readable",
            )),
        }
    }
    let expected = config
        .resources
        .ports
        .expose
        .iter()
        .map(|(name, exposure)| (name.clone(), exposure.container))
        .collect();
    if let Err(error) = compose::validate_port_contract(&files, &expected, &config.env.generate) {
        diagnostics.push(Diagnostic::error(
            "compose.isolation_contract.invalid",
            format!("Compose isolation contract is unsafe or disconnected: {error}"),
            "make every published host port consume its matching env.generate allocation",
        ));
    }
}

fn diagnose_state_root(state_root: &Path, diagnostics: &mut Vec<Diagnostic>) {
    if state_root.exists() && !state_root.is_dir() {
        diagnostics.push(Diagnostic::error(
            "state.root.not_directory",
            format!("state root is not a directory: {}", state_root.display()),
            "choose a writable directory for state.root",
        ));
        return;
    }

    let nearest = state_root.ancestors().find(|candidate| candidate.is_dir());
    match nearest {
        Some(directory)
            if !std::fs::metadata(directory).is_ok_and(|meta| meta.permissions().readonly()) =>
        {
            diagnostics.push(Diagnostic::info(
                "state.root.writable",
                format!(
                    "state root has a writable existing ancestor: {}",
                    directory.display()
                ),
            ));
        }
        Some(directory) => diagnostics.push(Diagnostic::error(
            "state.root.read_only",
            format!("state root ancestor is read-only: {}", directory.display()),
            "choose a writable state.root or update its filesystem permissions",
        )),
        None => diagnostics.push(Diagnostic::error(
            "state.root.unreachable",
            format!(
                "state root has no existing ancestor: {}",
                state_root.display()
            ),
            "choose a reachable state.root",
        )),
    }
}

fn read_manifests(
    project_state_dir: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<StacksteadManifest> {
    if !project_state_dir.exists() {
        diagnostics.push(Diagnostic::info(
            "state.no_stacksteads",
            format!("no stacksteads found under {}", project_state_dir.display()),
        ));
        return Vec::new();
    }

    let entries = match std::fs::read_dir(project_state_dir) {
        Ok(entries) => entries,
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                "state.unreadable",
                format!("cannot read {}: {error}", project_state_dir.display()),
                "make the project state directory readable",
            ));
            return Vec::new();
        }
    };

    let mut manifests = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                diagnostics.push(Diagnostic::error(
                    "state.entry_unreadable",
                    format!("cannot read project state entry: {error}"),
                    "check project state directory permissions",
                ));
                continue;
            }
        };
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }

        let manifest_path = entry.path().join("state/manifest.json");
        if !manifest_path.is_file() {
            diagnostics.push(Diagnostic::error(
                "manifest.missing",
                format!("stackstead directory has no manifest: {}", entry.path().display()),
                "remove the orphan only after verifying it is Stackstead-owned, or restore its manifest",
            ));
            continue;
        }
        match StacksteadManifest::read(&manifest_path) {
            Ok(manifest) => manifests.push(manifest),
            Err(error) => diagnostics.push(Diagnostic::error(
                "manifest.unreadable",
                format!("cannot read {}: {error}", manifest_path.display()),
                "restore a valid StacksteadManifest version 2 file; destroy version 1 stacksteads with the older binary that created them, then recreate them with this version",
            )),
        }
    }
    manifests.sort_by(|left, right| left.stackstead_id.cmp(&right.stackstead_id));
    diagnostics.push(Diagnostic::info(
        "manifest.count",
        format!("{} readable stackstead manifest(s) found", manifests.len()),
    ));
    manifests
}

fn diagnose_duplicate_ports(manifests: &[StacksteadManifest], diagnostics: &mut Vec<Diagnostic>) {
    let mut owners: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for manifest in manifests {
        for (service, port) in &manifest.ports {
            owners
                .entry(*port)
                .or_default()
                .push(format!("{}:{service}", manifest.stackstead_id));
        }
    }
    for (port, owners) in owners {
        if owners.len() > 1 {
            diagnostics.push(Diagnostic::error(
                "ports.duplicate_allocation",
                format!("host port {port} is allocated to {}", owners.join(", ")),
                "stop conflicting runtimes and repair or recreate one of the stacksteads",
            ));
        }
    }
}

fn diagnose_duplicate_compose_projects(
    manifests: &[StacksteadManifest],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut owners: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for manifest in manifests {
        owners
            .entry(&manifest.compose_project)
            .or_default()
            .push(&manifest.stackstead_id);
    }
    for (project, owners) in owners {
        if owners.len() > 1 {
            diagnostics.push(Diagnostic::error(
                "compose.duplicate_project",
                format!(
                    "Compose project `{project}` is shared by {}",
                    owners.join(", ")
                ),
                "recreate one stackstead with a unique Compose project identity",
            ));
        }
    }
}

fn diagnose_project_lock(project_state_dir: &Path, diagnostics: &mut Vec<Diagnostic>) {
    if !project_state_dir.is_dir() {
        return;
    }
    let path = lock::project_lock_path(project_state_dir);
    if !path.is_file() {
        diagnostics.push(Diagnostic::error(
            "lock.project.missing",
            format!("project lock is missing: {}", path.display()),
            "recreate the affected stacksteads; Stackstead will not infer or recreate missing lock ownership state",
        ));
        return;
    }
    diagnostics.push(if lock::LockGuard::can_acquire(&path) {
        Diagnostic::info(
            "lock.project.available",
            format!("project lock is available: {}", path.display()),
        )
    } else {
        Diagnostic::warning(
            "lock.project.busy",
            format!("project lock cannot be acquired: {}", path.display()),
            "wait for the active Stackstead operation to finish; do not infer staleness from the lock file alone",
        )
    });
}

fn diagnose_manifest(
    manifest: &StacksteadManifest,
    config: &StacksteadConfig,
    project: &str,
    state_root: &Path,
    tools: ToolStatus,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let label = &manifest.stackstead_id;
    if manifest.kind != "StacksteadManifest"
        || manifest.version != crate::manifest::MANIFEST_VERSION
    {
        diagnostics.push(Diagnostic::error(
            "manifest.contract.invalid",
            format!("{label} has unsupported manifest kind or version"),
            "restore the StacksteadManifest version 2 contract",
        ));
    }
    if manifest.project != project {
        diagnostics.push(Diagnostic::error(
            "manifest.project_mismatch",
            format!(
                "{label} belongs to project `{}` rather than `{project}`",
                manifest.project
            ),
            "move the manifest back to its owning project state directory",
        ));
    }
    if let Err(error) = paths::validate_destroy_target(manifest, state_root) {
        diagnostics.push(Diagnostic::error(
            "manifest.paths.unsafe",
            format!("{label} has unsafe contract paths: {error}"),
            "do not destroy this stackstead until its manifest ownership and paths are repaired",
        ));
    }
    if let Err(error) = crate::lifecycle::validate_pointer_binding(manifest) {
        diagnostics.push(Diagnostic::error(
            "pointer.binding.invalid",
            format!("{label} has an invalid reciprocal pointer: {error}"),
            "restore the exact generated pointer with `stackstead repair` only after verifying manifest ownership",
        ));
    }
    if let Err(error) = compose::validate_port_contract(
        &manifest.compose_files,
        &manifest.container_ports,
        &config.env.generate,
    ) {
        diagnostics.push(Diagnostic::error(
            "compose.worktree_isolation_contract.invalid",
            format!("{label} has an unsafe or disconnected Compose port contract: {error}"),
            "restore the reviewed Compose/env contract before starting this stackstead",
        ));
    }

    check_directory(
        "worktree.missing",
        label,
        &manifest.worktree,
        "restore the Git worktree or destroy the orphaned stackstead after review",
        diagnostics,
    );
    check_directory(
        "state.directory_missing",
        label,
        &manifest.state_dir,
        "run `stackstead repair` to recreate non-destructive state directories",
        diagnostics,
    );
    for (code, name, path, suggestion) in [
        (
            "pointer.missing",
            "pointer file",
            &manifest.pointer_file,
            "run `stackstead repair` to regenerate the pointer file",
        ),
        (
            "env.missing",
            "generated env file",
            &manifest.env_file,
            "run `stackstead repair` to regenerate the env file",
        ),
        (
            "context.missing",
            "agent context file",
            &manifest.agent_context,
            "run `stackstead repair` to regenerate agent context",
        ),
        (
            "events.missing",
            "event log",
            &manifest.event_log,
            "run `stackstead repair` to restore non-destructive state",
        ),
    ] {
        check_file(code, label, name, path, suggestion, diagnostics);
    }
    if manifest.event_log.is_file() {
        match events::read(&manifest.event_log) {
            Ok(log) if log.truncated_tail => diagnostics.push(Diagnostic::warning(
                "events.truncated_tail",
                format!("{label} event log has an unterminated final record"),
                "rerun the interrupted operation; only the incomplete final record is ignored",
            )),
            Ok(_) => diagnostics.push(Diagnostic::info(
                "events.valid",
                format!("{label} event log contains valid typed records"),
            )),
            Err(error) => diagnostics.push(Diagnostic::error(
                "events.invalid",
                format!("{label} event log is invalid: {error}"),
                "inspect the event journal before attempting recovery; completed malformed records are never ignored",
            )),
        }
    }
    for compose_file in &manifest.compose_files {
        check_file(
            "compose.worktree_file_missing",
            label,
            "worktree Compose file",
            compose_file,
            "restore the configured Compose file in the worktree",
            diagnostics,
        );
        if let Ok(fixed_ports) = compose::fixed_ports_in_file(compose_file) {
            for fixed in fixed_ports {
                diagnostics.push(Diagnostic::error(
                    "compose.worktree_fixed_host_port",
                    format!(
                        "fixed host port {} found in {}:{} (`{}`)",
                        fixed.host_port,
                        compose_file.display(),
                        fixed.file_line,
                        fixed.mapping
                    ),
                    "replace the fixed host port with a generated Stackstead env variable",
                ));
            }
        }
        if let Ok(unbound) = compose::unbound_ports_in_file(compose_file) {
            for (service, container) in unbound {
                diagnostics.push(Diagnostic::error(
                    "compose.worktree_unbound_host_port",
                    format!(
                        "{label} service `{service}` publishes container port {container} without a deterministic host port in {}",
                        compose_file.display()
                    ),
                    "add the manifest-generated host-port mapping before starting this stackstead",
                ));
            }
        }
        if let Ok(exposed) = compose::all_interface_ports_in_file(compose_file) {
            for (service, container) in exposed {
                diagnostics.push(Diagnostic::error(
                    "compose.worktree_all_interfaces_host_port",
                    format!(
                        "{label} Compose port `{service}` ({container}/tcp) binds all host interfaces in {}",
                        compose_file.display()
                    ),
                    "bind the generated port to 127.0.0.1",
                ));
            }
        }
    }

    diagnose_stackstead_discovery(manifest, diagnostics);
    if tools.git && manifest.worktree.is_dir() && !git::is_stackstead_ignored(&manifest.worktree) {
        diagnostics.push(Diagnostic::warning(
            "git.stackstead_not_ignored",
            format!("{label} does not ignore source/.stackstead/ through Git exclude"),
            "run `stackstead repair` to refresh the per-worktree Git exclude file",
        ));
    }

    if manifest.state_dir.is_dir() {
        let lock_path = manifest.state_dir.join("lock");
        let run_lock_path = manifest.state_dir.join("run.lock");
        for (code, name, path) in [
            ("lock.stackstead.missing", "mutation", &lock_path),
            ("lock.run.missing", "run lease", &run_lock_path),
        ] {
            if !path.is_file() {
                diagnostics.push(Diagnostic::error(
                    code,
                    format!("{label} {name} lock is missing: {}", path.display()),
                    "recreate the stackstead; lock ownership files are part of the strict state contract",
                ));
            }
        }
        if lock_path.is_file() && lock::LockGuard::can_acquire(&lock_path) {
            diagnostics.push(Diagnostic::info(
                "lock.stackstead.available",
                format!("{label} lock is available: {}", lock_path.display()),
            ));
        } else if lock_path.is_file() {
            diagnostics.push(Diagnostic::warning(
                "lock.stackstead.busy",
                format!("{label} lock cannot be acquired: {}", lock_path.display()),
                "wait for the active operation to finish; do not infer staleness from file presence",
            ));
        }
    }

    if tools.docker && tools.compose && tools.docker_daemon {
        diagnose_docker_project(manifest, diagnostics);
    }
}

fn check_directory(
    code: &str,
    label: &str,
    path: &Path,
    suggestion: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !path.is_dir() {
        diagnostics.push(Diagnostic::error(
            code,
            format!("{label} is missing directory {}", path.display()),
            suggestion,
        ));
    }
}

fn check_file(
    code: &str,
    label: &str,
    name: &str,
    path: &Path,
    suggestion: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !path.is_file() {
        diagnostics.push(Diagnostic::error(
            code,
            format!("{label} is missing {name}: {}", path.display()),
            suggestion,
        ));
    }
}

fn diagnose_stackstead_discovery(manifest: &StacksteadManifest, diagnostics: &mut Vec<Diagnostic>) {
    if !manifest.worktree.is_dir() {
        return;
    }
    match discovery::discover(&manifest.worktree) {
        Ok(Discovery::Stackstead {
            manifest: discovered,
            ..
        }) if discovered.stackstead_id == manifest.stackstead_id => {
            diagnostics.push(Diagnostic::info(
                "discovery.stackstead_valid",
                format!(
                    "{} is correctly discoverable from its worktree",
                    manifest.stackstead_id
                ),
            ))
        }
        Ok(_) => diagnostics.push(Diagnostic::error(
            "discovery.stackstead_mismatch",
            format!(
                "{} does not rediscover its own manifest from {}",
                manifest.stackstead_id,
                manifest.worktree.display()
            ),
            "run `stackstead repair` to regenerate and validate the pointer file",
        )),
        Err(error) => diagnostics.push(Diagnostic::error(
            "discovery.stackstead_failed",
            format!("{} cannot be rediscovered: {error}", manifest.stackstead_id),
            "run `stackstead repair` to regenerate and validate the pointer file",
        )),
    }
}

fn diagnose_docker_project(manifest: &StacksteadManifest, diagnostics: &mut Vec<Diagnostic>) {
    let args = vec![
        "ps".to_owned(),
        "-a".to_owned(),
        "--quiet".to_owned(),
        "--filter".to_owned(),
        format!(
            "label=com.docker.compose.project={}",
            manifest.compose_project
        ),
    ];
    match command::run("docker", &args, &manifest.repo_root, &BTreeMap::new()) {
        Ok(output) if output.stdout.iter().any(|byte| !byte.is_ascii_whitespace()) => {
            diagnostics.push(Diagnostic::info(
                "docker.project.present",
                format!(
                    "Docker Compose project `{}` has containers",
                    manifest.compose_project
                ),
            ));
        }
        Ok(_) => diagnostics.push(Diagnostic::warning(
            "docker.project.missing",
            format!(
                "Docker Compose project `{}` has no containers",
                manifest.compose_project
            ),
            "run `stackstead up` if this runtime should be active",
        )),
        Err(error) => diagnostics.push(Diagnostic::warning(
            "docker.project.unchecked",
            format!(
                "could not inspect Docker Compose project `{}`: {error}",
                manifest.compose_project
            ),
            "check Docker access and rerun `stackstead doctor`",
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use chrono::Utc;

    use super::*;
    use crate::manifest::{ManifestStatus, SourceOwnership};

    fn manifest(id: &str, port: u16) -> StacksteadManifest {
        let root = PathBuf::from(format!("/tmp/state/demo/{id}"));
        StacksteadManifest {
            kind: "StacksteadManifest".into(),
            version: crate::manifest::MANIFEST_VERSION.into(),
            stackstead_id: id.into(),
            slug: "feature".into(),
            short_id: id.rsplit('-').next().unwrap().into(),
            runtime_token: "0123456789abcdef0123456789abcdef".into(),
            project: "demo".into(),
            branch: "feature".into(),
            base: "main".into(),
            source_ownership: SourceOwnership::Stackstead,
            repo_root: "/tmp/repo".into(),
            project_state_root: "/tmp/state".into(),
            stackstead_root: root.clone(),
            worktree: root.join("source"),
            state_dir: root.join("state"),
            port_lease_state_dir: Some("/tmp/leases".into()),
            compose_project: format!("demo-{id}"),
            compose_files: vec![],
            ports: BTreeMap::from([("web".into(), port)]),
            container_ports: BTreeMap::new(),
            urls: BTreeMap::new(),
            env_file: root.join("source/.stackstead/.env"),
            agent_context: root.join("source/.stackstead/AGENT_CONTEXT.md"),
            pointer_file: root.join("source/.stackstead/stackstead.json"),
            event_log: root.join("state/events.jsonl"),
            env_keys: vec![],
            status: ManifestStatus::default(),
            database: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn diagnostic_severity_displays_stably() {
        assert_eq!(DiagnosticSeverity::Warning.to_string(), "warning");
    }

    #[test]
    fn repository_policy_reports_missing_and_current_files() {
        let directory = tempfile::tempdir().unwrap();
        let mut diagnostics = Vec::new();
        diagnose_repository_policy(directory.path(), &mut diagnostics);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "repository_policy.missing");
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);

        std::fs::write(
            directory.path().join("AGENTS.md"),
            format!(
                "{}\n{}",
                repository_policy::marker(),
                repository_policy::TEXT
            ),
        )
        .unwrap();
        std::fs::write(directory.path().join("CLAUDE.md"), "# Other policy\n").unwrap();
        diagnostics.clear();
        diagnose_repository_policy(directory.path(), &mut diagnostics);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "repository_policy.current");
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Info);
    }

    #[test]
    fn repository_policy_reports_older_and_newer_markers() {
        for (version, code) in [
            (repository_policy::VERSION - 1, "repository_policy.outdated"),
            (
                repository_policy::VERSION + 1,
                "repository_policy.binary_outdated",
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            std::fs::write(
                directory.path().join("AGENTS.md"),
                format!("<!-- stackstead-policy: {version} -->\n"),
            )
            .unwrap();
            let mut diagnostics = Vec::new();
            diagnose_repository_policy(directory.path(), &mut diagnostics);
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].code, code);
            assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
        }
    }

    #[test]
    fn repository_policy_reports_unversioned_and_invalid_markers() {
        for (contents, code) in [
            (
                "## Stackstead\nRead `$STACKSTEAD_CONTEXT`.\n",
                "repository_policy.unversioned",
            ),
            (
                "<!-- stackstead-policy: nope -->\n",
                "repository_policy.invalid",
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            std::fs::write(directory.path().join("CLAUDE.md"), contents).unwrap();
            let mut diagnostics = Vec::new();
            diagnose_repository_policy(directory.path(), &mut diagnostics);
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].code, code);
            assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
        }
    }

    #[test]
    fn reports_duplicate_port_allocations() {
        let manifests = [
            manifest("feature-a111", 39000),
            manifest("feature-b222", 39000),
        ];
        let mut diagnostics = Vec::new();
        diagnose_duplicate_ports(&manifests, &mut diagnostics);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "ports.duplicate_allocation");
        assert!(diagnostics[0].message.contains("feature-a111:web"));
        assert!(diagnostics[0].message.contains("feature-b222:web"));
    }

    #[test]
    fn project_doctor_finds_fixed_ports_without_requiring_docker() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("stackstead.yaml"),
            r#"
version: "1"
kind: StacksteadProject
project: { name: demo }
state: { root: ../state }
runtime: { files: [docker-compose.yml] }
"#,
        )
        .unwrap();
        std::fs::write(
            directory.path().join("docker-compose.yml"),
            "services:\n  web:\n    ports:\n      - \"3000:3000\"\n",
        )
        .unwrap();

        let diagnostics = run(directory.path()).unwrap();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "config.valid"),
            "{diagnostics:#?}"
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "compose.fixed_host_port")
        );
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "compose.all_interfaces_host_port"
                && diagnostic.severity == DiagnosticSeverity::Error
        }));
    }

    #[test]
    fn unreadable_manifest_becomes_a_diagnostic() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("demo/cell/state");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("manifest.json"), "not json").unwrap();
        let mut diagnostics = Vec::new();
        let manifests = read_manifests(&directory.path().join("demo"), &mut diagnostics);
        assert!(manifests.is_empty());
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "manifest.unreadable")
        );
    }

    #[test]
    fn missing_project_lock_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let mut diagnostics = Vec::new();
        diagnose_project_lock(directory.path(), &mut diagnostics);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "lock.project.missing");
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Error);
        assert!(!diagnostics[0].message.contains("stale"));
    }
}
