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
    assert!(
        codes.contains("lock.stackstead.missing"),
        "test contract condition failed"
    );
    assert!(
        codes.contains("lock.run.missing"),
        "test contract condition failed"
    );
    stackstead(&project.repo)
        .args(["run", "run-legacy", "--", "true"])
        .assert()
        .failure();
    stackstead(&project.repo)
        .args(["exec", "run-legacy", "web", "--", "true"])
        .assert()
        .failure();
    assert!(
        !run_cell.state_dir.join("lock").exists(),
        "test contract condition failed"
    );
    assert!(
        !run_cell.state_dir.join("run.lock").exists(),
        "test contract condition failed"
    );

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
    assert!(
        destroy_cell.stackstead_root.exists(),
        "test contract condition failed"
    );
    assert!(
        !destroy_cell.state_dir.join("lock").exists(),
        "test contract condition failed"
    );
    assert!(
        !destroy_cell.state_dir.join("run.lock").exists(),
        "test contract condition failed"
    );
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
    assert!(
        create.wait().test_context("wait for create")?.success(),
        "test contract condition failed"
    );
    assert!(
        second
            .wait()
            .test_context("wait for second create")?
            .success(),
        "test contract condition failed"
    );
    assert_eq!(
        state_stackstead_directories(&project)?.len(),
        2,
        "test contract values differ"
    );
    Ok(())
}
