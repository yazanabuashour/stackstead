use super::*;
use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};

fn manifest_value(version: &str) -> serde_json::Value {
    serde_json::json!({
        "kind":"StacksteadManifest","version":version,
        "stackstead_id":"a-b1230123456789abcdef0123456789ab","slug":"a","short_id":"b1230123456789abcdef0123456789ab",
        "runtime_token":"0123456789abcdef0123456789abcdef",
        "project":"demo","branch":"a","base":"main","repo_root":"/repo","project_state_root":"/state",
        "stackstead_root":"/state/demo/a-b1230123456789abcdef0123456789ab","worktree":"/state/demo/a-b1230123456789abcdef0123456789ab/source","state_dir":"/state/demo/a-b1230123456789abcdef0123456789ab/state",
        "compose_project":"demo-a-b1230123456789abcdef0123456789ab","compose_files":[],"ports":{},"container_ports":{},"urls":{},
        "env_file":"/env","agent_context":"/context","pointer_file":"/pointer","event_log":"/events","env_keys":[],
        "source_ownership":"stackstead",
        "status":{"source":"created","dependencies":"unknown","runtime":"stopped","database":"unknown","health":"unknown"},
        "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"
    })
}

#[test]
fn pointer_round_trip_is_atomic() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("stackstead.json");
    let pointer = StacksteadPointer {
        kind: "StacksteadPointer".into(),
        version: POINTER_VERSION.into(),
        stackstead_id: "feature-a-a17c0123456789abcdef0123456789ab".into(),
        manifest: directory.path().join("manifest.json"),
        project: "demo".into(),
        repo_root: directory.path().into(),
        project_state_root: directory.path().join("state"),
        stackstead_root: directory.path().join("cell"),
    };
    write_pointer(&path, &pointer).test()?;
    let actual: StacksteadPointer = serde_json::from_reader(File::open(&path).test()?).test()?;
    assert_eq!(actual, pointer, "test contract values differ");

    let mut legacy = serde_json::to_value(&pointer).test()?;
    legacy["version"] = serde_json::json!("1");
    write_json_atomic(&path, &legacy).test()?;
    assert_eq!(
        StacksteadPointer::read(&path).test()?.version,
        "1",
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn rejects_future_or_wrong_manifest_contracts() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("manifest.json");
    let mut value = manifest_value("3");
    write_json_atomic(&path, &value).test()?;
    (StacksteadManifest::read(&path)).test_err()?;
    value["kind"] = serde_json::json!("OtherManifest");
    value["version"] = serde_json::json!(MANIFEST_VERSION);
    write_json_atomic(&path, &value).test()?;
    (StacksteadManifest::read(&path)).test_err()?;
    Ok(())
}

#[test]
fn requires_explicit_v2_fields_and_rejects_unknown_fields() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("manifest.json");
    let mut value = manifest_value(MANIFEST_VERSION);
    value.as_object_mut().test()?.remove("source_ownership");
    write_json_atomic(&path, &value).test()?;
    assert!(
        StacksteadManifest::read(&path)
            .test_err()?
            .to_string()
            .contains("requires source_ownership"),
        "test contract condition failed"
    );

    value["source_ownership"] = serde_json::json!("stackstead");
    value["future_field"] = serde_json::json!(true);
    write_json_atomic(&path, &value).test()?;
    (StacksteadManifest::read(&path)).test_err()?;
    Ok(())
}

#[test]
fn rejects_v1_and_missing_or_invalid_runtime_tokens_with_recreation_guidance() -> anyhow::Result<()>
{
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("manifest.json");
    let mut value = manifest_value("1");
    value.as_object_mut().test()?.remove("runtime_token");
    write_json_atomic(&path, &value).test()?;
    let error = StacksteadManifest::read(&path).test_err()?.to_string();
    assert!(
        error.contains("version 1 lacks a cryptographic runtime token"),
        "test contract condition failed"
    );
    assert!(
        error.contains("compatible older Stackstead binary"),
        "test contract condition failed"
    );

    value["version"] = serde_json::json!(MANIFEST_VERSION);
    write_json_atomic(&path, &value).test()?;
    let error = StacksteadManifest::read(&path).test_err()?.to_string();
    assert!(
        error.contains("requires a cryptographic runtime_token"),
        "test contract condition failed"
    );
    assert!(
        error.contains("recreate this stackstead"),
        "test contract condition failed"
    );

    value["runtime_token"] = serde_json::json!("0123456789ABCDEF0123456789ABCDEF");
    write_json_atomic(&path, &value).test()?;
    let error = StacksteadManifest::read(&path).test_err()?.to_string();
    assert!(
        error.contains("32 lowercase hexadecimal characters"),
        "test contract condition failed"
    );

    for token in ["0".repeat(31), "0".repeat(33)] {
        value["runtime_token"] = serde_json::json!(token);
        write_json_atomic(&path, &value).test()?;
        (StacksteadManifest::read(&path)).test_err()?;
    }
    Ok(())
}

#[test]
fn generated_runtime_tokens_have_the_contract_shape() -> anyhow::Result<()> {
    let token = new_runtime_token().test()?;
    assert!(
        valid_runtime_token(&token),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn trusted_environment_pins_both_compose_project_variables() -> anyhow::Result<()> {
    let manifest: StacksteadManifest =
        serde_json::from_value(manifest_value(MANIFEST_VERSION)).test()?;
    let environment = manifest.trusted_environment(&BTreeMap::new());
    assert_eq!(
        environment.get("COMPOSE_PROJECT_NAME"),
        Some(&manifest.compose_project),
        "test contract values differ"
    );
    assert_eq!(
        environment.get("STACKSTEAD_COMPOSE_PROJECT"),
        Some(&manifest.compose_project),
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn pointer_reader_validates_header_before_body() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("stackstead.json");
    write_json_atomic(
        &path,
        &serde_json::json!({"kind":"StacksteadPointer","version":"3"}),
    )
    .test()?;
    assert!(
        StacksteadPointer::read(&path)
            .test_err()?
            .to_string()
            .contains("unsupported pointer contract"),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn internal_manifest_save_uses_the_canonical_path() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let initial = directory.path().join("initial.json");
    let mut value = manifest_value(MANIFEST_VERSION);
    value["state_dir"] = serde_json::json!(directory.path().join("state"));
    write_json_atomic(&initial, &value).test()?;
    let mut manifest = StacksteadManifest::read(&initial).test()?;
    manifest.save_atomic().test()?;
    assert_eq!(
        StacksteadManifest::read(&manifest.manifest_path())
            .test()?
            .stackstead_id,
        manifest.stackstead_id,
        "test contract values differ"
    );
    Ok(())
}
