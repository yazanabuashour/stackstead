use super::*;

#[test]
fn help_exposes_the_complete_command_surface() {
    let directory = tempfile::tempdir().expect("create command directory");
    let assert = stackstead(directory.path())
        .arg("--help")
        .assert()
        .success();
    let help = String::from_utf8_lossy(&assert.get_output().stdout);
    for command in [
        "init", "compose", "create", "adopt", "up", "run", "launch", "ps", "inspect", "env",
        "logs", "context", "open", "db", "stop", "destroy", "doctor", "repair",
    ] {
        assert!(help.contains(command), "top-level help omits {command:?}");
    }

    for args in [
        vec!["init", "--help"],
        vec!["compose", "plan", "--help"],
        vec!["compose", "apply", "--help"],
        vec!["create", "--help"],
        vec!["adopt", "--help"],
        vec!["up", "--help"],
        vec!["run", "--help"],
        vec!["launch", "--help"],
        vec!["ps", "--help"],
        vec!["inspect", "--help"],
        vec!["env", "--help"],
        vec!["logs", "--help"],
        vec!["context", "--help"],
        vec!["open", "--help"],
        vec!["db", "status", "--help"],
        vec!["stop", "--help"],
        vec!["destroy", "--help"],
        vec!["doctor", "--help"],
        vec!["repair", "--help"],
    ] {
        stackstead(directory.path()).args(args).assert().success();
    }

    stackstead(directory.path())
        .args(["env", "demo", "--show-secrets"])
        .assert()
        .failure();
}

#[test]
fn init_writes_a_valid_config_and_refuses_to_overwrite_it() {
    let project = Project::git_repo();
    let config_path = project.repo.join("stackstead.yaml");
    let assert = stackstead(&project.repo)
        .args(["init", "--json"])
        .assert()
        .success();
    let initialized: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(initialized["kind"], "StacksteadInit");
    assert_eq!(initialized["version"], "1");
    assert_eq!(initialized["path"], config_path.to_string_lossy().as_ref());

    let original = fs::read(&config_path).expect("read initialized config");
    let config = load_config(&config_path);
    assert_eq!(config["version"], "1");
    assert_eq!(config["kind"], "StacksteadProject");
    assert_eq!(config["project"]["name"], "demo-project");
    assert_eq!(config["source"]["base"], "main");

    let assert = stackstead(&project.repo).arg("init").assert().failure();
    assert!(String::from_utf8_lossy(&assert.get_output().stderr).contains("refusing to overwrite"));
    assert_eq!(
        fs::read(&config_path).expect("reread initialized config"),
        original
    );
}

#[test]
fn init_records_the_exact_commit_for_a_detached_head() {
    let project = Project::git_repo();
    let head = git(&project.repo, &["rev-parse", "HEAD"]);
    git(&project.repo, &["checkout", "--detach"]);

    stackstead(&project.repo).arg("init").assert().success();

    let config = load_config(&project.repo.join("stackstead.yaml"));
    assert_eq!(config["source"]["base"], head.trim());
}

#[test]
fn human_init_recommends_but_does_not_edit_repository_instructions() {
    let project = Project::git_repo();
    let instructions = project.repo.join("AGENTS.md");
    fs::write(&instructions, "# Human-owned policy\n").unwrap();

    let assert = stackstead(&project.repo).arg("init").assert().success();
    let stdout = output_text(&assert.get_output().stdout);
    for expected in [
        "review, add, and commit this policy",
        "lifecycle commands instead of bare Docker Compose",
        "stackstead --json create <name>",
        "stackstead up <full-id>",
        "stackstead run <full-id> -- <agent-or-command>",
        "use only the ports, URLs, and database it provides",
        "Reuse an environment only when the user or manager supplies its exact full ID",
        "Stackstead does not edit human-owned agent instructions",
    ] {
        assert!(
            stdout.contains(expected),
            "init output omitted {expected:?}"
        );
    }
    assert!(!stdout.contains("stackstead --json ps"));
    assert_eq!(
        fs::read_to_string(&instructions).unwrap(),
        "# Human-owned policy\n"
    );

    assert!(!project.repo.join("CLAUDE.md").exists());
}

