use super::*;

#[test]
fn tampered_manifest_destroy_fails_before_external_mutation() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let mut tampered = manifest.clone();
    tampered.repo_root = project.repo.join("different-repository");
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&tampered).expect("serialize tampered manifest"),
    )
    .expect("write tampered manifest");

    let assert = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--yes", "--json"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)
            .contains("project identity does not match the discovered project")
    );
    assert!(assert.get_output().stdout.is_empty());
    assert!(manifest.stackstead_root.is_dir());
    assert!(manifest.worktree.is_dir());
    assert!(!event_types(&manifest.event_log).contains(&"destroyed".into()));
}

#[cfg(unix)]
#[test]
fn repair_rejects_a_generated_directory_symlink_escape() {
    use std::os::unix::fs::symlink;

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let generated = manifest.worktree.join(".stackstead");
    fs::remove_dir_all(&generated).expect("remove generated contract directory");
    let outside = project
        .repo
        .parent()
        .expect("repository has parent")
        .join("escape-target");
    fs::create_dir(&outside).expect("create escape target");
    symlink(&outside, &generated).expect("create generated-directory symlink");

    let assert = stackstead(&project.repo)
        .args(["repair", "feature-a", "--json"])
        .assert()
        .failure();
    let stderr = output_text(&assert.get_output().stderr);
    assert!(
        stderr.contains("symlink") || stderr.contains("escapes") || stderr.contains("unsafe"),
        "unexpected symlink error: {stderr}"
    );
    assert!(!outside.join(".env").exists());
    assert!(!outside.join("AGENT_CONTEXT.md").exists());
    assert!(!outside.join("stackstead.json").exists());
    assert!(manifest.stackstead_root.is_dir());
}

#[test]
fn destroy_refuses_a_dirty_worktree_before_touching_runtime_state() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    fs::write(manifest.worktree.join("README.md"), "dirty\n").expect("dirty tracked file");

    let assert = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--yes", "--json"])
        .assert()
        .failure();
    assert!(
        String::from_utf8_lossy(&assert.get_output().stderr)
            .contains("uncommitted or untracked changes")
    );

    assert!(manifest.manifest_path().is_file());
    assert!(manifest.worktree.is_dir());
}

#[cfg(unix)]
#[test]
fn destroy_uses_the_durable_manifest_after_non_destructive_config_path_changes() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["env"]["file"] = ".stackstead-next/.env".into();
    config["agent"]["context_file"] = ".stackstead-next/AGENT_CONTEXT.md".into();
    project.write_config(&config, "change future generated paths");

    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "cleanup-fake-docker-bin",
        "#!/bin/sh\nexit 0\n",
    );
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", "feature-a", "--yes"])
        .assert()
        .success();

    assert!(!manifest.stackstead_root.exists());
    assert!(!manifest.worktree.exists());
}

#[test]
fn repair_rejects_changed_generated_paths_without_writing_them() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["env"]["file"] = ".stackstead-next/.env".into();
    config["agent"]["context_file"] = ".stackstead-next/AGENT_CONTEXT.md".into();
    project.write_config(&config, "change future repair paths");

    stackstead(&project.repo)
        .args(["repair", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(!manifest.worktree.join(".stackstead-next").exists());
    assert!(manifest.env_file.is_file());
    assert!(manifest.agent_context.is_file());
}

#[cfg(unix)]
#[test]
fn custom_compose_project_contract_is_rejected() {
    let project = Project::initialized();
    let mut manifest = project.create("feature-a");
    manifest.compose_project = format!(
        "{}_{}_{}",
        manifest.project, manifest.slug, manifest.short_id
    );
    manifest.save_atomic().expect("write legacy manifest");
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["runtime"]["project_name_template"] =
        "{{ project.name }}_{{ stackstead.slug }}_{{ stackstead.short_id }}".into();
    project.write_config(&config, "preserve legacy Compose identity");

    let marker = project.repo.parent().unwrap().join("docker-ran");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "legacy-project-fake-bin",
        &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
    );
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", "feature-a", "--yes"])
        .assert()
        .failure();
    assert!(manifest.stackstead_root.exists());
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
fn destroy_retries_the_failed_runtime_phase_once_without_touching_a_peer() {
    const DOCKER: &str = r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
printf '%s\n' "$*" >> "$FAKE_STATE/commands"
kind=${1-}
verb=${2-}
last=
for argument in "$@"; do last=$argument; done
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$kind $verb" in
  "container ls")
    case " $* " in
      *" name=^/"*) ;;
      *" {{.Names}} "*) test ! -f "$FAKE_STATE/runtime" || printf '%s\n' "$COMPOSE_PROJECT_NAME-web-1" ;;
      *) test ! -f "$FAKE_STATE/runtime" || printf '%s\n' runtime-id ;;
    esac
    ;;
  "container inspect"|"volume inspect")
    printf '{"io.stackstead.runtime-token":"%s"}\n' "$EXPECTED_TOKEN"
    ;;
  "network ls") ;;
  "volume ls") test ! -f "$FAKE_STATE/claim" || printf '%s\n' "$claim" ;;
  "volume rm") rm -f "$FAKE_STATE/claim" ;;
  "image inspect"|"run --rm") ;;
  "compose -p")
    case " $* " in
      *" down -v --remove-orphans --rmi local "*)
        if test ! -f "$FAKE_STATE/failed"; then
          : > "$FAKE_STATE/failed"
          echo injected-down-failure >&2
          exit 42
        fi
        rm -f "$FAKE_STATE/runtime"
        ;;
    esac
    ;;
