use super::*;

fn set_env_path(project: &Project, file: &str) -> anyhow::Result<()> {
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["env"]["file"] = file.into();
    project.write_config(&config, "change generated output contract")
}

#[cfg(unix)]
#[test]
fn shared_generated_exclusions_cannot_hide_sibling_or_stale_untracked_source() -> anyhow::Result<()>
{
    for keep_first in [true, false] {
        let project = Project::initialized()?;
        set_env_path(&project, "generated/old.env")?;
        let first = project.create("first")?;
        let marker = project.repo.parent().test()?.join("docker-called");
        let path = fake_docker_path(
            project.repo.parent().test()?,
            "shared-exclusion-docker",
            &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
        )?;
        if !keep_first {
            stackstead(&project.repo)
                .env("PATH", &path)
                .args(["destroy", &first.stackstead_id, "--yes"])
                .assert()
                .success();
            fs::remove_file(&marker).test()?;
        }
        set_env_path(&project, "generated/new.env")?;
        let second = project.create("second")?;
        let source = second.worktree.join("generated/old.env");
        fs::write(&source, "unrelated source\n").test()?;
        assert!(git(&second.worktree, &["status", "--porcelain"])?.is_empty());
        let rejected = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["destroy", &second.stackstead_id, "--yes"])
            .assert()
            .failure();
        assert!(
            output_text(&rejected.get_output().stderr)?
                .contains("uncommitted or untracked changes")
        );
        assert!(!marker.exists());
        assert_eq!(fs::read_to_string(&source).test()?, "unrelated source\n");
        assert!(second.manifest_path().is_file());
        if keep_first {
            assert!(first.manifest_path().is_file());
        }
        fs::remove_file(&source).test()?;
        fs::hard_link(&second.env_file, &source).test()?;
        stackstead(&project.repo)
            .env("PATH", &path)
            .args(["destroy", &second.stackstead_id, "--yes"])
            .assert()
            .failure();
        assert!(!marker.exists());
        assert!(source.is_file());
        fs::remove_file(&source).test()?;
        stackstead(&project.repo)
            .env("PATH", &path)
            .args(["destroy", &second.stackstead_id, "--yes"])
            .assert()
            .success();
        if keep_first {
            stackstead(&project.repo)
                .env("PATH", &path)
                .args(["destroy", &first.stackstead_id, "--yes"])
                .assert()
                .success();
        }
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn generated_exclusion_checks_preserve_user_ignore_rules() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let ignore = project.repo.join(".gitignore");
    let original = if ignore.exists() {
        fs::read_to_string(&ignore).test()?
    } else {
        String::new()
    };
    let contents = format!("{original}\ngenerated/old.env\ncache-link\ncache-unreadable\n");
    fs::write(&ignore, &contents).test()?;
    git(&project.repo, &["add", ".gitignore"])?;
    git(
        &project.repo,
        &["commit", "-m", "declare user-owned ignore rule"],
    )?;
    set_env_path(&project, "generated/old.env")?;
    let first = project.create("first")?;
    set_env_path(&project, "generated/new.env")?;
    let second = project.create("second")?;
    fs::write(
        second.worktree.join("generated/old.env"),
        "user-ignored disposable cache\n",
    )
    .test()?;
    std::os::unix::fs::symlink("absent-target", second.worktree.join("cache-link")).test()?;
    let unreadable = second.worktree.join("cache-unreadable");
    fs::write(&unreadable, "user-ignored cache\n").test()?;
    fs::set_permissions(
        &unreadable,
        std::os::unix::fs::PermissionsExt::from_mode(0o0),
    )
    .test()?;
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "user-exclusion-docker",
        "#!/bin/sh\nexit 0\n",
    )?;
    stackstead(&project.repo)
        .env("PATH", &path)
        .args(["destroy", &second.stackstead_id, "--yes"])
        .assert()
        .success();
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", &first.stackstead_id, "--yes"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(ignore).test()?, contents);
    Ok(())
}