#[test]
fn create_refuses_a_runtime_contract_missing_from_the_configured_base() {
    let project = Project::git_repo();
    stackstead(&project.repo).arg("init").assert().success();

    let assert = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    let stderr = output_text(&assert.get_output().stderr);
    assert!(
        stderr.contains("not present on source.base commit")
            && stderr.contains("commit or merge stackstead.yaml"),
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
fn create_refuses_locally_modified_contract_files_without_allocating_state() {
    let project = Project::initialized();
    let compose = project.repo.join("docker-compose.yml");
    let mut contents = fs::read_to_string(&compose).expect("read Compose fixture");
    contents.push_str("# uncommitted runtime change\n");
    fs::write(&compose, contents).expect("modify Compose fixture");

    let assert = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr).contains("differs from source.base commit"));
    assert!(state_stackstead_directories(&project).is_empty());
    assert!(
        git(&project.repo, &["branch", "--list", "feature-a"])
            .trim()
            .is_empty()
    );
}

#[test]
fn create_compares_clean_contract_files_through_git_filters() {
    let project = Project::initialized();
    fs::write(
        project.repo.join(".gitattributes"),
        "stackstead.yaml text eol=crlf\ndocker-compose.yml text eol=crlf\n",
    )
    .expect("write CRLF attributes");
    git(&project.repo, &["add", ".gitattributes"]);
    git(&project.repo, &["add", "--renormalize", "."]);
    git(&project.repo, &["commit", "-m", "configure CRLF contracts"]);
    for file in ["stackstead.yaml", "docker-compose.yml"] {
        let path = project.repo.join(file);
        fs::remove_file(&path).expect("remove LF contract fixture");
        git(&project.repo, &["checkout", "--", file]);
        assert!(
            fs::read(&path)
                .expect("read filtered contract")
                .windows(2)
                .any(|bytes| bytes == b"\r\n"),
            "Git did not apply the CRLF checkout filter to {file}"
        );
    }
    let status = git(
        &project.repo,
        &[
            "status",
            "--short",
            "--",
            "stackstead.yaml",
            "docker-compose.yml",
        ],
    );
    assert!(
        status.trim().is_empty(),
        "Git did not consider the filtered contract clean: {status:?}"
    );
    let manifest = project.create("feature-a");
    assert!(manifest.worktree.is_dir());
}

#[test]
fn create_pins_the_configured_base_when_called_from_another_branch() {
    let project = Project::initialized();
    git(&project.repo, &["switch", "-c", "caller-branch"]);
    fs::write(project.repo.join("README.md"), "caller-only change\n")
        .expect("write caller branch change");
    git(&project.repo, &["add", "README.md"]);
    git(&project.repo, &["commit", "-m", "caller-only change"]);
    let main = git(&project.repo, &["rev-parse", "main"]);
    let caller = git(&project.repo, &["rev-parse", "caller-branch"]);

    let manifest = project.create("feature-a");
    assert_eq!(manifest.base, main.trim());
    assert_ne!(manifest.base, caller.trim());
    assert_eq!(
        fs::read_to_string(manifest.worktree.join("README.md")).expect("read created README"),
        "# Demo project\n"
    );
}

#[cfg(unix)]
#[test]
fn recreating_an_existing_branch_rejects_a_base_it_does_not_contain() {
    let project = Project::initialized();
    let first = project.create("feature-a");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "base-fake-docker-bin",
        "#!/bin/sh\nexit 0\n",
    );
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", &first.stackstead_id, "--yes"])
        .assert()
        .success();

    fs::write(project.repo.join("README.md"), "# Advanced base\n").expect("advance base file");
    git(&project.repo, &["add", "README.md"]);
    git(&project.repo, &["commit", "-m", "advance configured base"]);
    let assert = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr).contains("does not contain pinned source.base")
    );
    assert!(state_stackstead_directories(&project).is_empty());
}

