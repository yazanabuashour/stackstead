use super::*;

#[test]
fn create_rejects_a_slug_that_matches_an_existing_full_id() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let existing = project.create("feature-a")?;
    let before = state_stackstead_directories(&project)?;

    let assert = stackstead(&project.repo)
        .args(["create", &existing.stackstead_id, "--json"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr)?.contains("already exists"));
    assert_eq!(state_stackstead_directories(&project)?, before);
    Ok(())
}

#[cfg(unix)]
#[test]
fn changed_worktree_branch_is_reported_and_rejected_before_agent_or_teardown() -> anyhow::Result<()>
{
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    git(&manifest.worktree, &["switch", "-c", "unexpected-source"])?;

    let inspected = stackstead(&project.repo)
        .args(["--json", "inspect", "feature-a"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout)
        .test_context("parse inspect output")?;
    assert!(inspected["warnings"].as_array().is_some_and(|warnings| {
        warnings.iter().any(|warning| {
            warning
                .as_str()
                .is_some_and(|warning| warning.contains("unexpected-source"))
        })
    }));

    let current = stackstead(&manifest.worktree)
        .arg("current")
        .assert()
        .failure();
    assert!(current.get_output().stdout.is_empty());
    let error = output_text(&current.get_output().stderr)?;
    assert!(
        error.contains("unexpected-source") && error.contains("refusing to use the wrong source"),
        "unexpected current source-binding error: {error}"
    );

    for args in [
        vec!["run", "feature-a", "--", "true"],
        vec!["exec", "feature-a", "web", "--", "true"],
        vec!["up", "feature-a"],
        vec!["stop", "feature-a"],
        vec!["repair", "feature-a"],
        vec!["destroy", "feature-a", "--yes"],
    ] {
        let assert = stackstead(&project.repo).args(args).assert().failure();
        let stderr = output_text(&assert.get_output().stderr)?;
        assert!(
            stderr.contains("unexpected-source")
                && stderr.contains("refusing to use the wrong source"),
            "unexpected source-binding error: {stderr}"
        );
    }
    git(&manifest.worktree, &["switch", "feature-a"])?;
    let tree = git(&project.repo, &["write-tree"])?;
    let unrelated = git(
        &project.repo,
        &["commit-tree", tree.trim(), "-m", "unrelated root commit"],
    )?;
    let mut changed_base = manifest.clone();
    changed_base.base = unrelated.trim().into();
    changed_base
        .write_fixture()
        .test_context("write changed pinned-base fixture")?;
    let current = stackstead(&manifest.worktree)
        .arg("current")
        .assert()
        .failure();
    assert!(current.get_output().stdout.is_empty());
    assert!(output_text(&current.get_output().stderr)?.contains("not based on pinned commit"));

    assert!(manifest.manifest_path().is_file());
    assert!(manifest.worktree.is_dir());
    Ok(())
}

#[test]
fn all_resolved_commands_reject_redirected_manifest_contract_fields() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let secret = project.repo.parent().test()?.join("must-not-read.env");
    fs::write(&secret, "PRIVATE_VALUE=must-not-leak\n")
        .test_context("write outside env fixture")?;

    let mut tampered = manifest.clone();
    tampered.env_file = secret;
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&tampered).test_context("serialize redirected manifest")?,
    )
    .test_context("write redirected manifest")?;
    let assert = stackstead(&project.repo)
        .args(["env", "feature-a", "--print", "--show-secrets"])
        .assert()
        .failure();
    assert!(!output_text(&assert.get_output().stdout)?.contains("must-not-leak"));
    assert!(output_text(&assert.get_output().stderr)?.contains("escapes worktree"));

    tampered = manifest.clone();
    tampered.compose_project = "unrelated-valid-project".into();
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&tampered)
            .test_context("serialize redirected Compose identity")?,
    )
    .test_context("write redirected Compose identity")?;
    let assert = stackstead(&project.repo)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)?
            .contains("manifest Compose project does not match the durable stackstead identity")
    );

    tampered = manifest.clone();
    tampered.short_id = "ffffffffffffffffffffffffffffffff".into();
    tampered.compose_project = format!("{}-feature-a-{}", tampered.project, tampered.short_id);
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&tampered).test_context("serialize forged redundant identity")?,
    )
    .test_context("write forged redundant identity")?;
    let assert = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--yes"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)?
            .contains("manifest stackstead ID does not match its slug and short ID")
    );
    Ok(())
}

#[test]
fn adopted_manifests_cannot_cross_bind_or_delete_another_checkout() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let parent = project.repo.parent().test()?;
    let first_path = parent.join("manager-first");
    let second_path = parent.join("manager-second");
    for (branch, path) in [
        ("manager-first", &first_path),
        ("manager-second", &second_path),
    ] {
        git(
            &project.repo,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                path.to_str().test_context("UTF-8 fixture path")?,
                "main",
            ],
        )?;
    }
    let adopt = |name: &str, path: &Path| {
        let assert = stackstead(&project.repo)
            .arg("--json")
            .arg("adopt")
            .arg(name)
            .arg("--worktree")
            .arg(path)
            .assert()
            .success();
        changed_manifest(&assert.get_output().stdout, "adopted")
    };
    let first = adopt("manager-first", &first_path)?;
    let second = adopt("manager-second", &second_path)?;
    let mut redirected = first.clone();
    redirected.worktree = second.worktree.clone();
    redirected.branch = second.branch.clone();
    redirected.compose_files = second.compose_files.clone();
    redirected.env_file = second.env_file.clone();
    redirected.agent_context = second.agent_context.clone();
    redirected.pointer_file = second.pointer_file.clone();
    redirected
        .write_fixture()
        .test_context("redirect first manifest to second checkout")?;

    for args in [
        vec!["run", &first.stackstead_id, "--", "true"],
        vec!["exec", &first.stackstead_id, "web", "--", "true"],
        vec!["repair", &first.stackstead_id],
        vec!["destroy", &first.stackstead_id, "--yes"],
    ] {
        let assert = stackstead(&project.repo).args(args).assert().failure();
        assert!(
            output_text(&assert.get_output().stderr)?.contains("reciprocal pointer"),
            "unexpected cross-binding failure: {}",
            output_text(&assert.get_output().stderr)?
        );
    }
    assert!(second.pointer_file.is_file());
    assert!(second.manifest_path().is_file());
    assert!(second.worktree.is_dir());
    assert!(first_path.join(".stackstead/stackstead.json").is_file());
    Ok(())
}
