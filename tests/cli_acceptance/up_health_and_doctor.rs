use super::*;

#[test]
fn dependency_failure_is_persisted_without_starting_compose() {
    let project = Project::initialized();
    project.replace_config(
        "    command: ''\n    shell: false\n",
        "    command: stackstead-command-that-does-not-exist\n    shell: false\n",
    );
    let manifest = project.create("feature-a");

    let assert = stackstead(&project.repo)
        .args(["up", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr).contains("stackstead-command-that-does-not-exist")
    );

    let persisted = StacksteadManifest::read(&manifest.manifest_path()).expect("read failed state");
    assert_eq!(
        serde_json::to_value(persisted.status.dependencies).expect("serialize status"),
        Value::String("failed".into())
    );
    let events = event_types(&persisted.event_log);
    assert!(events.contains(&"dependencies_install".into()));
    assert!(!events.contains(&"runtime_start".into()));
}

#[cfg(unix)]
#[test]
fn dependency_and_yarn_logs_redact_structured_and_environment_secrets() {
    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["dependencies"]["provider"] = "yarn-classic".into();
    config["dependencies"]["install"]["command"] = "printf 'Authorization: Bearer dependency-header-marker\\nhttps://user:dependency-url-marker@example.invalid/repo\\n%s\\nordinary dependency output\\n' \"$API_TOKEN\"".into();
    config["dependencies"]["install"]["shell"] = true.into();
    config["dependencies"]["link"] = serde_yaml::to_value(serde_json::json!({
        "enabled": true,
        "link_folder": ".stackstead/yarn-links",
        "command": "printf 'Proxy-Authorization: Basic yarn-header-marker\\nhttps://user:yarn-url-marker@example.invalid/repo\\n%s\\nordinary yarn output\\n' \"$API_TOKEN\"",
        "shell": true,
    }))
    .unwrap();
    config["env"]["generate"]["API_TOKEN"] = "known-environment-marker".into();
    project.write_config(&config, "configure secret-emitting dependency fixtures");
    let manifest = project.create("feature-a");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "redaction-fake-docker-bin",
        "#!/bin/sh\nexit 19\n",
    );

    stackstead(&project.repo)
        .env("PATH", path)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    for (name, ordinary) in [
        ("dependencies.log", "ordinary dependency output"),
        ("yarn-link.log", "ordinary yarn output"),
    ] {
        let log = fs::read_to_string(manifest.state_dir.join("logs").join(name)).unwrap();
        assert!(log.contains(ordinary));
        assert!(log.contains("[REDACTED]"));
        for marker in [
            "dependency-header-marker",
            "dependency-url-marker",
            "yarn-header-marker",
            "yarn-url-marker",
            "known-environment-marker",
        ] {
            assert!(!log.contains(marker), "{name} leaked {marker}");
        }
    }
}

#[test]
fn failed_dependency_diagnostics_and_events_share_structured_redaction() {
    let project = Project::initialized();
    project.replace_config(
        "    command: ''\n    shell: false\n",
        "    command: \"printf 'Authorization: Bearer event-header-marker\\\\n' >&2; exit 7\"\n    shell: true\n",
    );
    let manifest = project.create("feature-a");

    let rejected = stackstead(&project.repo)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    assert!(!output_text(&rejected.get_output().stderr).contains("event-header-marker"));
    let events = fs::read_to_string(&manifest.event_log).unwrap();
    assert!(events.contains("[REDACTED]"));
    assert!(!events.contains("event-header-marker"));
}

