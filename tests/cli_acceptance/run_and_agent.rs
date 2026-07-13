use super::*;

#[cfg(unix)]
#[test]
fn run_pins_stackstead_identity_and_preserves_the_child_exit_code() {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let script = project.repo.parent().unwrap().join("agent-probe");
    fs::write(
        &script,
        r#"#!/bin/sh
test "$PWD" = "$1" || exit 91
test "$STACKSTEAD_ID" = "$2" || exit 92
test "$STACKSTEAD_COMPOSE_PROJECT" = "$3" || exit 93
test "$COMPOSE_PROJECT_NAME" = "$3" || exit 94
test "$STACKSTEAD_WORKTREE" = "$1" || exit 95
printf '%s|%s\n' "$STACKSTEAD_ID" "$4"
exit 23
"#,
    )
    .expect("write agent probe");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
        .expect("make agent probe executable");

    let assert = stackstead(&project.repo)
        .env("STACKSTEAD_ID", "spoofed")
        .env("COMPOSE_PROJECT_NAME", "shared")
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
        output_text(&assert.get_output().stdout),
        format!("{}|argument with spaces\n", manifest.stackstead_id)
    );

    let json_run = stackstead(&project.repo)
        .args(["--json", "run", "feature-a", "--", "true"])
        .assert()
        .failure();
    assert!(
        output_text(&json_run.get_output().stderr).contains("--json cannot be combined with run")
    );
}

#[cfg(unix)]
#[test]
fn launch_creates_starts_and_runs_with_the_full_stackstead_identity() {
    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["database"]["postgres"] = serde_yaml::Value::Null;
    config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
    project.write_config(&config, "configure launch fixture");
    let fake_state = project.repo.parent().unwrap().join("launch-docker-state");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
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
    );

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

    let directories = state_stackstead_directories(&project);
    assert_eq!(directories.len(), 1);
    let manifest = StacksteadManifest::read(&directories[0].join("state/manifest.json")).unwrap();
    assert_eq!(manifest.status.runtime, ComponentStatus::Running);
    let stdout = output_text(&launched.get_output().stdout);
    assert!(stdout.contains(&format!("Created {}", manifest.stackstead_id)));
    assert!(stdout.contains("Timings:"));
    assert!(stdout.contains(&format!(
        "child:{}|{}",
        manifest.stackstead_id,
        manifest.worktree.display()
    )));
}

#[cfg(unix)]
#[test]
fn active_launch_blocks_destroy_without_blocking_another_run() {
    use std::time::Duration;

    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["database"]["postgres"] = serde_yaml::Value::Null;
    config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
    project.write_config(&config, "configure launch lease fixture");
    let fake_state = project
        .repo
        .parent()
        .unwrap()
        .join("launch-lease-docker-state");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "launch-lease-fake-docker-bin",
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
    );
    let ready = project.repo.parent().unwrap().join("launch-agent-ready");
    let mut launch = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .env("PATH", path)
        .env("FAKE_STATE", fake_state)
        .args(["launch", "feature-a", "--", "sh", "-c"])
        .arg("touch \"$1\"; while test -e \"$1\"; do sleep 0.05; done")
        .arg("stackstead-launch-lease")
        .arg(&ready)
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("start launch command");
    assert!(
        wait_for_file(&ready, 100, Duration::from_millis(20)),
        "launched child did not start"
    );

    let directories = state_stackstead_directories(&project);
    assert_eq!(directories.len(), 1);
    let manifest = StacksteadManifest::read(&directories[0].join("state/manifest.json")).unwrap();
    stackstead(&project.repo)
        .args(["run", &manifest.stackstead_id, "--", "true"])
        .assert()
        .success();
    let blocked = stackstead(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .failure();
    assert!(output_text(&blocked.get_output().stderr).contains("active stackstead agent"));
    assert!(manifest.worktree.is_dir());

    fs::remove_file(&ready).expect("release launched child");
    assert!(launch.wait().expect("wait for launch command").success());
}

