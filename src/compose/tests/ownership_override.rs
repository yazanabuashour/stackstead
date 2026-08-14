use super::*;

#[test]
fn ownership_mount_quotes_valid_commas_and_quotes() -> anyhow::Result<()> {
    assert_eq!(
        ownership_bind_mount("/tmp/source,\"quoted\""),
        "type=bind,\"src=/tmp/source,\"\"quoted\"\"\",dst=/stackstead-source",
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn ownership_override_attests_every_direct_managed_resource() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let compose = directory.path().join("compose.yaml");
    std::fs::write(
        &compose,
        r#"services:
  web:
    image: nginx
    networks: [backend]
    volumes: [cache:/cache]
networks:
  backend: {}
  upstream:
    external: true
volumes:
  cache: {}
  external-data:
    external: true
"#,
    )
    .test()?;
    let mut manifest = manifest()?;
    manifest.worktree = directory.path().into();
    manifest.compose_files = vec![compose];
    let rendered = render_ownership_override(&manifest).test()?;
    let document: serde_yaml::Value = serde_yaml::from_str(&rendered).test()?;
    for (field, names) in [
        ("services", &["web"][..]),
        ("networks", &["backend", "default"][..]),
        ("volumes", &["cache"][..]),
    ] {
        let values = yaml_field(&document, field)
            .and_then(serde_yaml::Value::as_mapping)
            .test()?;
        assert_eq!(values.len(), names.len(), "test contract values differ");
        for name in names {
            let token = values
                .get(serde_yaml::Value::String((*name).into()))
                .and_then(|resource| yaml_field(resource, "labels"))
                .and_then(|labels| yaml_field(labels, RUNTIME_TOKEN_LABEL))
                .and_then(serde_yaml::Value::as_str);
            assert_eq!(
                token,
                Some(manifest.runtime_token.as_str()),
                "test contract values differ"
            );
        }
    }
    assert!(
        !rendered.contains("upstream:"),
        "test contract condition failed"
    );
    assert!(
        !rendered.contains("external-data:"),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn ownership_override_rejects_unattestable_compose_shapes() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let compose = directory.path().join("compose.yaml");
    let mut manifest = manifest()?;
    manifest.compose_files = vec![compose.clone()];
    for (contents, expected) in [
        (
            "include: shared.yaml\nservices:\n  web: {image: nginx}\n",
            "`include`",
        ),
        ("services:\n  web:\n    extends: base\n", "`extends`"),
        (
            "services:\n  web:\n    volumes: [/cache]\n",
            "anonymous volume",
        ),
        (
            "services:\n  web:\n    volumes: [cache:/cache]\n",
            "without a top-level declaration",
        ),
    ] {
        std::fs::write(&compose, contents).test()?;
        let error = render_ownership_override(&manifest).test_err()?.to_string();
        assert!(error.contains(expected), "unexpected error: {error}");
    }
    Ok(())
}

#[test]
fn ownership_override_rejects_non_string_optional_fields() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let compose = directory.path().join("compose.yaml");
    let mut manifest = manifest()?;
    manifest.compose_files = vec![compose.clone()];
    for (contents, subject, expected) in [
        (
            "services:\n  web:\n    container_name: 7\n",
            "service `web`",
            "non-string container_name",
        ),
        (
            "services:\n  web:\n    volumes:\n      - type: 7\n        source: data\n        target: /data\nvolumes:\n  data: {}\n",
            "service `web`",
            "non-string volume type",
        ),
        (
            "services:\n  web: {}\nvolumes:\n  data:\n    name: 7\n",
            "volumes `data`",
            "non-string name",
        ),
    ] {
        std::fs::write(&compose, contents).test()?;
        let error = render_ownership_override(&manifest).test_err()?.to_string();
        assert!(error.contains(subject), "unexpected error: {error}");
        assert!(error.contains(expected), "unexpected error: {error}");
        assert!(
            error.contains(&compose.display().to_string()),
            "unexpected error: {error}"
        );
    }
    Ok(())
}
