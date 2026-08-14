use super::*;

#[test]
fn repair_regenerates_missing_contract_files_without_docker() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    fs::remove_file(&manifest.env_file).test_context("remove generated env")?;
    fs::remove_file(&manifest.agent_context).test_context("remove generated context")?;
    fs::remove_file(&manifest.pointer_file).test_context("remove generated pointer")?;

    let missing = stackstead(&manifest.worktree)
        .arg("current")
        .assert()
        .failure();
    assert!(
        missing.get_output().stdout.is_empty(),
        "test contract condition failed"
    );

    let assert = stackstead(&project.repo)
        .args(["repair", "feature-a", "--json"])
        .assert()
        .success();
    let repaired = changed_manifest(&assert.get_output().stdout, "repaired")?;
    assert_eq!(
        repaired.stackstead_id, manifest.stackstead_id,
        "test contract values differ"
    );
    assert!(
        repaired.env_file.is_file(),
        "test contract condition failed"
    );
    assert!(
        repaired.agent_context.is_file(),
        "test contract condition failed"
    );
    assert!(
        repaired.pointer_file.is_file(),
        "test contract condition failed"
    );
    assert_eq!(
        event_types(&repaired.event_log)?.last().map(String::as_str),
        Some("repair"),
        "test contract values differ"
    );

    let repaired_pointer: StacksteadPointer = serde_json::from_slice(
        &fs::read(&repaired.pointer_file).test_context("read repaired pointer")?,
    )
    .test_context("parse repaired pointer")?;
    assert_eq!(
        repaired_pointer.manifest,
        repaired.manifest_path(),
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn json_destroy_requires_yes_without_writing_a_prompt_to_stdout() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;

    let assert = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(
        assert.get_output().stdout.is_empty(),
        "JSON failure was contaminated by: {}",
        output_text(&assert.get_output().stdout)?
    );
    assert!(
        output_text(&assert.get_output().stderr)?.contains("--yes"),
        "test contract condition failed"
    );
    assert!(
        manifest.stackstead_root.is_dir(),
        "test contract condition failed"
    );
    assert!(manifest.worktree.is_dir(), "test contract condition failed");
    Ok(())
}

#[test]
fn unexposed_database_service_fails_without_publishing_partial_state() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    project.replace_config("    service: postgres\n", "    service: missing-postgres\n")?;

    let assert = stackstead(&project.repo)
        .args(["create", "feature-a", "--json"])
        .assert()
        .failure();
    let stderr = output_text(&assert.get_output().stderr)?;
    assert!(
        stderr.contains("resources.ports.expose") || stderr.contains("must be present"),
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

#[cfg(unix)]
#[test]
fn failed_git_worktree_add_leaves_no_published_manifest_or_stackstead_root() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized()?;
    let fake_bin = project
        .repo
        .parent()
        .test_context("repository has parent")?
        .join("fake-bin");
    fs::create_dir(&fake_bin).test_context("create fake binary directory")?;
    let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join("git"))
        .find(|candidate| candidate.is_file())
        .test_context("find real Git executable")?;
    let wrapper = fake_bin.join("git");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ \"$1\" = worktree ] && [ \"$2\" = add ]; then\n  echo 'intentional worktree failure' >&2\n  exit 19\nfi\nexec '{}' \"$@\"\n",
            real_git.display().to_string().replace('\'', "'\"'\"'")
        ),
    )
    .test_context("write Git wrapper")?;
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
        .test_context("make Git wrapper executable")?;
    let path = std::env::join_paths(std::iter::once(fake_bin.clone()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .test_context("construct command-local PATH")?;

    let mut command = stackstead(&project.repo);
    command.env("PATH", path);
    let assert = command
        .args(["create", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)?.contains("intentional worktree failure"),
        "test contract condition failed"
    );
    assert!(
        state_stackstead_directories(&project)?.is_empty(),
        "test contract condition failed"
    );
    let registry: Value = serde_json::from_slice(
        &fs::read(test_state_home(&project.repo).join("stackstead/port-leases.json"))
            .test_context("read rolled-back port lease registry")?,
    )
    .test_context("parse rolled-back port lease registry")?;
    assert!(
        registry["leases"].as_array().test()?.is_empty(),
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
