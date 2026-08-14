use super::*;

#[test]
fn create_refuses_a_runtime_contract_missing_from_the_configured_base() -> anyhow::Result<()> {
    let project = Project::git_repo()?;
    stackstead(&project.repo).arg("init").assert().success();

    let assert = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    let stderr = output_text(&assert.get_output().stderr)?;
    assert!(
        stderr.contains("not present on source.base commit")
            && stderr.contains("commit or merge stackstead.yaml"),
        "unexpected error: {stderr}"
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

#[test]
fn create_refuses_locally_modified_contract_files_without_allocating_state() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let compose = project.repo.join("docker-compose.yml");
    let mut contents = fs::read_to_string(&compose).test_context("read Compose fixture")?;
    contents.push_str("# uncommitted runtime change\n");
    fs::write(&compose, contents).test_context("modify Compose fixture")?;

    let assert = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)?.contains("differs from source.base commit"),
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

#[test]
fn create_compares_clean_contract_files_through_git_filters() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    fs::write(
        project.repo.join(".gitattributes"),
        "stackstead.yaml text eol=crlf\ndocker-compose.yml text eol=crlf\n",
    )
    .test_context("write CRLF attributes")?;
    git(&project.repo, &["add", ".gitattributes"])?;
    git(&project.repo, &["add", "--renormalize", "."])?;
    git(&project.repo, &["commit", "-m", "configure CRLF contracts"])?;
    for file in ["stackstead.yaml", "docker-compose.yml"] {
        let path = project.repo.join(file);
        fs::remove_file(&path).test_context("remove LF contract fixture")?;
        git(&project.repo, &["checkout", "--", file])?;
        assert!(
            fs::read(&path)
                .test_context("read filtered contract")?
                .windows(2)
                .any(|bytes| bytes == b"\r\n"),
            "Git did not apply the CRLF checkout filter to {file}"
        );
    }
    let status = git(
        &project.repo,
        &[
            "status",
            "--short",
            "--",
            "stackstead.yaml",
            "docker-compose.yml",
        ],
    )?;
    assert!(
        status.trim().is_empty(),
        "Git did not consider the filtered contract clean: {status:?}"
    );
    let manifest = project.create("feature-a")?;
    assert!(manifest.worktree.is_dir(), "test contract condition failed");
    Ok(())
}

#[test]
fn create_pins_the_configured_base_when_called_from_another_branch() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    git(&project.repo, &["switch", "-c", "caller-branch"])?;
    fs::write(project.repo.join("README.md"), "caller-only change\n")
        .test_context("write caller branch change")?;
    git(&project.repo, &["add", "README.md"])?;
    git(&project.repo, &["commit", "-m", "caller-only change"])?;
    let main = git(&project.repo, &["rev-parse", "main"])?;
    let caller = git(&project.repo, &["rev-parse", "caller-branch"])?;

    let manifest = project.create("feature-a")?;
    assert_eq!(manifest.base, main.trim(), "test contract values differ");
    assert_ne!(
        manifest.base,
        caller.trim(),
        "test contract values unexpectedly match"
    );
    assert_eq!(
        fs::read_to_string(manifest.worktree.join("README.md"))
            .test_context("read created README")?,
        "# Demo project\n",
        "test contract values differ"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn recreating_an_existing_branch_rejects_a_base_it_does_not_contain() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let first = project.create("feature-a")?;
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "base-fake-docker-bin",
        "#!/bin/sh\nexit 0\n",
    )?;
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", &first.stackstead_id, "--yes"])
        .assert()
        .success();

    fs::write(project.repo.join("README.md"), "# Advanced base\n")
        .test_context("advance base file")?;
    git(&project.repo, &["add", "README.md"])?;
    git(&project.repo, &["commit", "-m", "advance configured base"])?;
    let assert = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)?.contains("does not contain pinned source.base"),
        "test contract condition failed"
    );
    assert!(
        state_stackstead_directories(&project)?.is_empty(),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn normalized_compose_paths_survive_create_and_resolution() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["runtime"]["files"] = serde_yaml::Value::Sequence(vec!["./docker-compose.yml".into()]);
    project.write_config(&config, "use an explicitly relative Compose path")?;

    let manifest = project.create("feature-a")?;
    assert_eq!(
        manifest.compose_files,
        [manifest.worktree.join("docker-compose.yml")],
        "test contract values differ"
    );
    stackstead(&project.repo)
        .args(["context", "feature-a", "--json"])
        .assert()
        .success();
    Ok(())
}
