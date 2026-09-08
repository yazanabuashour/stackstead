use super::*;

#[cfg(unix)]
#[test]
fn current_resolves_the_manifest_worktree_from_inside_a_nested_git_repository() -> anyhow::Result<()>
{
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let nested = manifest.worktree.join("nested/agent");
    fs::create_dir_all(&nested).test_context("create nested worktree directory")?;
    git(&nested, &["init", "--initial-branch=nested-main"])?;
    fs::remove_file(&manifest.env_file).test_context("remove generated environment fixture")?;
    fs::write(
        nested.parent().test()?.join("stackstead.yaml"),
        "not valid yaml: [must-not-shadow\n",
    )
    .test_context("write nested project config")?;
    let docker_marker = project.repo.parent().test()?.join("current-probed-docker");
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "current-fake-docker-bin",
        &format!("#!/bin/sh\ntouch '{}'\nexit 97\n", docker_marker.display()),
    )?;

    stackstead(&manifest.worktree)
        .env("PATH", &path)
        .arg("current")
        .assert()
        .success()
        .stdout(format!("{}\n", manifest.stackstead_id));

    let current = stackstead(&nested)
        .env("PATH", &path)
        .args(["--json", "current"])
        .assert()
        .success();
    let current: Value = serde_json::from_slice(current.get_output().stdout.as_slice())
        .test_context("parse current output")?;
    assert_eq!(
        current,
        serde_json::json!({
            "kind": "StacksteadCurrent",
            "version": "1",
            "stackstead_id": manifest.stackstead_id,
            "source_ownership": "stackstead",
            "repo_root": manifest.repo_root,
            "worktree": manifest.worktree,
            "pointer": manifest.pointer_file,
        }),
        "current JSON did not report the validated manifest identity"
    );
    assert!(
        !docker_marker.exists(),
        "current unexpectedly invoked Docker"
    );

    fs::write(
        manifest.state_dir.join("teardown.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "kind": "StacksteadTeardown",
            "version": "1",
            "stackstead_id": &manifest.stackstead_id,
            "runtime_token": &manifest.runtime_token,
            "phase": "runtime_remove"
        }))
        .test()?,
    )
    .test_context("write retryable teardown fixture")?;
    stackstead(&nested)
        .env("PATH", path)
        .arg("current")
        .assert()
        .success()
        .stdout(format!("{}\n", manifest.stackstead_id));
    assert!(
        !docker_marker.exists(),
        "current unexpectedly invoked Docker"
    );
    Ok(())
}

#[test]
fn v2_manifest_requires_explicit_source_ownership() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let mut value: Value =
        serde_json::from_slice(&fs::read(manifest.manifest_path()).test()?).test()?;
    value.as_object_mut().test()?.remove("source_ownership");
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&value).test()?,
    )
    .test()?;

    let rejected = stackstead(&project.repo)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(rejected.get_output().stdout.is_empty());
    assert!(
        output_text(&rejected.get_output().stderr)?.contains("requires source_ownership"),
        "unexpected error: {}",
        output_text(&rejected.get_output().stderr)?
    );
    assert!(manifest.worktree.is_dir());
    assert!(manifest.stackstead_root.is_dir());
    Ok(())
}

#[test]
fn current_rejects_a_self_consistent_identity_outside_the_configured_state_root()
-> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let forged_state_root = project.repo.parent().test()?.join("forged-state-root");
    let forged_stackstead_root = forged_state_root
        .join(&manifest.project)
        .join(&manifest.stackstead_id);
    let mut forged = manifest.clone();
    forged.source_ownership = SourceOwnership::External;
    forged.project_state_root = forged_state_root;
    forged.stackstead_root = forged_stackstead_root.clone();
    forged.state_dir = forged_stackstead_root.join("state");
    forged.event_log = forged.state_dir.join("events.jsonl");
    fs::create_dir_all(&forged.state_dir).test_context("create forged state directory")?;
    forged
        .write_fixture()
        .test_context("write self-consistent forged manifest")?;

    let mut pointer: StacksteadPointer = serde_json::from_slice(
        &fs::read(&manifest.pointer_file).test_context("read generated pointer")?,
    )
    .test_context("parse generated pointer")?;
    pointer.manifest = forged.manifest_path();
    pointer.project_state_root = forged.project_state_root;
    pointer.stackstead_root = forged.stackstead_root;
    fs::write(
        &manifest.pointer_file,
        serde_json::to_vec_pretty(&pointer).test_context("serialize forged pointer")?,
    )
    .test_context("write forged pointer")?;

    let rejected = stackstead(&manifest.worktree)
        .arg("current")
        .assert()
        .failure();
    assert!(rejected.get_output().stdout.is_empty());
    assert!(
        output_text(&rejected.get_output().stderr)?.contains("configured state root"),
        "unexpected current identity error: {}",
        output_text(&rejected.get_output().stderr)?
    );
    Ok(())
}

