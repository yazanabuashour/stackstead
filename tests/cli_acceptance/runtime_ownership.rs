use super::*;

#[cfg(unix)]
#[test]
fn tampered_compose_project_is_rejected_before_destroy_runs_docker() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let mut manifest = project.create("feature-a")?;
    manifest.compose_project = format!(
        "{}_{}_{}",
        manifest.project, manifest.slug, manifest.short_id
    );
    manifest
        .write_fixture()
        .test_context("write tampered Compose identity")?;

    let marker = project.repo.parent().test()?.join("docker-ran");
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "tampered-project-fake-bin",
        &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
    )?;
    let rejected = stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .failure();
    assert!(
        output_text(&rejected.get_output().stderr)?
            .contains("manifest Compose project does not match the durable stackstead identity")
    );
    assert!(manifest.stackstead_root.exists());
    assert!(!marker.exists());
    Ok(())
}

const OWNERSHIP_DOCKER: &str = r#"#!/bin/sh
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

#[cfg(unix)]
#[test]
fn compose_runtime_ownership_rejects_foreign_resources_and_preserves_owned_lifecycle()
-> anyhow::Result<()> {
    for foreign_kind in ["container", "network", "volume", "claim"] {
        assert_foreign_resource_is_rejected(foreign_kind)?;
    }
    assert_owned_runtime_lifecycle()
}

#[cfg(unix)]
fn assert_foreign_resource_is_rejected(foreign_kind: &str) -> anyhow::Result<()> {
    let project = Project::initialized()?;
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
    .test()?;
    git(&project.repo, &["add", "docker-compose.yml"])?;
    git(
        &project.repo,
        &["commit", "-m", "add managed volume fixture"],
    )?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["database"]["postgres"] = serde_yaml::Value::Null;
    config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
    project.write_config(&config, "disable runtime readiness fixture")?;
    let manifest = project.create("feature-a")?;
    let state = project.repo.parent().test()?.join("fake-docker-state");
    fs::create_dir(&state).test()?;
    fs::write(state.join("foreign-resource"), foreign_kind).test()?;
    let foreign_name = match foreign_kind {
        "container" => format!("{}-web-1", manifest.compose_project),
        "network" => format!("{}_default", manifest.compose_project),
        "volume" => format!("{}_cache", manifest.compose_project),
        "claim" => String::new(),
        _ => anyhow::bail!("unexpected ownership fixture kind {foreign_kind}"),
    };
    if foreign_kind == "claim" {
        fs::write(state.join("claim-token"), "foreign-runtime-token").test()?;
    }
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "ownership-bin",
        OWNERSHIP_DOCKER,
    )?;
    let rejected = stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .env("FOREIGN_KIND", foreign_kind)
        .env("FOREIGN_NAME", &foreign_name)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    let error = output_text(&rejected.get_output().stderr)?;
    assert!(
        error.contains("foreign") || error.contains("not owned"),
        "unexpected {foreign_kind} error: {error}"
    );
    assert!(
        !state.join("compose-ran").exists(),
        "foreign runtime ownership was not rejected safely"
    );
    assert_eq!(
        fs::read_to_string(state.join("foreign-resource")).test()?,
        foreign_kind,
        "foreign runtime ownership was not rejected safely"
    );
    if foreign_kind == "container" {
        assert!(
            fs::read_to_string(state.join("commands"))
                .test()?
                .contains("container ls --all --format {{.Names}}"),
            "stopped containers must be included in exact-name ownership checks"
        );
    }
    if foreign_kind == "claim" {
        assert_eq!(
            fs::read_to_string(state.join("claim-token")).test()?,
            "foreign-runtime-token",
            "foreign runtime ownership was not rejected safely"
        );
    }
    Ok(())
}

#[cfg(unix)]
fn assert_owned_runtime_lifecycle() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["database"]["postgres"] = serde_yaml::Value::Null;
    config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
    project.write_config(&config, "disable runtime readiness fixture")?;
    let manifest = project.create("feature-a")?;
    let state = project.repo.parent().test()?.join("owned-docker-state");
    let path = fake_docker_path(project.repo.parent().test()?, "owned-bin", OWNERSHIP_DOCKER)?;
    for _ in 0..2 {
        stackstead(&project.repo)
            .env("PATH", &path)
            .env("FAKE_STATE", &state)
            .env("EXPECTED_TOKEN", &manifest.runtime_token)
            .args(["up", &manifest.stackstead_id])
            .assert()
            .success();
    }
    assert!(
        state.join("claim-token").is_file(),
        "owned runtime lifecycle broke its contract"
    );
    stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(
        !state.join("claim-token").exists(),
        "owned runtime lifecycle broke its contract"
    );
    let commands = fs::read_to_string(state.join("commands")).test()?;
    assert_eq!(
        commands.matches("compose -p").count(),
        2,
        "owned runtime lifecycle broke its contract"
    );
    assert!(
        commands.contains("up -d"),
        "owned runtime lifecycle broke its contract"
    );
    assert!(
        !commands.contains("down -v --remove-orphans"),
        "owned runtime lifecycle broke its contract"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn ownership_checks_exact_and_project_labeled_inventories_without_short_circuiting()
-> anyhow::Result<()> {
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
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let state = project.repo.parent().test()?.join("dual-inventory-state");
    fs::create_dir(&state).test()?;
    let path = fake_docker_path(project.repo.parent().test()?, "dual-inventory-bin", DOCKER)?;
    let rejected = stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    assert!(output_text(&rejected.get_output().stderr)?.contains("foreign"));
    assert!(!state.join("compose-ran").exists());
    Ok(())
}
