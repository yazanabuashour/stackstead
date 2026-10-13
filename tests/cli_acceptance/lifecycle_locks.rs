use super::*;

#[cfg(unix)]
#[test]
fn missing_lock_contract_is_rejected_without_recreation() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let run_cell = project.create("run-legacy")?;
    fs::remove_file(run_cell.state_dir.join("lock")).test_context("remove legacy mutation lock")?;
    fs::remove_file(run_cell.state_dir.join("run.lock")).test_context("remove legacy run lock")?;
    let diagnosed = stackstead(&project.repo)
        .args(["doctor", "--json", "--fail-on-error"])
        .assert()
        .code(1);
    let report: Value = serde_json::from_slice(&diagnosed.get_output().stdout).test()?;
    let codes = report["diagnostics"]
        .as_array()
        .test()?
        .iter()
        .filter_map(|item| item["code"].as_str())
        .collect::<BTreeSet<_>>();
    assert!(codes.contains("lock.stackstead.missing"));
    assert!(codes.contains("lock.run.missing"));
    stackstead(&project.repo)
        .args(["run", "run-legacy", "--", "true"])
        .assert()
        .failure();
    stackstead(&project.repo)
        .args(["exec", "run-legacy", "web", "--", "true"])
        .assert()
        .failure();
    assert!(!run_cell.state_dir.join("lock").exists());
    assert!(!run_cell.state_dir.join("run.lock").exists());

    let destroy_cell = project.create("destroy-legacy")?;
    fs::remove_file(destroy_cell.state_dir.join("lock"))
        .test_context("remove legacy mutation lock")?;
    fs::remove_file(destroy_cell.state_dir.join("run.lock"))
        .test_context("remove legacy run lock")?;
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "legacy-lock-fake-bin",
        "#!/bin/sh\nexit 0\n",
    )?;
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", "destroy-legacy", "--yes"])
        .assert()
        .failure();
    assert!(destroy_cell.stackstead_root.exists());
    assert!(!destroy_cell.state_dir.join("lock").exists());
    assert!(!destroy_cell.state_dir.join("run.lock").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn waiting_destroy_and_run_reject_changed_bindings_before_side_effects() -> anyhow::Result<()> {
    use std::{process::Stdio, thread, time::Duration};

    use fs2::FileExt as _;

    for operation in ["run", "destroy"] {
        let project = Project::initialized()?;
        let manifest = project.create("original")?;
        let peer = project.create("peer")?;
        let parent = project.repo.parent().test()?;
        let marker = parent.join("external-mutation");
        let path = fake_docker_path(
            parent,
            "waiting-docker-bin",
            &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
        )?;
        let lease = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(manifest.state_dir.join("run.lock"))
            .test()?;
        lease.lock_exclusive().test()?;
        let mut command = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"));
        command
            .current_dir(&project.repo)
            .env("XDG_STATE_HOME", test_state_home(&project.repo))
            .env("PATH", path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if operation == "destroy" {
            command.args(["destroy", &manifest.stackstead_id, "--yes", "--json"]);
        } else {
            command
                .args(["run", &manifest.stackstead_id, "--", "touch"])
                .arg(&marker);
        }
        let mut waiting = command.spawn().test()?;
        let lock_owner = format!("pid={} ", waiting.id());
        let lock_path = manifest.state_dir.join("lock");
        for _ in 0..100 {
            if fs::read_to_string(&lock_path)
                .test()?
                .starts_with(&lock_owner)
            {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            fs::read_to_string(&lock_path)
                .test()?
                .starts_with(&lock_owner),
            "{operation} did not acquire mutation access before waiting on the run lease"
        );
        assert!(waiting.try_wait().test()?.is_none());
        let original_events = fs::read(&manifest.event_log).test()?;
        let peer_events = fs::read(&peer.event_log).test()?;
        if operation == "destroy" {
            fs::write(
                manifest.manifest_path(),
                serde_json::to_vec_pretty(&peer).test()?,
            )
            .test()?;
        } else {
            let mut pointer: Value =
                serde_json::from_slice(&fs::read(&manifest.pointer_file).test()?).test()?;
            pointer["stackstead_id"] = peer.stackstead_id.clone().into();
            fs::write(
                &manifest.pointer_file,
                serde_json::to_vec_pretty(&pointer).test()?,
            )
            .test()?;
        }
        fs2::FileExt::unlock(&lease).test()?;
        let output = waiting.wait_with_output().test()?;
        assert!(
            !output.status.success(),
            "{operation} accepted changed bindings"
        );
        let expected = if operation == "destroy" {
            "environment identity changed while holding access"
        } else {
            "reciprocal pointer"
        };
        assert!(
            output_text(&output.stderr)?.contains(expected),
            "{operation}: {}",
            output_text(&output.stderr)?
        );
        assert!(output.stdout.is_empty());
        assert!(!marker.exists(), "{operation} executed an external command");
        for cell in [&manifest, &peer] {
            assert!(cell.manifest_path().is_file());
            assert!(cell.worktree.is_dir());
        }
        assert_eq!(fs::read(&manifest.event_log).test()?, original_events);
        assert_eq!(fs::read(&peer.event_log).test()?, peer_events);
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn post_create_holds_the_cell_lock_after_manifest_publication() -> anyhow::Result<()> {
    use std::{thread, time::Duration};

    let project = Project::initialized()?;
    let ready = project.repo.parent().test()?.join("post-create-ready");
    let release = project.repo.parent().test()?.join("post-create-release");
    fs::write(&release, "wait\n").test_context("create hook release gate")?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["hooks"]["post_create"] = serde_yaml::to_value([serde_json::json!({
        "command": format!(
            "touch '{}'; while test -e '{}'; do sleep 0.02; done",
            ready.display(),
            release.display()
        ),
        "shell": true,
    })])
    .test()?;
    project.write_config(&config, "add blocking post-create hook")?;

    let mut create = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .args(["create", "feature-a"])
        .spawn()
        .test_context("spawn blocked create")?;
    assert!(
        wait_for_file(&ready, 200, Duration::from_millis(10)),
        "post-create hook did not start"
    );
    let mut second = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .args(["create", "feature-b"])
        .spawn()
        .test_context("spawn waiting create")?;
    thread::sleep(Duration::from_millis(150));
    assert!(
        second.try_wait().test()?.is_none(),
        "second create did not wait"
    );
    fs::remove_file(&release).test_context("release post-create hook")?;
    assert!(create.wait().test_context("wait for create")?.success());
    assert!(
        second
            .wait()
            .test_context("wait for second create")?
            .success()
    );
    assert_eq!(state_stackstead_directories(&project)?.len(), 2);
    Ok(())
}
