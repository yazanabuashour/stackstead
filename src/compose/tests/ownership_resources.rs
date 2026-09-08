use super::*;

#[test]
fn ownership_and_runtime_names_accept_null_name_resets() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let primary = directory.path().join("compose.yaml");
    let overlay = directory.path().join("compose.reset.yaml");
    std::fs::write(
        &primary,
        "services:\n  web:\n    container_name: custom-web\n",
    )
    .test()?;
    std::fs::write(
        &overlay,
        "services:\n  web:\n    container_name: !reset null\nvolumes:\n  data:\n    name: null\nnetworks:\n  backend:\n    name: !reset null\n",
    )
    .test()?;
    let mut manifest = manifest()?;
    manifest.compose_files = vec![primary, overlay];

    (render_ownership_override(&manifest)).test()?;
    let runtime_names = expected_runtime_names(&manifest).test()?;
    let names = |kind: &str| {
        runtime_names
            .iter()
            .find(|(candidate, ..)| candidate == kind)
            .map(|(_, _, _, names)| names)
            .test()
    };
    assert!(names("container")?.contains("demo-a-b123-web-1"));
    assert!(!names("container")?.contains("custom-web"));
    assert!(names("volume")?.contains("demo-a-b123_data"));
    assert!(names("network")?.contains("demo-a-b123_backend"));
    Ok(())
}

#[test]
fn ownership_override_defaults_an_omitted_volume_type() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let compose = directory.path().join("compose.yaml");
    std::fs::write(
        &compose,
        "services:\n  web:\n    volumes:\n      - source: data\n        target: /data\nvolumes:\n  data: {}\n",
    )
    .test()?;
    let mut manifest = manifest()?;
    manifest.compose_files = vec![compose];

    (render_ownership_override(&manifest)).test()?;
    Ok(())
}

#[test]
fn ownership_override_supports_later_volume_overlays_and_external_volumes() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let primary = directory.path().join("compose.yaml");
    let overlay = directory.path().join("compose.volumes.yaml");
    std::fs::write(
        &primary,
        "services:\n  web:\n    volumes: [cache:/cache, shared:/shared]\n",
    )
    .test()?;
    std::fs::write(
        &overlay,
        "volumes:\n  cache:\n  shared:\n    external: true\nnetworks:\n  default:\n    external: true\n",
    )
    .test()?;
    let mut manifest = manifest()?;
    manifest.compose_files = vec![primary, overlay];
    let rendered = render_ownership_override(&manifest).test()?;
    assert!(rendered.contains("cache:"));
    assert!(!rendered.contains("shared:"));
    assert!(!rendered.contains("default:"));
    Ok(())
}

#[test]
fn ownership_override_rejects_interpolated_and_redeclared_resource_names() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let primary = directory.path().join("compose.yaml");
    let overlay = directory.path().join("compose.overlay.yaml");
    let mut manifest = manifest()?;
    manifest.compose_files = vec![primary.clone()];

    std::fs::write(
        &primary,
        "services:\n  web: {image: nginx}\nvolumes:\n  data:\n    name: ${GLOBAL_DATA}\n",
    )
    .test()?;
    let error = render_ownership_override(&manifest).test_err()?.to_string();
    assert!(error.contains("requires a literal name"), "{error}");

    std::fs::write(
        &primary,
        "services:\n  web: {image: nginx}\nvolumes:\n  data: {}\n",
    )
    .test()?;
    std::fs::write(&overlay, "volumes:\n  data:\n    driver: local\n").test()?;
    manifest.compose_files.push(overlay);
    let error = render_ownership_override(&manifest).test_err()?.to_string();
    assert!(
        error.contains("declared in multiple Compose files"),
        "{error}"
    );
    Ok(())
}
