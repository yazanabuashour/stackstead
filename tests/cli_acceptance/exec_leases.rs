use super::{exec::service_exec_docker_path, *};

#[cfg(unix)]
#[test]
fn exec_holds_the_run_lease_until_the_compose_client_finishes() -> anyhow::Result<()> {
    use std::{thread, time::Duration};

    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let parent = project.repo.parent().test()?;
    let fake_state = parent.join("service-exec-lease-state");
    let ready = fake_state.join("ready");
    let release = fake_state.join("release");
    fs::create_dir_all(&fake_state).test()?;
    fs::write(&release, "wait\n").test()?;
    let path = service_exec_docker_path(parent, "service-exec-lease-bin")?;

    let mut executing = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_PROJECT", &manifest.compose_project)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .env("EXEC_READY", &ready)
        .env("EXEC_RELEASE", &release)
        .args([
            "exec",
            &manifest.stackstead_id,
            "web",
            "--",
            "long-running-command",
        ])
        .spawn()
        .test_context("start service command")?;
    assert!(
        wait_for_file(&ready, 100, Duration::from_millis(20)),
        "service command did not start"
    );

    let mut stopping = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_PROJECT", &manifest.compose_project)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["stop", &manifest.stackstead_id])
        .spawn()
        .test_context("start waiting stop")?;
    thread::sleep(Duration::from_millis(150));
    assert!(
        stopping.try_wait().test()?.is_none(),
        "stop did not wait for service exec"
    );

    fs::remove_file(&release).test_context("release service command")?;
    assert!(
        executing
            .wait()
            .test_context("wait for service command")?
            .success(),
        "test contract condition failed"
    );
    assert!(
        stopping.wait().test_context("wait for stop")?.success(),
        "test contract condition failed"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn killed_exec_wrapper_leaves_the_run_lease_with_the_compose_client() -> anyhow::Result<()> {
    use std::{thread, time::Duration};

    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let parent = project.repo.parent().test()?;
    let fake_state = parent.join("interrupted-service-exec-state");
    let ready = fake_state.join("ready");
    let release = fake_state.join("release");
    fs::create_dir_all(&fake_state).test()?;
    fs::write(&release, "wait\n").test()?;
    let path = service_exec_docker_path(parent, "interrupted-service-exec-bin")?;

    let mut wrapper = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_PROJECT", &manifest.compose_project)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .env("EXEC_READY", &ready)
        .env("EXEC_RELEASE", &release)
        .args([
            "exec",
            &manifest.stackstead_id,
            "web",
            "--",
            "long-running-command",
        ])
        .spawn()
        .test_context("start service exec wrapper")?;
    assert!(
        wait_for_file(&ready, 100, Duration::from_millis(20)),
        "Compose client did not start"
    );
    rustix::process::kill_process(
        rustix::process::Pid::from_child(&wrapper),
        rustix::process::Signal::KILL,
    )
    .test_context("kill service exec wrapper")?;
    wrapper.wait().test_context("reap service exec wrapper")?;

    let mut stopping = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_PROJECT", &manifest.compose_project)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["stop", &manifest.stackstead_id])
        .spawn()
        .test_context("start stop behind inherited service exec lease")?;
    thread::sleep(Duration::from_millis(150));
    assert!(
        stopping.try_wait().test()?.is_none(),
        "stop overtook the surviving Compose client"
    );

    fs::remove_file(&release).test_context("release Compose client")?;
    assert!(
        stopping.wait().test_context("wait for stop")?.success(),
        "test contract condition failed"
    );
    Ok(())
}
