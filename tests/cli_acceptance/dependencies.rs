use super::*;

#[test]
fn dependency_failure_is_persisted_without_starting_compose() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    project.replace_config(
        "    command: ''\n    shell: false\n",
        "    command: stackstead-command-that-does-not-exist\n    shell: false\n",
    )?;
    let manifest = project.create("feature-a")?;

    let assert = stackstead(&project.repo)
        .args(["up", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)?
            .contains("stackstead-command-that-does-not-exist"),
        "test contract condition failed"
    );

    let persisted =
        StacksteadManifest::read(&manifest.manifest_path()).test_context("read failed state")?;
    assert_eq!(
        serde_json::to_value(persisted.status.dependencies).test_context("serialize status")?,
        Value::String("failed".into()),
        "test contract values differ"
    );
    let events = event_types(&persisted.event_log)?;
    assert!(
        events.contains(&"dependencies_install".into()),
        "test contract condition failed"
    );
    assert!(
        !events.contains(&"runtime_start".into()),
        "test contract condition failed"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn dependency_and_yarn_logs_redact_structured_and_environment_secrets() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["dependencies"]["provider"] = "yarn-classic".into();
    config["dependencies"]["install"]["command"] = "printf 'Authorization: Bearer dependency-header-marker\\nhttps://user:dependency-url-marker@example.invalid/repo\\n%s\\nordinary dependency output\\n' \"$API_TOKEN\"".into();
    config["dependencies"]["install"]["shell"] = true.into();
    config["dependencies"]["link"] = serde_yaml::to_value(serde_json::json!({
        "enabled": true,
        "link_folder": ".stackstead/yarn-links",
        "command": "printf 'Proxy-Authorization: Basic yarn-header-marker\\nhttps://user:yarn-url-marker@example.invalid/repo\\n%s\\nordinary yarn output\\n' \"$API_TOKEN\"",
        "shell": true,
    }))
    .test()?;
    config["env"]["generate"]["API_TOKEN"] = "known-environment-marker".into();
    project.write_config(&config, "configure secret-emitting dependency fixtures")?;
    let manifest = project.create("feature-a")?;
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "redaction-fake-docker-bin",
        "#!/bin/sh\nexit 19\n",
    )?;

    stackstead(&project.repo)
        .env("PATH", path)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    for (name, ordinary) in [
        ("dependencies.log", "ordinary dependency output"),
        ("yarn-link.log", "ordinary yarn output"),
    ] {
        let log = fs::read_to_string(manifest.state_dir.join("logs").join(name)).test()?;
        assert!(log.contains(ordinary), "test contract condition failed");
        assert!(log.contains("[REDACTED]"), "test contract condition failed");
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
    Ok(())
}

#[test]
fn failed_dependency_diagnostics_and_events_share_structured_redaction() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    project.replace_config(
        "    command: ''\n    shell: false\n",
        "    command: \"printf 'Authorization: Bearer event-header-marker\\\\n' >&2; exit 7\"\n    shell: true\n",
    )?;
    let manifest = project.create("feature-a")?;

    let rejected = stackstead(&project.repo)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    assert!(
        !output_text(&rejected.get_output().stderr)?.contains("event-header-marker"),
        "test contract condition failed"
    );
    let events = fs::read_to_string(&manifest.event_log).test()?;
    assert!(
        events.contains("[REDACTED]"),
        "test contract condition failed"
    );
    assert!(
        !events.contains("event-header-marker"),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn pre_up_failure_preserves_completed_dependency_status() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    project.replace_config(
        "  pre_up: []\n",
        "  pre_up:\n  - command: stackstead-pre-up-command-that-does-not-exist\n    shell: false\n",
    )?;
    let mut manifest = project.create("feature-a")?;
    manifest.status.database = ComponentStatus::Reachable;
    manifest.status.health = ComponentStatus::Ready;
    manifest.save_atomic().test()?;

    stackstead(&project.repo)
        .args(["up", "feature-a", "--json"])
        .assert()
        .failure();

    let persisted =
        StacksteadManifest::read(&manifest.manifest_path()).test_context("read failed state")?;
    assert_eq!(
        persisted.status.dependencies,
        ComponentStatus::Ready,
        "test contract values differ"
    );
    assert_eq!(
        persisted.status.database,
        ComponentStatus::Unknown,
        "test contract values differ"
    );
    assert_eq!(
        persisted.status.health,
        ComponentStatus::Unknown,
        "test contract values differ"
    );
    assert!(
        !event_types(&persisted.event_log)?.contains(&"runtime_start".into()),
        "test contract condition failed"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn up_revalidates_contract_mutations_after_pre_and_post_hooks() -> anyhow::Result<()> {
    for post_up in [false, true] {
        let project = Project::initialized()?;
        let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
        config["database"]["postgres"] = serde_yaml::Value::Null;
        config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
        let mutation = serde_json::json!({
            "command": "printf 'services:\\n  web:\\n    ports: [\"80\"]\\n' > docker-compose.yml",
            "shell": true,
        });
        if post_up {
            config["hooks"]["post_up"] = serde_yaml::to_value([mutation]).test()?;
        } else {
            config["hooks"]["pre_up"] = serde_yaml::to_value([mutation]).test()?;
        }
        project.write_config(&config, "configure contract mutation hook")?;
        let manifest = project.create("feature-a")?;
        let marker = project.repo.parent().test()?.join(if post_up {
            "post-up-docker-ran"
        } else {
            "pre-up-docker-ran"
        });
        let fake_state = project
            .repo
            .parent()
            .test()?
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
            project.repo.parent().test()?,
            if post_up {
                "post-up-contract-fake-bin"
            } else {
                "pre-up-contract-fake-bin"
            },
            &docker_script,
        )?;
        let rejected = stackstead(&project.repo)
            .env("PATH", path)
            .env("FAKE_STATE", fake_state)
            .env("EXPECTED_TOKEN", &manifest.runtime_token)
            .args(["up", &manifest.stackstead_id])
            .assert()
            .failure();
        assert!(
            output_text(&rejected.get_output().stderr)?.contains("deterministic host binding"),
            "test contract condition failed"
        );
        assert_eq!(marker.exists(), post_up, "Docker stage ordering changed");
    }
    Ok(())
}
