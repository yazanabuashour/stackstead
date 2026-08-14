use super::*;

#[cfg(unix)]
#[test]
fn adopted_worktree_is_bound_but_preserved_on_destroy() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let external = project
        .repo
        .parent()
        .test_context("repository has parent")?
        .join("manager-owned");
    git(
        &project.repo,
        &[
            "worktree",
            "add",
            "-b",
            "manager-feature",
            external.to_str().test_context("UTF-8 fixture path")?,
            "main",
        ],
    )?;
    let adopted = stackstead(&project.repo)
        .args([
            "--json",
            "adopt",
            "manager-feature",
            "--worktree",
            external.to_str().test_context("UTF-8 fixture path")?,
        ])
        .assert()
        .success();
    let manifest = changed_manifest(&adopted.get_output().stdout, "adopted")?;
    assert_eq!(
        manifest.source_ownership,
        SourceOwnership::External,
        "test contract values differ"
    );
    assert_eq!(manifest.worktree, external, "test contract values differ");
    assert!(
        manifest.pointer_file.is_file(),
        "test contract condition failed"
    );

    stackstead(&project.repo)
        .arg("adopt")
        .arg("duplicate-manager-feature")
        .arg("--worktree")
        .arg(&external)
        .assert()
        .failure();

    assert!(
        manifest.pointer_file.is_file(),
        "test contract condition failed"
    );
    assert!(
        manifest.manifest_path().is_file(),
        "test contract condition failed"
    );

    let path = fake_docker_path(
        project.repo.parent().test()?,
        "adopt-fake-docker-bin",
        "#!/bin/sh\nexit 0\n",
    )?;
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", "manager-feature", "--yes"])
        .assert()
        .success();

    assert!(external.is_dir(), "manager-owned worktree was removed");
    assert!(
        !external.join(".stackstead").exists(),
        "test contract condition failed"
    );
    assert!(
        !manifest.stackstead_root.exists(),
        "test contract condition failed"
    );
    assert_eq!(
        git(&external, &["branch", "--show-current"])?.trim(),
        "manager-feature",
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn adoption_rejects_a_manager_worktree_that_does_not_contain_the_pinned_base() -> anyhow::Result<()>
{
    let project = Project::initialized()?;
    let external = project.repo.parent().test()?.join("stale-manager-owned");
    git(
        &project.repo,
        &[
            "worktree",
            "add",
            "-b",
            "stale-manager-feature",
            external.to_str().test()?,
            "main",
        ],
    )?;
    fs::write(project.repo.join("README.md"), "# Advanced base\n").test()?;
    git(&project.repo, &["add", "README.md"])?;
    git(
        &project.repo,
        &["commit", "-m", "advance base before adoption"],
    )?;

    let rejected = stackstead(&project.repo)
        .args(["adopt", "stale-manager-feature", "--worktree"])
        .arg(&external)
        .assert()
        .failure();
    assert!(
        output_text(&rejected.get_output().stderr)?.contains("not based on pinned commit"),
        "test contract condition failed"
    );
    assert!(
        !external.join(".stackstead").exists(),
        "test contract condition failed"
    );
    assert!(
        state_stackstead_directories(&project)?.is_empty(),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn adoption_rejects_nested_unrelated_and_detached_checkouts_without_state() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let parent = project.repo.parent().test_context("repository parent")?;

    let registered = parent.join("registered-manager-worktree");
    git(
        &project.repo,
        &[
            "worktree",
            "add",
            "-b",
            "registered-manager",
            registered.to_str().test_context("UTF-8 fixture path")?,
            "main",
        ],
    )?;
    let nested = registered.join("nested");
    fs::create_dir(&nested).test_context("create nested checkout path")?;
    stackstead(&project.repo)
        .arg("adopt")
        .arg("nested")
        .arg("--worktree")
        .arg(&nested)
        .assert()
        .failure();

    let mut stale_compose = fs::read_to_string(registered.join("docker-compose.yml"))
        .test_context("read manager Compose")?;
    stale_compose.push_str("# stale manager contract\n");
    fs::write(registered.join("docker-compose.yml"), stale_compose)
        .test_context("change manager Compose contract")?;
    stackstead(&project.repo)
        .arg("adopt")
        .arg("stale-manager")
        .arg("--worktree")
        .arg(&registered)
        .assert()
        .failure();

    let unrelated = parent.join("unrelated-repository");
    fs::create_dir(&unrelated).test_context("create unrelated repository")?;
    git(&unrelated, &["init", "--initial-branch=other"])?;
    git(&unrelated, &["config", "user.name", "Stackstead Tests"])?;
    git(
        &unrelated,
        &["config", "user.email", "stackstead-tests@example.invalid"],
    )?;
    fs::write(unrelated.join("README.md"), "unrelated\n")
        .test_context("write unrelated fixture")?;
    git(&unrelated, &["add", "."])?;
    git(&unrelated, &["commit", "-m", "unrelated fixture"])?;
    stackstead(&project.repo)
        .arg("adopt")
        .arg("unrelated")
        .arg("--worktree")
        .arg(&unrelated)
        .assert()
        .failure();

    let detached = parent.join("detached-manager-worktree");
    git(
        &project.repo,
        &[
            "worktree",
            "add",
            "--detach",
            detached.to_str().test_context("UTF-8 fixture path")?,
            "main",
        ],
    )?;
    stackstead(&project.repo)
        .arg("adopt")
        .arg("detached")
        .arg("--worktree")
        .arg(&detached)
        .assert()
        .failure();

    assert!(registered.is_dir(), "test contract condition failed");
    assert!(unrelated.is_dir(), "test contract condition failed");
    assert!(detached.is_dir(), "test contract condition failed");
    assert!(
        state_stackstead_directories(&project)?.is_empty(),
        "test contract condition failed"
    );
    assert!(
        !registered.join(".stackstead").exists(),
        "test contract condition failed"
    );
    assert!(
        !unrelated.join(".stackstead").exists(),
        "test contract condition failed"
    );
    assert!(
        !detached.join(".stackstead").exists(),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn create_rejects_a_compose_template_that_omits_the_durable_identity() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    project.replace_config(
        "{{ project.name }}-{{ stackstead.id }}",
        "{{ project.name }}",
    )?;
    let assert = stackstead(&project.repo)
        .args(["create", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)?.contains("must render the durable identity"),
        "test contract condition failed"
    );
    assert!(
        state_stackstead_directories(&project)?.is_empty(),
        "test contract condition failed"
    );
    assert!(
        git(&project.repo, &["branch", "--list", "feature-a"])?
            .trim()
            .is_empty(),
        "test contract condition failed"
    );
    Ok(())
}
