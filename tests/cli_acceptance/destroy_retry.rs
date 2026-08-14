use super::*;

const RETRY_DOCKER: &str = r#"#!/bin/sh
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

#[cfg(unix)]
#[test]
fn destroy_retries_the_failed_runtime_phase_once_without_touching_a_peer() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let counter = project.repo.parent().test()?.join("pre-destroy-count");
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["hooks"]["pre_destroy"] = serde_yaml::to_value([serde_json::json!({
        "command": format!("printf x >> '{}'", counter.display()),
        "shell": true,
    })])
    .test()?;
    project.write_config(&config, "count pre-destroy invocations")?;
    let manifest = project.create("feature-a")?;
    let peer = project.create("feature-b")?;
    let state = project.repo.parent().test()?.join("retry-docker-state");
    fs::create_dir(&state).test()?;
    fs::write(state.join("claim"), "").test()?;
    fs::write(state.join("runtime"), "").test()?;
    let path = fake_docker_path(project.repo.parent().test()?, "retry-bin", RETRY_DOCKER)?;

    let first = stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .failure();
    assert!(
        output_text(&first.get_output().stderr)?.contains("injected-down-failure"),
        "test contract condition failed"
    );
    let teardown: Value =
        serde_json::from_slice(&fs::read(manifest.state_dir.join("teardown.json")).test()?)
            .test()?;
    assert_eq!(
        teardown["phase"], "runtime_remove",
        "test contract values differ"
    );
    assert_eq!(
        teardown["stackstead_id"], manifest.stackstead_id,
        "test contract values differ"
    );
    assert_eq!(
        teardown["runtime_token"], manifest.runtime_token,
        "test contract values differ"
    );
    assert_eq!(
        fs::read_to_string(&counter).test()?,
        "x",
        "test contract values differ"
    );
    assert!(manifest.worktree.is_dir(), "test contract condition failed");
    assert!(peer.worktree.is_dir(), "test contract condition failed");

    stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&counter).test()?,
        "x",
        "test contract values differ"
    );
    assert!(
        !manifest.stackstead_root.exists(),
        "test contract condition failed"
    );
    assert!(peer.worktree.is_dir(), "test contract condition failed");
    let commands = fs::read_to_string(state.join("commands")).test()?;
    assert!(
        !commands.contains(&peer.compose_project),
        "test contract condition failed"
    );
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
            .test()?
            .lines()
            .count(),
        command_count,
        "test contract values differ"
    );
    Ok(())
}
