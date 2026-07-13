use super::*;

#[test]
fn runtime_probe_failure_is_reported_without_breaking_inspect_json() {
    let project = Project::initialized();
    project.create("feature-a");

    let ps = stackstead(&project.repo)
        .env("PATH", "")
        .args(["ps", "--json"])
        .assert()
        .success();
    let listed: Value = serde_json::from_slice(&ps.get_output().stdout).expect("parse ps output");
    assert_eq!(listed["stacksteads"][0]["runtime"], "unknown");

    let inspect = stackstead(&project.repo)
        .env("PATH", "")
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value =
        serde_json::from_slice(&inspect.get_output().stdout).expect("parse inspect output");
    assert_eq!(inspected["version"], "2");
    assert_eq!(inspected["live"]["runtime"]["running"], false);
    assert_eq!(inspected["live"]["runtime"]["status"], "unknown");
    assert_eq!(inspected["live"]["database"]["status"], "unknown");
    assert!(
        inspected["warnings"]
            .as_array()
            .is_some_and(|warnings| !warnings.is_empty())
    );
}

#[test]
fn inspect_reports_unreadable_compose_files_instead_of_hiding_them() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    fs::remove_file(&manifest.compose_files[0]).expect("remove generated Compose fixture");

    let inspected = stackstead(&project.repo)
        .env("PATH", "")
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value =
        serde_json::from_slice(&inspected.get_output().stdout).expect("parse inspect output");

    assert!(inspected["warnings"].as_array().is_some_and(|warnings| {
        warnings.iter().any(|warning| {
            warning.as_str().is_some_and(|warning| {
                warning.contains("could not inspect fixed ports")
                    && warning.contains(manifest.compose_files[0].to_string_lossy().as_ref())
            })
        })
    }));
}

#[test]
fn inspect_rejects_mismatched_manifest_port_service_sets() {
    let project = Project::initialized();
    let mut manifest = project.create("feature-a");
    manifest.container_ports.remove("web");
    manifest
        .save_atomic()
        .expect("write malformed manifest fixture");

    let rejected = stackstead(&project.repo)
        .args(["inspect", "feature-a"])
        .assert()
        .failure();

    assert!(
        output_text(&rejected.get_output().stderr)
            .contains("manifest host and container port service sets differ")
    );
    assert!(!output_text(&rejected.get_output().stdout).contains("-> 0"));
}

#[cfg(unix)]
#[test]
fn inspect_distinguishes_completed_and_failed_compose_services() {
    let project = Project::initialized();
    project.create("feature-a");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "inspect-services-bin",
        r#"#!/bin/sh
case " $* " in
  *" ps --all --format json "*)
    printf '%s\n' '[{"Name":"demo-web-1","Service":"web","State":"running","ExitCode":0},{"Name":"demo-init-1","Service":"init","State":"exited","ExitCode":0},{"Name":"demo-migrate-1","Service":"migrate","State":"exited","ExitCode":7}]'
    exit 0
    ;;
esac
exit 97
"#,
    );

    let json = stackstead(&project.repo)
        .env("PATH", &path)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&json.get_output().stdout).unwrap();
    assert_eq!(inspected["version"], "2");
    assert_eq!(inspected["live"]["runtime"]["status"], "running");
    assert_eq!(inspected["live"]["services"][0]["status"], "completed (0)");
    assert_eq!(inspected["live"]["services"][1]["status"], "exited (7)");
    assert_eq!(inspected["live"]["services"][2]["status"], "running");

    let human = stackstead(&project.repo)
        .env("PATH", path)
        .args(["inspect", "feature-a"])
        .assert()
        .success();
    let stdout = output_text(&human.get_output().stdout);
    assert!(stdout.contains("init           completed (0)"), "{stdout}");
    assert!(stdout.contains("migrate        exited (7)"), "{stdout}");
    assert!(stdout.contains("web            running"), "{stdout}");
}

#[test]
fn inspect_human_output_ends_with_full_id_actions() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");

    let inspect = stackstead(&project.repo)
        .env("PATH", "")
        .args(["inspect", "feature-a"])
        .assert()
        .success();
    let stdout = output_text(&inspect.get_output().stdout);

    assert!(stdout.contains(&format!(
        "\nNext:\n  stackstead doctor\n  stackstead context {} --print\n",
        manifest.stackstead_id
    )));
}

