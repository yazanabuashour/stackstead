use std::{collections::BTreeMap, path::Path};

use chrono::Utc;

use crate::{
    compose, events,
    manifest::{ManifestStatus, SourceOwnership, StacksteadManifest},
    state::ProjectPaths,
    test_support::{TestResultErrorExt as _, TestResultExt as _},
};

use super::{
    project::default_config,
    teardown::{TeardownPhase, validate_completed_source_cleanup, write_teardown},
    types::ProjectRuntime,
    validation::{validate_compose_project, validate_manifest_binding},
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
fn generated_config_parses() -> anyhow::Result<()> {
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
    let yaml = default_config("demo", "main", &plan).test()?;
    let config = crate::config::StacksteadConfig::from_yaml(&yaml).test()?;
    assert_eq!(config.project.name, "demo");
    assert_eq!(config.resources.ports.expose.len(), 2);
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
fn manifest_binding_rejects_mismatched_port_service_sets() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let mut manifest = cleanup_manifest(directory.path(), SourceOwnership::Stackstead)?;
    manifest.ports.insert("web".into(), 39000);
    let mut config = crate::config::StacksteadConfig::default();
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
        .test_err()?
        .to_string();
    assert_eq!(
        error,
        "manifest host and container port service sets differ"
    );
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