esac
exit 0
"#;

    let project = Project::initialized();
    let counter = project.repo.parent().unwrap().join("pre-destroy-count");
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["hooks"]["pre_destroy"] = serde_yaml::to_value([serde_json::json!({
        "command": format!("printf x >> '{}'", counter.display()),
        "shell": true,
    })])
    .unwrap();
    project.write_config(&config, "count pre-destroy invocations");
    let manifest = project.create("feature-a");
    let peer = project.create("feature-b");
    let state = project.repo.parent().unwrap().join("retry-docker-state");
    fs::create_dir(&state).unwrap();
    fs::write(state.join("claim"), "").unwrap();
    fs::write(state.join("runtime"), "").unwrap();
    let path = fake_docker_path(project.repo.parent().unwrap(), "retry-bin", DOCKER);

    let first = stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .failure();
    assert!(output_text(&first.get_output().stderr).contains("injected-down-failure"));
    let teardown: Value =
        serde_json::from_slice(&fs::read(manifest.state_dir.join("teardown.json")).unwrap())
            .unwrap();
    assert_eq!(teardown["phase"], "runtime_remove");
    assert_eq!(teardown["stackstead_id"], manifest.stackstead_id);
    assert_eq!(teardown["runtime_token"], manifest.runtime_token);
    assert_eq!(fs::read_to_string(&counter).unwrap(), "x");
    assert!(manifest.worktree.is_dir());
    assert!(peer.worktree.is_dir());

    stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&counter).unwrap(), "x");
    assert!(!manifest.stackstead_root.exists());
    assert!(peer.worktree.is_dir());
    let commands = fs::read_to_string(state.join("commands")).unwrap();
    assert!(!commands.contains(&peer.compose_project));
    let command_count = commands.lines().count();

    stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .failure();
    assert_eq!(
        fs::read_to_string(state.join("commands"))
            .unwrap()
            .lines()
            .count(),
        command_count
    );
}

#[cfg(unix)]
#[test]
fn compose_runtime_ownership_rejects_foreign_resources_and_preserves_owned_lifecycle() {
    const DOCKER: &str = r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
printf '%s\n' "$*" >> "$FAKE_STATE/commands"
kind=${1-}
verb=${2-}
last=
for argument in "$@"; do last=$argument; done
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$kind $verb" in
  "container ls")
    test "${FOREIGN_KIND-}" = container && printf '%s\n' "$FOREIGN_NAME"
    ;;
  "network ls")
    test "${FOREIGN_KIND-}" = network && printf '%s\n' "$FOREIGN_NAME"
    ;;
  "volume ls")
    test -f "$FAKE_STATE/claim-token" && printf '%s\n' "$claim"
    test "${FOREIGN_KIND-}" = volume && printf '%s\n' "$FOREIGN_NAME"
    ;;
  "volume create")
    test -f "$FAKE_STATE/claim-token" || printf '%s' "$EXPECTED_TOKEN" > "$FAKE_STATE/claim-token"
    printf '%s\n' "$claim"
    ;;
  "volume rm")
    test "$last" = "$claim"
    rm "$FAKE_STATE/claim-token"
    ;;
  "container inspect"|"network inspect"|"volume inspect")
    if test "$last" = "$claim"; then
      test -f "$FAKE_STATE/claim-token" || exit 41
      token=$(cat "$FAKE_STATE/claim-token")
    elif test "$last" = "${FOREIGN_NAME-}"; then
      token=foreign-runtime-token
    else
      exit 42
    fi
    printf '{"io.stackstead.runtime-token":"%s"}\n' "$token"
    ;;
  "compose -p")
    touch "$FAKE_STATE/compose-ran"
    ;;