#[cfg(unix)]
#[test]
fn database_status_requires_the_exact_compose_port_publication() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let port = manifest.ports["postgres"];
    let _listener = TcpListener::bind(("127.0.0.1", port)).expect("bind unrelated listener");

    let wrong_path = fake_docker_path(
        project.repo.parent().unwrap(),
        "wrong-database-publication-fake-bin",
        &format!(
            "#!/bin/sh\ncase \" $* \" in *\" ps --all --format json \"*) printf '%s\\n' '[{{\"Name\":\"demo-postgres-1\",\"Service\":\"postgres\",\"State\":\"running\",\"ExitCode\":0}}]'; exit 0;; esac\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    ps) printf 'container-id\\n'; exit 0 ;;\n    port) printf '127.0.0.2:{port}\\n'; exit 0 ;;\n  esac\ndone\nexit 0\n"
        ),
    );
    let status = stackstead(&project.repo)
        .env("PATH", &wrong_path)
        .args(["db", "status", "feature-a", "--json"])
        .assert()
        .success();
    let status: Value = serde_json::from_slice(&status.get_output().stdout).unwrap();
    for key in [
        "stackstead_id",
        "strategy",
        "service",
        "host",
        "port",
        "database",
        "reachable",
        "identity_status",
        "seed_status",
        "last_seed_at",
    ] {
        assert!(status.get(key).is_some(), "db status JSON omitted {key}");
    }
    assert_eq!(status["reachable"], true);
    assert_eq!(status["identity_status"], "unreachable");

    let inspected = stackstead(&project.repo)
        .env("PATH", &wrong_path)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout).unwrap();
    assert_eq!(inspected["live"]["database"]["reachable"], true);
    assert_eq!(inspected["live"]["database"]["status"], "unreachable");

    let exact_path = fake_docker_path(
        project.repo.parent().unwrap(),
        "exact-database-publication-fake-bin",
        &format!(
            "#!/bin/sh\ncase \" $* \" in *\" ps --all --format json \"*) printf '%s\\n' '[{{\"Name\":\"demo-postgres-1\",\"Service\":\"postgres\",\"State\":\"running\",\"ExitCode\":0}}]'; exit 0;; esac\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    ps) printf 'container-id\\n'; exit 0 ;;\n    port) printf '127.0.0.1:{port}\\n'; exit 0 ;;\n  esac\ndone\nexit 0\n"
        ),
    );
    let status = stackstead(&project.repo)
        .env("PATH", &exact_path)
        .args(["db", "status", "feature-a", "--json"])
        .assert()
        .success();
    let status: Value = serde_json::from_slice(&status.get_output().stdout).unwrap();
    assert_eq!(status["reachable"], true);
    assert_eq!(status["identity_status"], "reachable");

    let inspected = stackstead(&project.repo)
        .env("PATH", exact_path)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout).unwrap();
    assert_eq!(inspected["live"]["database"]["reachable"], true);
    assert_eq!(inspected["live"]["database"]["status"], "reachable");
}

#[test]
fn v2_manifest_requires_explicit_source_ownership() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let mut value: Value =
        serde_json::from_slice(&fs::read(manifest.manifest_path()).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("source_ownership");
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();

    let rejected = stackstead(&project.repo)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(rejected.get_output().stdout.is_empty());
    assert!(
        output_text(&rejected.get_output().stderr).contains("requires source_ownership"),
        "unexpected error: {}",
        output_text(&rejected.get_output().stderr)
    );
    assert!(manifest.worktree.is_dir());
    assert!(manifest.stackstead_root.is_dir());
}

#[test]
fn repair_regenerates_missing_contract_files_without_docker() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    fs::remove_file(&manifest.env_file).expect("remove generated env");
    fs::remove_file(&manifest.agent_context).expect("remove generated context");
    fs::remove_file(&manifest.pointer_file).expect("remove generated pointer");

    let assert = stackstead(&project.repo)
        .args(["repair", "feature-a", "--json"])
        .assert()
        .success();
    let repaired = changed_manifest(&assert.get_output().stdout, "repaired");
    assert_eq!(repaired.stackstead_id, manifest.stackstead_id);
    assert!(repaired.env_file.is_file());
    assert!(repaired.agent_context.is_file());
    assert!(repaired.pointer_file.is_file());
    assert_eq!(
        event_types(&repaired.event_log).last().map(String::as_str),
        Some("repair")
    );

    let repaired_pointer: StacksteadPointer =
        serde_json::from_slice(&fs::read(&repaired.pointer_file).expect("read repaired pointer"))
            .expect("parse repaired pointer");
    assert_eq!(repaired_pointer.manifest, repaired.manifest_path());
}

#[test]
fn json_destroy_requires_yes_without_writing_a_prompt_to_stdout() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");

    let assert = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(
        assert.get_output().stdout.is_empty(),
        "JSON failure was contaminated by: {}",
        output_text(&assert.get_output().stdout)
    );
    assert!(output_text(&assert.get_output().stderr).contains("--yes"));
    assert!(manifest.stackstead_root.is_dir());
    assert!(manifest.worktree.is_dir());
}

#[test]
fn unexposed_database_service_fails_without_publishing_partial_state() {
    let project = Project::initialized();
    project.replace_config("    service: postgres\n", "    service: missing-postgres\n");

    let assert = stackstead(&project.repo)
        .args(["create", "feature-a", "--json"])
        .assert()
        .failure();
    let stderr = output_text(&assert.get_output().stderr);
    assert!(
        stderr.contains("resources.ports.expose") || stderr.contains("must be present"),
        "unexpected error: {stderr}"
    );
    assert!(state_stackstead_directories(&project).is_empty());
    assert!(
        git(&project.repo, &["branch", "--list", "feature-a"])
            .trim()
            .is_empty()
    );
}

