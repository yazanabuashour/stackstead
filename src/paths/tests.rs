use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};
use std::collections::BTreeMap;

use chrono::Utc;

use super::*;
use crate::manifest::{ManifestStatus, SourceOwnership, StacksteadManifest};

#[test]
fn generated_paths_cannot_escape() -> anyhow::Result<()> {
    (safe_generated_path(Path::new("/tmp/cell/source"), Path::new("../other"))).test_err()?;
    assert_eq!(
        safe_generated_path(Path::new("/tmp/cell/source"), Path::new(".stackstead/.env")).test()?,
        Path::new("/tmp/cell/source/.stackstead/.env")
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn generated_paths_cannot_traverse_symlinks() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let worktree = directory.path().join("source");
    let outside = directory.path().join("outside");
    std::fs::create_dir_all(&worktree).test()?;
    std::fs::create_dir_all(&outside).test()?;
    std::os::unix::fs::symlink(&outside, worktree.join(".stackstead")).test()?;
    (safe_generated_path(&worktree, Path::new(".stackstead/.env"))).test_err()?;

    std::fs::remove_dir_all(&worktree).test()?;
    std::os::unix::fs::symlink(&outside, &worktree).test()?;
    (safe_generated_path(&worktree, Path::new(".stackstead/.env"))).test_err()?;
    Ok(())
}

#[test]
fn destroy_requires_exact_layout() -> anyhow::Result<()> {
    let root = PathBuf::from("/tmp/state/demo/cell-a");
    let manifest = StacksteadManifest {
        kind: "StacksteadManifest".into(),
        version: crate::manifest::MANIFEST_VERSION.into(),
        stackstead_id: "cell-a".into(),
        slug: "cell".into(),
        short_id: "a".into(),
        runtime_token: "0123456789abcdef0123456789abcdef".into(),
        project: "demo".into(),
        branch: "cell".into(),
        base: "main".into(),
        readiness: crate::readiness::Contract::Unconfigured {},
        source_ownership: SourceOwnership::Stackstead,
        repo_root: "/tmp/repo".into(),
        project_state_root: "/tmp/state".into(),
        stackstead_root: root.clone(),
        worktree: root.join("source"),
        state_dir: root.join("state"),
        port_lease_state_dir: None,
        compose_project: "demo-cell-a".into(),
        compose_files: vec![],
        ports: BTreeMap::new(),
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
    };
    validate_destroy_target(&manifest, Path::new("/tmp/state")).test()?;
    let mut unsafe_manifest = manifest;
    unsafe_manifest.stackstead_root = PathBuf::from("/tmp");
    (validate_destroy_target(&unsafe_manifest, Path::new("/tmp/state"))).test_err()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn destroy_rejects_a_symlinked_project_state_directory() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let state_root = directory.path().join("state");
    let outside_parent = directory.path().join("outside/demo");
    let root = state_root.join("demo/cell-a");
    std::fs::create_dir_all(root.join("source")).test()?;
    std::fs::create_dir_all(root.join("state")).test()?;
    std::fs::create_dir_all(&state_root).test()?;
    std::fs::create_dir_all(&outside_parent).test()?;
    std::fs::remove_dir_all(state_root.join("demo")).test()?;
    std::os::unix::fs::symlink(&outside_parent, state_root.join("demo")).test()?;

    let manifest = StacksteadManifest {
        kind: "StacksteadManifest".into(),
        version: crate::manifest::MANIFEST_VERSION.into(),
        stackstead_id: "cell-a".into(),
        slug: "cell".into(),
        short_id: "a".into(),
        runtime_token: "0123456789abcdef0123456789abcdef".into(),
        project: "demo".into(),
        branch: "cell".into(),
        base: "main".into(),
        readiness: crate::readiness::Contract::Unconfigured {},
        source_ownership: SourceOwnership::Stackstead,
        repo_root: directory.path().join("repo"),
        project_state_root: state_root.clone(),
        stackstead_root: root.clone(),
        worktree: root.join("source"),
        state_dir: root.join("state"),
        port_lease_state_dir: None,
        compose_project: "demo-cell-a".into(),
        compose_files: vec![],
        ports: BTreeMap::new(),
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
    };

    (validate_destroy_target(&manifest, &state_root)).test_err()?;
    Ok(())
}
