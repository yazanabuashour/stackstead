use std::path::Path;

use super::{
    model::{FixedPort, HostBinding},
    yaml::{parse_compose_port, port_declarations, yaml_field},
};

pub fn detect_fixed_host_ports(contents: &str) -> Vec<FixedPort> {
    contents
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("published:") {
                let raw = value.trim();
                let published = raw.trim_matches(['\'', '"']);
                return published.parse::<u16>().ok().map(|host_port| FixedPort {
                    file_line: index.saturating_add(1),
                    host_port,
                    mapping: format!("published: {raw}"),
                });
            }
            let mapping = trimmed.strip_prefix('-')?.trim().trim_matches(['\'', '"']);
            if mapping.contains("${") || mapping.contains("{{") {
                return None;
            }
            let parts = mapping.split(':').collect::<Vec<_>>();
            let host = match parts.as_slice() {
                [host, container] if container_port(container).is_some() => *host,
                [ip, host, container]
                    if (ip.parse::<std::net::IpAddr>().is_ok() || *ip == "localhost")
                        && container_port(container).is_some() =>
                {
                    *host
                }
                _ => return None,
            };
            host.parse::<u16>().ok().map(|host_port| FixedPort {
                file_line: index.saturating_add(1),
                host_port,
                mapping: mapping.to_string(),
            })
        })
        .collect()
}

fn container_port(value: &str) -> Option<u16> {
    value.split('/').next()?.parse().ok()
}

pub fn fixed_ports_in_file(path: &Path) -> anyhow::Result<Vec<FixedPort>> {
    Ok(detect_fixed_host_ports(&std::fs::read_to_string(path)?))
}

pub fn unbound_ports_in_file(path: &Path) -> anyhow::Result<Vec<(String, u16)>> {
    let document: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(path)?)?;
    let Some(services) = yaml_field(&document, "services").and_then(serde_yaml::Value::as_mapping)
    else {
        return Ok(vec![]);
    };
    let mut unbound = Vec::new();
    for (service, value) in services {
        let Some(service) = service.as_str() else {
            continue;
        };
        let Some(ports) = yaml_field(value, "ports").and_then(serde_yaml::Value::as_sequence)
        else {
            continue;
        };
        for value in ports {
            if let Some((container, HostBinding::Missing, _, _)) = parse_compose_port(value) {
                unbound.push((service.into(), container));
            }
        }
    }
    Ok(unbound)
}

pub fn all_interface_ports_in_file(path: &Path) -> anyhow::Result<Vec<(String, u16)>> {
    let document: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(path)?)?;
    let mut exposed = Vec::new();
    for declaration in port_declarations(&document, path)? {
        let all_interfaces = declaration.host_ip.as_deref().is_none_or(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_unspecified())
        });
        if all_interfaces {
            exposed.push((declaration.name, declaration.container_port));
        }
    }
    Ok(exposed)
}