#[test]
fn pointer_project_identity_fields_are_checked_against_the_manifest() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let original: Value =
        serde_json::from_slice(&fs::read(&manifest.pointer_file).expect("read generated pointer"))
            .expect("parse generated pointer");

    for (field, value) in [
        ("project", Value::String("different-project".into())),
        (
            "project_state_root",
            Value::String("/definitely/not/the/stackstead/state".into()),
        ),
    ] {
        let mut tampered = original.clone();
        tampered[field] = value;
        fs::write(
            &manifest.pointer_file,
            serde_json::to_vec_pretty(&tampered).expect("serialize tampered pointer"),
        )
        .expect("write tampered pointer");

        let assert = stackstead(&manifest.worktree)
            .args(["context", "feature-a", "--json"])
            .assert()
            .failure();
        let stderr = output_text(&assert.get_output().stderr);
        assert!(
            stderr.contains("does not match its manifest"),
            "tampered {field} was not rejected during discovery: {stderr}"
        );
    }

    fs::write(
        &manifest.pointer_file,
        serde_json::to_vec_pretty(&original).expect("serialize original pointer"),
    )
    .expect("restore original pointer");
}

#[cfg(unix)]
#[test]
fn copied_pointer_rejects_every_affected_command_before_external_mutation() {
    let victim = Project::initialized();
    let manifest = victim.create("victim");
    let caller = Project::initialized();
    let copied_pointer = caller.repo.join(".stackstead/stackstead.json");
    fs::create_dir_all(copied_pointer.parent().unwrap()).unwrap();
    fs::copy(&manifest.pointer_file, &copied_pointer).unwrap();
    let manifest_before = fs::read(manifest.manifest_path()).unwrap();
    let events_before = fs::read(&manifest.event_log).unwrap();
    let docker_marker = caller
        .repo
        .parent()
        .unwrap()
        .join("copied-pointer-docker-ran");
    let probe = caller
        .repo
        .parent()
        .unwrap()
        .join("copied-pointer-probe-ran");
    let path = fake_docker_path(
        caller.repo.parent().unwrap(),
        "copied-pointer-fake-bin",
        &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", docker_marker.display()),
    );
    let caller_path = caller.repo.to_str().unwrap();
    let probe_command = format!("touch '{}'", probe.display());

    for args in [
        vec!["create", "redirected"],
        vec!["adopt", "redirected", "--worktree", caller_path],
        vec!["up", &manifest.stackstead_id],
        vec![
            "run",
            &manifest.stackstead_id,
            "--",
            "sh",
            "-c",
            &probe_command,
        ],
        vec!["stop", &manifest.stackstead_id],
        vec!["destroy", &manifest.stackstead_id, "--yes"],
        vec!["repair", &manifest.stackstead_id],
    ] {
        let rejected = stackstead(&caller.repo)
            .env("PATH", &path)
            .args(args)
            .assert()
            .failure();
        assert!(
            output_text(&rejected.get_output().stderr).contains("does not match its manifest"),
            "unexpected copied-pointer error: {}",
            output_text(&rejected.get_output().stderr)
        );
    }
    assert!(!docker_marker.exists());
    assert!(!probe.exists());
    assert_eq!(fs::read(manifest.manifest_path()).unwrap(), manifest_before);
    assert_eq!(fs::read(&manifest.event_log).unwrap(), events_before);
    assert!(manifest.worktree.is_dir());
}

#[cfg(unix)]
#[test]
fn failed_git_worktree_add_leaves_no_published_manifest_or_stackstead_root() {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized();
    let fake_bin = project
        .repo
        .parent()
        .expect("repository has parent")
        .join("fake-bin");
    fs::create_dir(&fake_bin).expect("create fake binary directory");
    let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join("git"))
        .find(|candidate| candidate.is_file())
        .expect("find real Git executable");
    let wrapper = fake_bin.join("git");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ \"$1\" = worktree ] && [ \"$2\" = add ]; then\n  echo 'intentional worktree failure' >&2\n  exit 19\nfi\nexec '{}' \"$@\"\n",
            real_git.display().to_string().replace('\'', "'\"'\"'")
        ),
    )
    .expect("write Git wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
        .expect("make Git wrapper executable");
    let path = std::env::join_paths(std::iter::once(fake_bin.clone()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .expect("construct command-local PATH");

    let mut command = stackstead(&project.repo);
    command.env("PATH", path);
    let assert = command
        .args(["create", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr).contains("intentional worktree failure"));
    assert!(state_stackstead_directories(&project).is_empty());
    let registry: Value = serde_json::from_slice(
        &fs::read(test_state_home(&project.repo).join("stackstead/port-leases.json"))
            .expect("read rolled-back port lease registry"),
    )
    .expect("parse rolled-back port lease registry");
    assert!(registry["leases"].as_array().unwrap().is_empty());
    assert!(
        git(&project.repo, &["branch", "--list", "feature-a"])
            .trim()
            .is_empty()
    );
}
