use super::*;

#[cfg(unix)]
struct CommandHealthFixture {
    project: Project,
    config: StacksteadConfig,
    manifest: StacksteadManifest,
    fake_state: PathBuf,
    path: OsString,
    docker_script: String,
    docker: PathBuf,
}

#[cfg(unix)]
#[test]
fn command_health_persists_ready_failed_and_stop_reset_states() -> anyhow::Result<()> {
    let mut fixture = command_health_fixture()?;
    assert_ready_command_health(&fixture)?;
    assert_compose_failure_resets_health(&fixture)?;
    assert_stop_and_failed_health(&mut fixture)
}

#[cfg(unix)]
fn command_health_fixture() -> anyhow::Result<CommandHealthFixture> {
    let project = Project::git_repo()?;
    fs::write(
    project.repo.join("docker-compose.yml"),
    "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"127.0.0.1:${WEB_PORT}:80\"\n",
)
.test_context("write web-only Compose fixture")?;
    git(&project.repo, &["add", "docker-compose.yml"])?;
    git(
        &project.repo,
        &["commit", "-m", "use web-only health fixture"],
    )?;
    stackstead(&project.repo).arg("init").assert().success();

    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
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
.test()?;
    config["hooks"]["pre_up"] = serde_yaml::to_value([serde_json::json!({
        "command": "true",
        "shell": false,
    })])
    .test()?;
    config["hooks"]["post_up"] = serde_yaml::to_value([serde_json::json!({
        "command": "true",
        "shell": false,
    })])
    .test()?;
    project.write_config(&config, "configure command health fixture")?;
    let manifest = project.create("feature-a")?;

    let fake_state = project.repo.parent().test()?.join("health-docker-state");
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
        project.repo.parent().test()?,
        "health-fake-docker-bin",
        &docker_script,
    )?;
    let docker = std::env::split_paths(&path).next().test()?.join("docker");
    Ok(CommandHealthFixture {
        project,
        config,
        manifest,
        fake_state,
        path,
        docker_script,
        docker,
    })
}

#[cfg(unix)]
fn assert_ready_command_health(fixture: &CommandHealthFixture) -> anyhow::Result<()> {
    let CommandHealthFixture {
        project,
        manifest,
        fake_state,
        path,
        ..
    } = fixture;
    let ready = stackstead(&project.repo)
        .env("PATH", path)
        .env("WEB_PORT", "9")
        .env("STACKSTEAD_ID", "spoofed")
        .env("COMPOSE_PROJECT_NAME", "shared")
        .env("FAKE_STATE", fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["--json", "up", "feature-a"])
        .assert()
        .success();
    assert!(
        !output_text(&ready.get_output().stdout)?.contains("Timings"),
        "ready command health broke its contract"
    );
    let ready = changed_manifest(&ready.get_output().stdout, "started")?;
    assert_eq!(
        ready.status.health,
        ComponentStatus::Ready,
        "ready command health broke its contract"
    );
    let human = stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", "feature-a"])
        .assert()
        .success();
    let human = output_text(&human.get_output().stdout)?;
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
        .env("PATH", path)
        .env("FAKE_STATE", fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["--json", "inspect", "feature-a"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout)
        .test_context("parse inspect output")?;
    assert!(
        inspected["live"]["health"].is_null(),
        "ready command health broke its contract"
    );
    assert_eq!(
        inspected["stackstead"]["status"]["health"], "ready",
        "ready command health broke its contract"
    );
    Ok(())
}

#[cfg(unix)]
fn assert_compose_failure_resets_health(fixture: &CommandHealthFixture) -> anyhow::Result<()> {
    let CommandHealthFixture {
        project,
        manifest,
        fake_state,
        path,
        docker_script,
        docker,
        ..
    } = fixture;
    fs::write(docker, "#!/bin/sh\nexit 19\n").test_context("make Compose fail")?;
    stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", "feature-a"])
        .assert()
        .failure();
    let compose_failed = StacksteadManifest::read(&manifest.manifest_path())
        .test_context("read Compose failure state")?;
    assert_eq!(
        compose_failed.status.health,
        ComponentStatus::Unknown,
        "Compose failure did not reset health"
    );
    fs::write(docker, docker_script).test_context("restore fake Docker")?;
    Ok(())
}

#[cfg(unix)]
fn assert_stop_and_failed_health(fixture: &mut CommandHealthFixture) -> anyhow::Result<()> {
    let CommandHealthFixture {
        project,
        config,
        manifest,
        fake_state,
        path,
        ..
    } = fixture;
    let stopped = stackstead(&project.repo)
        .env("PATH", &*path)
        .env("FAKE_STATE", &*fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["--json", "stop", "feature-a"])
        .assert()
        .success();
    let stopped = changed_manifest(&stopped.get_output().stdout, "stopped")?;
    assert_eq!(
        stopped.status.health,
        ComponentStatus::Unknown,
        "health lifecycle state broke its contract"
    );

    config["health"]["checks"][0]["command"]["command"] = "false".into();
    project.write_config(config, "make command health fail")?;
    stackstead(&project.repo)
        .env("PATH", &*path)
        .env("FAKE_STATE", &*fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", "feature-a"])
        .assert()
        .failure();
    let failed =
        StacksteadManifest::read(&manifest.manifest_path()).test_context("read failed manifest")?;
    assert_eq!(
        failed.status.health,
        ComponentStatus::Failed,
        "health lifecycle state broke its contract"
    );
    let mut health_error = false;
    for line in fs::read_to_string(&failed.event_log)
        .test_context("read health events")?
        .lines()
    {
        let event: Value = serde_json::from_str(line).test_context("parse health event")?;
        health_error |= event["type"] == "health_wait" && event["status"] == "failed";
    }
    assert!(health_error, "health lifecycle state broke its contract");
    Ok(())
}
