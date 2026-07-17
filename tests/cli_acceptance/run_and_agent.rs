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
        format!("{}|argument with spaces\n", manifest.stackstead_id)
    );

    let json_run = stackstead(&project.repo)
        .args(["--json", "run", "feature-a", "--", "true"])
        .assert()
        .failure();
    assert!(
        output_text(&json_run.get_output().stderr)?.contains("--json cannot be combined with run")
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn launch_creates_starts_and_runs_with_the_full_stackstead_identity() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["database"]["postgres"] = serde_yaml::Value::Null;
    config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
    project.write_config(&config, "configure launch fixture")?;
    let fake_state = project.repo.parent().test()?.join("launch-docker-state");
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "launch-fake-docker-bin",
        r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$1 $2" in
  "volume ls") test ! -f "$FAKE_STATE/claim" || printf '%s\n' "$claim" ;;
  "volume create")
    for argument in "$@"; do
      case "$argument" in
        io.stackstead.runtime-token=*) printf '%s' "${argument#*=}" > "$FAKE_STATE/token" ;;
      esac
    done
    : > "$FAKE_STATE/claim"
    ;;
  "volume inspect") printf '{"io.stackstead.runtime-token":"%s"}\n' "$(cat "$FAKE_STATE/token")" ;;
esac
exit 0
"#,
    )?;

    let launched = stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", fake_state)
        .args([
            "launch",
            "feature-a",
            "--",
            "sh",
            "-c",
            "printf 'child:%s|%s\\n' \"$STACKSTEAD_ID\" \"$PWD\"; exit 23",
        ])
        .assert()
        .code(23);

    let directories = state_stackstead_directories(&project)?;
    assert_eq!(directories.len(), 1);
    let manifest = StacksteadManifest::read(&directories[0].join("state/manifest.json")).test()?;
    assert_eq!(manifest.status.runtime, ComponentStatus::Running);
    let stdout = output_text(&launched.get_output().stdout)?;
    assert!(stdout.contains(&format!("Created {}", manifest.stackstead_id)));
    assert!(stdout.contains("Timings:"));
    assert!(stdout.contains(&format!(
        "child:{}|{}",
        manifest.stackstead_id,
        manifest.worktree.display()
    )));
    Ok(())
}

#[test]
fn launch_preserves_the_created_cell_when_up_fails() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    project.replace_config(
        "    command: ''\n    shell: false\n",
        "    command: stackstead-launch-dependency-that-does-not-exist\n    shell: false\n",
    )?;
    let child_marker = project.repo.parent().test()?.join("launch-child-ran");

    let rejected = stackstead(&project.repo)
        .arg("launch")
        .arg("feature-a")
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg(format!("touch '{}'", child_marker.display()))
        .assert()
        .failure();

    let directories = state_stackstead_directories(&project)?;
    assert_eq!(directories.len(), 1);
    let manifest = StacksteadManifest::read(&directories[0].join("state/manifest.json")).test()?;
    assert_eq!(manifest.status.dependencies, ComponentStatus::Failed);
    assert!(
        output_text(&rejected.get_output().stdout)?
            .contains(&format!("Created {}", manifest.stackstead_id))
    );
    assert!(!child_marker.exists());
    Ok(())
}

#[test]
fn launch_refuses_to_reuse_an_existing_cell() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let existing = project.create("feature-a")?;
    let child_marker = project
        .repo
        .parent()
        .test()?
        .join("duplicate-launch-child-ran");

    let rejected = stackstead(&project.repo)
        .arg("launch")
        .arg("feature-a")
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg(format!("touch '{}'", child_marker.display()))
        .assert()
        .failure();

    assert!(output_text(&rejected.get_output().stderr)?.contains("already exists"));
    assert_eq!(state_stackstead_directories(&project)?.len(), 1);
    assert!(existing.manifest_path().is_file());
    assert!(!child_marker.exists());
    Ok(())
}

#[test]
fn launch_rejects_json_before_creating_state() -> anyhow::Result<()> {
    let project = Project::initialized()?;

    let rejected = stackstead(&project.repo)
        .args(["--json", "launch", "feature-a", "--", "true"])
        .assert()
        .failure();

    assert!(
        output_text(&rejected.get_output().stderr)?
            .contains("--json cannot be combined with launch")
    );
    assert!(state_stackstead_directories(&project)?.is_empty());
    Ok(())
}

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
    assert!(output_text(&rejected.get_output().stderr)?.contains("do not match the manifest"));
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
    assert!(manifest.manifest_path().is_file());
    assert!(manifest.worktree.is_dir());

    fs::remove_file(&ready).test_context("release agent probe")?;
    assert!(
        child
            .wait()
            .test_context("wait for agent command")?
            .success()
    );
    assert!(!waiting.wait().test_context("wait for repair")?.success());
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
    assert!(wait_for_file(
        &detached_pid_file,
        100,
        Duration::from_millis(20)
    ));
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
    assert!(destroy.wait().test_context("wait for destroy")?.success());
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
    assert!(!manifest.stackstead_root.exists());
    Ok(())
}

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

    for args in [
        vec!["run", "feature-a", "--", "true"],
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
        .save_atomic()
        .test_context("redirect first manifest to second checkout")?;

    for args in [
        vec!["run", &first.stackstead_id, "--", "true"],
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
