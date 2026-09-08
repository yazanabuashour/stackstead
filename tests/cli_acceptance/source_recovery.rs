use super::*;

#[test]
fn nested_worktree_commands_use_the_pointer_before_the_copied_config() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let nested = manifest.worktree.join("scratch/deeply/nested");
    fs::create_dir_all(&nested).test_context("create nested worktree directory")?;

    let assert = stackstead(&nested)
        .args(["context", "feature-a", "--json"])
        .assert()
        .success();
    let output: Value =
        serde_json::from_slice(&assert.get_output().stdout).test_context("parse context output")?;
    assert_eq!(output["stackstead_id"], manifest.stackstead_id);
    assert_eq!(
        output["path"],
        manifest.agent_context.to_string_lossy().as_ref()
    );
    Ok(())
}

#[test]
fn pointer_state_root_cannot_normalize_to_the_filesystem_root() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let mut manifest = project.create("feature-a")?;
    let mut pointer: StacksteadPointer =
        serde_json::from_slice(&fs::read(&manifest.pointer_file).test_context("read pointer")?)
            .test_context("parse pointer")?;
    manifest.project_state_root = PathBuf::from("/tmp/..");
    pointer.project_state_root = manifest.project_state_root.clone();
    manifest
        .write_fixture()
        .test_context("write tampered manifest")?;
    fs::write(
        &manifest.pointer_file,
        serde_json::to_vec_pretty(&pointer).test_context("serialize pointer")?,
    )
    .test_context("write tampered pointer")?;

    let assert = stackstead(&manifest.worktree)
        .args(["context", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr)?.contains("filesystem root"));
    Ok(())
}

#[test]
fn legacy_pointer_v1_discovers_normally_and_repair_rewrites_v2() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let mut pointer: StacksteadPointer =
        serde_json::from_slice(&fs::read(&manifest.pointer_file).test()?).test()?;
    pointer.version = "1".into();
    fs::write(
        &manifest.pointer_file,
        serde_json::to_vec_pretty(&pointer).test()?,
    )
    .test()?;
    stackstead(&manifest.worktree)
        .args(["context", "feature-a", "--json"])
        .assert()
        .success();
    stackstead(&project.repo)
        .args(["repair", "feature-a", "--json"])
        .assert()
        .success();
    let rewritten: StacksteadPointer =
        serde_json::from_slice(&fs::read(&manifest.pointer_file).test()?).test()?;
    assert_eq!(rewritten.version, "2");
    Ok(())
}

#[test]
fn destroy_recovers_a_persisted_prepublication_create() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let mut manifest = project.create("feature-a")?;
    fs::remove_file(&manifest.pointer_file).test()?;
    fs::remove_file(&manifest.event_log).test()?;
    manifest.status.source = ComponentStatus::Created;
    manifest.write_fixture().test()?;
    stackstead(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(!manifest.stackstead_root.exists());
    let registry: Value = serde_json::from_slice(
        &fs::read(test_state_home(&project.repo).join("stackstead/port-leases.json")).test()?,
    )
    .test()?;
    assert!(registry["leases"].as_array().test()?.is_empty());
    Ok(())
}