#[test]
fn normalized_compose_paths_survive_create_and_resolution() {
    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["runtime"]["files"] = serde_yaml::Value::Sequence(vec!["./docker-compose.yml".into()]);
    project.write_config(&config, "use an explicitly relative Compose path");

    let manifest = project.create("feature-a");
    assert_eq!(
        manifest.compose_files,
        [manifest.worktree.join("docker-compose.yml")]
    );
    stackstead(&project.repo)
        .args(["context", "feature-a", "--json"])
        .assert()
        .success();
}

#[test]
fn host_wide_port_leases_keep_stopped_projects_on_disjoint_ports() {
    let registry = tempfile::tempdir().unwrap();
    let first_project = Project::initialized();
    let second_project = Project::initialized();
    let mut second_config = load_config(&second_project.repo.join("stackstead.yaml"));
    second_config["project"]["name"] = "second-project".into();
    second_project.write_config(&second_config, "use a distinct project identity");
    let create = |project: &Project, name: &str| {
        let created = stackstead(&project.repo)
            .env("XDG_STATE_HOME", registry.path())
            .args(["--json", "create", name])
            .assert()
            .success();
        changed_manifest(&created.get_output().stdout, "created")
    };

    let first = create(&first_project, "first");
    let second = create(&second_project, "second");
    let first_ports = first.ports.values().copied().collect::<BTreeSet<_>>();
    let second_ports = second.ports.values().copied().collect::<BTreeSet<_>>();
    assert!(first_ports.is_disjoint(&second_ports));
    assert_eq!(first_ports.len(), 2);
    assert_eq!(second_ports.len(), 2);
    assert_eq!(
        first_ports.iter().next_back().unwrap() - first_ports.iter().next().unwrap(),
        1
    );
    assert_eq!(
        second_ports.iter().next_back().unwrap() - second_ports.iter().next().unwrap(),
        1
    );

    #[cfg(unix)]
    {
        let path = fake_docker_path(
            first_project.repo.parent().unwrap(),
            "lease-release-fake-bin",
            "#!/bin/sh\ncase \"$1 $2\" in 'container ls'|'network ls'|'volume ls') exit 0;; esac\nexit 97\n",
        );
        stackstead(&first_project.repo)
            .env("XDG_STATE_HOME", registry.path())
            .env("PATH", path)
            .args(["destroy", &first.stackstead_id, "--yes"])
            .assert()
            .success();
        let registry: Value = serde_json::from_slice(
            &fs::read(registry.path().join("stackstead/port-leases.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            registry["leases"]
                .as_array()
                .unwrap()
                .iter()
                .map(|lease| lease["port"].as_u64().unwrap() as u16)
                .collect::<BTreeSet<_>>(),
            second_ports
        );
    }
}

#[cfg(unix)]
#[test]
fn lifecycle_commands_reject_a_port_lease_that_no_longer_belongs_to_the_manifest() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let git_common_dir = PathBuf::from(
        git(
            &project.repo,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .trim(),
    );
    let registry_path = git_common_dir.join("stackstead-test-state/stackstead/port-leases.json");
    let mut registry: Value =
        serde_json::from_slice(&fs::read(&registry_path).expect("read port lease registry"))
            .expect("parse port lease registry");
    for lease in registry["leases"].as_array_mut().unwrap() {
        lease["owner"] = "ffffffffffffffffffffffffffffffff".into();
    }
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).unwrap(),
    )
    .expect("replace port lease owner");

    let marker = project.repo.parent().unwrap().join("lease-docker-ran");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "lease-mismatch-fake-bin",
        &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
    );
    for args in [
        vec!["up", &manifest.stackstead_id],
        vec!["stop", &manifest.stackstead_id],
        vec!["db", "status", &manifest.stackstead_id],
        vec!["run", &manifest.stackstead_id, "--", "true"],
        vec!["repair", &manifest.stackstead_id],
        vec!["destroy", &manifest.stackstead_id, "--yes"],
    ] {
        let rejected = stackstead(&project.repo)
            .env("PATH", &path)
            .args(args)
            .assert()
            .failure();
        assert!(
            output_text(&rejected.get_output().stderr).contains("port leases for owner"),
            "unexpected error: {}",
            output_text(&rejected.get_output().stderr)
        );
    }
    assert!(
        !marker.exists(),
        "Docker ran before lease ownership validation"
    );
    assert!(manifest.manifest_path().is_file());
    assert!(manifest.worktree.is_dir());
}

