use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};
use std::{collections::BTreeMap, fs, process::ExitStatus};

use chrono::Utc;

use super::*;
use crate::manifest::{ManifestStatus, SourceOwnership, StacksteadManifest};
use crate::{config::StacksteadConfig, state::ProjectPaths};

fn manifest(root: &Path) -> anyhow::Result<StacksteadManifest> {
    let short_id = "a17ca17ca17ca17ca17ca17ca17ca17c";
    let stackstead_id = format!("feature-a-{short_id}");
    let stackstead_root = root.join("demo").join(&stackstead_id);
    let worktree = stackstead_root.join("source");
    let state_dir = stackstead_root.join("state");
    fs::create_dir_all(worktree.join(".stackstead")).test()?;
    fs::create_dir_all(&state_dir).test()?;
    Ok(StacksteadManifest {
        kind: "StacksteadManifest".into(),
        version: crate::manifest::MANIFEST_VERSION.into(),
        stackstead_id: stackstead_id.clone(),
        slug: "feature-a".into(),
        short_id: short_id.into(),
        runtime_token: "0123456789abcdef0123456789abcdef".into(),
        project: "demo".into(),
        branch: "feature-a".into(),
        base: "main".into(),
        source_ownership: SourceOwnership::Stackstead,
        repo_root: root.join("repo"),
        project_state_root: root.to_path_buf(),
        stackstead_root,
        worktree: worktree.clone(),
        state_dir: state_dir.clone(),
        port_lease_state_dir: None,
        compose_project: format!("demo-{stackstead_id}"),
        compose_files: vec![],
        ports: BTreeMap::new(),
        container_ports: BTreeMap::new(),
        urls: BTreeMap::new(),
        env_file: worktree.join(".stackstead/.env"),
        agent_context: worktree.join(".stackstead/AGENT_CONTEXT.md"),
        pointer_file: worktree.join(".stackstead/stackstead.json"),
        event_log: state_dir.join("events.jsonl"),
        env_keys: vec![],
        status: ManifestStatus::default(),
        database: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    })
}

#[cfg(unix)]
#[test]
fn command_preserves_arguments_cwd_environment_and_exit_status() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().test()?;
    let root = directory.path().canonicalize().test()?;
    let manifest = manifest(&root)?;
    let script = root.join("probe");
    let script_body = format!(
        r#"#!/bin/sh
test "$API_TOKEN" = "private" || exit 90
test "$STACKSTEAD_ID" = "{}" || exit 91
test "$COMPOSE_PROJECT_NAME" = "{}" || exit 92
printf '%s\n' "$PWD" "$1" "$2" "$STACKSTEAD_MANIFEST" "$STACKSTEAD_CONTEXT"
exit "$3"
"#,
        manifest.stackstead_id, manifest.compose_project
    );
    fs::write(&script, script_body).test()?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).test()?;
    let generated = BTreeMap::from([
        ("API_TOKEN".into(), "private".into()),
        ("STACKSTEAD_ID".into(), "spoofed".into()),
        ("COMPOSE_PROJECT_NAME".into(), "shared".into()),
    ]);
    let environment = manifest.trusted_environment(&generated);
    let args = ["one argument".into(), "; touch nowhere".into(), "23".into()];

    let output = command(&manifest, script.as_os_str(), &args, &environment, &[])
        .output()
        .test()?;

    assert_eq!(exit_code(output.status), 23, "test contract values differ");
    assert_eq!(
        String::from_utf8(output.stdout).test()?,
        format!(
            "{}\none argument\n; touch nowhere\n{}\n{}\n",
            manifest.worktree.display(),
            manifest.manifest_path().display(),
            manifest.agent_context.display()
        ),
        "test contract values differ"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn signal_status_uses_conventional_shell_exit_code() -> anyhow::Result<()> {
    use std::os::unix::process::ExitStatusExt;

    let status = ExitStatus::from_raw(9);
    assert_eq!(exit_code(status), 137, "test contract values differ");
    Ok(())
}

#[test]
fn validation_rejects_contract_files_outside_the_worktree() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let mut manifest = manifest(directory.path())?;
    let compose = manifest.worktree.join("docker-compose.yml");
    fs::write(&compose, "services: {}\n").test()?;
    manifest.compose_files = vec![compose];
    fs::write(&manifest.agent_context, "# context\n").test()?;
    let mut config = StacksteadConfig::default();
    config.project.name = "demo".into();
    let runtime = lifecycle::ProjectRuntime {
        config,
        paths: ProjectPaths::new(
            manifest.repo_root.clone(),
            directory.path().to_path_buf(),
            "demo",
        ),
    };
    lifecycle::validate_manifest_binding(&runtime, &manifest).test()?;

    manifest.env_file = directory.path().join("shared.env");
    (lifecycle::validate_manifest_binding(&runtime, &manifest)).test_err()?;
    Ok(())
}
