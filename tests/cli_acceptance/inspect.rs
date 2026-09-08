use super::*;

#[test]
fn runtime_probe_failure_is_reported_without_breaking_inspect_json() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    project.create("feature-a")?;

    let ps = stackstead(&project.repo)
        .env("PATH", "")
        .args(["ps", "--json"])
        .assert()
        .success();
    let listed: Value =
        serde_json::from_slice(&ps.get_output().stdout).test_context("parse ps output")?;
    assert_eq!(listed["stacksteads"][0]["runtime"], "unknown");

    let inspect = stackstead(&project.repo)
        .env("PATH", "")
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspect.get_output().stdout)
        .test_context("parse inspect output")?;
    assert_eq!(inspected["version"], "4");
    assert!(inspected["live"]["runtime"]["running"].is_null());
    assert_eq!(inspected["live"]["runtime"]["activity"], "unknown");
    assert!(inspected["live"]["services"].is_null());
    assert_eq!(inspected["live"]["readiness"]["status"], "unconfigured");
    assert_eq!(inspected["live"]["runtime"]["status"], "unknown");
    assert_eq!(inspected["live"]["database"]["status"], "unknown");
    assert_eq!(inspected["effective"]["runtime"]["basis"], "live");
    assert!(
        inspected["warnings"]
            .as_array()
            .is_some_and(|warnings| !warnings.is_empty())
    );
    Ok(())
}

#[test]
fn inspect_reports_unreadable_compose_files_instead_of_hiding_them() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    fs::remove_file(&manifest.compose_files[0]).test_context("remove generated Compose fixture")?;

    let inspected = stackstead(&project.repo)
        .env("PATH", "")
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout)
        .test_context("parse inspect output")?;

    assert!(inspected["warnings"].as_array().is_some_and(|warnings| {
        warnings.iter().any(|warning| {
            warning.as_str().is_some_and(|warning| {
                warning.contains("could not inspect fixed ports")
                    && warning.contains(manifest.compose_files[0].to_string_lossy().as_ref())
            })
        })
    }));
    Ok(())
}

#[test]
fn inspect_rejects_mismatched_manifest_port_service_sets() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let mut manifest = project.create("feature-a")?;
    manifest.container_ports.remove("web");
    manifest
        .write_fixture()
        .test_context("write malformed manifest fixture")?;

    let rejected = stackstead(&project.repo)
        .args(["inspect", "feature-a"])
        .assert()
        .failure();

    assert!(
        output_text(&rejected.get_output().stderr)?
            .contains("manifest host and container port service sets differ")
    );
    assert!(!output_text(&rejected.get_output().stdout)?.contains("-> 0"));
    Ok(())
}

#[test]
fn inspect_human_output_ends_with_full_id_actions() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;

    let inspect = stackstead(&project.repo)
        .env("PATH", "")
        .args(["inspect", "feature-a"])
        .assert()
        .success();
    let stdout = output_text(&inspect.get_output().stdout)?;

    assert!(stdout.contains(&format!(
        "\nNext:\n  stackstead doctor\n  stackstead context {} --print\n",
        manifest.stackstead_id
    )));
    Ok(())
}

#[cfg(unix)]
#[test]
fn database_status_requires_the_exact_compose_port_publication() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let port = manifest.ports["postgres"];
    let root = project.repo.parent().test()?;
    let _listener =
        TcpListener::bind(("127.0.0.1", port)).test_context("bind unrelated listener")?;

    let docker = super::readiness::fixture::OwnedDocker::new(&manifest)?;
    docker.rows(&[super::readiness::fixture::row(
        &manifest, "postgres", 1, 1, "running", 0,
    )])?;
    for (host, expected) in [("127.0.0.2", "unreachable"), ("127.0.0.1", "reachable")] {
        let status = docker
            .command(&manifest)
            .env("PUBLICATION_HOST", host)
            .args(["db", "status", "feature-a", "--json"])
            .assert()
            .success();
        let status: Value = serde_json::from_slice(&status.get_output().stdout).test()?;
        for key in [
            "stackstead_id",
            "strategy",
            "service",
            "host",
            "port",
            "database",
            "reachable",
            "identity_status",
            "seed_status",
            "last_seed_at",
        ] {
            assert!(status.get(key).is_some(), "db status JSON omitted {key}");
        }
        assert_eq!(status["reachable"], true);
        assert_eq!(status["identity_status"], expected);

        let inspected = docker
            .command(&manifest)
            .env("PUBLICATION_HOST", host)
            .args(["inspect", "feature-a", "--json"])
            .assert()
            .success();
        let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout).test()?;
        assert_eq!(inspected["live"]["database"]["reachable"], true);
        assert_eq!(inspected["live"]["database"]["status"], expected);

        // A later probe would see a changed publication; presentation must use the captured one.
        let receipt = root.join(format!("publication-{host}"));
        let human = docker
            .command(&manifest)
            .env("PUBLICATION_HOST", host)
            .env("PUBLICATION_PROBE_RECEIPT", &receipt)
            .args(["inspect", "feature-a"])
            .assert()
            .success();
        let human = output_text(&human.get_output().stdout)?;
        assert!(receipt.exists());
        assert!(human.contains(&format!("Database:      {expected}")));
        assert!(human.contains(&format!(
            "Application health: {} ({})",
            inspected["effective"]["health"]["status"].as_str().test()?,
            inspected["effective"]["health"]["basis"].as_str().test()?
        )));
    }
    docker.assert_supported()
}