#[test]
fn pre_up_failure_preserves_completed_dependency_status() {
    let project = Project::initialized();
    project.replace_config(
        "  pre_up: []\n",
        "  pre_up:\n  - command: stackstead-pre-up-command-that-does-not-exist\n    shell: false\n",
    );
    let mut manifest = project.create("feature-a");
    manifest.status.database = ComponentStatus::Reachable;
    manifest.status.health = ComponentStatus::Ready;
    manifest.save_atomic().unwrap();

    stackstead(&project.repo)
        .args(["up", "feature-a", "--json"])
        .assert()
        .failure();

    let persisted = StacksteadManifest::read(&manifest.manifest_path()).expect("read failed state");
    assert_eq!(persisted.status.dependencies, ComponentStatus::Ready);
    assert_eq!(persisted.status.database, ComponentStatus::Unknown);
    assert_eq!(persisted.status.health, ComponentStatus::Unknown);
    assert!(!event_types(&persisted.event_log).contains(&"runtime_start".into()));
}

#[cfg(unix)]
#[test]
fn up_revalidates_contract_mutations_after_pre_and_post_hooks() {
    for post_up in [false, true] {
        let project = Project::initialized();
        let mut config = load_config(&project.repo.join("stackstead.yaml"));
        config["database"]["postgres"] = serde_yaml::Value::Null;
        config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
        let mutation = serde_json::json!({
            "command": "printf 'services:\\n  web:\\n    ports: [\"80\"]\\n' > docker-compose.yml",
            "shell": true,
        });
        if post_up {
            config["hooks"]["post_up"] = serde_yaml::to_value([mutation]).unwrap();
        } else {
            config["hooks"]["pre_up"] = serde_yaml::to_value([mutation]).unwrap();
        }
        project.write_config(&config, "configure contract mutation hook");
        let manifest = project.create("feature-a");
        let marker = project.repo.parent().unwrap().join(if post_up {
            "post-up-docker-ran"
        } else {
            "pre-up-docker-ran"
        });
        let fake_state = project
            .repo
            .parent()
            .unwrap()
            .join("contract-hook-docker-state");
        let docker_script = format!(
            r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$1 $2" in
  "container ls"|"network ls") exit 0 ;;
  "volume ls") test ! -f "$FAKE_STATE/claim" || printf '%s\n' "$claim"; exit 0 ;;
  "volume create") printf '%s' "$EXPECTED_TOKEN" > "$FAKE_STATE/claim"; exit 0 ;;
  "volume inspect") printf '{{"io.stackstead.runtime-token":"%s"}}\n' "$(cat "$FAKE_STATE/claim")"; exit 0 ;;
  "compose -p") touch '{}'; exit 0 ;;
esac
exit 0
"#,
            marker.display()
        );
        let path = fake_docker_path(
            project.repo.parent().unwrap(),
            if post_up {
                "post-up-contract-fake-bin"
            } else {
                "pre-up-contract-fake-bin"
            },
            &docker_script,
        );
        let rejected = stackstead(&project.repo)
            .env("PATH", path)
            .env("FAKE_STATE", fake_state)
            .env("EXPECTED_TOKEN", &manifest.runtime_token)
            .args(["up", &manifest.stackstead_id])
            .assert()
            .failure();
        assert!(output_text(&rejected.get_output().stderr).contains("deterministic host binding"));
        assert_eq!(marker.exists(), post_up, "Docker stage ordering changed");
    }
}

