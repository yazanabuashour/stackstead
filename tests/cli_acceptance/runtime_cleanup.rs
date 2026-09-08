use super::*;

#[cfg(unix)]
#[test]
fn destroy_removes_reverified_residual_owned_resources() -> anyhow::Result<()> {
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
    printf '{"io.stackstead.runtime-token":"%s","com.docker.compose.project":"%s"}\n' "$EXPECTED_TOKEN" "$COMPOSE_PROJECT_NAME"
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

    let project = Project::initialized()?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["database"]["postgres"] = serde_yaml::Value::Null;
    config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
    project.write_config(&config, "disable runtime readiness fixture")?;
    let manifest = project.create("feature-a")?;
    let state = project.repo.parent().test()?.join("residual-docker-state");
    let path = fake_docker_path(project.repo.parent().test()?, "residual-bin", DOCKER)?;
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
            .test()?
            .contains("down -v --remove-orphans --rmi local")
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn stop_and_destroy_without_runtime_resources_skip_compose_and_claim_removal() -> anyhow::Result<()>
{
    let project = Project::initialized()?;
    let manifest = project.create("never-started")?;
    let state = project.repo.parent().test()?.join("empty-docker-state");
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "empty-runtime-bin",
        "#!/bin/sh\ncase \"$1 $2\" in 'container ls'|'network ls'|'volume ls') exit 0;; esac\nexit 97\n",
    )?;
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
    Ok(())
}
