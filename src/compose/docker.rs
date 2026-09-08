use std::collections::BTreeMap;

use crate::{command, manifest::StacksteadManifest};

use super::ownership::{ownership_override_path, verify_ownership_override};

pub fn base_args(manifest: &StacksteadManifest) -> Vec<String> {
    let mut args = vec![
        "compose".into(),
        "-p".into(),
        manifest.compose_project.clone(),
        "--env-file".into(),
        manifest.env_file.display().to_string(),
    ];
    for file in &manifest.compose_files {
        args.push("-f".into());
        args.push(file.display().to_string());
    }
    args.push("-f".into());
    args.push(ownership_override_path(manifest).display().to_string());
    args
}

pub fn docker_environment(
    manifest: &StacksteadManifest,
) -> anyhow::Result<(Vec<String>, BTreeMap<String, String>)> {
    let generated = manifest.validated_environment()?;
    Ok(environment_for_generated(manifest, &generated))
}

pub(super) fn environment_for_generated(
    manifest: &StacksteadManifest,
    generated: &BTreeMap<String, String>,
) -> (Vec<String>, BTreeMap<String, String>) {
    let removed = generated
        .keys()
        .filter(|key| !crate::config::reserved_process_env(key))
        .cloned()
        .collect();
    let environment = BTreeMap::from([(
        "COMPOSE_PROJECT_NAME".into(),
        manifest.compose_project.clone(),
    )]);
    (removed, environment)
}

pub(super) fn sanitize_generated_error(
    error: &anyhow::Error,
    generated: &BTreeMap<String, String>,
) -> anyhow::Error {
    // Generated keys are absent from the runner's subprocess and redaction environment,
    // but Compose still reads their values through --env-file.
    anyhow::anyhow!(command::redact_with_env(&format!("{error:#}"), generated))
}

pub(super) fn run_docker_compose(
    manifest: &StacksteadManifest,
    args: &[String],
) -> anyhow::Result<std::process::Output> {
    verify_ownership_override(manifest)?;
    run_docker(manifest, args)
}

fn run_docker(
    manifest: &StacksteadManifest,
    args: &[String],
) -> anyhow::Result<std::process::Output> {
    let (removed, environment) = docker_environment(manifest)?;
    command::run_sanitized(
        "docker",
        args,
        &manifest.worktree,
        &environment,
        removed.iter().map(String::as_str),
    )
}

pub(super) fn run_docker_control(
    manifest: &StacksteadManifest,
    args: &[String],
    deadline: Option<std::time::Instant>,
) -> anyhow::Result<std::process::Output> {
    let removed = manifest
        .env_keys
        .iter()
        .filter(|key| !crate::config::reserved_process_env(key))
        .map(String::as_str);
    let environment = BTreeMap::from([(
        "COMPOSE_PROJECT_NAME".into(),
        manifest.compose_project.clone(),
    )]);
    let cwd = if manifest.worktree.is_dir() {
        &manifest.worktree
    } else {
        &manifest.repo_root
    };
    command::run_sanitized_until("docker", args, cwd, &environment, removed, deadline)
}
