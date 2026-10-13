use std::{collections::BTreeMap, path::Path};

use chrono::Utc;

use crate::{
    events,
    lock::LockGuard,
    manifest::{ManifestStatus, SourceOwnership, StacksteadManifest},
    test_support::{TestResultErrorExt as _, TestResultExt as _},
};

use super::{
    CreateOutcome, HeldEnvironment,
    teardown::{TeardownPhase, validate_completed_source_cleanup, write_teardown},
    validation::validate_compose_project,
};

fn cleanup_manifest(root: &Path, ownership: SourceOwnership) -> anyhow::Result<StacksteadManifest> {
    let short_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let stackstead_id = format!("feature-a-{short_id}");
    let stackstead_root = root.join("state/demo").join(&stackstead_id);
    let worktree = match ownership {
        SourceOwnership::Stackstead => stackstead_root.join("source"),
        SourceOwnership::External => root.join("manager-source"),
    };
    let state_dir = stackstead_root.join("state");
    std::fs::create_dir_all(&state_dir).test()?;
    if ownership == SourceOwnership::External {
        std::fs::create_dir_all(&worktree).test()?;
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
        events::append(&event_log, event_type, status, None).test()?;
    }
    Ok(StacksteadManifest {
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
        readiness: crate::readiness::Contract::Unconfigured {},
        database: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    })
}

#[test]
fn held_environment_reloads_state_and_transfers_only_the_run_lease() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let mut manifest = cleanup_manifest(directory.path(), SourceOwnership::Stackstead)?;
    let mutation_path = manifest.state_dir.join("lock");
    let run_path = manifest.state_dir.join("run.lock");
    let mutation_lock = LockGuard::acquire(&mutation_path, "test creation").test()?;
    drop(LockGuard::acquire(&run_path, "test run lease").test()?);
    manifest.save_atomic().test()?;
    let created = CreateOutcome::new(manifest.clone(), mutation_lock);
    manifest.status.dependencies = crate::manifest::ComponentStatus::Ready;
    manifest.save_atomic().test()?;

    let held = HeldEnvironment::after_create(created, &manifest).test()?;
    assert_eq!(
        held.manifest().status.dependencies,
        manifest.status.dependencies
    );
    assert!(!LockGuard::can_acquire(&mutation_path));
    assert!(!LockGuard::can_acquire(&run_path));
    let held = held.for_run(&manifest).test()?;
    assert!(!LockGuard::can_acquire(&mutation_path));
    let (resolved, lease) = held.into_run().test()?;
    assert_eq!(resolved.stackstead_id, manifest.stackstead_id);
    assert!(LockGuard::can_acquire(&mutation_path));
    assert!(!LockGuard::can_acquire(&run_path));
    drop(lease);
    assert!(LockGuard::can_acquire(&run_path));
    Ok(())
}

#[test]
fn held_environment_rejects_identity_changes_at_acquisition_and_handoff() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let mut manifest = cleanup_manifest(directory.path(), SourceOwnership::Stackstead)?;
    drop(LockGuard::acquire(&manifest.state_dir.join("lock"), "test mutation").test()?);
    drop(LockGuard::acquire(&manifest.state_dir.join("run.lock"), "test run lease").test()?);
    manifest.save_atomic().test()?;
    let mut foreign = manifest.clone();
    foreign.runtime_token = "fedcba9876543210fedcba9876543210".into();
    foreign.save_atomic().test()?;
    let error = HeldEnvironment::exclusive(manifest.clone()).test_err()?;
    assert!(error.to_string().contains("environment identity changed"));

    manifest.save_atomic().test()?;
    let held = HeldEnvironment::exclusive(manifest.clone()).test()?;
    let error = held.for_run(&foreign).test_err()?;
    assert!(error.to_string().contains("environment identity changed"));

    let held = HeldEnvironment::exclusive(manifest.clone()).test()?;
    foreign.save_atomic().test()?;
    let error = held.for_run(&manifest).test_err()?;
    assert!(error.to_string().contains("environment identity changed"));
    Ok(())
}

#[test]
fn compose_project_identity_is_docker_safe() -> anyhow::Result<()> {
    (validate_compose_project("demo-feature-a17c")).test()?;
    (validate_compose_project("Demo-feature")).test_err()?;
    (validate_compose_project("../demo")).test_err()?;
    Ok(())
}

#[test]
fn partial_destroy_retry_requires_truthful_source_cleanup_state() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let external = cleanup_manifest(directory.path(), SourceOwnership::External)?;
    (validate_completed_source_cleanup(&external)).test_err()?;
    write_teardown(&external, TeardownPhase::Finalize, None).test()?;
    validate_completed_source_cleanup(&external).test()?;
    std::fs::create_dir(external.worktree.join(".stackstead")).test()?;
    (validate_completed_source_cleanup(&external)).test_err()?;

    std::fs::remove_dir(external.worktree.join(".stackstead")).test()?;
    let owned = cleanup_manifest(directory.path(), SourceOwnership::Stackstead)?;
    write_teardown(&owned, TeardownPhase::Finalize, None).test()?;
    validate_completed_source_cleanup(&owned).test()?;
    std::fs::create_dir_all(&owned.worktree).test()?;
    (validate_completed_source_cleanup(&owned)).test_err()?;
    Ok(())
}