#[test]
fn launch_preserves_the_created_cell_when_up_fails() {
    let project = Project::initialized();
    project.replace_config(
        "    command: ''\n    shell: false\n",
        "    command: stackstead-launch-dependency-that-does-not-exist\n    shell: false\n",
    );
    let child_marker = project.repo.parent().unwrap().join("launch-child-ran");

    let rejected = stackstead(&project.repo)
        .arg("launch")
        .arg("feature-a")
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg(format!("touch '{}'", child_marker.display()))
        .assert()
        .failure();

    let directories = state_stackstead_directories(&project);
    assert_eq!(directories.len(), 1);
    let manifest = StacksteadManifest::read(&directories[0].join("state/manifest.json")).unwrap();
    assert_eq!(manifest.status.dependencies, ComponentStatus::Failed);
    assert!(
        output_text(&rejected.get_output().stdout)
            .contains(&format!("Created {}", manifest.stackstead_id))
    );
    assert!(!child_marker.exists());
}

#[test]
fn launch_refuses_to_reuse_an_existing_cell() {
    let project = Project::initialized();
    let existing = project.create("feature-a");
    let child_marker = project
        .repo
        .parent()
        .unwrap()
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

    assert!(output_text(&rejected.get_output().stderr).contains("already exists"));
    assert_eq!(state_stackstead_directories(&project).len(), 1);
    assert!(existing.manifest_path().is_file());
    assert!(!child_marker.exists());
}

#[test]
fn launch_rejects_json_before_creating_state() {
    let project = Project::initialized();

    let rejected = stackstead(&project.repo)
        .args(["--json", "launch", "feature-a", "--", "true"])
        .assert()
        .failure();

    assert!(
        output_text(&rejected.get_output().stderr)
            .contains("--json cannot be combined with launch")
    );
    assert!(state_stackstead_directories(&project).is_empty());
}

#[test]
fn create_rejects_a_slug_that_matches_an_existing_full_id() {
    let project = Project::initialized();
    let existing = project.create("feature-a");
    let before = state_stackstead_directories(&project);

    let assert = stackstead(&project.repo)
        .args(["create", &existing.stackstead_id, "--json"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr).contains("already exists"));
    assert_eq!(state_stackstead_directories(&project), before);
}

#[test]
fn generated_environment_cannot_add_process_control_keys() {
    use std::io::Write;

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(&manifest.env_file)
            .unwrap(),
        "PATH=/attacker/bin"
    )
    .unwrap();
    let rejected = stackstead(&project.repo)
        .args(["run", "feature-a", "--", "true"])
        .assert()
        .failure();
    assert!(output_text(&rejected.get_output().stderr).contains("do not match the manifest"));
}

#[cfg(unix)]
#[test]
fn active_agent_run_blocks_destroy_until_the_child_exits() {
    use std::time::Duration;

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let ready = project.repo.parent().unwrap().join("agent-run-ready");
    let mut child = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .args(["run", "feature-a", "--", "sh", "-c"])
        .arg("touch \"$1\"; while test -e \"$1\"; do sleep 0.05; done")
        .arg("stackstead-agent-lease")
        .arg(&ready)
        .spawn()
        .expect("start leased agent command");
    assert!(
        wait_for_file(&ready, 100, Duration::from_millis(20)),
        "agent child did not start"
    );

    for args in [
        vec!["up", "feature-a"],
        vec!["stop", "feature-a"],
        vec!["repair", "feature-a"],
        vec!["destroy", "feature-a", "--yes"],
    ] {
        let blocked = stackstead(&project.repo).args(args).assert().failure();
        assert!(output_text(&blocked.get_output().stderr).contains("active stackstead agent"));
    }
    assert!(manifest.manifest_path().is_file());
    assert!(manifest.worktree.is_dir());

    fs::remove_file(&ready).expect("release agent probe");
    assert!(child.wait().expect("wait for agent command").success());
}

#[cfg(target_os = "linux")]
#[test]
fn normal_agent_completion_terminates_background_descendants() {
    use std::{thread, time::Duration};

    let project = Project::initialized();
    project.create("feature-a");
    let pid_file = project.repo.parent().unwrap().join("background-agent.pid");
    stackstead(&project.repo)
        .args(["run", "feature-a", "--", "sh", "-c"])
        .arg("sleep 30 & echo $! > \"$1\"")
        .arg("stackstead-background-agent")
        .arg(&pid_file)
        .assert()
        .success();
    let pid = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    for _ in 0..100 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("background agent descendant {pid} survived normal wrapper completion");
}

