use std::collections::BTreeMap;

use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::{
    command,
    manifest::StacksteadManifest,
    readiness::{Requirements, Role, ServiceRequirement},
};

use super::{
    docker::{base_args, environment_for_generated, sanitize_generated_error},
    model::is_sha256,
    ownership::verify_ownership_override,
};

/// Resolve only the effective, profile-selected model. No normalized configuration is retained.
/// Every subprocess consumes the same optional deadline; captures never reset it.
pub fn resolve_requirements(
    manifest: &StacksteadManifest,
    required: &BTreeMap<String, Role>,
    profiles: Option<&str>,
    deadline: Option<std::time::Instant>,
) -> anyhow::Result<Requirements> {
    let generated = validated_environment(manifest)?;
    resolve(manifest, required, profiles, &generated, deadline)
        .map_err(|error| sanitize_generated_error(&error, &generated))
}

fn resolve(
    manifest: &StacksteadManifest,
    required: &BTreeMap<String, Role>,
    profiles: Option<&str>,
    generated: &BTreeMap<String, String>,
    deadline: Option<std::time::Instant>,
) -> anyhow::Result<Requirements> {
    verify_ownership_override(manifest)?;
    let (mut removed, mut environment) = environment_for_generated(manifest, generated);
    pin_profiles(&mut removed, &mut environment, profiles);
    let capture = |options: &[&str]| {
        let mut args = base_args(manifest);
        args.push("config".into());
        args.extend(options.iter().map(|option| (*option).to_owned()));
        command::run_sanitized_until(
            "docker",
            &args,
            &manifest.worktree,
            &environment,
            removed.iter().map(String::as_str),
            deadline,
        )
        .map(|output| output.stdout)
    };
    let model = parse_model(&capture(&["--format", "json"])?)?;
    let hashes = capture(&["--hash", "*"])?;
    let requirements = project_requirements(model, &hashes, required, profiles)?;
    // Separate Compose invocations cannot form an atomic snapshot. Reject observed drift
    // across the native-hash capture rather than combining known-different models.
    let after = parse_model(&capture(&["--format", "json"])?)?;
    if model_hash(after)? != requirements.model_hash
        || validated_environment(manifest)? != *generated
    {
        anyhow::bail!("effective Compose inputs changed while resolving readiness requirements");
    }
    Ok(requirements)
}

fn validated_environment(
    manifest: &StacksteadManifest,
) -> anyhow::Result<BTreeMap<String, String>> {
    manifest.validated_environment().map_err(|_error| {
        anyhow::anyhow!(
            "cannot validate generated Compose environment; run Stackstead repair; details withheld"
        )
    })
}

fn pin_profiles(
    removed: &mut Vec<String>,
    environment: &mut BTreeMap<String, String>,
    profiles: Option<&str>,
) {
    removed.push("COMPOSE_PROFILES".into());
    environment.remove("COMPOSE_PROFILES");
    if let Some(profiles) = profiles {
        environment.insert("COMPOSE_PROFILES".into(), profiles.into());
    }
}

fn parse_model(output: &[u8]) -> anyhow::Result<Value> {
    serde_json::from_slice(output).map_err(|_error| {
        anyhow::anyhow!("Docker Compose returned invalid normalized model JSON; content withheld")
    })
}

fn project_requirements(
    model: Value,
    hashes: &[u8],
    required: &BTreeMap<String, Role>,
    profiles: Option<&str>,
) -> anyhow::Result<Requirements> {
    if required.is_empty() {
        anyhow::bail!("readiness requires a nonempty service declaration");
    }
    let services = model
        .get("services")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("normalized Compose model has no service map"))?;
    let hashes = parse_service_hashes(hashes)?;
    if services.len() != hashes.len() || services.keys().any(|name| !hashes.contains_key(name)) {
        anyhow::bail!("Compose native service hashes do not match the selected model services");
    }
    let mut resolved = BTreeMap::new();
    for name in required.keys() {
        let service = services.get(name).ok_or_else(|| {
            anyhow::anyhow!(
                "required Compose service `{name}` is missing or inactive under the pinned profiles"
            )
        })?;
        let replicas = service_replicas(service)?;
        let config_hash = hashes.get(name).ok_or_else(|| {
            anyhow::anyhow!("required Compose service has no native configuration hash")
        })?;
        resolved.insert(
            name.clone(),
            ServiceRequirement {
                replicas,
                config_hash: config_hash.clone(),
            },
        );
    }
    Ok(Requirements {
        profiles: profiles.map(str::to_owned),
        model_hash: model_hash(model)?,
        services: resolved,
    })
}

fn service_replicas(service: &Value) -> anyhow::Result<u64> {
    let service = service
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("normalized Compose service is not an object"))?;
    if service
        .get("provider")
        .is_some_and(|value| !value.is_null())
    {
        anyhow::bail!("provider-managed Compose services cannot prove owned container readiness");
    }
    let scale = optional_count(service.get("scale"))?;
    let deploy = service.get("deploy").filter(|value| !value.is_null());
    if deploy.is_some_and(|value| !value.is_object()) {
        anyhow::bail!("normalized Compose service has invalid deployment metadata");
    }
    let replicas = optional_count(deploy.and_then(|value| value.get("replicas")))?;
    if scale
        .zip(replicas)
        .is_some_and(|(scale, replicas)| scale != replicas)
    {
        anyhow::bail!("normalized Compose scale and deployment replica counts disagree");
    }
    // Compose v5.5.0 convergence calls compose-go v2.14.0 ServiceConfig.GetScale:
    // scale, then deploy.replicas, then 1. Deployment mode does not change this rule.
    let count = scale.or(replicas).unwrap_or(1);
    if count == 0 {
        anyhow::bail!("required Compose service has zero effective replicas");
    }
    Ok(count)
}

fn optional_count(value: Option<&Value>) -> anyhow::Result<Option<u64>> {
    value
        .filter(|value| !value.is_null())
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                anyhow::anyhow!("normalized Compose service has an invalid replica count")
            })
        })
        .transpose()
}

fn parse_service_hashes(output: &[u8]) -> anyhow::Result<BTreeMap<String, String>> {
    let output = std::str::from_utf8(output)
        .map_err(|_error| anyhow::anyhow!("Compose native service hashes are not UTF-8"))?;
    let mut hashes = BTreeMap::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let Some((name, hash)) = fields.next().zip(fields.next()) else {
            anyhow::bail!("Compose returned malformed native service hashes; content withheld");
        };
        if fields.next().is_some() || !is_sha256(hash) {
            anyhow::bail!("Compose returned malformed native service hashes; content withheld");
        }
        if hashes.insert(name.into(), hash.into()).is_some() {
            anyhow::bail!("Compose returned duplicate native service hashes; content withheld");
        }
    }
    Ok(hashes)
}

fn model_hash(mut model: Value) -> anyhow::Result<String> {
    model.sort_all_objects();
    let canonical = serde_json::to_vec(&model)
        .map_err(|_error| anyhow::anyhow!("cannot canonicalize normalized Compose model"))?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

#[cfg(test)]
#[path = "requirements_tests.rs"]
mod tests;
