use super::*;

#[test]
fn up_resolves_declared_expectations_and_keeps_empty_application_health_unknown()
-> anyhow::Result<()> {
    let fixture =
        ReadinessFixture::new(serde_json::json!({"worker": "long-running", "migrate": "job"}))?;
    assert_eq!(fixture.manifest.version, "3");
    assert!(fixture.manifest.readiness["resolved"].is_null());
    assert_eq!(
        fixture.readings()?["live"]["readiness"]["status"],
        "unknown"
    );
    let started = fixture.up()?;
    assert_eq!(started.status.health, ComponentStatus::Unknown);
    let resolved = &started.readiness["resolved"];
    assert!(resolved["profiles"].is_null());
    assert_eq!(resolved["services"]["worker"]["replicas"], 2);
    assert_eq!(resolved["services"]["migrate"]["replicas"], 1);
    assert_eq!(resolved["services"].as_object().test()?.len(), 2);
    assert_eq!(resolved["services"]["worker"]["config_hash"], HASH);
    assert!(
        resolved["model_hash"]
            .as_str()
            .is_some_and(|hash| hash.len() == 64)
    );
    let inspected = fixture.readings()?;
    assert_eq!(inspected["live"]["readiness"]["status"], "ready");
    assert_eq!(inspected["effective"]["health"]["status"], "unknown");
    assert_startup_refuses_unsatisfied_requirements(&fixture)
}

fn assert_startup_refuses_unsatisfied_requirements(
    fixture: &ReadinessFixture,
) -> anyhow::Result<()> {
    let mut rows = fixture.rows.clone();
    rows.remove(0);
    fixture.docker.rows(&rows)?;
    let failed = fixture
        .docker
        .command(&fixture.manifest)
        .args(["up", &fixture.manifest.stackstead_id])
        .assert()
        .failure();
    assert!(
        output_text(&failed.get_output().stderr)?.contains("readiness"),
        "{failed:?}"
    );
    let saved = StacksteadManifest::read(&fixture.manifest.manifest_path())?;
    assert_eq!(saved.status.health, ComponentStatus::Unknown);
    assert!(saved.readiness["resolved"].is_null());
    assert_eq!(
        fixture.readings()?["live"]["readiness"]["status"],
        "unknown"
    );
    let mut config = load_config(&fixture.project.repo.join("stackstead.yaml"))?;
    // Reject model verification after a successful application check. Waiting for
    // the deadline could instead leave a legitimately timed-out final probe.
    config["health"]["checks"] = serde_yaml::to_value([serde_json::json!({
        "name": "application",
        "command": {"command": "touch \"$FAKE_STATE/fail-config\"", "shell": true},
    })])
    .test()?;
    fixture.project.write_config(
        &config,
        "separate application health from runtime verification failure",
    )?;
    let rejected = fixture
        .docker
        .command(&fixture.manifest)
        .args(["up", &fixture.manifest.stackstead_id])
        .assert()
        .failure();
    assert!(
        output_text(&rejected.get_output().stderr)?
            .contains("cannot verify effective Compose inputs after startup"),
        "{rejected:?}"
    );
    let failed = StacksteadManifest::read(&fixture.manifest.manifest_path())?;
    assert_eq!(failed.status.health, ComponentStatus::Ready);
    assert!(failed.readiness["resolved"].is_null());
    fixture.docker.assert_supported()
}

#[test]
fn up_invalidates_previous_expectations_before_hooks_normalization_and_compose()
-> anyhow::Result<()> {
    let fixture = ReadinessFixture::new(serde_json::json!({"worker": "long-running"}))?;
    for failure in ["fail-pre", "fail-config", "fail-up"] {
        assert!(fixture.up()?.readiness["resolved"].is_object());
        fs::write(fixture.docker.state.join(failure), "").test()?;
        fixture
            .docker
            .command(&fixture.manifest)
            .args(["up", &fixture.manifest.stackstead_id])
            .assert()
            .failure();
        let saved = StacksteadManifest::read(&fixture.manifest.manifest_path())?;
        assert!(
            saved.readiness["resolved"].is_null(),
            "{failure}: {:?}",
            saved.readiness
        );
        fs::remove_file(fixture.docker.state.join(failure)).test()?;
        assert_eq!(
            fixture.readings()?["live"]["readiness"]["status"],
            "unknown",
            "{failure}"
        );
    }
    fixture.docker.assert_supported()
}

#[test]
fn exited_zero_requires_declared_job_intent_and_does_not_imply_runtime_activity()
-> anyhow::Result<()> {
    for (required, expected) in [
        (Value::Null, "unconfigured"),
        (serde_json::json!({"migrate": "job"}), "ready"),
    ] {
        let fixture = ReadinessFixture::new(required)?;
        fixture
            .docker
            .rows(&[row(&fixture.manifest, "migrate", 1, 4, "exited", 0)])?;
        let started = fixture.up()?;
        assert_eq!(started.status.health, ComponentStatus::Unknown);
        let inspected = fixture.readings()?;
        assert_eq!(inspected["live"]["readiness"]["status"], expected);
        assert_eq!(inspected["live"]["runtime"]["activity"], "inactive");
        assert_eq!(inspected["live"]["runtime"]["running"], false);
        assert_eq!(inspected["live"]["services"][0]["status"], "exited (0)");
    }
    Ok(())
}
