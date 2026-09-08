use std::{collections::BTreeMap, path::PathBuf};

use super::{
    model::{ComposePortTarget, HostBinding},
    yaml::port_declarations,
};

pub fn validate_port_contract(
    files: &[PathBuf],
    expected_containers: &BTreeMap<String, u16>,
    generated_environment: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    let mut actual = BTreeMap::new();
    let mut variable_owners = BTreeMap::new();
    for file in files {
        let document: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(file)?)
            .map_err(|error| anyhow::anyhow!("cannot parse {}: {error}", file.display()))?;
        for declaration in port_declarations(&document, file)? {
            let loopback = declaration.host_ip.as_deref().is_some_and(|host| {
                host == "localhost"
                    || host
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|address| address.is_loopback())
            });
            let variable = match declaration.host_binding {
                HostBinding::Variable(variable) => variable,
                HostBinding::Fixed(port) => anyhow::bail!(
                    "fixed host port {port} found for Compose port `{}` in {}; use a generated environment variable",
                    declaration.name,
                    file.display()
                ),
                HostBinding::Missing => anyhow::bail!(
                    "Compose port `{}` in {} has no deterministic host binding",
                    declaration.name,
                    file.display()
                ),
            };
            if !loopback {
                anyhow::bail!(
                    "Compose port `{}` in {} must bind a loopback host such as 127.0.0.1",
                    declaration.name,
                    file.display()
                );
            }
            let generated = generated_environment.get(&variable).ok_or_else(|| {
                anyhow::anyhow!(
                    "Compose port `{}` uses `${{{variable}}}`, but env.generate does not define `{variable}`",
                    declaration.name
                )
            })?;
            let contract_name = port_template_name(generated).ok_or_else(|| {
                anyhow::anyhow!(
                    "env.generate.{variable} must be exactly a `{{{{ ports.<name> }}}}` allocation"
                )
            })?;
            if let Some(owner) = variable_owners.insert(variable.clone(), contract_name.clone()) {
                anyhow::bail!(
                    "Compose ports `{owner}` and `{contract_name}` both use `{variable}`; every published port needs its own host variable"
                );
            }
            let contract = (declaration.container_port, variable, file.clone());
            if actual.insert(contract_name.clone(), contract).is_some() {
                anyhow::bail!(
                    "Compose port `{contract_name}` is declared more than once across runtime files"
                );
            }
        }
    }
    if actual.keys().ne(expected_containers.keys()) {
        anyhow::bail!(
            "Compose published-port names do not match the durable Stackstead contract; expected {:?}, found {:?}",
            expected_containers.keys().collect::<Vec<_>>(),
            actual.keys().collect::<Vec<_>>()
        );
    }
    for (name, expected_container) in expected_containers {
        let (container, variable, file) = actual.get(name).ok_or_else(|| {
            anyhow::anyhow!("Compose port `{name}` disappeared during validation")
        })?;
        if container != expected_container {
            anyhow::bail!(
                "Compose port `{name}` publishes container port {container}, expected {expected_container} in {}",
                file.display()
            );
        }
        debug_assert_eq!(
            port_template_name(&generated_environment[variable]).as_deref(),
            Some(name.as_str())
        );
    }
    Ok(())
}

pub fn resolve_port_target(
    files: &[PathBuf],
    expected_containers: &BTreeMap<String, u16>,
    generated_environment: &BTreeMap<String, String>,
    contract_key: &str,
) -> anyhow::Result<ComposePortTarget> {
    let expected_container = expected_containers.get(contract_key).ok_or_else(|| {
        anyhow::anyhow!("port contract has no `{contract_key}` container mapping")
    })?;
    validate_port_contract(files, expected_containers, generated_environment)?;
    let mut target = None;
    for file in files {
        let document: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(file)?)
            .map_err(|error| anyhow::anyhow!("cannot parse {}: {error}", file.display()))?;
        for declaration in port_declarations(&document, file)? {
            let HostBinding::Variable(variable) = declaration.host_binding else {
                continue;
            };
            if generated_environment
                .get(&variable)
                .and_then(|value| port_template_name(value))
                .as_deref()
                != Some(contract_key)
            {
                continue;
            }
            let candidate = ComposePortTarget {
                service: declaration.service,
                container_port: declaration.container_port,
            };
            if target.replace(candidate).is_some() {
                anyhow::bail!("port contract `{contract_key}` maps to more than one Compose port");
            }
        }
    }
    let target = target.ok_or_else(|| {
        anyhow::anyhow!("port contract `{contract_key}` has no direct Compose port mapping")
    })?;
    if target.container_port != *expected_container {
        anyhow::bail!(
            "port contract `{contract_key}` maps to container port {}, expected {expected_container}",
            target.container_port
        );
    }
    Ok(target)
}

fn port_template_name(value: &str) -> Option<String> {
    value
        .trim()
        .strip_prefix("{{")?
        .strip_suffix("}}")?
        .trim()
        .strip_prefix("ports.")
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}
