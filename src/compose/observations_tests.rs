use super::*;
use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};
use serde_json::{Value, json};

const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PROJECT: &str = "demo-a-b123";
const TOKEN: &str = "0123456789abcdef0123456789abcdef";

fn metadata() -> Value {
    json!({
        "id": ID,
        "container": "/demo-a-b123-worker-2",
        "project": PROJECT,
        "runtime_token": TOKEN,
        "service": "worker",
        "state": "running",
        "exit_code": 0,
        "health": "healthy",
        "healthcheck_enabled": true,
        "oneoff": "False",
        "container_number": "2",
        "config_hash": ID
    })
}

fn parse(metadata: &Value) -> anyhow::Result<ServiceObservation> {
    parse_owned_container(&serde_json::to_vec(metadata).test()?, ID, PROJECT, TOKEN)
}

#[test]
fn captures_owned_identity_raw_state_and_actual_health() -> anyhow::Result<()> {
    let observation = parse(&metadata()).test()?;
    assert_eq!(observation.service, "worker");
    assert_eq!(observation.container, "demo-a-b123-worker-2");
    assert_eq!(observation.id, ID);
    assert_eq!(observation.state, "running");
    assert_eq!(observation.exit_code, Some(0));
    assert_eq!(observation.health.as_deref(), Some("healthy"));
    assert_eq!(observation.healthcheck_enabled, Some(true));
    assert_eq!(observation.oneoff, Some(false));
    assert_eq!(observation.container_number, Some(2));
    assert_eq!(observation.config_hash.as_deref(), Some(ID));
    Ok(())
}

#[test]
fn missing_fields_remain_unknown_and_unattributed_containers_remain_visible() -> anyhow::Result<()>
{
    let mut value = metadata();
    for key in [
        "service",
        "state",
        "exit_code",
        "health",
        "healthcheck_enabled",
        "oneoff",
        "container_number",
        "config_hash",
    ] {
        value.as_object_mut().test()?.remove(key);
    }
    let observation = parse(&value).test()?;
    assert!(observation.service.is_empty());
    assert!(observation.state.is_empty());
    assert_eq!(observation.exit_code, None);
    assert_eq!(observation.health, None);
    assert_eq!(observation.healthcheck_enabled, None);
    assert_eq!(observation.oneoff, None);
    assert_eq!(observation.container_number, None);
    assert_eq!(observation.config_hash, None);
    assert_eq!(observation.id, ID);
    Ok(())
}

#[test]
fn instance_labels_require_positive_identity_without_dense_ordinals() -> anyhow::Result<()> {
    let mut value = metadata();
    for (ordinal, expected) in [
        ("2", Some(2)),
        ("3", Some(3)),
        ("0", None),
        ("-1", None),
        ("+1", None),
        ("", None),
        ("1.0", None),
        (" 1", None),
        ("18446744073709551616", None),
    ] {
        *value.get_mut("container_number").test()? = json!(ordinal);
        assert_eq!(parse(&value).test()?.container_number, expected);
    }
    for (oneoff, expected) in [
        ("True", Some(true)),
        ("true", Some(true)),
        ("FALSE", Some(false)),
        ("", None),
        ("normal", None),
        ("0", None),
    ] {
        *value.get_mut("oneoff").test()? = json!(oneoff);
        assert_eq!(parse(&value).test()?.oneoff, expected);
    }
    for hash in ["", "not-a-hash", &ID.to_ascii_uppercase()] {
        *value.get_mut("config_hash").test()? = json!(hash);
        assert_eq!(parse(&value).test()?.config_hash, None);
    }
    Ok(())
}

#[test]
fn refuses_foreign_or_mismatched_identity_without_echoing_evidence() -> anyhow::Result<()> {
    for (key, bad_value) in [
        ("id", json!("secret-short-id")),
        ("id", json!("f".repeat(64))),
        ("container", json!("")),
        ("project", json!("secret-foreign-project")),
        ("project", json!(null)),
        ("runtime_token", json!("secret-foreign-token")),
        ("runtime_token", json!(null)),
        ("exit_code", json!("secret-invalid-exit")),
        ("healthcheck_enabled", json!("secret-not-a-boolean")),
        ("oneoff", json!(false)),
    ] {
        let mut value = metadata();
        *value.get_mut(key).test()? = bad_value;
        let error = parse(&value).test_err()?.to_string();
        assert!(!error.contains("secret-"));
        assert!(!error.contains(TOKEN));
    }
    for output in [
        br#"{"secret-invalid-json""#.as_slice(),
        br#"{"id":"secret-a","id":"secret-b"}"#.as_slice(),
        b"[]".as_slice(),
        b"".as_slice(),
    ] {
        let error = parse_owned_container(output, ID, PROJECT, TOKEN)
            .test_err()?
            .to_string();
        assert!(!error.contains("secret-"));
    }
    Ok(())
}
