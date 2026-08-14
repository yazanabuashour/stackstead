use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use super::{
    resource_config::{contains_interpolation, is_null_or_tagged_null, resource_is_external},
    yaml::yaml_field,
};

#[derive(Default)]
pub(super) struct OwnershipModel {
    pub services: BTreeSet<String>,
    pub networks: BTreeSet<String>,
    pub volumes: BTreeSet<String>,
}

pub(super) fn ownership_model(files: &[PathBuf]) -> anyhow::Result<OwnershipModel> {
    let mut model = OwnershipModel::default();
    let mut documents = Vec::new();
    let mut network_states = BTreeMap::new();
    let mut volume_states = BTreeMap::new();
    for file in files {
        let mut document: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(file)?)
            .map_err(|error| anyhow::anyhow!("cannot parse {}: {error}", file.display()))?;
        document.apply_merge().map_err(|error| {
            anyhow::anyhow!(
                "cannot resolve YAML merge keys in Compose file {}: {error}",
                file.display()
            )
        })?;
        if yaml_field(&document, "include").is_some() {
            anyhow::bail!(
                "Compose `include` is not supported in {}; Stackstead cannot attest included resources",
                file.display()
            );
        }
        collect_resource_states(&document, file, "networks", &mut network_states)?;
        collect_resource_states(&document, file, "volumes", &mut volume_states)?;
        documents.push((file, document));
    }
    model.networks.extend(
        network_states
            .iter()
            .filter(|(_, managed)| **managed)
            .map(|(name, _)| name.clone()),
    );
    model.volumes.extend(
        volume_states
            .iter()
            .filter(|(_, managed)| **managed)
            .map(|(name, _)| name.clone()),
    );
    let declared_volumes = volume_states.keys().cloned().collect();
    for (file, document) in documents {
        let Some(services_value) = yaml_field(&document, "services") else {
            continue;
        };
        let services = services_value.as_mapping().ok_or_else(|| {
            anyhow::anyhow!(
                "{} has a services value that is not a mapping",
                file.display()
            )
        })?;
        for (name, value) in services {
            let name = name.as_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "{} contains a non-string Compose service name",
                    file.display()
                )
            })?;
            let service = value.as_mapping().ok_or_else(|| {
                anyhow::anyhow!(
                    "Compose service `{name}` in {} is not a mapping",
                    file.display()
                )
            })?;
            if service.contains_key(serde_yaml::Value::String("extends".into())) {
                anyhow::bail!(
                    "Compose service `{name}` in {} uses unsupported `extends`; Stackstead cannot attest inherited resources",
                    file.display()
                );
            }
            if let Some(container_name) =
                service.get(serde_yaml::Value::String("container_name".into()))
                && !is_null_or_tagged_null(container_name)
            {
                let container_name = container_name.as_str().ok_or_else(|| {
                    anyhow::anyhow!(
                        "Compose service `{name}` in {} has a non-string container_name",
                        file.display()
                    )
                })?;
                if contains_interpolation(container_name) {
                    anyhow::bail!(
                        "Compose service `{name}` in {} uses interpolation in container_name; Stackstead requires a literal name for ownership attestation",
                        file.display()
                    );
                }
            }
            validate_service_volumes(name, service, &declared_volumes, file)?;
            model.services.insert(name.into());
        }
    }
    if !model.services.is_empty() && !network_states.contains_key("default") {
        model.networks.insert("default".into());
    }
    Ok(model)
}

fn collect_resource_states(
    document: &serde_yaml::Value,
    file: &Path,
    field: &str,
    output: &mut BTreeMap<String, bool>,
) -> anyhow::Result<()> {
    let Some(value) = yaml_field(document, field) else {
        return Ok(());
    };
    let values = value.as_mapping().ok_or_else(|| {
        anyhow::anyhow!("Compose `{field}` in {} is not a mapping", file.display())
    })?;
    for (name, value) in values {
        let name = name.as_str().ok_or_else(|| {
            anyhow::anyhow!(
                "{} contains a non-string Compose {field} name",
                file.display()
            )
        })?;
        let mapping = value.as_mapping();
        if mapping.is_none() && !value.is_null() {
            anyhow::bail!(
                "Compose {field} `{name}` in {} is not a mapping",
                file.display()
            );
        }
        let external = resource_is_external(mapping, field, name, file)?;
        let custom_name = mapping
            .and_then(|mapping| mapping.get(serde_yaml::Value::String("name".into())))
            .filter(|value| !is_null_or_tagged_null(value))
            .map(|value| {
                value.as_str().ok_or_else(|| {
                    anyhow::anyhow!(
                        "Compose {field} `{name}` in {} has a non-string name",
                        file.display()
                    )
                })
            })
            .transpose()?;
        if custom_name.is_some_and(contains_interpolation) {
            anyhow::bail!(
                "Compose {field} `{name}` in {} uses interpolation in name; Stackstead requires a literal name for ownership attestation",
                file.display()
            );
        }
        if !external && custom_name.is_some() {
            anyhow::bail!(
                "Compose managed {field} `{name}` in {} uses a global custom name; Stackstead requires project-scoped managed resource names",
                file.display()
            );
        }
        if output.contains_key(name) {
            anyhow::bail!(
                "Compose {field} `{name}` is declared in multiple Compose files; consolidate the declaration so Stackstead can attest the effective resource name"
            );
        }
        output.insert(name.into(), !external);
    }
    Ok(())
}

fn validate_service_volumes(
    service_name: &str,
    service: &serde_yaml::Mapping,
    declared_volumes: &BTreeSet<String>,
    file: &Path,
) -> anyhow::Result<()> {
    let Some(volumes) = service
        .get(serde_yaml::Value::String("volumes".into()))
        .and_then(serde_yaml::Value::as_sequence)
    else {
        return Ok(());
    };
    for volume in volumes {
        let source = if let Some(value) = volume.as_str() {
            match value.split_once(':') {
                Some((source, _)) if !source.is_empty() && !source.starts_with(['.', '/']) => {
                    Some(source)
                }
                Some(_) => None,
                None => anyhow::bail!(
                    "Compose service `{service_name}` in {} uses an anonymous volume; declare a named top-level volume so Stackstead can attest it",
                    file.display()
                ),
            }
        } else if let Some(mapping) = volume.as_mapping() {
            let kind = match mapping.get(serde_yaml::Value::String("type".into())) {
                Some(value) => value.as_str().ok_or_else(|| {
                    anyhow::anyhow!(
                        "Compose service `{service_name}` in {} has a non-string volume type",
                        file.display()
                    )
                })?,
                None => "volume",
            };
            if kind == "volume" {
                Some(
                    mapping
                        .get(serde_yaml::Value::String("source".into()))
                        .and_then(serde_yaml::Value::as_str)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Compose service `{service_name}` in {} uses an anonymous volume; declare a named top-level volume so Stackstead can attest it",
                                file.display()
                            )
                        })?,
                )
            } else {
                None
            }
        } else {
            anyhow::bail!(
                "Compose service `{service_name}` in {} has an unsupported volume declaration",
                file.display()
            );
        };
        if let Some(source) = source
            && !declared_volumes.contains(source)
        {
            anyhow::bail!(
                "Compose service `{service_name}` in {} uses named volume `{source}` without a top-level declaration",
                file.display()
            );
        }
    }
    Ok(())
}
