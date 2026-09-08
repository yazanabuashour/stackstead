use super::*;

#[test]
fn tampered_manifest_destroy_fails_before_external_mutation() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let mut tampered = manifest.clone();
    tampered.repo_root = project.repo.join("different-repository");
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&tampered).test_context("serialize tampered manifest")?,
    )
    .test_context("write tampered manifest")?;

    let assert = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--yes", "--json"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)?
            .contains("project identity does not match the discovered project")
    );
    assert!(assert.get_output().stdout.is_empty());
    assert!(manifest.stackstead_root.is_dir());
    assert!(manifest.worktree.is_dir());
    assert!(!event_types(&manifest.event_log)?.contains(&"destroyed".into()));
    Ok(())
}

#[cfg(unix)]
#[test]
fn repair_rejects_a_generated_directory_symlink_escape() -> anyhow::Result<()> {
    use std::os::unix::fs::symlink;

    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let generated = manifest.worktree.join(".stackstead");
    fs::remove_dir_all(&generated).test_context("remove generated contract directory")?;
    let outside = project
        .repo
        .parent()
        .test_context("repository has parent")?
        .join("escape-target");
    fs::create_dir(&outside).test_context("create escape target")?;
    symlink(&outside, &generated).test_context("create generated-directory symlink")?;

    let assert = stackstead(&project.repo)
        .args(["repair", "feature-a", "--json"])
        .assert()
        .failure();
    let stderr = output_text(&assert.get_output().stderr)?;
    assert!(
        stderr.contains("symlink") || stderr.contains("escapes") || stderr.contains("unsafe"),
        "unexpected symlink error: {stderr}"
    );
    assert!(!outside.join(".env").exists());
    assert!(!outside.join("AGENT_CONTEXT.md").exists());
    assert!(!outside.join("stackstead.json").exists());
    assert!(manifest.stackstead_root.is_dir());
    Ok(())
}

#[test]
fn destroy_refuses_a_dirty_worktree_before_touching_runtime_state() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    fs::write(manifest.worktree.join("README.md"), "dirty\n").test_context("dirty tracked file")?;

    let assert = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--yes", "--json"])
        .assert()
        .failure();
    assert!(
        String::from_utf8_lossy(&assert.get_output().stderr)
            .contains("uncommitted or untracked changes")
    );

    assert!(manifest.manifest_path().is_file());
    assert!(manifest.worktree.is_dir());
    Ok(())
}

#[cfg(unix)]
#[test]
fn destroy_uses_the_durable_manifest_after_non_destructive_config_path_changes()
-> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["env"]["file"] = ".stackstead-next/.env".into();
    config["agent"]["context_file"] = ".stackstead-next/AGENT_CONTEXT.md".into();
    project.write_config(&config, "change future generated paths")?;

    let path = fake_docker_path(
        project.repo.parent().test()?,
        "cleanup-fake-docker-bin",
        "#!/bin/sh\nexit 0\n",
    )?;
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", "feature-a", "--yes"])
        .assert()
        .success();

    assert!(!manifest.stackstead_root.exists());
    assert!(!manifest.worktree.exists());
    Ok(())
}

#[test]
fn repair_rejects_changed_generated_paths_without_writing_them() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["env"]["file"] = ".stackstead-next/.env".into();
    config["agent"]["context_file"] = ".stackstead-next/AGENT_CONTEXT.md".into();
    project.write_config(&config, "change future repair paths")?;

    stackstead(&project.repo)
        .args(["repair", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(!manifest.worktree.join(".stackstead-next").exists());
    assert!(manifest.env_file.is_file());
    assert!(manifest.agent_context.is_file());
    Ok(())
}