#[cfg(target_os = "linux")]
#[test]
fn open_refuses_a_stopped_runtime_before_invoking_the_browser() {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let marker = project.repo.parent().unwrap().join("browser-opened");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "stale-open-fake-bin",
        "#!/bin/sh\ncase \"$1 $2\" in 'container ls'|'network ls'|'volume ls') exit 0;; esac\nexit 97\n",
    );
    let fake_bin = std::env::split_paths(&path).next().unwrap();
    let opener = fake_bin.join("xdg-open");
    fs::write(
        &opener,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&opener, fs::Permissions::from_mode(0o755)).unwrap();

    let rejected = stackstead(&project.repo)
        .env("PATH", path)
        .args(["open", &manifest.stackstead_id, "web"])
        .assert()
        .failure();
    assert!(
        output_text(&rejected.get_output().stderr).contains("has no Stackstead ownership claim")
    );
    assert!(
        !marker.exists(),
        "browser launched for an unrelated listener"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn open_launches_only_after_owned_service_publication_is_proven() {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let marker = project.repo.parent().unwrap().join("owned-browser-url");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "owned-open-fake-bin",
        r#"#!/bin/sh
case "$1 $2" in
  "container ls"|"network ls") exit 0 ;;
  "volume ls") printf '%s\n' "$COMPOSE_PROJECT_NAME-stackstead-claim"; exit 0 ;;
  "volume inspect") printf '{"io.stackstead.runtime-token":"%s"}\n' "$EXPECTED_TOKEN"; exit 0 ;;
esac
for argument in "$@"; do
  case "$argument" in
    ps) printf 'owned-container\n'; exit 0 ;;
    port) printf '127.0.0.1:%s\n' "$EXPECTED_PORT"; exit 0 ;;
  esac
