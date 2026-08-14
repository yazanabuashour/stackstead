use super::*;

#[test]
fn host_wide_port_leases_keep_stopped_projects_on_disjoint_ports() -> anyhow::Result<()> {
    let registry = tempfile::tempdir().test()?;
    let first_project = Project::initialized()?;
    let second_project = Project::initialized()?;
    let mut second_config = load_config(&second_project.repo.join("stackstead.yaml"))?;
    second_config["project"]["name"] = "second-project".into();
    second_project.write_config(&second_config, "use a distinct project identity")?;
    let create = |project: &Project, name: &str| {
        let created = stackstead(&project.repo)
            .env("XDG_STATE_HOME", registry.path())
            .args(["--json", "create", name])
            .assert()
            .success();
        changed_manifest(&created.get_output().stdout, "created")
    };

    let first = create(&first_project, "first")?;
    let second = create(&second_project, "second")?;
    let first_ports = first.ports.values().copied().collect::<BTreeSet<_>>();
    let second_ports = second.ports.values().copied().collect::<BTreeSet<_>>();
    assert!(
        first_ports.is_disjoint(&second_ports),
        "test contract condition failed"
    );
    assert_eq!(first_ports.len(), 2, "test contract values differ");
    assert_eq!(second_ports.len(), 2, "test contract values differ");
    assert_eq!(
        first_ports.iter().next_back().test()? - first_ports.iter().next().test()?,
        1,
        "test contract values differ"
    );
    assert_eq!(
        second_ports.iter().next_back().test()? - second_ports.iter().next().test()?,
        1,
        "test contract values differ"
    );

    #[cfg(unix)]
    {
        let path = fake_docker_path(
            first_project.repo.parent().test()?,
            "lease-release-fake-bin",
            "#!/bin/sh\ncase \"$1 $2\" in 'container ls'|'network ls'|'volume ls') exit 0;; esac\nexit 97\n",
        )?;
        stackstead(&first_project.repo)
            .env("XDG_STATE_HOME", registry.path())
            .env("PATH", path)
            .args(["destroy", &first.stackstead_id, "--yes"])
            .assert()
            .success();
        let registry: Value = serde_json::from_slice(
            &fs::read(registry.path().join("stackstead/port-leases.json")).test()?,
        )
        .test()?;
        assert_eq!(
            registry["leases"]
                .as_array()
                .test()?
                .iter()
                .map(|lease| lease["port"].as_u64().test().map(|port| port as u16))
                .collect::<anyhow::Result<BTreeSet<_>>>()?,
            second_ports,
            "test contract values differ"
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn lifecycle_commands_reject_a_port_lease_that_no_longer_belongs_to_the_manifest()
-> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let git_common_dir = PathBuf::from(
        git(
            &project.repo,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?
        .trim(),
    );
    let registry_path = git_common_dir.join("stackstead-test-state/stackstead/port-leases.json");
    let mut registry: Value =
        serde_json::from_slice(&fs::read(&registry_path).test_context("read port lease registry")?)
            .test_context("parse port lease registry")?;
    for lease in registry["leases"].as_array_mut().test()? {
        lease["owner"] = "ffffffffffffffffffffffffffffffff".into();
    }
    fs::write(&registry_path, serde_json::to_vec_pretty(&registry).test()?)
        .test_context("replace port lease owner")?;

    let marker = project.repo.parent().test()?.join("lease-docker-ran");
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "lease-mismatch-fake-bin",
        &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
    )?;
    for args in [
        vec!["up", &manifest.stackstead_id],
        vec!["stop", &manifest.stackstead_id],
        vec!["db", "status", &manifest.stackstead_id],
        vec!["run", &manifest.stackstead_id, "--", "true"],
        vec!["repair", &manifest.stackstead_id],
        vec!["destroy", &manifest.stackstead_id, "--yes"],
    ] {
        let rejected = stackstead(&project.repo)
            .env("PATH", &path)
            .args(args)
            .assert()
            .failure();
        assert!(
            output_text(&rejected.get_output().stderr)?.contains("port leases for owner"),
            "unexpected error: {}",
            output_text(&rejected.get_output().stderr)?
        );
    }
    assert!(
        !marker.exists(),
        "Docker ran before lease ownership validation"
    );
    assert!(
        manifest.manifest_path().is_file(),
        "test contract condition failed"
    );
    assert!(manifest.worktree.is_dir(), "test contract condition failed");
    Ok(())
}
