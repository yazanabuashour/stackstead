use super::*;

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