done
exit 97
"#,
    );
    let fake_bin = std::env::split_paths(&path).next().unwrap();
    let opener = fake_bin.join("xdg-open");
    fs::write(
        &opener,
        format!(
            "#!/bin/sh\n: > '{}'\nsleep 0.05\nprintf '%s' \"$1\" > '{}'\n",
            marker.display(),
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&opener, fs::Permissions::from_mode(0o755)).unwrap();

    stackstead(&project.repo)
        .env("PATH", path)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .env("EXPECTED_PORT", manifest.ports["web"].to_string())
        .args(["open", &manifest.stackstead_id, "web"])
        .assert()
        .success();
    let mut opened_url = String::new();
    for _ in 0..100 {
        match fs::read_to_string(&marker) {
            Ok(url) => opened_url = url,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to read browser URL marker: {error}"),
        }
        if opened_url == manifest.urls["web"] {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(opened_url, manifest.urls["web"]);
}

#[test]
fn compose_discovery_generates_config_and_rewrites_only_after_confirmation() {
    let project = Project::git_repo();
    let compose = project.repo.join("docker-compose.yml");
    fs::write(
        &compose,
        r#"services:
  web:
    image: nginx:alpine
    ports:
      - "3000:80"
  postgres:
    image: postgres:16-alpine
    ports:
      - "5432:5432"
"#,
    )
    .expect("write fixed-port Compose fixture");
    git(&project.repo, &["add", "docker-compose.yml"]);
    git(&project.repo, &["commit", "-m", "use fixed fixture ports"]);

    stackstead(&project.repo).arg("init").assert().success();
    let config = load_config(&project.repo.join("stackstead.yaml"));
    assert_eq!(
        config["resources"]["ports"]["expose"]["web"]["container"],
        80
    );
    assert_eq!(
        config["resources"]["ports"]["expose"]["postgres"]["container"],
        5432
    );
    assert_eq!(config["health"]["checks"].as_sequence().unwrap().len(), 1);
    assert_eq!(config["health"]["checks"][0]["name"], "web");

    let plan = stackstead(&project.repo)
        .args(["--json", "compose", "plan"])
        .assert()
        .success();
    let plan: Value =
        serde_json::from_slice(&plan.get_output().stdout).expect("parse Compose plan");
    assert_eq!(plan["kind"], "ComposePlan");
    assert_eq!(plan["version"], "1");
    assert_eq!(plan["file"], "docker-compose.yml");
    let ports = plan["ports"].as_array().expect("Compose plan ports");
    assert_eq!(
        ports
            .iter()
            .find(|port| port["name"] == "web")
            .expect("web plan")["current_host_port"],
        3000
    );
    assert_eq!(
        ports
            .iter()
            .find(|port| port["name"] == "postgres")
            .expect("Postgres plan")["current_host_port"],
        5432
    );

    let original = fs::read(&compose).expect("read original Compose fixture");
    stackstead(&project.repo)
        .args(["compose", "apply"])
        .assert()
        .failure();
    assert_eq!(
        fs::read(&compose).expect("reread Compose fixture"),
        original
    );

    stackstead(&project.repo)
        .args(["compose", "apply", "--yes"])
        .assert()
        .success();
    let rewritten = fs::read_to_string(&compose).expect("read rewritten Compose fixture");
    assert!(rewritten.contains("127.0.0.1:${WEB_PORT}:80"));
    assert!(rewritten.contains("127.0.0.1:${POSTGRES_PORT}:5432"));
}

#[test]
fn explicit_nested_compose_file_drives_init_plan_and_apply() {
    let project = Project::git_repo();
    git(&project.repo, &["rm", "docker-compose.yml"]);
    let nested = project.repo.join("infra/docker/compose.yml");
    fs::create_dir_all(nested.parent().unwrap()).expect("create nested Compose directory");
    fs::write(
        &nested,
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"3000:80\"\n",
    )
    .expect("write nested Compose file");
    git(&project.repo, &["add", "infra/docker/compose.yml"]);
    git(&project.repo, &["commit", "-m", "add nested Compose file"]);

    let missing = stackstead(&project.repo).arg("init").assert().failure();
    let error = output_text(&missing.get_output().stderr);
    assert!(error.contains("--compose-file"));
    assert!(error.contains("infra/docker/compose.yml"));

    stackstead(&project.repo)
        .args(["init", "--compose-file", "infra/docker/compose.yml"])
        .assert()
        .success();
    let config = load_config(&project.repo.join("stackstead.yaml"));
    assert_eq!(
        config["runtime"]["files"],
        serde_yaml::Value::Sequence(vec!["infra/docker/compose.yml".into()])
    );

    let plan = stackstead(&project.repo)
        .args(["compose", "plan", "--json"])
        .assert()
        .success();
    let plan: Value = serde_json::from_slice(&plan.get_output().stdout).expect("parse plan");
    assert_eq!(plan["file"], "infra/docker/compose.yml");

    let second = project.repo.join("infra/docker/admin-compose.yml");
    fs::write(
        &second,
        "services:\n  admin:\n    image: nginx:alpine\n    ports:\n      - \"4000:81\"\n",
    )
    .expect("write second Compose file");
    let second_before = fs::read(&second).unwrap();
    let mut config = config;
    config["runtime"]["files"]
        .as_sequence_mut()
        .unwrap()
        .push("infra/docker/admin-compose.yml".into());
    fs::write(
        project.repo.join("stackstead.yaml"),
        serde_yaml::to_string(&config).unwrap(),
    )
    .unwrap();

    stackstead(&project.repo)
        .args(["compose", "plan"])
        .assert()
        .failure();
    let explicit = stackstead(&project.repo)
        .args([
            "compose",
            "plan",
            "--compose-file",
            "infra/docker/compose.yml",
            "--json",
        ])
        .assert()
        .success();
    let explicit: Value = serde_json::from_slice(&explicit.get_output().stdout).unwrap();
    assert_eq!(explicit["file"], "infra/docker/compose.yml");

    stackstead(&project.repo)
        .args([
            "compose",
            "apply",
            "--compose-file",
            "infra/docker/compose.yml",
            "--yes",
        ])
        .assert()
        .success();
    assert!(
        fs::read_to_string(&nested)
            .expect("read rewritten nested Compose file")
            .contains("127.0.0.1:${WEB_PORT}:80")
    );
    assert_eq!(fs::read(&second).unwrap(), second_before);
}

#[test]
fn multi_file_config_keeps_the_conventional_root_plan_fallback() {
    let project = Project::initialized();
    fs::write(
        project.repo.join("compose.override.yml"),
        "services:\n  web:\n    environment:\n      TRIAL: yes\n",
    )
    .unwrap();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["runtime"]["files"]
        .as_sequence_mut()
        .unwrap()
        .push("compose.override.yml".into());
    fs::write(
        project.repo.join("stackstead.yaml"),
        serde_yaml::to_string(&config).unwrap(),
    )
    .unwrap();

    let plan = stackstead(&project.repo)
        .args(["compose", "plan", "--json"])
        .assert()
        .success();
    let plan: Value = serde_json::from_slice(&plan.get_output().stdout).unwrap();
    assert_eq!(plan["file"], "docker-compose.yml");
}

#[cfg(unix)]
#[test]
fn up_rejects_every_structurally_unsafe_or_disconnected_port_contract() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "port-fake-docker-bin",
        "#!/bin/sh\necho docker-must-not-run >&2\nexit 97\n",
    );

    for (mapping, expected) in [
        ("3000", "unsupported"),
        ("\"80\"", "no deterministic host binding"),
        ("\"3000-3002:80-82\"", "unsupported"),
        ("\"3000:80\"", "fixed host port"),
        (
            "\"127.0.0.1:${APP_PORT}:80\"",
            "env.generate does not define `APP_PORT`",
        ),
    ] {
        fs::write(
            &manifest.compose_files[0],
            format!(
                "services:\n  web:\n    image: nginx\n    ports: [{mapping}]\n  postgres:\n    image: postgres:16\n    ports: [\"127.0.0.1:${{POSTGRES_PORT}}:5432\"]\n"
            ),
        )
        .expect("write unsafe Compose fixture");
        let assert = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["up", &manifest.stackstead_id])
            .assert()
            .failure();
        let stderr = output_text(&assert.get_output().stderr);
        assert!(
            stderr.contains(expected),
            "unexpected error for {mapping}: {stderr}"
        );
        assert!(!stderr.contains("docker-must-not-run"));
    }

    fs::write(
        &manifest.compose_files[0],
        "services:\n  web:\n    ports: [\"127.0.0.1:${WEB_PORT}:80\"]\n  postgres:\n    ports: [\"127.0.0.1:${POSTGRES_PORT}:5432\"]\n",
    )
    .unwrap();
    project.replace_config(
        "    WEB_PORT: '{{ ports.web }}'\n",
        "    WEB_PORT: '39000'\n",
    );
    let literal = stackstead(&project.repo)
        .env("PATH", &path)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    assert!(output_text(&literal.get_output().stderr).contains("ports.<name>"));
    assert!(!output_text(&literal.get_output().stderr).contains("docker-must-not-run"));
}