esac
exit 0
"#;

    for foreign_kind in ["container", "network", "volume", "claim"] {
        let project = Project::initialized();
        fs::write(
            project.repo.join("docker-compose.yml"),
            r#"services:
  web:
    image: nginx:alpine
    ports: ["127.0.0.1:${WEB_PORT}:80"]
    volumes: [cache:/cache]
  postgres:
    image: postgres:16-alpine
    ports: ["127.0.0.1:${POSTGRES_PORT}:5432"]
volumes:
  cache: {}
"#,
        )
        .unwrap();
        git(&project.repo, &["add", "docker-compose.yml"]);
        git(
            &project.repo,
            &["commit", "-m", "add managed volume fixture"],
        );
        let mut config = load_config(&project.repo.join("stackstead.yaml"));
        config["database"]["postgres"] = serde_yaml::Value::Null;
        config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
        project.write_config(&config, "disable runtime readiness fixture");
        let manifest = project.create("feature-a");
        let state = project.repo.parent().unwrap().join("fake-docker-state");
        fs::create_dir(&state).unwrap();
        fs::write(state.join("foreign-resource"), foreign_kind).unwrap();
        let foreign_name = match foreign_kind {
            "container" => format!("{}-web-1", manifest.compose_project),
            "network" => format!("{}_default", manifest.compose_project),
            "volume" => format!("{}_cache", manifest.compose_project),
            "claim" => String::new(),
            _ => unreachable!(),
        };
        if foreign_kind == "claim" {
            fs::write(state.join("claim-token"), "foreign-runtime-token").unwrap();
        }
        let path = fake_docker_path(project.repo.parent().unwrap(), "ownership-bin", DOCKER);
        let rejected = stackstead(&project.repo)
            .env("PATH", path)
            .env("FAKE_STATE", &state)
            .env("EXPECTED_TOKEN", &manifest.runtime_token)
            .env("FOREIGN_KIND", foreign_kind)
            .env("FOREIGN_NAME", &foreign_name)
            .args(["up", &manifest.stackstead_id])
            .assert()
            .failure();
        let error = output_text(&rejected.get_output().stderr);
        assert!(
            error.contains("foreign") || error.contains("not owned"),
            "unexpected {foreign_kind} error: {error}"
        );
        assert!(!state.join("compose-ran").exists());
        assert_eq!(
            fs::read_to_string(state.join("foreign-resource")).unwrap(),
            foreign_kind
        );
        if foreign_kind == "container" {
            assert!(
                fs::read_to_string(state.join("commands"))
                    .unwrap()
                    .contains("container ls --all --format {{.Names}}"),
                "stopped containers must be included in exact-name ownership checks"
            );
        }
        if foreign_kind == "claim" {
            assert_eq!(
                fs::read_to_string(state.join("claim-token")).unwrap(),
                "foreign-runtime-token"
            );
        }
    }

    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["database"]["postgres"] = serde_yaml::Value::Null;
    config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
    project.write_config(&config, "disable runtime readiness fixture");
    let manifest = project.create("feature-a");
    let state = project.repo.parent().unwrap().join("owned-docker-state");
    let path = fake_docker_path(project.repo.parent().unwrap(), "owned-bin", DOCKER);
    for _ in 0..2 {
        stackstead(&project.repo)
            .env("PATH", &path)
            .env("FAKE_STATE", &state)
            .env("EXPECTED_TOKEN", &manifest.runtime_token)
            .args(["up", &manifest.stackstead_id])
            .assert()
            .success();
    }
    assert!(state.join("claim-token").is_file());
    stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(!state.join("claim-token").exists());
    let commands = fs::read_to_string(state.join("commands")).unwrap();
    assert_eq!(commands.matches("compose -p").count(), 2);
    assert!(commands.contains("up -d"));
    assert!(!commands.contains("down -v --remove-orphans"));
}

