use std::{collections::BTreeMap, path::Path};

use super::{Diagnostic, ToolStatus};
use crate::{
    command, compose,
    config::StacksteadConfig,
    discovery::{self, Discovery},
    git, lock,
    manifest::StacksteadManifest,
    paths,
};

mod compose_files;
mod files;

pub(super) fn diagnose_manifest(
    manifest: &StacksteadManifest,
    config: &StacksteadConfig,
    project: &str,
    state_root: &Path,
    tools: ToolStatus,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnose_contract(manifest, config, project, state_root, diagnostics);
    files::diagnose(manifest, diagnostics);
    compose_files::diagnose(manifest, diagnostics);
    diagnose_source(manifest, tools.git, diagnostics);
    diagnose_locks(manifest, diagnostics);
    if tools.docker && tools.compose && tools.docker_daemon {
        diagnose_docker_project(manifest, diagnostics);
    }
}

fn diagnose_contract(
    manifest: &StacksteadManifest,
    config: &StacksteadConfig,
    project: &str,
    state_root: &Path,
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
}

fn diagnose_source(
    manifest: &StacksteadManifest,
    git_available: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnose_stackstead_discovery(manifest, diagnostics);
    if git_available
        && manifest.worktree.is_dir()
        && !git::is_stackstead_ignored(&manifest.worktree)
    {
        diagnostics.push(Diagnostic::warning(
            "git.stackstead_not_ignored",
            format!(
                "{} does not ignore source/.stackstead/ through Git exclude",
                manifest.stackstead_id
            ),
            "run `stackstead repair` to refresh the per-worktree Git exclude file",
        ));
    }
}

fn diagnose_locks(manifest: &StacksteadManifest, diagnostics: &mut Vec<Diagnostic>) {
    if !manifest.state_dir.is_dir() {
        return;
    }
    let label = &manifest.stackstead_id;
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
            ));
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
