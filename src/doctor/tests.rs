use crate::test_support::TestResultExt as _;
use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::Utc;

use super::{
    state::{diagnose_duplicate_ports, diagnose_project_lock, read_manifests},
    tools::diagnose_repository_policy,
    *,
};
use crate::{
    manifest::{ManifestStatus, SourceOwnership, StacksteadManifest},
    repository_policy,
};

fn manifest(id: &str, port: u16) -> anyhow::Result<StacksteadManifest> {
    let root = PathBuf::from(format!("/tmp/state/demo/{id}"));
    Ok(StacksteadManifest {
        kind: "StacksteadManifest".into(),
        version: crate::manifest::MANIFEST_VERSION.into(),
        stackstead_id: id.into(),
        slug: "feature".into(),
        short_id: id.rsplit('-').next().test()?.into(),
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
    })
}

#[test]
fn diagnostic_severity_displays_stably() -> anyhow::Result<()> {
    assert_eq!(
        DiagnosticSeverity::Warning.to_string(),
        "warning",
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn repository_policy_reports_missing_and_current_files() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let mut diagnostics = Vec::new();
    diagnose_repository_policy(directory.path(), &mut diagnostics);
    assert_eq!(diagnostics.len(), 1, "test contract values differ");
    assert_eq!(
        diagnostics[0].code, "repository_policy.missing",
        "test contract values differ"
    );
    assert_eq!(
        diagnostics[0].severity,
        DiagnosticSeverity::Warning,
        "test contract values differ"
    );

    std::fs::write(
        directory.path().join("AGENTS.md"),
        format!(
            "{}\n{}",
            repository_policy::marker(),
            repository_policy::TEXT
        ),
    )
    .test()?;
    std::fs::write(directory.path().join("CLAUDE.md"), "# Other policy\n").test()?;
    diagnostics.clear();
    diagnose_repository_policy(directory.path(), &mut diagnostics);
    assert_eq!(diagnostics.len(), 1, "test contract values differ");
    assert_eq!(
        diagnostics[0].code, "repository_policy.current",
        "test contract values differ"
    );
    assert_eq!(
        diagnostics[0].severity,
        DiagnosticSeverity::Info,
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn repository_policy_reports_older_and_newer_markers() -> anyhow::Result<()> {
    for (version, code) in [
        (repository_policy::VERSION - 1, "repository_policy.outdated"),
        (
            repository_policy::VERSION + 1,
            "repository_policy.binary_outdated",
        ),
    ] {
        let directory = tempfile::tempdir().test()?;
        std::fs::write(
            directory.path().join("AGENTS.md"),
            format!("<!-- stackstead-policy: {version} -->\n"),
        )
        .test()?;
        let mut diagnostics = Vec::new();
        diagnose_repository_policy(directory.path(), &mut diagnostics);
        assert_eq!(diagnostics.len(), 1, "test contract values differ");
        assert_eq!(diagnostics[0].code, code, "test contract values differ");
        assert_eq!(
            diagnostics[0].severity,
            DiagnosticSeverity::Warning,
            "test contract values differ"
        );
    }
    Ok(())
}

#[test]
fn repository_policy_reports_unversioned_and_invalid_markers() -> anyhow::Result<()> {
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
        let directory = tempfile::tempdir().test()?;
        std::fs::write(directory.path().join("CLAUDE.md"), contents).test()?;
        let mut diagnostics = Vec::new();
        diagnose_repository_policy(directory.path(), &mut diagnostics);
        assert_eq!(diagnostics.len(), 1, "test contract values differ");
        assert_eq!(diagnostics[0].code, code, "test contract values differ");
        assert_eq!(
            diagnostics[0].severity,
            DiagnosticSeverity::Warning,
            "test contract values differ"
        );
    }
    Ok(())
}

#[test]
fn reports_duplicate_port_allocations() -> anyhow::Result<()> {
    let manifests = [
        manifest("feature-a111", 39000)?,
        manifest("feature-b222", 39000)?,
    ];
    let mut diagnostics = Vec::new();
    diagnose_duplicate_ports(&manifests, &mut diagnostics);
    assert_eq!(diagnostics.len(), 1, "test contract values differ");
    assert_eq!(
        diagnostics[0].code, "ports.duplicate_allocation",
        "test contract values differ"
    );
    assert!(
        diagnostics[0].message.contains("feature-a111:web"),
        "test contract condition failed"
    );
    assert!(
        diagnostics[0].message.contains("feature-b222:web"),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn project_doctor_finds_fixed_ports_without_requiring_docker() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
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
    .test()?;
    std::fs::write(
        directory.path().join("docker-compose.yml"),
        "services:\n  web:\n    ports:\n      - \"3000:3000\"\n",
    )
    .test()?;

    let diagnostics = run(directory.path());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "config.valid"),
        "{diagnostics:#?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "compose.fixed_host_port"),
        "test contract condition failed"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "compose.all_interfaces_host_port"
                && diagnostic.severity == DiagnosticSeverity::Error
        }),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn unreadable_manifest_becomes_a_diagnostic() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let project = directory.path().join("demo/cell/state");
    std::fs::create_dir_all(&project).test()?;
    std::fs::write(project.join("manifest.json"), "not json").test()?;
    let mut diagnostics = Vec::new();
    let manifests = read_manifests(&directory.path().join("demo"), &mut diagnostics);
    assert!(manifests.is_empty(), "test contract condition failed");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "manifest.unreadable"),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn missing_project_lock_is_an_error() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let mut diagnostics = Vec::new();
    diagnose_project_lock(directory.path(), &mut diagnostics);
    assert_eq!(diagnostics.len(), 1, "test contract values differ");
    assert_eq!(
        diagnostics[0].code, "lock.project.missing",
        "test contract values differ"
    );
    assert_eq!(
        diagnostics[0].severity,
        DiagnosticSeverity::Error,
        "test contract values differ"
    );
    assert!(
        !diagnostics[0].message.contains("stale"),
        "test contract condition failed"
    );
    Ok(())
}
