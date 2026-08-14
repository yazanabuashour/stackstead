use super::*;

#[cfg(unix)]
#[test]
fn up_rejects_every_structurally_unsafe_or_disconnected_port_contract() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "port-fake-docker-bin",
        "#!/bin/sh\necho docker-must-not-run >&2\nexit 97\n",
    )?;

    for (mapping, expected) in [
        ("3000", "unsupported"),
        ("\"80\"", "no deterministic host binding"),
        ("\"3000-3002:80-82\"", "unsupported"),
        ("\"3000:80\"", "fixed host port"),
        (
            "\"127.0.0.1:${APP_PORT}:80\"",
            "env.generate does not define `APP_PORT`",
        ),
    ] {
        fs::write(
            &manifest.compose_files[0],
            format!(
                "services:\n  web:\n    image: nginx\n    ports: [{mapping}]\n  postgres:\n    image: postgres:16\n    ports: [\"127.0.0.1:${{POSTGRES_PORT}}:5432\"]\n"
            ),
        )
        .test_context("write unsafe Compose fixture")?;
        let assert = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["up", &manifest.stackstead_id])
            .assert()
            .failure();
        let stderr = output_text(&assert.get_output().stderr)?;
        assert!(
            stderr.contains(expected),
            "unexpected error for {mapping}: {stderr}"
        );
        assert!(
            !stderr.contains("docker-must-not-run"),
            "test contract condition failed"
        );
    }

    fs::write(
        &manifest.compose_files[0],
        "services:\n  web:\n    ports: [\"127.0.0.1:${WEB_PORT}:80\"]\n  postgres:\n    ports: [\"127.0.0.1:${POSTGRES_PORT}:5432\"]\n",
    )
    .test()?;
    project.replace_config(
        "    WEB_PORT: '{{ ports.web }}'\n",
        "    WEB_PORT: '39000'\n",
    )?;
    let literal = stackstead(&project.repo)
        .env("PATH", &path)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    assert!(
        output_text(&literal.get_output().stderr)?.contains("ports.<name>"),
        "test contract condition failed"
    );
    assert!(
        !output_text(&literal.get_output().stderr)?.contains("docker-must-not-run"),
        "test contract condition failed"
    );
    Ok(())
}
