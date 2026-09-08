use super::*;

#[test]
fn doctor_scans_branch_local_compose_files_for_fixed_ports() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    fs::write(
        &manifest.compose_files[0],
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"3000:80\"\n",
    )
    .test_context("write branch-local Compose change")?;

    let assert = stackstead(&project.repo)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let diagnostics: Value =
        serde_json::from_slice(&assert.get_output().stdout).test_context("parse diagnostics")?;
    assert!(
        diagnostics["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["code"]
                == "compose.worktree_fixed_host_port"
                && item["message"].as_str().is_some_and(|message| {
                    message.contains("3000") && message.contains("docker-compose.yml:5")
                })))
    );
    assert!(
        diagnostics["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| {
                item["code"] == "compose.worktree_all_interfaces_host_port"
                    && item["severity"] == "error"
                    && item["message"].as_str().is_some_and(|message| {
                        message.contains("web")
                            && message.contains("80/tcp")
                            && message
                                .contains(manifest.compose_files[0].to_string_lossy().as_ref())
                    })
            }))
    );

    fs::write(
        &manifest.compose_files[0],
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"127.0.0.1:${WEB_PORT}:80\"\n",
    )
    .test()?;
    let loopback = stackstead(&project.repo)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let loopback: Value = serde_json::from_slice(&loopback.get_output().stdout).test()?;
    assert!(loopback["diagnostics"].as_array().is_some_and(|items| {
        items
            .iter()
            .all(|item| item["code"] != "compose.worktree_all_interfaces_host_port")
    }));
    Ok(())
}

#[test]
fn doctor_reports_project_worktree_and_pointer_contract_failures_together() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    fs::write(
        project.repo.join("docker-compose.yml"),
        "services:\n  web:\n    ports: [\"80\"]\n",
    )
    .test()?;
    fs::write(
        &manifest.compose_files[0],
        "services:\n  web:\n    ports: [\"${APP_PORT}:80\"]\n",
    )
    .test()?;
    let mut pointer: Value = serde_json::from_slice(&fs::read(&manifest.pointer_file).test()?)
        .test_context("parse pointer")?;
    pointer["stackstead_id"] = Value::String("copied-pointer-a123".into());
    fs::write(
        &manifest.pointer_file,
        serde_json::to_vec_pretty(&pointer).test()?,
    )
    .test()?;

    let output = stackstead(&project.repo)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let diagnostics: Value = serde_json::from_slice(&output.get_output().stdout).test()?;
    let codes = diagnostics["diagnostics"]
        .as_array()
        .test()?
        .iter()
        .filter_map(|item| item["code"].as_str())
        .collect::<BTreeSet<_>>();
    for expected in [
        "compose.unbound_host_port",
        "compose.isolation_contract.invalid",
        "compose.worktree_isolation_contract.invalid",
        "pointer.binding.invalid",
    ] {
        assert!(
            codes.contains(expected),
            "missing diagnostic {expected}: {codes:?}"
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn doctor_fail_on_error_keeps_complete_json_and_ignores_warnings() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "doctor-ci-fake-bin",
        "#!/bin/sh\ntest \"${1-}\" != info\n",
    )?;

    let warning_only = stackstead(&project.repo)
        .env("PATH", &path)
        .args(["doctor", "--json", "--fail-on-error"])
        .assert()
        .success();
    let warning_report: Value = serde_json::from_slice(&warning_only.get_output().stdout).test()?;
    assert_eq!(warning_report["kind"], "DoctorReport");
    assert_eq!(warning_report["version"], "1");
    assert_eq!(warning_report["error_count"], 0);
    assert!(warning_report["warning_count"].as_u64().test()? > 0);

    fs::write(
        project.repo.join("docker-compose.yml"),
        "services:\n  web:\n    ports: [\"80\"]\n",
    )
    .test()?;
    stackstead(&project.repo)
        .env("PATH", &path)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let failed = stackstead(&project.repo)
        .env("PATH", path)
        .args(["doctor", "--json", "--fail-on-error"])
        .assert()
        .code(1);
    let error_report: Value = serde_json::from_slice(&failed.get_output().stdout).test()?;
    assert_eq!(error_report["ok"], false);
    assert!(error_report["error_count"].as_u64().test()? > 0);
    assert!(!error_report["diagnostics"].as_array().test()?.is_empty());
    Ok(())
}

#[cfg(unix)]
#[test]
fn doctor_reports_repository_policy_freshness_without_failing() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let instructions = project.repo.join("AGENTS.md");
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "policy-doctor-fake-bin",
        "#!/bin/sh\nexit 0\n",
    )?;

    let report = stackstead(&project.repo)
        .env("PATH", &path)
        .args(["doctor", "--json", "--fail-on-error"])
        .assert()
        .success();
    let report: Value = serde_json::from_slice(&report.get_output().stdout).test()?;
    assert!(has_diagnostic(
        &report,
        "repository_policy.missing",
        "warning"
    ));

    for (contents, code, severity) in [
        (
            "<!-- stackstead-policy: 0 -->\n",
            "repository_policy.outdated",
            "warning",
        ),
        (
            "## Stackstead\nRead `$STACKSTEAD_CONTEXT`.\n",
            "repository_policy.unversioned",
            "warning",
        ),
        (
            "<!-- stackstead-policy: 2 -->\n",
            "repository_policy.binary_outdated",
            "warning",
        ),
        (
            "<!-- stackstead-policy: 1 -->\n",
            "repository_policy.current",
            "info",
        ),
    ] {
        fs::write(&instructions, contents).test()?;
        let report = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["doctor", "--json", "--fail-on-error"])
            .assert()
            .success();
        let report: Value = serde_json::from_slice(&report.get_output().stdout).test()?;
        assert!(has_diagnostic(&report, code, severity), "{report:#}");
        assert!(!has_diagnostic(
            &report,
            "repository_policy.missing",
            "warning"
        ));
    }
    Ok(())
}
