use std::{collections::HashMap, path::Path};

use super::model::HostBinding;

#[derive(Debug)]
pub(super) struct PortDeclaration {
    pub name: String,
    pub service: String,
    pub container_port: u16,
    pub host_binding: HostBinding,
    pub protocol: String,
    pub host_ip: Option<String>,
}

pub(super) fn port_declarations(
    document: &serde_yaml::Value,
    file: &Path,
) -> anyhow::Result<Vec<PortDeclaration>> {
    let mut document = document.clone();
    document.apply_merge().map_err(|error| {
        anyhow::anyhow!(
            "cannot resolve YAML merge keys in Compose file {}: {error}",
            file.display()
        )
    })?;
    if yaml_field(&document, "include").is_some() {
        anyhow::bail!(
            "Compose `include` is not supported in {}; list each file explicitly in runtime.files",
            file.display()
        );
    }
    let Some(services_value) = yaml_field(&document, "services") else {
        return Ok(vec![]);
    };
    let services = services_value.as_mapping().ok_or_else(|| {
        anyhow::anyhow!(
            "{} has a services value that is not a mapping",
            file.display()
        )
    })?;
    let mut declarations = Vec::new();
    let mut service_counts = HashMap::new();
    for (service, value) in services {
        let service = service.as_str().ok_or_else(|| {
            anyhow::anyhow!(
                "{} contains a non-string Compose service name",
                file.display()
            )
        })?;
        if yaml_field(value, "extends").is_some() {
            anyhow::bail!(
                "Compose service `{service}` in {} uses unsupported `extends`; list each Compose file explicitly in runtime.files",
                file.display()
            );
        }
        let Some(raw_ports) = yaml_field(value, "ports") else {
            continue;
        };
        let ports = raw_ports.as_sequence().ok_or_else(|| {
            anyhow::anyhow!(
                "Compose service `{service}` has a ports value that is not a YAML sequence in {}",
                file.display()
            )
        })?;
        for value in ports {
            let (container_port, host_binding, protocol, host_ip) = parse_compose_port(value).ok_or_else(|| {
                anyhow::anyhow!(
                    "unsupported or ambiguous port mapping for Compose service `{service}` in {}: {value:?}",
                    file.display()
                )
            })?;
            if !matches!(protocol.as_str(), "" | "/tcp") {
                anyhow::bail!(
                    "Compose service `{service}` publishes unsupported protocol `{}` in {}; Stackstead exposes TCP ports only",
                    protocol.trim_start_matches('/'),
                    file.display()
                );
            }
            let count = service_counts.entry(service.to_owned()).or_insert(0usize);
            *count = count.saturating_add(1);
            let name = if *count == 1 {
                service.to_owned()
            } else {
                format!("{service}-{container_port}")
            };
            declarations.push(PortDeclaration {
                name,
                service: service.to_owned(),
                container_port,
                host_binding,
                protocol,
                host_ip,
            });
        }
    }
    Ok(declarations)
}

pub(super) fn yaml_field<'a>(
    value: &'a serde_yaml::Value,
    key: &str,
) -> Option<&'a serde_yaml::Value> {
    value
        .as_mapping()?
        .get(serde_yaml::Value::String(key.to_owned()))
}

pub(super) fn parse_compose_port(
    value: &serde_yaml::Value,
) -> Option<(u16, HostBinding, String, Option<String>)> {
    if let Some(mapping) = value.as_mapping() {
        let container = yaml_u16(mapping.get(serde_yaml::Value::String("target".into()))?)?;
        let host = match mapping.get(serde_yaml::Value::String("published".into())) {
            None => HostBinding::Missing,
            Some(value) => match yaml_u16(value) {
                Some(port) => HostBinding::Fixed(port),
                None => HostBinding::Variable(compose_variable(value.as_str()?)?),
            },
        };
        let protocol = mapping
            .get(serde_yaml::Value::String("protocol".into()))
            .and_then(serde_yaml::Value::as_str)
            .filter(|protocol| *protocol != "tcp")
            .map_or_else(String::new, |protocol| format!("/{protocol}"));
        let host_ip = mapping
            .get(serde_yaml::Value::String("host_ip".into()))
            .and_then(serde_yaml::Value::as_str);
        if host_ip.is_some_and(|host_ip| !safe_host_ip(host_ip)) {
            return None;
        }
        return Some((container, host, protocol, host_ip.map(str::to_owned)));
    }
    let value = value.as_str()?;
    let (mapping, protocol) = value
        .split_once('/')
        .map_or((value, String::new()), |(mapping, protocol)| {
            (mapping, format!("/{protocol}"))
        });
    let Some((host, container)) = rsplit_port_separator(mapping) else {
        return Some((mapping.parse().ok()?, HostBinding::Missing, protocol, None));
    };
    let mut host_ip = None;
    let host = if host.starts_with("${") {
        host
    } else if let Some((address, published)) = rsplit_port_separator(host) {
        let address = address.trim_matches(['[', ']']);
        if safe_host_ip(address) {
            host_ip = Some(address.to_owned());
            published
        } else {
            return None;
        }
    } else {
        host
    };
    let host = host.parse().map_or_else(
        |_| compose_variable(host).map(HostBinding::Variable),
        |port| Some(HostBinding::Fixed(port)),
    )?;
    Some((container.parse().ok()?, host, protocol, host_ip))
}

fn rsplit_port_separator(value: &str) -> Option<(&str, &str)> {
    let mut braces = 0usize;
    let mut brackets = 0usize;
    for (index, character) in value.char_indices().rev() {
        match character {
            '}' => braces = braces.saturating_add(1),
            '{' => braces = braces.saturating_sub(1),
            ']' => brackets = brackets.saturating_add(1),
            '[' => brackets = brackets.saturating_sub(1),
            ':' if braces == 0 && brackets == 0 => {
                let (left, right) = value.split_at(index);
                return Some((left, right.strip_prefix(':')?));
            }
            _ => {}
        }
    }
    None
}

fn compose_variable(value: &str) -> Option<String> {
    let raw = value.strip_prefix('$')?;
    let (candidate, suffix) = if let Some(braced) = raw.strip_prefix('{') {
        let inner = braced.strip_suffix('}')?;
        let name_len = inner
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .count();
        (inner.get(..name_len)?, inner.get(name_len..)?)
    } else {
        (raw, "")
    };
    let name = candidate
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect::<String>();
    let safe_suffix = suffix.is_empty()
        || suffix
            .strip_prefix(":-")
            .or_else(|| suffix.strip_prefix('-'))
            .is_some_and(|default| {
                !default.is_empty() && default.chars().all(|c| c.is_ascii_digit())
            });
    (name == candidate
        && !name.is_empty()
        && safe_suffix
        && name
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic() || character == '_'))
    .then_some(name)
}

fn safe_host_ip(value: &str) -> bool {
    value == "localhost"
        || value
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback() || address.is_unspecified())
}

fn yaml_u16(value: &serde_yaml::Value) -> Option<u16> {
    value
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .or_else(|| value.as_str()?.parse().ok())
}

pub(super) fn env_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) fn http_port(service: &str, container: u16, protocol: &str) -> bool {
    matches!(protocol, "" | "/tcp")
        && matches!(container, 80 | 3000 | 5173 | 8000 | 8080)
        && ["app", "api", "backend", "frontend", "server", "web"]
            .iter()
            .any(|candidate| service.eq_ignore_ascii_case(candidate))
}