#[cfg(unix)]
#[test]
fn command_health_persists_ready_failed_and_stop_reset_states() {
    let project = Project::git_repo();
    fs::write(
        project.repo.join("docker-compose.yml"),
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"127.0.0.1:${WEB_PORT}:80\"\n",
    )
    .expect("write web-only Compose fixture");
    git(&project.repo, &["add", "docker-compose.yml"]);
    git(
        &project.repo,
        &["commit", "-m", "use web-only health fixture"],
    );
    stackstead(&project.repo).arg("init").assert().success();

    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["health"]["timeout_seconds"] = 1.into();
    config["health"]["interval_millis"] = 10.into();
    config["health"]["checks"] = serde_yaml::to_value([serde_json::json!({
        "name": "worker",
        "url": null,
        "expect_status": 200,
        "command": {
            "command": "test -f README.md && test \"$COMPOSE_PROJECT_NAME\" = \"demo-project-$STACKSTEAD_ID\"",
            "shell": true,
        },
    })])
    .unwrap();
    config["hooks"]["pre_up"] = serde_yaml::to_value([serde_json::json!({
        "command": "true",
        "shell": false,
    })])
    .unwrap();
    config["hooks"]["post_up"] = serde_yaml::to_value([serde_json::json!({
        "command": "true",
        "shell": false,
    })])
    .unwrap();
    project.write_config(&config, "configure command health fixture");
    let manifest = project.create("feature-a");

    let fake_state = project.repo.parent().unwrap().join("health-docker-state");
    let docker_script = format!(
        r#"#!/bin/sh
set -eu
test -z "${{WEB_PORT+x}}" || exit 90
test "$COMPOSE_PROJECT_NAME" = "{}" || exit 92
mkdir -p "$FAKE_STATE"
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$1 $2" in
  "container ls"|"network ls") exit 0 ;;
  "volume ls") test ! -f "$FAKE_STATE/claim" || printf '%s\n' "$claim"; exit 0 ;;
  "volume create") printf '%s' "$EXPECTED_TOKEN" > "$FAKE_STATE/claim"; exit 0 ;;
  "volume inspect") printf '{{"io.stackstead.runtime-token":"%s"}}\n' "$(cat "$FAKE_STATE/claim")"; exit 0 ;;
esac
while [ "$#" -gt 0 ]; do
  if [ "$1" = --env-file ]; then shift; env_file=$1; break; fi
  shift
done
. "$env_file"
test "$WEB_PORT" = "{}" || exit 91
exit 0
"#,
        manifest.compose_project, manifest.ports["web"]
    );
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "health-fake-docker-bin",
        &docker_script,
    );
    let docker = std::env::split_paths(&path).next().unwrap().join("docker");

    let ready = stackstead(&project.repo)
        .env("PATH", &path)
        .env("WEB_PORT", "9")
        .env("STACKSTEAD_ID", "spoofed")
        .env("COMPOSE_PROJECT_NAME", "shared")
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["--json", "up", "feature-a"])
        .assert()
        .success();
    assert!(!output_text(&ready.get_output().stdout).contains("Timings"));
    let ready = changed_manifest(&ready.get_output().stdout, "started");
    assert_eq!(ready.status.health, ComponentStatus::Ready);
    let human = stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", "feature-a"])
        .assert()
        .success();
    let human = output_text(&human.get_output().stdout);
    for phase in [
        "Timings:",
        "Dependencies",
        "Runtime start",
        "Hooks",
        "Health checks",
        "Total",
    ] {
        assert!(
            human.contains(phase),
            "human output omitted {phase:?}: {human}"
        );
    }
    for omitted in ["DB readiness", "Seed"] {
        assert!(
            !human.contains(omitted),
            "human output included unconfigured phase {omitted:?}: {human}"
        );
    }
    let inspected = stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["--json", "inspect", "feature-a"])
        .assert()
        .success();
    let inspected: Value =
        serde_json::from_slice(&inspected.get_output().stdout).expect("parse inspect output");
    assert!(inspected["live"]["health"].is_null());
    assert_eq!(inspected["stackstead"]["status"]["health"], "ready");

    fs::write(&docker, "#!/bin/sh\nexit 19\n").expect("make Compose fail");
    stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", "feature-a"])
        .assert()
        .failure();
    let compose_failed =
        StacksteadManifest::read(&manifest.manifest_path()).expect("read Compose failure state");
    assert_eq!(compose_failed.status.health, ComponentStatus::Unknown);
    fs::write(&docker, &docker_script).expect("restore fake Docker");

    let stopped = stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["--json", "stop", "feature-a"])
        .assert()
        .success();
    let stopped = changed_manifest(&stopped.get_output().stdout, "stopped");
    assert_eq!(stopped.status.health, ComponentStatus::Unknown);

    config["health"]["checks"][0]["command"]["command"] = "false".into();
    project.write_config(&config, "make command health fail");
    stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", "feature-a"])
        .assert()
        .failure();
    let failed = StacksteadManifest::read(&manifest.manifest_path()).expect("read failed manifest");
    assert_eq!(failed.status.health, ComponentStatus::Failed);
    let health_error = fs::read_to_string(&failed.event_log)
        .expect("read health events")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("parse health event"))
        .any(|event| event["type"] == "health_wait" && event["status"] == "failed");
    assert!(health_error);
}

