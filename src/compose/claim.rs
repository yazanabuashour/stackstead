use crate::manifest::StacksteadManifest;

use super::{
    docker::run_docker_control,
    model::{COMPOSE_PROJECT_LABEL, OWNERSHIP_HELPER_IMAGE, RUNTIME_TOKEN_LABEL},
    resources::{
        verify_labeled_runtime_resources, verify_resource_label, verify_runtime_resources,
    },
};

pub fn prepare_owned_source_removal(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if manifest.source_ownership != crate::manifest::SourceOwnership::Stackstead {
        return Ok(());
    }
    if !runtime_claim_exists(manifest, None)? {
        return Ok(());
    }
    verify_runtime_claim(manifest, None)?;
    if !manifest.worktree.is_dir() {
        anyhow::bail!(
            "managed worktree is missing at {}",
            manifest.worktree.display()
        );
    }
    let source = manifest
        .worktree
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("managed worktree path is not UTF-8"))?;
    let owner = manifest
        .state_dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("state directory path is not UTF-8"))?;
    let helper = format!("{}-stackstead-owner", manifest.compose_project);
    let list = vec![
        "container".into(),
        "ls".into(),
        "--all".into(),
        "--filter".into(),
        format!("name=^/{helper}$"),
        "--filter".into(),
        format!("label={COMPOSE_PROJECT_LABEL}={}", manifest.compose_project),
        "--filter".into(),
        format!("label={RUNTIME_TOKEN_LABEL}={}", manifest.runtime_token),
        "--format".into(),
        "{{.ID}}".into(),
    ];
    let existing = String::from_utf8(run_docker_control(manifest, &list, None)?.stdout)?;
    let existing = existing
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    if existing.len() > 1 {
        anyhow::bail!(
            "more than one exact ownership helper exists for {}",
            manifest.stackstead_id
        );
    }
    if let Some(identifier) = existing.first() {
        verify_resource_label(manifest, "container", identifier, ".Config.Labels", None)?;
        run_docker_control(
            manifest,
            &[
                "container".into(),
                "rm".into(),
                "--force".into(),
                (*identifier).into(),
            ],
            None,
        )?;
    }
    let mut args = vec![
        "run".into(),
        "--rm".into(),
        "--pull=missing".into(),
        "--name".into(),
        helper,
        "--label".into(),
        format!("{COMPOSE_PROJECT_LABEL}={}", manifest.compose_project),
        "--label".into(),
        format!("{RUNTIME_TOKEN_LABEL}={}", manifest.runtime_token),
        "--user".into(),
        "0:0".into(),
    ];
    #[cfg(target_os = "linux")]
    args.push("--userns=host".into());
    // Read the unchanged state owner's IDs inside the helper's namespace. Host IDs
    // differ under rootless Docker, and source ownership may already be damaged.
    args.extend([
        "--mount".into(),
        ownership_bind_mount(source, "/stackstead-source"),
        "--mount".into(),
        format!("{},readonly", ownership_bind_mount(owner, "/stackstead-owner")),
        OWNERSHIP_HELPER_IMAGE.into(),
        "sh".into(),
        "-ceu".into(),
        "owner=$(stat -c '%u:%g' /stackstead-owner); chown -R \"$owner\" /stackstead-source; chmod -R u+rwX /stackstead-source".into(),
    ]);
    run_docker_control(manifest, &args, None)?;
    Ok(())
}

pub(super) fn ownership_bind_mount(source: &str, destination: &str) -> String {
    format!(
        "type=bind,\"src={}\",dst={destination}",
        source.replace('"', "\"\"")
    )
}

pub(super) fn runtime_claim_name(manifest: &StacksteadManifest) -> String {
    format!("{}-stackstead-claim", manifest.compose_project)
}

pub(super) fn ensure_runtime_claim(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    let args = vec![
        "volume".into(),
        "create".into(),
        "--label".into(),
        format!("{COMPOSE_PROJECT_LABEL}={}", manifest.compose_project),
        "--label".into(),
        format!("{RUNTIME_TOKEN_LABEL}={}", manifest.runtime_token),
        runtime_claim_name(manifest),
    ];
    run_docker_control(manifest, &args, None)?;
    Ok(())
}

pub(super) fn verify_runtime_claim(
    manifest: &StacksteadManifest,
    deadline: Option<std::time::Instant>,
) -> anyhow::Result<()> {
    verify_resource_label(
        manifest,
        "volume",
        &runtime_claim_name(manifest),
        ".Labels",
        deadline,
    )
    .map_err(|error| {
        anyhow::anyhow!(
            "Compose namespace `{}` is not owned by runtime token {}: {error}",
            manifest.compose_project,
            manifest.runtime_token
        )
    })
}

pub fn verify_owned_runtime(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if !runtime_claim_exists(manifest, None)? {
        anyhow::bail!(
            "Compose namespace `{}` has no Stackstead ownership claim",
            manifest.compose_project
        );
    }
    verify_runtime_claim(manifest, None)?;
    verify_runtime_resources(manifest, None)?;
    Ok(())
}

pub fn remove_runtime_claim(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if !runtime_claim_exists(manifest, None)? {
        return Ok(());
    }
    verify_runtime_claim(manifest, None)?;
    if verify_labeled_runtime_resources(manifest, None)? {
        anyhow::bail!(
            "Compose namespace `{}` still has Stackstead runtime resources; refusing to remove its ownership claim",
            manifest.compose_project
        );
    }
    let args = vec!["volume".into(), "rm".into(), runtime_claim_name(manifest)];
    run_docker_control(manifest, &args, None)?;
    Ok(())
}

pub(super) fn runtime_claim_exists(
    manifest: &StacksteadManifest,
    deadline: Option<std::time::Instant>,
) -> anyhow::Result<bool> {
    let args = vec![
        "volume".into(),
        "ls".into(),
        "--format".into(),
        "{{.Name}}".into(),
    ];
    let output = run_docker_control(manifest, &args, deadline)?;
    let names = String::from_utf8(output.stdout)?;
    Ok(names
        .lines()
        .map(str::trim)
        .any(|name| name == runtime_claim_name(manifest)))
}
