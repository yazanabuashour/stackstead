use super::*;

#[test]
fn startup_pins_profiles_and_rejects_post_hook_model_drift() -> anyhow::Result<()> {
    let fixture = ReadinessFixture::new(serde_json::json!({"worker": "long-running"}))?;
    let profiles = "workers,ops";
    fs::write(fixture.docker.state.join("expected-profiles"), profiles).test()?;
    let started = fixture
        .docker
        .command(&fixture.manifest)
        .env("COMPOSE_PROFILES", profiles)
        .args(["--json", "up", &fixture.manifest.stackstead_id])
        .assert()
        .success();
    let started = changed_manifest(&started.get_output().stdout, "started")?;
    assert_eq!(started.readiness["resolved"]["profiles"], profiles);
    assert_eq!(fixture.readings()?["live"]["readiness"]["status"], "ready");

    // Build is excluded from Compose's native service hash, but not the full model fingerprint.
    let mut drifted = fixture.model.clone();
    drifted["services"]["worker"]["build"] = serde_json::json!({"context": "./changed-fixture"});
    fixture.docker.model(&drifted)?;
    assert_eq!(
        fixture.readings()?["live"]["readiness"]["status"],
        "unknown"
    );
    fixture.docker.model(&fixture.model)?;
    fs::write(
        fixture.docker.state.join("after-model.json"),
        serde_json::to_vec(&drifted).test()?,
    )
    .test()?;
    let failed = fixture
        .docker
        .command(&fixture.manifest)
        .env("COMPOSE_PROFILES", profiles)
        .args(["up", &fixture.manifest.stackstead_id])
        .assert()
        .failure();
    let error = output_text(&failed.get_output().stderr)?;
    assert!(
        error.contains("changed") || error.contains("differ"),
        "{error}"
    );
    let saved = StacksteadManifest::read(&fixture.manifest.manifest_path())?;
    assert!(saved.readiness["resolved"].is_null());
    assert_eq!(
        fixture.readings()?["live"]["readiness"]["status"],
        "unknown"
    );
    fixture.docker.assert_supported()
}

#[test]
fn missing_selected_services_and_invalid_hash_evidence_cannot_resolve_startup() -> anyhow::Result<()>
{
    let fixture = ReadinessFixture::new(serde_json::json!({"worker": "long-running"}))?;
    for case in [
        "inactive service",
        "hash service-set mismatch",
        "corrupt model",
    ] {
        fixture.docker.model(&fixture.model)?;
        fixture.up()?;
        fs::remove_file(fixture.docker.state.join("up-ran")).test()?;
        match case {
            "inactive service" => {
                let mut model = fixture.model.clone();
                model["services"].as_object_mut().test()?.remove("worker");
                fixture.docker.model(&model)?;
            }
            "hash service-set mismatch" => {
                fs::write(
                    fixture.docker.state.join("hashes"),
                    format!("worker {HASH}\n"),
                )
                .test()?;
            }
            "corrupt model" => {
                fs::write(
                    fixture.docker.state.join("model.json"),
                    "invalid-fixture-json",
                )
                .test()?;
            }
            _ => anyhow::bail!("unknown normalization case"),
        }
        fixture
            .docker
            .command(&fixture.manifest)
            .args(["up", &fixture.manifest.stackstead_id])
            .assert()
            .failure();
        let saved = StacksteadManifest::read(&fixture.manifest.manifest_path())?;
        assert!(saved.readiness["resolved"].is_null(), "{case}");
        assert!(!fixture.docker.state.join("up-ran").exists(), "{case}");
        assert_eq!(
            fixture.readings()?["live"]["readiness"]["status"],
            "unknown",
            "{case}"
        );
    }
    fixture.docker.assert_supported()
}
