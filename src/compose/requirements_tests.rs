use super::*;
use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};
use serde_json::json;

const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn projects_selected_services_and_compose_replica_semantics() -> anyhow::Result<()> {
    let required = BTreeMap::from([
        ("api.v2".into(), Role::LongRunning),
        ("worker".into(), Role::LongRunning),
        ("setup".into(), Role::Job),
    ]);
    let model = json!({"services": {
        "api.v2": {"image": "api"},
        "worker": {"scale": 2, "deploy": {"replicas": 2}},
        "setup": {"deploy": {"mode": "global", "replicas": 3}},
        "optional": {"scale": 0}
    }});
    let hashes = format!("api.v2 {HASH}\nworker {HASH}\nsetup {HASH}\noptional {HASH}\n");
    let projection = project_requirements(model, hashes.as_bytes(), &required, None).test()?;
    assert_eq!(projection.profiles, None);
    assert_eq!(projection.services.len(), 3);
    assert_eq!(projection.services.get("api.v2").test()?.replicas, 1);
    assert_eq!(projection.services.get("worker").test()?.replicas, 2);
    assert_eq!(projection.services.get("setup").test()?.replicas, 3);
    assert_eq!(projection.services.get("worker").test()?.config_hash, HASH);
    assert!(is_sha256(&projection.model_hash));
    for (service, expected) in [
        (json!({"deploy": {"mode": "global"}}), 1),
        (json!({"scale": 4}), 4),
        (json!({"deploy": {"replicas": 5}}), 5),
        (json!({"scale": null, "deploy": null}), 1),
    ] {
        assert_eq!(service_replicas(&service).test()?, expected);
    }
    Ok(())
}

#[test]
fn rejects_unprovable_counts_and_containerless_services() -> anyhow::Result<()> {
    for service in [
        json!({"scale": 0}),
        json!({"deploy": {"replicas": 0}}),
        json!({"scale": -1}),
        json!({"scale": 1.5}),
        json!({"scale": "2"}),
        json!({"scale": 2, "deploy": {"replicas": 3}}),
        json!({"deploy": {"replicas": "2"}}),
        json!({"deploy": []}),
        json!({"provider": {"type": "external"}}),
        json!(null),
    ] {
        service_replicas(&service).test_err()?;
    }
    Ok(())
}

#[test]
fn pinned_profiles_preserve_raw_selection_and_remove_later_callers_value() -> anyhow::Result<()> {
    for profiles in [None, Some(""), Some("utility, other"), Some("*")] {
        let mut removed = vec!["APP_PASSWORD".into()];
        let mut environment = BTreeMap::from([("COMPOSE_PROFILES".into(), "later".into())]);
        pin_profiles(&mut removed, &mut environment, profiles);
        assert!(removed.iter().any(|key| key == "COMPOSE_PROFILES"));
        assert_eq!(
            environment.get("COMPOSE_PROFILES").map(String::as_str),
            profiles
        );
    }
    let required = BTreeMap::from([("utility".into(), Role::Job)]);
    let inactive = json!({"services": {"web": {"image": "web"}}});
    project_requirements(inactive, format!("web {HASH}").as_bytes(), &required, None).test_err()?;
    let active = json!({"services": {"utility": {"profiles": ["utility"], "image": "utility"}}});
    let projection = project_requirements(
        active,
        format!("utility {HASH}").as_bytes(),
        &required,
        Some("utility"),
    )
    .test()?;
    assert_eq!(projection.profiles.as_deref(), Some("utility"));
    assert_eq!(projection.services.get("utility").test()?.replicas, 1);
    Ok(())
}

#[test]
fn native_hash_output_must_match_the_whole_selected_service_set() -> anyhow::Result<()> {
    let required = BTreeMap::from([("web".into(), Role::LongRunning)]);
    let model = json!({"services": {"web": {"image": "web"}}});
    for hashes in [
        String::new(),
        format!("foreign {HASH}"),
        format!("web {HASH}\nextra {HASH}"),
        format!("web {HASH}\nweb {HASH}"),
        format!("web {HASH} extra"),
        "web secret-invalid-hash".into(),
        "secret-missing-hash".into(),
        format!("web {}", HASH.to_ascii_uppercase()),
    ] {
        let error = project_requirements(model.clone(), hashes.as_bytes(), &required, None)
            .test_err()?
            .to_string();
        assert!(!error.contains("secret-"));
        assert!(!error.contains(HASH));
    }
    parse_service_hashes(&[0xff]).test_err()?;
    parse_model(br#"{"services":"secret-invalid-model""#).test_err()?;
    project_requirements(
        model,
        format!("web {HASH}").as_bytes(),
        &BTreeMap::new(),
        None,
    )
    .test_err()?;
    Ok(())
}

#[test]
fn full_model_hash_is_canonical_and_covers_native_hash_omissions() -> anyhow::Result<()> {
    assert_eq!(
        model_hash(json!({})).test()?,
        "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
    );
    let first = parse_model(br#"{"services":{"web":{"environment":{"Z":"secret","A":"value"},"image":"web"}},"name":"project"}"#).test()?;
    let reordered = parse_model(br#"{"name":"project","services":{"web":{"image":"web","environment":{"A":"value","Z":"secret"}}}}"#).test()?;
    assert_eq!(
        model_hash(first.clone()).test()?,
        model_hash(reordered).test()?
    );
    let baseline = model_hash(first).test()?;
    for definition in [
        json!({"build": {"context": "other"}}),
        json!({"pull_policy": "always"}),
        json!({"scale": 2}),
        json!({"deploy": {"replicas": 2}}),
        json!({"depends_on": {"setup": {"condition": "service_completed_successfully"}}}),
        json!({"profiles": ["utility"]}),
        json!({"environment": {"Z": "changed", "A": "value"}}),
    ] {
        let mut changed = json!({"name": "project", "services": {
            "web": {"image": "web", "environment": {"Z": "secret", "A": "value"}}
        }});
        let web = changed
            .get_mut("services")
            .test()?
            .get_mut("web")
            .test()?
            .as_object_mut()
            .test()?;
        web.extend(definition.as_object().test()?.clone());
        assert_ne!(baseline, model_hash(changed).test()?);
    }
    Ok(())
}

#[test]
fn generated_file_secrets_are_removed_from_error_chains() {
    let generated = BTreeMap::from([("APP_PASSWORD".into(), "file-only-private-value".into())]);
    let error = anyhow::anyhow!("expanded value file-only-private-value rejected")
        .context("normalization failed");
    let sanitized = sanitize_generated_error(&error, &generated);
    assert!(!format!("{sanitized:#}").contains("file-only-private-value"));
    assert!(sanitized.to_string().contains("[REDACTED]"));
    assert!(sanitized.to_string().contains("normalization failed"));
}
