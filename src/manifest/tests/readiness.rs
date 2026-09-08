use super::*;
use crate::{
    config::{ReadinessConfig, StacksteadConfig},
    lifecycle::{ProjectRuntime, validate_current_contract, validate_manifest_binding},
    readiness::{Contract, Requirements, Role, ServiceRequirement},
    state::ProjectPaths,
};

fn declared() -> Contract {
    Contract::Declared {
        required: BTreeMap::from([("Worker.api_1".into(), Role::Job)]),
        resolved: Some(Requirements {
            profiles: None,
            model_hash: "ab".repeat(32),
            services: BTreeMap::from([(
                "Worker.api_1".into(),
                ServiceRequirement {
                    replicas: 2,
                    config_hash: "cd".repeat(32),
                },
            )]),
        }),
    }
}

#[test]
fn persists_readiness_and_rejects_corruption_on_read_and_save() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("manifest.json");
    let mut value = manifest_value(MANIFEST_VERSION);
    value["state_dir"] = serde_json::json!(directory.path());
    let mut manifest: StacksteadManifest = serde_json::from_value(value.clone()).test()?;
    manifest.readiness = declared();
    manifest.save_atomic().test()?;
    assert_eq!(StacksteadManifest::read(&path).test()?, manifest);
    manifest.readiness.invalidate();
    manifest.save_atomic().test()?;
    assert_eq!(
        StacksteadManifest::read(&path).test()?.readiness,
        manifest.readiness
    );
    let before = std::fs::read(&path).test()?;
    manifest.readiness = Contract::Declared {
        required: BTreeMap::new(),
        resolved: None,
    };
    manifest.save_atomic().test_err()?;
    assert_eq!(std::fs::read(&path).test()?, before);

    let mut zero_replicas = serde_json::to_value(declared()).test()?;
    zero_replicas["resolved"]["services"]["Worker.api_1"]["replicas"] = serde_json::json!(0);
    for readiness in [
        serde_json::Value::Null,
        serde_json::json!({"configuration": "unconfigured", "required": {"web": "job"}}),
        serde_json::json!({"configuration": "declared", "required": {}}),
        zero_replicas,
    ] {
        value["readiness"] = readiness;
        write_json_atomic(&path, &value).test()?;
        StacksteadManifest::read(&path).test_err()?;
    }
    Ok(())
}

fn bound_manifest(root: &Path) -> anyhow::Result<(ProjectRuntime, StacksteadManifest)> {
    let mut config = StacksteadConfig::default();
    config.project.name = "demo".into();
    let paths = ProjectPaths::new(root.join("repo"), root.join("state"), "demo");
    let mut manifest: StacksteadManifest =
        serde_json::from_value(manifest_value(MANIFEST_VERSION)).test()?;
    manifest.repo_root = paths.repo_root.clone();
    manifest.project_state_root = paths.state_root.clone();
    manifest.stackstead_root = paths.project_state_dir.join(&manifest.stackstead_id);
    manifest.worktree = manifest.stackstead_root.join("source");
    manifest.state_dir = manifest.stackstead_root.join("state");
    manifest.env_file = manifest.worktree.join(&config.env.file);
    manifest.agent_context = manifest.worktree.join(&config.agent.context_file);
    manifest.compose_files = vec![manifest.worktree.join("docker-compose.yml")];
    manifest.pointer_file = manifest.worktree.join(".stackstead/stackstead.json");
    manifest.event_log = manifest.state_dir.join("events.jsonl");
    std::fs::create_dir_all(&manifest.worktree).test()?;
    std::fs::write(
        &manifest.compose_files[0],
        "services: {Worker.api_1: {image: busybox}}\n",
    )
    .test()?;
    Ok((ProjectRuntime { config, paths }, manifest))
}

#[test]
fn current_contract_rejects_declaration_drift_without_weakening_durable_binding()
-> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let (mut runtime, mut manifest) = bound_manifest(directory.path())?;
    validate_current_contract(&runtime, &manifest).test()?;
    manifest.readiness = declared();
    let declaration = ReadinessConfig {
        required: manifest.readiness.required().test()?.clone(),
    };
    runtime.config.runtime.readiness = Some(declaration.clone());
    validate_current_contract(&runtime, &manifest).test()?;
    manifest.readiness.invalidate();
    validate_current_contract(&runtime, &manifest).test()?;

    for readiness in [
        None,
        Some(ReadinessConfig {
            required: BTreeMap::from([("Worker.api_1".into(), Role::LongRunning)]),
        }),
        Some(ReadinessConfig {
            required: BTreeMap::from([("other".into(), Role::Job)]),
        }),
    ] {
        runtime.config.runtime.readiness = readiness;
        validate_manifest_binding(&runtime, &manifest).test()?;
        let error = validate_current_contract(&runtime, &manifest)
            .test_err()?
            .to_string();
        assert!(error.contains("readiness role declaration differs"));
        crate::lifecycle::regenerate_contract(&runtime.config, &mut manifest).test_err()?;
        assert!(!manifest.env_file.exists());
    }
    runtime.config.runtime.readiness = Some(declaration);
    manifest.readiness = Contract::Unconfigured {};
    validate_current_contract(&runtime, &manifest).test_err()?;

    manifest.readiness = declared();
    let mut tampered = manifest.clone();
    tampered.compose_project.push_str("-foreign");
    validate_manifest_binding(&runtime, &tampered).test_err()?;
    tampered = manifest;
    tampered.env_file = directory.path().join("foreign.env");
    validate_manifest_binding(&runtime, &tampered).test_err()?;
    Ok(())
}
