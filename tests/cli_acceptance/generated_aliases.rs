use super::*;

#[cfg(unix)]
#[test]
fn repair_rejects_physical_output_aliases_before_replacing_either_file() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("physical-alias")?;
    let before = fs::read(&manifest.env_file).test()?;
    fs::remove_file(&manifest.agent_context).test()?;
    fs::hard_link(&manifest.env_file, &manifest.agent_context).test()?;
    let rejected = stackstead(&project.repo)
        .args(["--json", "repair", &manifest.stackstead_id])
        .assert()
        .failure();
    assert!(output_text(&rejected.get_output().stderr)?.contains("aliases or overlaps"));
    assert!(rejected.get_output().stdout.is_empty());
    assert_eq!(fs::read(&manifest.env_file).test()?, before);
    assert_eq!(fs::read(&manifest.agent_context).test()?, before);
    Ok(())
}

#[cfg(unix)]
#[test]
fn repair_rejects_nonregular_generated_files_without_blocking() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("nonregular-output")?;
    let before = fs::read(&manifest.env_file).test()?;
    let ownership = manifest.worktree.join(".stackstead/compose-ownership.yaml");
    fs::remove_file(&ownership).test()?;
    Command::new("mkfifo").arg(&ownership).assert().success();
    let rejected = stackstead(&project.repo)
        .timeout(std::time::Duration::from_secs(30))
        .args(["--json", "repair", &manifest.stackstead_id])
        .assert()
        .failure();
    assert!(
        output_text(&rejected.get_output().stderr)?
            .contains("expected a regular file or directory")
    );
    assert!(rejected.get_output().stdout.is_empty());
    assert_eq!(fs::read(&manifest.env_file).test()?, before);
    assert!(manifest.manifest_path().is_file());
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn unpublished_directory_alias_rejection_preserves_adopted_source_and_retry_state()
-> anyhow::Result<()> {
    let project = Project::initialized()?;
    let probe = project.repo.parent().test()?.join("directory-case-probe");
    fs::write(&probe, "fixture\n").test()?;
    let insensitive = project
        .repo
        .parent()
        .test()?
        .join("DIRECTORY-CASE-PROBE")
        .exists();
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["env"]["file"] = ".STACKSTEAD".into();
    project.write_config(&config, "configure native directory alias")?;
    let external = project.repo.parent().test()?.join("manager-owned");
    git(
        &project.repo,
        &[
            "worktree",
            "add",
            "-b",
            "manager",
            external.to_str().test()?,
            "main",
        ],
    )?;
    let before = fs::read(external.join("README.md")).test()?;
    let output = stackstead(&project.repo)
        .args(["--json", "adopt", "manager", "--worktree"])
        .arg(&external)
        .output()
        .test()?;
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "directory-alias-docker",
        "#!/bin/sh\nexit 0\n",
    )?;
    let manifest = if insensitive {
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!external.join(".stackstead").exists());
        assert!(state_stackstead_directories(&project)?.is_empty());
        config["env"]["file"] = ".stackstead/.env".into();
        project.write_config(&config, "restore distinct output contract")?;
        git(&external, &["merge", "main"])?;
        let adopted = stackstead(&project.repo)
            .args(["--json", "adopt", "manager", "--worktree"])
            .arg(&external)
            .assert()
            .success();
        changed_manifest(&adopted.get_output().stdout, "adopted")?
    } else {
        assert!(output.status.success());
        changed_manifest(&output.stdout, "adopted")?
    };
    stackstead(&project.repo)
        .env("PATH", &path)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert_eq!(fs::read(external.join("README.md")).test()?, before);
    Ok(())
}

#[test]
fn unpublished_tracked_output_rejection_cleans_owned_checkout_without_overwriting_source()
-> anyhow::Result<()> {
    let project = Project::initialized()?;
    let before = fs::read(project.repo.join("README.md")).test()?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["env"]["file"] = "README.md".into();
    project.write_config(&config, "configure existing tracked output")?;
    let rejected = stackstead(&project.repo)
        .args(["--json", "create", "tracked-output"])
        .assert()
        .failure();
    assert!(rejected.get_output().stdout.is_empty());
    assert!(output_text(&rejected.get_output().stderr)?.contains("already exists"));
    assert!(state_stackstead_directories(&project)?.is_empty());
    assert_eq!(
        git(&project.repo, &["worktree", "list", "--porcelain"])?
            .matches("worktree ")
            .count(),
        1
    );
    assert_eq!(fs::read(project.repo.join("README.md")).test()?, before);
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn owned_output_cleanliness_uses_native_entry_spelling_without_case_folding() -> anyhow::Result<()>
{
    let project = Project::initialized()?;
    fs::create_dir(project.repo.join("Generated")).test()?;
    fs::write(
        project.repo.join("Generated/README.md"),
        "tracked parent fixture\n",
    )
    .test()?;
    git(&project.repo, &["add", "Generated/README.md"])?;
    git(
        &project.repo,
        &["commit", "-m", "declare differently cased parent"],
    )?;
    let insensitive = project.repo.join("generated/README.md").exists();
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["env"]["file"] = "generated/env".into();
    config["agent"]["context_file"] = "generated/context.md".into();
    project.write_config(&config, "configure native output spelling")?;
    let manifest = project.create("native-entry")?;
    if insensitive {
        fs::rename(&manifest.env_file, manifest.env_file.with_file_name("ENV")).test()?;
    }
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "native-entry-docker",
        "#!/bin/sh\nexit 0\n",
    )?;
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(!manifest.worktree.exists());
    assert!(!manifest.stackstead_root.exists());
    assert_eq!(
        fs::read_to_string(project.repo.join("Generated/README.md")).test()?,
        "tracked parent fixture\n"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn generated_output_separation_follows_the_destination_filesystem_case_behavior()
-> anyhow::Result<()> {
    use std::io::Write as _;

    let project = Project::initialized()?;
    let probe = project.repo.parent().test()?.join("case-probe");
    fs::write(&probe, "fixture\n").test()?;
    let insensitive = project.repo.parent().test()?.join("CASE-PROBE").exists();
    writeln!(
        std::io::stdout(),
        "generated alias fixture: case-insensitive={insensitive}"
    )
    .test()?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["agent"]["context_file"] = ".stackstead/COMPOSE-OWNERSHIP.YAML".into();
    project.write_config(&config, "configure filesystem-dependent output alias")?;
    let output = stackstead(&project.repo)
        .args(["--json", "create", "case-alias"])
        .output()
        .test()?;
    if insensitive {
        assert!(!output.status.success());
        assert!(output_text(&output.stderr)?.contains("aliases or overlaps"));
        assert!(state_stackstead_directories(&project)?.is_empty());
    } else {
        assert!(output.status.success());
        let manifest = changed_manifest(&output.stdout, "created")?;
        assert!(
            fs::read_to_string(&manifest.agent_context)
                .test()?
                .starts_with("# Stackstead:")
        );
    }
    Ok(())
}