#[test]
fn pointer_project_identity_fields_are_checked_against_the_manifest() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let original: Value = serde_json::from_slice(
        &fs::read(&manifest.pointer_file).test_context("read generated pointer")?,
    )
    .test_context("parse generated pointer")?;

    for (field, value) in [
        ("project", Value::String("different-project".into())),
        (
            "project_state_root",
            Value::String("/definitely/not/the/stackstead/state".into()),
        ),
    ] {
        let mut tampered = original.clone();
        tampered[field] = value;
        fs::write(
            &manifest.pointer_file,
            serde_json::to_vec_pretty(&tampered).test_context("serialize tampered pointer")?,
        )
        .test_context("write tampered pointer")?;

        let context = stackstead(&manifest.worktree)
            .args(["context", "feature-a", "--json"])
            .assert()
            .failure();
        assert!(context.get_output().stdout.is_empty());
        let stderr = output_text(&context.get_output().stderr)?;
        assert!(
            stderr.contains("does not match its manifest"),
            "tampered {field} was not rejected during discovery: {stderr}"
        );
    }

    fs::write(
        &manifest.pointer_file,
        serde_json::to_vec_pretty(&original).test_context("serialize original pointer")?,
    )
    .test_context("restore original pointer")?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn copied_pointer_rejects_every_affected_command_before_external_mutation() -> anyhow::Result<()> {
    let victim = Project::initialized()?;
    let manifest = victim.create("victim")?;
    let caller = Project::initialized()?;
    let copied_pointer = caller.repo.join(".stackstead/stackstead.json");
    fs::create_dir_all(copied_pointer.parent().test()?).test()?;
    fs::copy(&manifest.pointer_file, &copied_pointer).test()?;
    let manifest_before = fs::read(manifest.manifest_path()).test()?;
    let events_before = fs::read(&manifest.event_log).test()?;
    let docker_marker = caller
        .repo
        .parent()
        .test()?
        .join("copied-pointer-docker-ran");
    let probe = caller
        .repo
        .parent()
        .test()?
        .join("copied-pointer-probe-ran");
    let path = fake_docker_path(
        caller.repo.parent().test()?,
        "copied-pointer-fake-bin",
        &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", docker_marker.display()),
    )?;
    let caller_path = caller.repo.to_str().test()?;
    let probe_command = format!("touch '{}'", probe.display());

    for args in [
        vec!["current"],
        vec!["create", "redirected"],
        vec!["adopt", "redirected", "--worktree", caller_path],
        vec!["up", &manifest.stackstead_id],
        vec![
            "run",
            &manifest.stackstead_id,
            "--",
            "sh",
            "-c",
            &probe_command,
        ],
        vec!["stop", &manifest.stackstead_id],
        vec!["destroy", &manifest.stackstead_id, "--yes"],
        vec!["repair", &manifest.stackstead_id],
    ] {
        let rejected = stackstead(&caller.repo)
            .env("PATH", &path)
            .args(args)
            .assert()
            .failure();
        assert!(
            output_text(&rejected.get_output().stderr)?.contains("does not match its manifest"),
            "unexpected copied-pointer error: {}",
            output_text(&rejected.get_output().stderr)?
        );
    }
    assert!(!docker_marker.exists());
    assert!(!probe.exists());
    assert_eq!(fs::read(manifest.manifest_path()).test()?, manifest_before);
    assert_eq!(fs::read(&manifest.event_log).test()?, events_before);
    assert!(manifest.worktree.is_dir());
    Ok(())
}