#[cfg(unix)]
#[test]
fn inspect_passively_checks_http_health_only_for_a_running_runtime() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["resources"]["ports"]["base"] = serde_yaml::Value::Number(50_000.into());
    project.write_config(&config, "isolate passive health test ports");
    let manifest = project.create("feature-a");
    let listener =
        TcpListener::bind(("127.0.0.1", manifest.ports["web"])).expect("bind allocated web port");
    let server = thread::spawn(move || {
        for status in ["200 OK", "500 Internal Server Error"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let read = stream.read(&mut buffer).unwrap();
                assert_ne!(read, 0, "client closed before sending HTTP headers");
                request.extend_from_slice(&buffer[..read]);
            }
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            stream.flush().unwrap();
        }
    });
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "inspect-health-fake-bin",
        "#!/bin/sh\nprintf '%s\n' '[{\"Name\":\"demo-web-1\",\"Service\":\"web\",\"State\":\"running\",\"ExitCode\":0}]'\n",
    );

    for expected in [true, false] {
        let inspected = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["inspect", "feature-a", "--json"])
            .assert()
            .success();
        let value: Value = serde_json::from_slice(&inspected.get_output().stdout).unwrap();
        assert_eq!(value["live"]["runtime"]["running"], true);
        assert_eq!(value["live"]["health"]["healthy"], expected);
    }
    server.join().unwrap();
}

#[test]
fn in_repo_state_is_rejected_before_creating_state() {
    let project = Project::initialized();
    project.replace_config("root: ../.stacksteads", "root: .stacksteads");
    let rejected = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    assert!(
        output_text(&rejected.get_output().stderr)
            .contains("state.root must resolve outside the repository")
    );
    assert!(!project.repo.join(".stacksteads").exists());
}

#[cfg(unix)]
#[test]
fn create_rejects_a_project_lock_symlink_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let project = Project::initialized();
    let marker = project.repo.parent().unwrap().join("lock-marker");
    fs::write(&marker, "unchanged\n").unwrap();
    let lock = project
        .repo
        .parent()
        .unwrap()
        .join(".stacksteads/demo-project/project.lock");
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    symlink(&marker, &lock).unwrap();

    stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    assert_eq!(fs::read_to_string(&marker).unwrap(), "unchanged\n");
    assert!(
        fs::symlink_metadata(&lock)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::read_dir(lock.parent().unwrap())
            .unwrap()
            .all(|entry| !entry.unwrap().file_type().unwrap().is_dir())
    );

    fs::remove_file(&lock).unwrap();
    let manifest = project.create("feature-a");
    assert!(manifest.manifest_path().is_file());
    assert_eq!(fs::read_to_string(marker).unwrap(), "unchanged\n");
}

#[cfg(unix)]
#[test]
fn state_parent_symlinks_are_resolved_to_safe_external_targets() {
    use std::os::unix::fs::symlink;

    let project = Project::initialized();
    project.replace_config("root: ../.stacksteads", "root: .stacksteads");
    let outside = project.repo.parent().unwrap().join("outside-state-root");
    let project_target = outside.join("demo-project");
    fs::create_dir_all(&project_target).unwrap();
    let link = project.repo.join(".stacksteads");
    let lock_target = project_target.join("project.lock");
    fs::write(&lock_target, "unchanged\n").unwrap();
    symlink(&outside, &link).unwrap();
    git(&project.repo, &["add", ".stacksteads"]);
    git(&project.repo, &["commit", "-m", "add external state alias"]);

    let created = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .success();
    assert!(!created.get_output().stdout.is_empty());
    assert!(
        fs::read_to_string(lock_target)
            .unwrap()
            .contains("acquired_at=")
    );
}

