use std::net::IpAddr;

use crate::{error::StacksteadError, manifest::StacksteadManifest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTarget {
    pub contract_key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopbackEndpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchEndpoint {
    pub contract_key: String,
    pub endpoint: LoopbackEndpoint,
}

pub fn resolve(manifest: &StacksteadManifest, service: Option<&str>) -> anyhow::Result<OpenTarget> {
    if let Some(service) = service {
        if let Some(url) = manifest.urls.get(service) {
            return Ok(OpenTarget {
                contract_key: service.into(),
                value: url.clone(),
            });
        }
        if let Some(port) = manifest.ports.get(service) {
            return Ok(OpenTarget {
                contract_key: service.into(),
                value: format!("127.0.0.1:{port}"),
            });
        }
        return Err(StacksteadError::UnknownService(service.into()).into());
    }
    manifest
        .urls
        .iter()
        .next()
        .map(|(contract_key, value)| OpenTarget {
            contract_key: contract_key.clone(),
            value: value.clone(),
        })
        .ok_or_else(|| {
            anyhow::anyhow!("stackstead has no configured HTTP URLs; run `stackstead inspect`")
        })
}

pub fn launch(url: &str) -> anyhow::Result<()> {
    opener::open(url).map_err(|error| anyhow::anyhow!("could not open {url}: {error}"))
}

pub fn is_loopback_url(url: &str) -> bool {
    parse_loopback_endpoint(url).is_some()
}

pub fn launch_endpoint(
    target: &OpenTarget,
    manifest: &StacksteadManifest,
) -> anyhow::Result<Option<LaunchEndpoint>> {
    if !target.value.starts_with("http://") && !target.value.starts_with("https://") {
        return Ok(None);
    }
    let endpoint = parse_loopback_endpoint(&target.value).ok_or_else(|| {
        anyhow::anyhow!(
            "refusing to open non-loopback URL `{}`; use --print to inspect it without launching a browser",
            target.value
        )
    })?;
    let matching = manifest
        .ports
        .iter()
        .filter(|(_, port)| **port == endpoint.port)
        .map(|(key, _)| key)
        .collect::<Vec<_>>();
    let [contract_key] = matching.as_slice() else {
        anyhow::bail!(
            "URL `{}` must target exactly one manifest port contract; found {} matches",
            target.value,
            matching.len()
        );
    };
    if contract_key.as_str() != target.contract_key {
        anyhow::bail!(
            "URL contract `{}` targets manifest port contract `{}`; refusing to open a different service",
            target.contract_key,
            contract_key
        );
    }
    Ok(Some(LaunchEndpoint {
        contract_key: (*contract_key).clone(),
        endpoint,
    }))
}

fn parse_loopback_endpoint(url: &str) -> Option<LoopbackEndpoint> {
    let (scheme, remainder) = url.split_once("://")?;
    let default_port = match scheme {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    let authority = remainder.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, suffix) = bracketed.split_once(']')?;
        let port = match suffix {
            "" => default_port,
            suffix => suffix.strip_prefix(':')?.parse().ok()?,
        };
        (host, port)
    } else {
        match authority.split_once(':') {
            Some((host, port)) if !host.is_empty() && !port.contains(':') => {
                (host, port.parse().ok()?)
            }
            Some(_) => return None,
            None => (authority, default_port),
        }
    };
    let host = if host.eq_ignore_ascii_case("localhost") {
        "localhost".into()
    } else {
        let address = host.parse::<IpAddr>().ok()?;
        address.is_loopback().then(|| address.to_string())?
    };
    Some(LoopbackEndpoint { host, port })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;

    use super::*;
    use crate::manifest::{ManifestStatus, SourceOwnership, StacksteadManifest};

    fn manifest() -> StacksteadManifest {
        let root = std::path::PathBuf::from("/tmp/cell");
        StacksteadManifest {
            kind: "StacksteadManifest".into(),
            version: "2".into(),
            stackstead_id: "a-a111".into(),
            slug: "a".into(),
            short_id: "a111".into(),
            runtime_token: "0123456789abcdef0123456789abcdef".into(),
            project: "demo".into(),
            branch: "a".into(),
            base: "main".into(),
            source_ownership: SourceOwnership::Stackstead,
            repo_root: "/tmp/repo".into(),
            project_state_root: "/tmp/state".into(),
            stackstead_root: root.clone(),
            worktree: root.join("source"),
            state_dir: root.join("state"),
            port_lease_state_dir: Some("/tmp/leases".into()),
            compose_project: "demo-a-a111".into(),
            compose_files: vec![],
            ports: BTreeMap::from([("dashboard".into(), 39000)]),
            container_ports: BTreeMap::from([("dashboard".into(), 3000)]),
            urls: BTreeMap::from([("dashboard".into(), "http://127.0.0.1:39000".into())]),
            env_file: root.join(".env"),
            agent_context: root.join("context"),
            pointer_file: root.join("pointer"),
            event_log: root.join("events"),
            env_keys: vec![],
            status: ManifestStatus::default(),
            database: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn resolution_retains_the_contract_key_and_value() {
        let manifest = manifest();
        assert_eq!(
            resolve(&manifest, None).unwrap(),
            OpenTarget {
                contract_key: "dashboard".into(),
                value: "http://127.0.0.1:39000".into()
            }
        );
        assert_eq!(
            resolve(&manifest, Some("dashboard")).unwrap().contract_key,
            "dashboard"
        );
    }

    #[test]
    fn parses_only_loopback_http_endpoints_with_effective_ports() {
        for (url, host, port) in [
            ("http://127.0.0.1:3000/path", "127.0.0.1", 3000),
            ("https://LOCALHOST/path", "localhost", 443),
            ("http://[::1]:8080", "::1", 8080),
            ("http://127.0.0.1", "127.0.0.1", 80),
        ] {
            assert_eq!(
                parse_loopback_endpoint(url),
                Some(LoopbackEndpoint {
                    host: host.into(),
                    port
                }),
                "rejected {url}"
            );
        }
        for url in [
            "https://example.com",
            "http://user:pass@localhost",
            "http://localhost:",
            "http://::1:8080",
            "file:///tmp/page.html",
        ] {
            assert_eq!(parse_loopback_endpoint(url), None, "accepted {url}");
        }
    }

    #[test]
    fn launcher_requires_the_url_port_to_match_its_contract_key() {
        let mut manifest = manifest();
        let good = resolve(&manifest, Some("dashboard")).unwrap();
        assert_eq!(
            launch_endpoint(&good, &manifest)
                .unwrap()
                .unwrap()
                .endpoint
                .port,
            39000
        );

        let stale = OpenTarget {
            contract_key: "dashboard".into(),
            value: "http://127.0.0.1:39001".into(),
        };
        assert!(
            launch_endpoint(&stale, &manifest)
                .unwrap_err()
                .to_string()
                .contains("found 0 matches")
        );

        manifest.ports.insert("api".into(), 39001);
        let wrong_service = OpenTarget {
            contract_key: "dashboard".into(),
            value: "http://127.0.0.1:39001".into(),
        };
        assert!(
            launch_endpoint(&wrong_service, &manifest)
                .unwrap_err()
                .to_string()
                .contains("different service")
        );
    }

    #[test]
    fn raw_ports_are_non_launching_metadata() {
        let target = OpenTarget {
            contract_key: "dashboard".into(),
            value: "127.0.0.1:39000".into(),
        };
        assert_eq!(launch_endpoint(&target, &manifest()).unwrap(), None);
    }
}
