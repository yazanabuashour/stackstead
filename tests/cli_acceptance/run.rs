use super::*;

#[cfg(unix)]
#[test]
fn run_pins_stackstead_identity_and_preserves_the_child_exit_code() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let script = project.repo.parent().test()?.join("agent-probe");
    fs::write(
        &script,
        r#"#!/bin/sh
test "$PWD" = "$1" || exit 91
test "$STACKSTEAD_ID" = "$2" || exit 92
test "$STACKSTEAD_COMPOSE_PROJECT" = "$3" || exit 93
test "$COMPOSE_PROJECT_NAME" = "$3" || exit 94
test "$STACKSTEAD_WORKTREE" = "$1" || exit 95
test "$STACKSTEAD_PRIVATE_RUN_SUPERVISOR" = "preserved" || exit 96
printf '%s|%s\n' "$STACKSTEAD_ID" "$4"
exit 23
"#,
    )
    .test_context("write agent probe")?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
        .test_context("make agent probe executable")?;

    let assert = stackstead(&project.repo)
        .env("STACKSTEAD_ID", "spoofed")
        .env("COMPOSE_PROJECT_NAME", "shared")
        .env("STACKSTEAD_PRIVATE_RUN_SUPERVISOR", "preserved")
        .arg("run")
        .arg("feature-a")
        .arg("--")
        .arg(&script)
        .arg(&manifest.worktree)
        .arg(&manifest.stackstead_id)
        .arg(&manifest.compose_project)
        .arg("argument with spaces")
        .assert()
        .code(23);
    assert_eq!(
        output_text(&assert.get_output().stdout)?,
        format!("{}|argument with spaces\n", manifest.stackstead_id),
        "test contract values differ"
    );

    let json_run = stackstead(&project.repo)
        .args(["--json", "run", "feature-a", "--", "true"])
        .assert()
        .failure();
    assert!(
        output_text(&json_run.get_output().stderr)?.contains("--json cannot be combined with run"),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn generated_environment_cannot_add_process_control_keys() -> anyhow::Result<()> {
    use std::io::Write;

    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(&manifest.env_file)
            .test()?,
        "PATH=/attacker/bin"
    )
    .test()?;
    let rejected = stackstead(&project.repo)
        .args(["run", "feature-a", "--", "true"])
        .assert()
        .failure();
    assert!(
        output_text(&rejected.get_output().stderr)?.contains("do not match the manifest"),
        "test contract condition failed"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn queued_lifecycle_rechecks_teardown_after_the_run_lease_wait() -> anyhow::Result<()> {
    use std::{thread, time::Duration};

    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let ready = project.repo.parent().test()?.join("agent-run-ready");
    let mut child = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .args(["run", "feature-a", "--", "sh", "-c"])
        .arg("touch \"$1\"; while test -e \"$1\"; do sleep 0.05; done")
        .arg("stackstead-agent-lease")
        .arg(&ready)
        .spawn()
        .test_context("start leased agent command")?;
    assert!(
        wait_for_file(&ready, 100, Duration::from_millis(20)),
        "agent child did not start"
    );

    let mut waiting = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .args(["repair", "feature-a"])
        .spawn()
        .test_context("start waiting lifecycle command")?;
    thread::sleep(Duration::from_millis(150));
    assert!(waiting.try_wait().test()?.is_none(), "repair did not wait");
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
    .test()?;
    assert!(
        manifest.manifest_path().is_file(),
        "test contract condition failed"
    );
    assert!(manifest.worktree.is_dir(), "test contract condition failed");

    fs::remove_file(&ready).test_context("release agent probe")?;
    assert!(
        child
            .wait()
            .test_context("wait for agent command")?
            .success(),
        "test contract condition failed"
    );
    assert!(
        !waiting.wait().test_context("wait for repair")?.success(),
        "test contract condition failed"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn normal_agent_completion_terminates_background_descendants() -> anyhow::Result<()> {
    use std::{thread, time::Duration};

    let project = Project::initialized()?;
    project.create("feature-a")?;
    let pid_file = project.repo.parent().test()?.join("background-agent.pid");
    stackstead(&project.repo)
        .args(["run", "feature-a", "--", "sh", "-c"])
        .arg("sleep 30 & echo $! > \"$1\"")
        .arg("stackstead-background-agent")
        .arg(&pid_file)
        .assert()
        .success();
    let pid = fs::read_to_string(pid_file)
        .test()?
        .trim()
        .parse::<i32>()
        .test()?;
    for _ in 0..100 {
        if rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).test()?).is_err()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    anyhow::bail!("background agent descendant {pid} survived normal wrapper completion")
}

#[cfg(target_os = "linux")]
#[test]
fn killed_run_wrapper_cleans_direct_and_detached_children_before_releasing_destroy()
-> anyhow::Result<()> {
    use std::{os::unix::fs::PermissionsExt, thread, time::Duration};

    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let parent = project.repo.parent().test()?;
    let direct_pid_file = parent.join("interrupted-direct.pid");
    let detached_pid_file = parent.join("interrupted-detached.pid");
    let script = parent.join("interrupted-agent");
    fs::write(
        &script,
        "#!/bin/sh\ntrap '' TERM\necho $$ > \"$1\"\nsetsid sh -c 'trap \"\" TERM; echo $$ > \"$1\"; exec sleep 30' stackstead-detached \"$2\" &\nwait\n",
    )
    .test_context("write agent script")?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
        .test_context("make agent script executable")?;
    let mut wrapper = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .args(["run", "feature-a", "--"])
        .arg(&script)
        .arg(&direct_pid_file)
        .arg(&detached_pid_file)
        .spawn()
        .test_context("start stackstead wrapper")?;
    assert!(
        wait_for_file(&detached_pid_file, 100, Duration::from_millis(20)),
        "test contract condition failed"
    );
    let direct_pid = fs::read_to_string(&direct_pid_file)
        .test_context("direct child wrote PID")?
        .trim()
        .parse::<i32>()
        .test_context("parse direct PID")?;
    let detached_pid = fs::read_to_string(&detached_pid_file)
        .test_context("detached child wrote PID")?
        .trim()
        .parse::<i32>()
        .test_context("parse detached PID")?;
    rustix::process::kill_process(
        rustix::process::Pid::from_child(&wrapper),
        rustix::process::Signal::KILL,
    )
    .test_context("kill stackstead wrapper")?;
    wrapper.wait().test_context("reap killed wrapper")?;

    let path = fake_docker_path(parent, "orphan-lease-fake-bin", "#!/bin/sh\nexit 0\n")?;
    let mut destroy = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .env("PATH", path)
        .args(["destroy", "feature-a", "--yes"])
        .spawn()
        .test_context("start waiting destroy")?;
    thread::sleep(Duration::from_millis(100));
    assert!(
        destroy.try_wait().test()?.is_none(),
        "destroy overtook cleanup"
    );
    assert!(
        destroy.wait().test_context("wait for destroy")?.success(),
        "test contract condition failed"
    );
    for pid in [direct_pid, detached_pid] {
        for _ in 0..100 {
            if rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).test()?)
                .is_err()
            {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).test()?)
            .test_err()
            .map_err(|error| anyhow::anyhow!("child {pid} survived: {error}"))?;
    }
    assert!(
        !manifest.stackstead_root.exists(),
        "test contract condition failed"
    );
    Ok(())
}
