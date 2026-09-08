use super::*;

#[test]
fn missing_and_corrupt_durable_readiness_fail_before_docker() -> anyhow::Result<()> {
    let fixture = ReadinessFixture::new(serde_json::json!({"worker": "long-running"}))?;
    let started = fixture.up()?;
    let original: Value =
        serde_json::from_slice(&fs::read(started.manifest_path()).test()?).test()?;
    fs::remove_file(fixture.docker.state.join("commands")).test()?;
    for case in ["missing", "unknown role", "bad hash", "wrong service set"] {
        let mut corrupt = original.clone();
        match case {
            "missing" => {
                corrupt.as_object_mut().test()?.remove("readiness");
            }
            "unknown role" => corrupt["readiness"]["required"]["worker"] = "inferred-job".into(),
            "bad hash" => corrupt["readiness"]["resolved"]["model_hash"] = "not-a-hash".into(),
            "wrong service set" => {
                corrupt["readiness"]["resolved"]["services"]
                    .as_object_mut()
                    .test()?
                    .remove("worker");
            }
            _ => anyhow::bail!("unknown durable corruption case"),
        }
        fs::write(
            started.manifest_path(),
            serde_json::to_vec(&corrupt).test()?,
        )
        .test()?;
        fixture
            .docker
            .command(&fixture.manifest)
            .args(["--json", "inspect", &fixture.manifest.stackstead_id])
            .assert()
            .failure();
        assert!(!fixture.docker.state.join("commands").exists(), "{case}");
    }
    Ok(())
}

#[test]
fn unavailable_or_foreign_snapshots_are_null_not_empty_or_inactive() -> anyhow::Result<()> {
    let fixture = ReadinessFixture::new(serde_json::json!({"worker": "long-running"}))?;
    fixture.up()?;
    assert_eq!(fixture.readings()?["live"]["readiness"]["status"], "ready");
    for case in [
        "foreign token",
        "foreign project",
        "missing project",
        "wrong id",
        "corrupt metadata",
        "disappeared",
        "missing claim",
    ] {
        let mut rows = fixture.rows.clone();
        match case {
            "foreign token" => rows[0]["runtime_token"] = "foreign-token".into(),
            "foreign project" | "missing project" => {
                let mut candidate = row(&fixture.manifest, "worker", 1, 1, "running", 0);
                if case == "missing project" {
                    candidate.as_object_mut().test()?.remove("project");
                } else {
                    candidate["project"] = "foreign-project".into();
                }
                rows.push(candidate);
            }
            _ => {}
        }
        fixture.docker.rows(&rows)?;
        let first = fixture.docker.state.join(rows[0]["id"].as_str().test()?);
        match case {
            "wrong id" => {
                rows[0]["id"] = format!("{:064x}", 99).into();
                fs::write(&first, serde_json::to_vec(&rows[0]).test()?).test()?;
            }
            "corrupt metadata" => {
                fs::write(&first, "{not-metadata").test()?;
            }
            "disappeared" => {
                fs::write(fixture.docker.state.join("disappear"), "").test()?;
            }
            "missing claim" => {
                fs::remove_file(fixture.docker.state.join("claim")).test()?;
            }
            _ => {}
        }
        let inspected = fixture.readings()?;
        assert_eq!(
            inspected["live"]["readiness"]["status"], "unknown",
            "{case}"
        );
        assert_eq!(
            inspected["live"]["runtime"]["activity"], "unknown",
            "{case}"
        );
        assert!(inspected["live"]["runtime"]["running"].is_null(), "{case}");
        assert!(inspected["live"]["services"].is_null(), "{case}");
        if case == "disappeared" {
            fs::remove_file(fixture.docker.state.join("disappear")).test()?;
        }
        fs::write(
            fixture.docker.state.join("claim"),
            &fixture.manifest.runtime_token,
        )
        .test()?;
    }
    fixture.docker.rows(&[])?;
    let empty = fixture.readings()?;
    assert_eq!(empty["live"]["services"], serde_json::json!([]));
    assert_eq!(empty["live"]["runtime"]["activity"], "inactive");
    assert_eq!(empty["live"]["runtime"]["running"], false);
    assert_eq!(empty["live"]["readiness"]["status"], "not_ready");
    Ok(())
}

#[test]
fn ps_runs_neither_application_commands_nor_http_probes() -> anyhow::Result<()> {
    let fixture = ReadinessFixture::new(serde_json::json!({"worker": "long-running"}))?;
    fixture.up()?;
    let listener = TcpListener::bind(("127.0.0.1", 0)).test()?;
    listener.set_nonblocking(true).test()?;
    let mut config = load_config(&fixture.project.repo.join("stackstead.yaml"))?;
    config["health"]["checks"] = serde_yaml::to_value([
        serde_json::json!({"name": "http", "url": format!("http://{}/", listener.local_addr().test()?)}),
        serde_json::json!({"name": "command", "command": {"command": "touch \"$FAKE_STATE/app-probe\"", "shell": true}}),
    ]).test()?;
    fixture
        .project
        .write_config(&config, "add application probes")?;
    let listed = fixture
        .docker
        .command(&fixture.manifest)
        .args(["--json", "ps"])
        .assert()
        .success();
    let listed: Value = serde_json::from_slice(&listed.get_output().stdout).test()?;
    assert_eq!(listed["stacksteads"][0]["readiness"]["status"], "ready");
    assert!(!fixture.docker.state.join("app-probe").exists());
    assert_eq!(
        listener.accept().test_err()?.kind(),
        std::io::ErrorKind::WouldBlock
    );
    fixture.docker.assert_supported()
}