#[test]
fn doctor_scans_branch_local_compose_files_for_fixed_ports() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    fs::write(
        &manifest.compose_files[0],
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"3000:80\"\n",
    )
    .expect("write branch-local Compose change");

    let assert = stackstead(&project.repo)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let diagnostics: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("parse diagnostics");
    assert!(
        diagnostics["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["code"]
                == "compose.worktree_fixed_host_port"
                && item["message"].as_str().is_some_and(|message| {
                    message.contains("3000") && message.contains("docker-compose.yml:5")
                })))
    );
    assert!(
        diagnostics["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| {
                item["code"] == "compose.worktree_all_interfaces_host_port"
                    && item["severity"] == "error"
                    && item["message"].as_str().is_some_and(|message| {
                        message.contains("web")
                            && message.contains("80/tcp")
                            && message
                                .contains(manifest.compose_files[0].to_string_lossy().as_ref())
                    })
            }))
    );

    fs::write(
        &manifest.compose_files[0],
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"127.0.0.1:${WEB_PORT}:80\"\n",
    )
    .unwrap();
    let loopback = stackstead(&project.repo)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let loopback: Value = serde_json::from_slice(&loopback.get_output().stdout).unwrap();
    assert!(loopback["diagnostics"].as_array().is_some_and(|items| {
        items
            .iter()
            .all(|item| item["code"] != "compose.worktree_all_interfaces_host_port")
    }));
}

