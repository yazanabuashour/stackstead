use std::path::{Path, PathBuf};

use super::{
    Diagnostic, ToolStatus,
    manifest::diagnose_manifest,
    project::{diagnose_compose_files, diagnose_state_root},
    state::{
        diagnose_duplicate_compose_projects, diagnose_duplicate_ports, diagnose_project_lock,
        read_manifests,
    },
    tools::{
        diagnose_initial_discovery, diagnose_repository, diagnose_repository_policy, diagnose_tools,
    },
};
use crate::{
    config::StacksteadConfig,
    discovery::{self, Discovery},
    paths, state,
};

pub(super) fn run(cwd: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let tools = diagnose_tools(&mut diagnostics);
    let Some(discovery) = discover(cwd, &mut diagnostics) else {
        return diagnostics;
    };
    diagnose_initial_discovery(&discovery, &mut diagnostics);

    let repo_root = discovery::project_root(&discovery).to_path_buf();
    diagnose_repository_policy(&repo_root, &mut diagnostics);
    let Some(config) = load_config(&repo_root, &mut diagnostics) else {
        return diagnostics;
    };
    diagnose_repository(&repo_root, tools.git, &mut diagnostics);
    diagnose_compose_files(&repo_root, &config, &mut diagnostics);

    let Some(state_root) = resolve_state_root(&repo_root, &config, &discovery, &mut diagnostics)
    else {
        return diagnostics;
    };
    diagnose_state(&repo_root, &state_root, &config, tools, &mut diagnostics);
    diagnostics
}

fn discover(cwd: &Path, diagnostics: &mut Vec<Diagnostic>) -> Option<Discovery> {
    match discovery::discover(cwd) {
        Ok(discovery) => Some(discovery),
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                "discovery.project_not_found",
                error.to_string(),
                "run `stackstead init` from the root of a Git repository",
            ));
            None
        }
    }
}

fn load_config(repo_root: &Path, diagnostics: &mut Vec<Diagnostic>) -> Option<StacksteadConfig> {
    let config_path = repo_root.join(crate::config::CONFIG_FILE);
    let config = match StacksteadConfig::load(&config_path) {
        Ok(config) => config,
        Err(error) => {
            invalid_config(&config_path, &error, diagnostics);
            return None;
        }
    };
    match config.validate_for_repo(repo_root) {
        Ok(()) => diagnostics.push(Diagnostic::info(
            "config.valid",
            format!("configuration is valid: {}", config_path.display()),
        )),
        Err(error) => {
            invalid_config(&config_path, &error, diagnostics);
            return None;
        }
    }
    Some(config)
}

fn invalid_config(
    config_path: &Path,
    error: &crate::config::ConfigError,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnostics.push(Diagnostic::error(
        "config.invalid",
        error.to_string(),
        format!(
            "fix {} and rerun `stackstead doctor`",
            config_path.display()
        ),
    ));
}

fn resolve_state_root(
    repo_root: &Path,
    config: &StacksteadConfig,
    discovery: &Discovery,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<PathBuf> {
    let configured = match paths::absolute_from(repo_root, &config.state.root) {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                "state.root.invalid",
                format!("cannot resolve state.root: {error}"),
                "choose a state.root that resolves to a safe absolute path",
            ));
            return None;
        }
    };
    let Discovery::Stackstead { pointer, .. } = discovery else {
        return Some(configured);
    };
    let pointer_root = match paths::absolute_from(repo_root, &pointer.project_state_root) {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                "state.pointer_root.invalid",
                format!("cannot resolve pointer project state root: {error}"),
                "repair the generated pointer before operating on this stackstead",
            ));
            return None;
        }
    };
    if pointer_root != configured {
        diagnostics.push(Diagnostic::warning(
            "state.pointer_config_mismatch",
            format!(
                "pointer state root {} differs from current config {}",
                pointer_root.display(),
                configured.display()
            ),
            "use the pointer state root for this stackstead and review intentional config migrations",
        ));
    }
    Some(pointer_root)
}

fn diagnose_state(
    repo_root: &Path,
    state_root: &Path,
    config: &StacksteadConfig,
    tools: ToolStatus,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnose_state_root(state_root, diagnostics);
    let project_paths = state::ProjectPaths::new(
        repo_root.to_path_buf(),
        state_root.to_path_buf(),
        &config.project.name,
    );
    let project_state_dir = project_paths.project_state_dir;
    let manifests = read_manifests(&project_state_dir, diagnostics);
    diagnose_duplicate_ports(&manifests, diagnostics);
    diagnose_duplicate_compose_projects(&manifests, diagnostics);
    diagnose_project_lock(&project_state_dir, diagnostics);
    for manifest in &manifests {
        diagnose_manifest(
            manifest,
            config,
            &config.project.name,
            state_root,
            tools,
            diagnostics,
        );
    }
}