#[cfg(unix)]
#[test]
fn agent_child_keeps_the_destroy_lease_after_the_cli_is_killed() {
    use std::{os::unix::fs::PermissionsExt, thread, time::Duration};

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let parent = project.repo.parent().unwrap();
    let pid_file = parent.join("orphaned-agent.pid");
    let script = parent.join("orphaned-agent");
    fs::write(&script, "#!/bin/sh\necho $$ > \"$1\"\nexec sleep 30\n").expect("write agent script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
        .expect("make agent script executable");
    let mut wrapper = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .args(["run", "feature-a", "--"])
        .arg(&script)
        .arg(&pid_file)
        .spawn()
        .expect("start stackstead wrapper");
    assert!(wait_for_file(&pid_file, 100, Duration::from_millis(20)));
    let agent_pid = fs::read_to_string(&pid_file)
        .expect("agent wrote PID")
        .trim()
        .parse::<i32>()
        .expect("parse agent PID");
    assert_eq!(unsafe { libc::kill(wrapper.id() as i32, libc::SIGKILL) }, 0);
    wrapper.wait().expect("reap killed wrapper");

    let blocked = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--yes"])
        .assert()
        .failure();
    assert!(output_text(&blocked.get_output().stderr).contains("active stackstead agent"));
    assert_eq!(unsafe { libc::kill(agent_pid, libc::SIGKILL) }, 0);

    let path = fake_docker_path(parent, "orphan-lease-fake-bin", "#!/bin/sh\nexit 0\n");
    for _ in 0..100 {
        let output = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["destroy", "feature-a", "--yes"])
            .output()
            .expect("retry destroy");
        if output.status.success() {
            assert!(!manifest.stackstead_root.exists());
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("destroy lease did not release after the orphaned agent exited");
}

#[cfg(unix)]
#[test]
fn missing_lock_contract_is_rejected_without_recreation() {
    let project = Project::initialized();
    let run_cell = project.create("run-legacy");
    fs::remove_file(run_cell.state_dir.join("lock")).expect("remove legacy mutation lock");
    fs::remove_file(run_cell.state_dir.join("run.lock")).expect("remove legacy run lock");
    let diagnosed = stackstead(&project.repo)
        .args(["doctor", "--json", "--fail-on-error"])
        .assert()
        .code(1);
    let report: Value = serde_json::from_slice(&diagnosed.get_output().stdout).unwrap();
    let codes = report["diagnostics"]
        .as_array()
        .unwrap()
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

    let destroy_cell = project.create("destroy-legacy");
    fs::remove_file(destroy_cell.state_dir.join("lock")).expect("remove legacy mutation lock");
    fs::remove_file(destroy_cell.state_dir.join("run.lock")).expect("remove legacy run lock");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "legacy-lock-fake-bin",
        "#!/bin/sh\nexit 0\n",
    );
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", "destroy-legacy", "--yes"])
        .assert()
        .failure();
    assert!(destroy_cell.stackstead_root.exists());
    assert!(!destroy_cell.state_dir.join("lock").exists());
    assert!(!destroy_cell.state_dir.join("run.lock").exists());
}

#[cfg(unix)]
#[test]
fn changed_worktree_branch_is_reported_and_rejected_before_agent_or_teardown() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    git(&manifest.worktree, &["switch", "-c", "unexpected-source"]);

    let inspected = stackstead(&project.repo)
        .args(["--json", "inspect", "feature-a"])
        .assert()
        .success();
    let inspected: Value =
        serde_json::from_slice(&inspected.get_output().stdout).expect("parse inspect output");
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
        let stderr = output_text(&assert.get_output().stderr);
        assert!(
            stderr.contains("unexpected-source")
                && stderr.contains("refusing to use the wrong source"),
            "unexpected source-binding error: {stderr}"
        );
    }
    assert!(manifest.manifest_path().is_file());
    assert!(manifest.worktree.is_dir());
}