#[cfg(unix)]
#[test]
fn ownership_checks_exact_and_project_labeled_inventories_without_short_circuiting() {
    const DOCKER: &str = r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
printf '%s\n' "$*" >> "$FAKE_STATE/commands"
last=
for argument in "$@"; do last=$argument; done
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "${1-} ${2-}" in
  "container ls")
    case " $* " in *" --filter "*) printf '%s\n' orphan-id;; *) printf '%s\n' "$COMPOSE_PROJECT_NAME-web-1";; esac
    ;;
  "container inspect")
    if test "$last" = orphan-id; then token=foreign; else token="$EXPECTED_TOKEN"; fi
    printf '{"io.stackstead.runtime-token":"%s"}\n' "$token"
    ;;
  "network ls") ;;
  "volume ls") printf '%s\n' "$claim" ;;
  "volume inspect") printf '{"io.stackstead.runtime-token":"%s"}\n' "$EXPECTED_TOKEN" ;;
  "compose -p") touch "$FAKE_STATE/compose-ran" ;;
esac
"#;
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let state = project.repo.parent().unwrap().join("dual-inventory-state");
    fs::create_dir(&state).unwrap();
    let path = fake_docker_path(project.repo.parent().unwrap(), "dual-inventory-bin", DOCKER);
    let rejected = stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    assert!(output_text(&rejected.get_output().stderr).contains("foreign"));
    assert!(!state.join("compose-ran").exists());
}

#[cfg(unix)]
#[test]
fn destroy_removes_reverified_residual_owned_resources() {
    const DOCKER: &str = r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
printf '%s\n' "$*" >> "$FAKE_STATE/commands"
kind=${1-}
verb=${2-}
last=
for argument in "$@"; do last=$argument; done
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$kind $verb" in
  "container ls")
    test -f "$FAKE_STATE/runtime" && printf '%s\n' "$COMPOSE_PROJECT_NAME-web-1"
    ;;
  "container inspect")
    printf '{"io.stackstead.runtime-token":"%s"}\n' "$EXPECTED_TOKEN"
    ;;
  "network ls") ;;
  "volume ls")
    test -f "$FAKE_STATE/claim" && printf '%s\n' "$claim"
    test -f "$FAKE_STATE/residual" && printf '%s\n' "$COMPOSE_PROJECT_NAME-retired"
    ;;
  "volume create")
    : > "$FAKE_STATE/claim"
    printf '%s\n' "$claim"
    ;;
  "volume inspect")
    printf '{"io.stackstead.runtime-token":"%s"}\n' "$EXPECTED_TOKEN"
    ;;
  "volume rm")
    if test "$last" = "$claim"; then rm -f "$FAKE_STATE/claim"; else rm -f "$FAKE_STATE/residual"; fi
    ;;
  "compose -p")
    case " $* " in
      *" up -d "*) : > "$FAKE_STATE/runtime" ;;
      *" down -v --remove-orphans --rmi local "*)
        rm "$FAKE_STATE/runtime"
        : > "$FAKE_STATE/residual"
        ;;
    esac
    ;;
esac
exit 0
"#;

    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["database"]["postgres"] = serde_yaml::Value::Null;
    config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
    project.write_config(&config, "disable runtime readiness fixture");
    let manifest = project.create("feature-a");
    let state = project.repo.parent().unwrap().join("residual-docker-state");
    let path = fake_docker_path(project.repo.parent().unwrap(), "residual-bin", DOCKER);
    stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .success();

    stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(!manifest.stackstead_root.exists());
    assert!(!state.join("claim").exists());
    assert!(
        fs::read_to_string(state.join("commands"))
            .unwrap()
            .contains("down -v --remove-orphans --rmi local")
    );
}

#[cfg(unix)]
#[test]
fn stop_and_destroy_without_runtime_resources_skip_compose_and_claim_removal() {
    let project = Project::initialized();
    let manifest = project.create("never-started");
    let state = project.repo.parent().unwrap().join("empty-docker-state");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "empty-runtime-bin",
        "#!/bin/sh\ncase \"$1 $2\" in 'container ls'|'network ls'|'volume ls') exit 0;; esac\nexit 97\n",
    );
    stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &state)
        .args(["stop", &manifest.stackstead_id])
        .assert()
        .success();
    stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", state)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(!manifest.stackstead_root.exists());
}
