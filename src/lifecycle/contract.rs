use std::{collections::BTreeMap, path::Path};

use crate::{
    command, compose,
    config::{CommandConfig, StacksteadConfig},
    context, envfile,
    manifest::{POINTER_VERSION, StacksteadManifest, StacksteadPointer, write_pointer},
    paths,
    template::{TemplateContext, render_template},
};

use super::validation::validate_contract_binding;

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
    Ok(())
}

pub(super) fn write_contract(
    config: &StacksteadConfig,
    manifest: &mut StacksteadManifest,
    context_values: &TemplateContext,
) -> anyhow::Result<()> {
    validate_generated_paths(config, manifest)?;
    let generated = config
        .env
        .generate
        .iter()
        .map(|(key, template)| Ok((key.clone(), render_template(template, context_values)?)))
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    manifest.env_keys = generated.keys().cloned().collect();
    envfile::write_generated(manifest, &generated)?;
    context::write_agent_context(manifest, &config.agent.rules)?;
    compose::write_ownership_override(manifest)?;
    write_pointer(
        &manifest.pointer_file,
        &StacksteadPointer {
            kind: "StacksteadPointer".into(),
            version: POINTER_VERSION.into(),
            stackstead_id: manifest.stackstead_id.clone(),
            manifest: manifest.manifest_path(),
            project: manifest.project.clone(),
            repo_root: manifest.repo_root.clone(),
            project_state_root: manifest.project_state_root.clone(),
            stackstead_root: manifest.stackstead_root.clone(),
        },
    )?;
    manifest.save_atomic()?;
    Ok(())
}

fn validate_generated_paths(
    config: &StacksteadConfig,
    manifest: &StacksteadManifest,
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
    Ok(())
}

pub fn template_context(manifest: &StacksteadManifest) -> TemplateContext {
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

pub(super) fn run_commands(
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