#[test]
fn doctor_reports_project_worktree_and_pointer_contract_failures_together() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    fs::write(
        project.repo.join("docker-compose.yml"),
        "services:\n  web:\n    ports: [\"80\"]\n",
    )
    .unwrap();
    fs::write(
        &manifest.compose_files[0],
        "services:\n  web:\n    ports: [\"${APP_PORT}:80\"]\n",
    )
    .unwrap();
    let mut pointer: Value =
        serde_json::from_slice(&fs::read(&manifest.pointer_file).unwrap()).expect("parse pointer");
    pointer["stackstead_id"] = Value::String("copied-pointer-a123".into());
    fs::write(
        &manifest.pointer_file,
        serde_json::to_vec_pretty(&pointer).unwrap(),
    )
    .unwrap();

    let output = stackstead(&project.repo)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let diagnostics: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    let codes = diagnostics["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["code"].as_str())
        .collect::<BTreeSet<_>>();
    for expected in [
        "compose.unbound_host_port",
        "compose.isolation_contract.invalid",
        "compose.worktree_isolation_contract.invalid",
        "pointer.binding.invalid",
    ] {
        assert!(
            codes.contains(expected),
            "missing diagnostic {expected}: {codes:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn doctor_fail_on_error_keeps_complete_json_and_ignores_warnings() {
    let project = Project::initialized();
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "doctor-ci-fake-bin",
        "#!/bin/sh\ntest \"${1-}\" != info\n",
    );

    let warning_only = stackstead(&project.repo)
        .env("PATH", &path)
        .args(["doctor", "--json", "--fail-on-error"])
        .assert()
        .success();
    let warning_report: Value = serde_json::from_slice(&warning_only.get_output().stdout).unwrap();
    assert_eq!(warning_report["kind"], "DoctorReport");
    assert_eq!(warning_report["version"], "1");
    assert_eq!(warning_report["error_count"], 0);
    assert!(warning_report["warning_count"].as_u64().unwrap() > 0);

    fs::write(
        project.repo.join("docker-compose.yml"),
        "services:\n  web:\n    ports: [\"80\"]\n",
    )
    .unwrap();
    stackstead(&project.repo)
        .env("PATH", &path)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let failed = stackstead(&project.repo)
        .env("PATH", path)
        .args(["doctor", "--json", "--fail-on-error"])
        .assert()
        .code(1);
    let error_report: Value = serde_json::from_slice(&failed.get_output().stdout).unwrap();
    assert_eq!(error_report["ok"], false);
    assert!(error_report["error_count"].as_u64().unwrap() > 0);
    assert!(!error_report["diagnostics"].as_array().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn doctor_reports_repository_policy_freshness_without_failing() {
    let project = Project::initialized();
    let instructions = project.repo.join("AGENTS.md");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "policy-doctor-fake-bin",
        "#!/bin/sh\nexit 0\n",
    );

    let report = stackstead(&project.repo)
        .env("PATH", &path)
        .args(["doctor", "--json", "--fail-on-error"])
        .assert()
        .success();
    let report: Value = serde_json::from_slice(&report.get_output().stdout).unwrap();
    assert!(has_diagnostic(
        &report,
        "repository_policy.missing",
        "warning"
    ));

    for (contents, code, severity) in [
        (
            "<!-- stackstead-policy: 0 -->\n",
            "repository_policy.outdated",
            "warning",
        ),
        (
            "## Stackstead\nRead `$STACKSTEAD_CONTEXT`.\n",
            "repository_policy.unversioned",
            "warning",
        ),
        (
            "<!-- stackstead-policy: 2 -->\n",
            "repository_policy.binary_outdated",
            "warning",
        ),
        (
            "<!-- stackstead-policy: 1 -->\n",
            "repository_policy.current",
            "info",
        ),
    ] {
        fs::write(&instructions, contents).unwrap();
        let report = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["doctor", "--json", "--fail-on-error"])
            .assert()
            .success();
        let report: Value = serde_json::from_slice(&report.get_output().stdout).unwrap();
        assert!(has_diagnostic(&report, code, severity), "{report:#}");
        assert!(!has_diagnostic(
            &report,
            "repository_policy.missing",
            "warning"
        ));
    }
}

#[test]
fn env_outputs_redact_credentials_and_generation_is_deterministic() {
    let project = Project::initialized();
    project.replace_config(
        "    DATABASE_URL: postgres://app:app@127.0.0.1:{{ ports.postgres }}/app\n",
        "    Z_LAST: \"value#hash\"\n    SERVICE_DSN: postgresql://worker:dnspass@127.0.0.1:{{ ports.postgres }}/app\n    DATABASE_URL: postgres://app:app@127.0.0.1:{{ ports.postgres }}/app\n    A_FIRST: \"hello world\"\n",
    );
    let manifest = project.create("feature-a");

    let env = fs::read_to_string(&manifest.env_file).expect("read generated env");
    assert!(env.contains("A_FIRST=\"hello world\""));
    assert!(env.contains("Z_LAST=\"value#hash\""));
    let keys = env
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split_once('=').expect("env assignment").0)
        .collect::<Vec<_>>();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted, "generated env assignments are not sorted");

    for args in [
        vec!["env", "feature-a"],
        vec!["env", "feature-a", "--json"],
        vec!["env", "feature-a", "--print"],
    ] {
        let assert = stackstead(&project.repo).args(args).assert().success();
        let stdout = output_text(&assert.get_output().stdout);
        assert!(
            !stdout.contains("postgres://app:app@"),
            "DATABASE_URL leaked: {stdout}"
        );
        assert!(
            !stdout.contains("worker:dnspass@"),
            "SERVICE_DSN leaked: {stdout}"
        );
        assert!(stdout.contains("DATABASE_URL"));
        assert!(stdout.contains("SERVICE_DSN"));
        assert!(stdout.contains("[REDACTED]"));
    }
}
