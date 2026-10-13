use super::*;

fn configure_outputs(project: &Project, env: &str, context: &str) -> anyhow::Result<()> {
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["env"]["file"] = env.into();
    config["agent"]["context_file"] = context.into();
    project.write_config(&config, "configure custom generated outputs")
}

#[cfg(unix)]
#[test]
fn custom_outputs_do_not_hide_source_dirtiness_and_external_cleanup_removes_only_owned_files()
-> anyhow::Result<()> {
    for (env, context) in [
        ("generated/env*[x].txt", "generated/context ?[x] .md"),
        ("generated/line\nbreak.env", "generated/context\rname.md"),
    ] {
        let project = Project::initialized()?;
        configure_outputs(&project, env, context)?;
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
        let adopted = stackstead(&project.repo)
            .args(["--json", "adopt", "manager", "--worktree"])
            .arg(&external)
            .assert()
            .success();
        let manifest = changed_manifest(&adopted.get_output().stdout, "adopted")?;
        let readme = fs::read(external.join("README.md")).test()?;
        let unrelated = external.join("generated/env-ax.txt");
        fs::write(&unrelated, "user-owned source\n").test()?;
        let marker = project.repo.parent().test()?.join("docker-called");
        let path = fake_docker_path(
            project.repo.parent().test()?,
            "custom-output-docker",
            &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
        )?;
        let rejected = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["destroy", &manifest.stackstead_id, "--yes"])
            .assert()
            .failure();
        assert!(
            output_text(&rejected.get_output().stderr)?
                .contains("uncommitted or untracked changes")
        );
        assert!(!marker.exists());
        assert_eq!(
            fs::read_to_string(&unrelated).test()?,
            "user-owned source\n"
        );
        fs::remove_file(&unrelated).test()?;
        stackstead(&project.repo)
            .env("PATH", &path)
            .args(["destroy", &manifest.stackstead_id, "--yes"])
            .assert()
            .success();
        assert!(external.is_dir());
        assert_eq!(fs::read(external.join("README.md")).test()?, readme);
        assert!(!manifest.env_file.exists());
        assert!(!manifest.agent_context.exists());
        assert!(!external.join(".stackstead").exists());
        assert!(!manifest.stackstead_root.exists());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn custom_outputs_in_owned_source_allow_destroy_and_exact_excludes_preserve_neighbors()
-> anyhow::Result<()> {
    for (env, context, ignored) in [
        ("generated/env*[x].txt ", "generated/context.md", true),
        (
            "generated/line\nbreak.env",
            "generated/context\rname.md",
            false,
        ),
    ] {
        let project = Project::initialized()?;
        configure_outputs(&project, env, context)?;
        let manifest = project.create("custom")?;
        assert_eq!(
            git(&manifest.worktree, &["status", "--porcelain"])?.is_empty(),
            ignored
        );
        let path = fake_docker_path(
            project.repo.parent().test()?,
            "owned-custom-output-docker",
            "#!/bin/sh\nexit 0\n",
        )?;
        stackstead(&project.repo)
            .env("PATH", path)
            .args(["destroy", &manifest.stackstead_id, "--yes"])
            .assert()
            .success();
        assert!(!manifest.worktree.exists());
        assert!(!manifest.stackstead_root.exists());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn failed_custom_output_publication_preserves_source_and_allows_adoption_retry()
-> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    for failure in ["unwritable-context", "ownership-render"] {
        let project = Project::initialized()?;
        configure_outputs(&project, "generated.env", "docs/generated-context.md")?;
        if failure == "ownership-render" {
            let compose = project.repo.join("docker-compose.yml");
            let mut value: serde_yaml::Value =
                serde_yaml::from_slice(&fs::read(&compose).test()?).test()?;
            value["services"]["web"]["volumes"] = serde_yaml::to_value(["/cache"]).test()?;
            fs::write(&compose, serde_yaml::to_string(&value).test()?).test()?;
            git(&project.repo, &["add", "docker-compose.yml"])?;
            git(
                &project.repo,
                &["commit", "-m", "declare unsupported anonymous volume"],
            )?;
        }
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
        let docs = external.join("docs");
        fs::create_dir(&docs).test()?;
        let readme = fs::read(external.join("README.md")).test()?;
        if failure == "unwritable-context" {
            fs::set_permissions(&docs, fs::Permissions::from_mode(0o500)).test()?;
        }
        let rejected = stackstead(&project.repo)
            .args(["--json", "adopt", "manager", "--worktree"])
            .arg(&external)
            .assert()
            .failure();
        fs::set_permissions(&docs, fs::Permissions::from_mode(0o700)).test()?;
        let error = output_text(&rejected.get_output().stderr)?;
        assert!(
            error.contains(if failure == "ownership-render" {
                "anonymous volume"
            } else {
                "Permission denied"
            }),
            "{failure}: {error}"
        );
        assert!(rejected.get_output().stdout.is_empty());
        assert_eq!(fs::read(external.join("README.md")).test()?, readme);
        assert!(!external.join("generated.env").exists());
        assert!(!external.join("docs/generated-context.md").exists());
        assert!(!external.join(".stackstead").exists());
        assert!(state_stackstead_directories(&project)?.is_empty());
        if failure == "ownership-render" {
            git(&project.repo, &["revert", "--no-edit", "HEAD"])?;
            git(&external, &["merge", "main"])?;
        }
        let adopted = stackstead(&project.repo)
            .args(["--json", "adopt", "manager", "--worktree"])
            .arg(&external)
            .assert()
            .success();
        let manifest = changed_manifest(&adopted.get_output().stdout, "adopted")?;
        let path = fake_docker_path(
            project.repo.parent().test()?,
            "retry-output-docker",
            "#!/bin/sh\nexit 0\n",
        )?;
        stackstead(&project.repo)
            .env("PATH", path)
            .args(["destroy", &manifest.stackstead_id, "--yes"])
            .assert()
            .success();
        assert!(!manifest.env_file.exists());
        assert!(!manifest.agent_context.exists());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn custom_output_cleanup_rejects_tracked_and_redirected_files_with_matching_headers()
-> anyhow::Result<()> {
    for scenario in ["tracked", "redirected", "redirected-internal"] {
        let project = Project::initialized()?;
        configure_outputs(&project, "generated.env", "generated-context.md")?;
        let manifest = project.create("custom")?;
        let original = fs::read(&manifest.env_file).test()?;
        let source = if scenario == "tracked" {
            manifest.env_file.clone()
        } else {
            manifest
                .worktree
                .join(if scenario == "redirected-internal" {
                    ".stackstead/unrelated.env"
                } else {
                    "unrelated.env"
                })
        };
        let contents = format!(
            "# Generated by Stackstead. Do not edit by hand.\n# Stackstead: {}\nuser-owned source\n",
            manifest.stackstead_id
        );
        fs::write(&source, &contents).test()?;
        if scenario == "tracked" {
            git(&manifest.worktree, &["add", "-f", "generated.env"])?;
            git(
                &manifest.worktree,
                &["commit", "-m", "track declared output as source"],
            )?;
        } else {
            let mut tampered = manifest.clone();
            tampered.env_file = source.clone();
            if scenario == "redirected-internal" {
                tampered.agent_context = manifest.worktree.join(".stackstead/unrelated.md");
            }
            tampered.write_fixture()?;
        }
        let marker = project.repo.parent().test()?.join("docker-called");
        let path = fake_docker_path(
            project.repo.parent().test()?,
            "unowned-output-docker",
            &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
        )?;
        let rejected = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["destroy", &manifest.stackstead_id, "--yes"])
            .assert()
            .failure();
        let error = output_text(&rejected.get_output().stderr)?;
        assert!(error.contains(if scenario == "tracked" {
            "cannot prove generated output ownership at"
        } else {
            "paths differ from the pinned source declaration"
        }));
        assert!(rejected.get_output().stdout.is_empty());
        assert!(!marker.exists());
        assert_eq!(fs::read_to_string(&source).test()?, contents);
        assert!(manifest.manifest_path().is_file());
        if scenario != "tracked" {
            assert_eq!(fs::read(&manifest.env_file).test()?, original);
        }
        if scenario == "tracked" {
            git(&manifest.worktree, &["rm", "--cached", "generated.env"])?;
            git(
                &manifest.worktree,
                &["commit", "-m", "restore generated ownership"],
            )?;
            fs::write(&source, original).test()?;
        } else {
            fs::remove_file(&source).test()?;
            manifest.write_fixture()?;
        }
        stackstead(&project.repo)
            .env("PATH", path)
            .args(["destroy", &manifest.stackstead_id, "--yes"])
            .assert()
            .success();
    }
    Ok(())
}
