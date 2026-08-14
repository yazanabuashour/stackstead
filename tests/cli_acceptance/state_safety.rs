use super::*;

#[test]
fn in_repo_state_is_rejected_before_creating_state() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    project.replace_config("root: ../.stacksteads", "root: .stacksteads")?;
    let rejected = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    assert!(
        output_text(&rejected.get_output().stderr)?
            .contains("state.root must resolve outside the repository"),
        "test contract condition failed"
    );
    assert!(
        !project.repo.join(".stacksteads").exists(),
        "test contract condition failed"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn create_rejects_a_project_lock_symlink_without_touching_its_target() -> anyhow::Result<()> {
    use std::os::unix::fs::symlink;

    let project = Project::initialized()?;
    let marker = project.repo.parent().test()?.join("lock-marker");
    fs::write(&marker, "unchanged\n").test()?;
    let lock = project
        .repo
        .parent()
        .test()?
        .join(".stacksteads/demo-project/project.lock");
    fs::create_dir_all(lock.parent().test()?).test()?;
    symlink(&marker, &lock).test()?;

    stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    assert_eq!(
        fs::read_to_string(&marker).test()?,
        "unchanged\n",
        "test contract values differ"
    );
    assert!(
        fs::symlink_metadata(&lock).test()?.file_type().is_symlink(),
        "test contract condition failed"
    );
    for entry in fs::read_dir(lock.parent().test()?).test()? {
        assert!(
            !entry.test()?.file_type().test()?.is_dir(),
            "test contract condition failed"
        );
    }

    fs::remove_file(&lock).test()?;
    let manifest = project.create("feature-a")?;
    assert!(
        manifest.manifest_path().is_file(),
        "test contract condition failed"
    );
    assert_eq!(
        fs::read_to_string(marker).test()?,
        "unchanged\n",
        "test contract values differ"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn state_parent_symlinks_are_resolved_to_safe_external_targets() -> anyhow::Result<()> {
    use std::os::unix::fs::symlink;

    let project = Project::initialized()?;
    project.replace_config("root: ../.stacksteads", "root: .stacksteads")?;
    let outside = project.repo.parent().test()?.join("outside-state-root");
    let project_target = outside.join("demo-project");
    fs::create_dir_all(&project_target).test()?;
    let link = project.repo.join(".stacksteads");
    let lock_target = project_target.join("project.lock");
    fs::write(&lock_target, "unchanged\n").test()?;
    symlink(&outside, &link).test()?;
    git(&project.repo, &["add", ".stacksteads"])?;
    git(&project.repo, &["commit", "-m", "add external state alias"])?;

    let created = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .success();
    assert!(
        !created.get_output().stdout.is_empty(),
        "test contract condition failed"
    );
    assert!(
        fs::read_to_string(lock_target)
            .test()?
            .contains("acquired_at="),
        "test contract condition failed"
    );
    Ok(())
}
