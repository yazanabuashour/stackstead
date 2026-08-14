use super::{
    source_contract::{
        assert_generated_agent_contract, assert_manifest_identity, assert_source_publication,
    },
    *,
};

#[test]
fn create_generates_the_durable_runtime_contract() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    assert_manifest_identity(&project, &manifest)?;
    assert_generated_agent_contract(&manifest)?;
    assert_source_publication(&project, &manifest)
}

#[test]
fn two_stacksteads_have_distinct_runtime_identity_and_state() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let first = project.create("feature-a")?;
    let second = project.create("feature-b")?;

    assert_ne!(
        first.stackstead_id, second.stackstead_id,
        "test contract values unexpectedly match"
    );
    assert_ne!(
        first.worktree, second.worktree,
        "test contract values unexpectedly match"
    );
    assert_ne!(
        first.stackstead_root, second.stackstead_root,
        "test contract values unexpectedly match"
    );
    assert_ne!(
        first.compose_project, second.compose_project,
        "test contract values unexpectedly match"
    );
    assert_ne!(
        first.env_file, second.env_file,
        "test contract values unexpectedly match"
    );
    assert_ne!(
        first.agent_context, second.agent_context,
        "test contract values unexpectedly match"
    );
    assert_ne!(
        first.manifest_path(),
        second.manifest_path(),
        "test contract values unexpectedly match"
    );
    assert_ne!(
        first.pointer_file, second.pointer_file,
        "test contract values unexpectedly match"
    );
    let first_ports = first.ports.values().copied().collect::<BTreeSet<_>>();
    let second_ports = second.ports.values().copied().collect::<BTreeSet<_>>();
    assert!(
        first_ports.is_disjoint(&second_ports),
        "test contract condition failed"
    );
    assert_eq!(
        first.ports.keys().collect::<Vec<_>>(),
        second.ports.keys().collect::<Vec<_>>(),
        "test contract values differ"
    );

    let assert = stackstead(&project.repo)
        .args(["ps", "--json"])
        .assert()
        .success();
    let listed: Value = serde_json::from_slice(&assert.get_output().stdout)
        .test_context("parse stackstead list")?;
    assert_eq!(
        listed["kind"], "StacksteadList",
        "test contract values differ"
    );
    assert_eq!(listed["version"], "1", "test contract values differ");
    let listed = listed["stacksteads"]
        .as_array()
        .test_context("stackstead list items")?;
    assert_eq!(listed.len(), 2, "test contract values differ");
    assert_eq!(
        listed
            .iter()
            .filter_map(|item| item["stackstead_id"].as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([first.stackstead_id.as_str(), second.stackstead_id.as_str()]),
        "test contract values differ"
    );
    assert!(
        listed
            .iter()
            .all(|item| matches!(item["runtime"].as_str(), Some("stopped" | "unknown"))),
        "test contract condition failed"
    );
    Ok(())
}
