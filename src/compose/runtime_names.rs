use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use crate::manifest::StacksteadManifest;

use super::{
    resource_config::{contains_interpolation, is_null_or_tagged_null, resource_is_external},
    yaml::yaml_field,
};

type ExpectedRuntimeNames = Vec<(String, String, String, BTreeSet<String>)>;

pub(super) fn expected_runtime_names(
    manifest: &StacksteadManifest,
) -> anyhow::Result<ExpectedRuntimeNames> {
    let mut services = BTreeMap::<String, Option<String>>::new();
    let mut networks = BTreeMap::<String, (bool, Option<String>)>::new();
    let mut volumes = BTreeMap::<String, (bool, Option<String>)>::new();
    for file in &manifest.compose_files {
        let mut document: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(file)?)?;
        document.apply_merge()?;
        collect_runtime_names(&document, "networks", &mut networks, file)?;
        collect_runtime_names(&document, "volumes", &mut volumes, file)?;
        collect_service_names(&document, &mut services, file)?;
    }
    let mut container_names = BTreeSet::new();
    for (service, custom) in services {
        if let Some(custom) = custom {
            container_names.insert(custom);
        } else {
            container_names.insert(format!("{}-{service}-1", manifest.compose_project));
            container_names.insert(format!("{}_{service}_1", manifest.compose_project));
        }
    }
    let mut network_names = BTreeSet::from([
        format!("{}_default", manifest.compose_project),
        format!("{}-default", manifest.compose_project),
    ]);
    for (name, (managed, custom)) in networks {
        if managed {
            network_names
                .insert(custom.unwrap_or_else(|| format!("{}_{name}", manifest.compose_project)));
        }
    }
    let volume_names = volumes
        .into_iter()
        .filter(|(_, (managed, _))| *managed)
        .map(|(name, (_, custom))| {
            custom.unwrap_or_else(|| format!("{}_{name}", manifest.compose_project))
        })
        .collect();
    Ok(vec![
        (
            "container".into(),
            "{{.Names}}".into(),
            ".Config.Labels".into(),
            container_names,
        ),
        (
            "network".into(),
            "{{.Name}}".into(),
            ".Labels".into(),
            network_names,
        ),
        (
            "volume".into(),
            "{{.Name}}".into(),
            ".Labels".into(),
            volume_names,
        ),
    ])
}

fn collect_service_names(
    document: &serde_yaml::Value,
    services: &mut BTreeMap<String, Option<String>>,
    file: &Path,
) -> anyhow::Result<()> {
    let Some(values) = yaml_field(document, "services") else {
        return Ok(());
    };
    let values = values.as_mapping().ok_or_else(|| {
        anyhow::anyhow!(
            "{} has a services value that is not a mapping",
            file.display()
        )
    })?;
    for (name, value) in values {
        let name = name.as_str().ok_or_else(|| {
            anyhow::anyhow!(
                "{} contains a non-string Compose service name",
                file.display()
            )
        })?;
        let value = value.as_mapping().ok_or_else(|| {
            anyhow::anyhow!(
                "Compose service `{name}` in {} is not a mapping",
                file.display()
            )
        })?;
        let custom = match value.get(serde_yaml::Value::String("container_name".into())) {
            None => services.get(name).cloned().flatten(),
            Some(value) if is_null_or_tagged_null(value) => None,
            Some(value) => Some(value.as_str().map(str::to_owned).ok_or_else(|| {
                anyhow::anyhow!(
                    "Compose service `{name}` in {} has a non-string container_name",
                    file.display()
                )
            })?),
        };
        if custom.as_deref().is_some_and(contains_interpolation) {
            anyhow::bail!(
                "Compose service `{name}` in {} uses interpolation in container_name; Stackstead requires a literal name for ownership attestation",
                file.display()
            );
        }
        services.insert(name.into(), custom);
    }
    Ok(())
}

fn collect_runtime_names(
    document: &serde_yaml::Value,
    field: &str,
    output: &mut BTreeMap<String, (bool, Option<String>)>,
    file: &Path,
) -> anyhow::Result<()> {
    let Some(values) = yaml_field(document, field) else {
        return Ok(());
    };
    let values = values.as_mapping().ok_or_else(|| {
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
        let managed = !resource_is_external(mapping, field, name, file)?;
        let custom = mapping
            .and_then(|mapping| mapping.get(serde_yaml::Value::String("name".into())))
            .filter(|value| !is_null_or_tagged_null(value))
            .map(|value| {
                value.as_str().map(str::to_owned).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Compose {field} `{name}` in {} has a non-string name",
                        file.display()
                    )
                })
            })
            .transpose()?;
        if custom.as_deref().is_some_and(contains_interpolation) {
            anyhow::bail!(
                "Compose {field} `{name}` in {} uses interpolation in name; Stackstead requires a literal name for ownership attestation",
                file.display()
            );
        }
        if managed && custom.is_some() {
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
        output.insert(name.into(), (managed, custom));
    }
    Ok(())
}