#[test]
fn all_resolved_commands_reject_redirected_manifest_contract_fields() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let secret = project.repo.parent().unwrap().join("must-not-read.env");
    fs::write(&secret, "PRIVATE_VALUE=must-not-leak\n").expect("write outside env fixture");

    let mut tampered = manifest.clone();
    tampered.env_file = secret;
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&tampered).expect("serialize redirected manifest"),
    )
    .expect("write redirected manifest");
    let assert = stackstead(&project.repo)
        .args(["env", "feature-a", "--print", "--show-secrets"])
        .assert()
        .failure();
    assert!(!output_text(&assert.get_output().stdout).contains("must-not-leak"));
    assert!(output_text(&assert.get_output().stderr).contains("escapes worktree"));

    tampered = manifest.clone();
    tampered.compose_project = "unrelated-valid-project".into();
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&tampered).expect("serialize redirected Compose identity"),
    )
    .expect("write redirected Compose identity");
    let assert = stackstead(&project.repo)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)
            .contains("manifest Compose project does not match the durable stackstead identity")
    );

    tampered = manifest.clone();
    tampered.short_id = "ffffffffffffffffffffffffffffffff".into();
    tampered.compose_project = format!("{}-feature-a-{}", tampered.project, tampered.short_id);
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&tampered).expect("serialize forged redundant identity"),
    )
    .expect("write forged redundant identity");
    let assert = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--yes"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)
            .contains("manifest stackstead ID does not match its slug and short ID")
    );
}

#[test]
fn adopted_manifests_cannot_cross_bind_or_delete_another_checkout() {
    let project = Project::initialized();
    let parent = project.repo.parent().unwrap();
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
                path.to_str().expect("UTF-8 fixture path"),
                "main",
            ],
        );
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
    let first = adopt("manager-first", &first_path);
    let second = adopt("manager-second", &second_path);
    let mut redirected = first.clone();
    redirected.worktree = second.worktree.clone();
    redirected.branch = second.branch.clone();
    redirected.compose_files = second.compose_files.clone();
    redirected.env_file = second.env_file.clone();
    redirected.agent_context = second.agent_context.clone();
    redirected.pointer_file = second.pointer_file.clone();
    redirected
        .save_atomic()
        .expect("redirect first manifest to second checkout");

    for args in [
        vec!["run", &first.stackstead_id, "--", "true"],
        vec!["repair", &first.stackstead_id],
        vec!["destroy", &first.stackstead_id, "--yes"],
    ] {
        let assert = stackstead(&project.repo).args(args).assert().failure();
        assert!(
            output_text(&assert.get_output().stderr).contains("reciprocal pointer"),
            "unexpected cross-binding failure: {}",
            output_text(&assert.get_output().stderr)
        );
    }
    assert!(second.pointer_file.is_file());
    assert!(second.manifest_path().is_file());
    assert!(second.worktree.is_dir());
    assert!(first_path.join(".stackstead/stackstead.json").is_file());
}

#[cfg(unix)]
#[test]
fn post_create_holds_the_cell_lock_after_manifest_publication() {
    use std::time::Duration;

    let project = Project::initialized();
    let ready = project.repo.parent().unwrap().join("post-create-ready");
    let release = project.repo.parent().unwrap().join("post-create-release");
    fs::write(&release, "wait\n").expect("create hook release gate");
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["hooks"]["post_create"] = serde_yaml::to_value([serde_json::json!({
        "command": format!(
            "touch '{}'; while test -e '{}'; do sleep 0.02; done",
            ready.display(),
            release.display()
        ),
        "shell": true,
    })])
    .unwrap();
    project.write_config(&config, "add blocking post-create hook");

    let mut create = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .args(["create", "feature-a"])
        .spawn()
        .expect("spawn blocked create");
    assert!(
        wait_for_file(&ready, 200, Duration::from_millis(10)),
        "post-create hook did not start"
    );
    let manifests = state_stackstead_directories(&project)
        .into_iter()
        .map(|root| root.join("state/manifest.json"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    assert_eq!(manifests.len(), 1, "manifest was not published during hook");
    let manifest = StacksteadManifest::read(&manifests[0]).expect("read published manifest");
    let assert = stackstead(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr).contains("lock"));
    assert!(manifest.worktree.is_dir());
    fs::remove_file(&release).expect("release post-create hook");
    assert!(create.wait().expect("wait for create").success());
}
