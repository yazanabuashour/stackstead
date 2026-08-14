use serde::Deserialize;

use crate::{command, manifest::StacksteadManifest};

use super::{
    docker::{base_args, docker_environment, run_docker_compose},
    model::ServiceObservation,
    ownership::verify_ownership_override,
};

pub fn logs(
    manifest: &StacksteadManifest,
    service: Option<&str>,
    tail: usize,
) -> anyhow::Result<String> {
    let mut args = base_args(manifest);
    args.extend(["logs".into(), format!("--tail={tail}")]);
    if let Some(service) = service {
        args.push(service.into());
    }
    let output = run_docker_compose(manifest, &args)?;
    Ok(String::from_utf8(output.stdout)?)
}

pub fn follow_logs(
    manifest: &StacksteadManifest,
    service: Option<&str>,
    tail: usize,
) -> anyhow::Result<()> {
    verify_ownership_override(manifest)?;
    let mut args = base_args(manifest);
    args.extend(["logs".into(), format!("--tail={tail}"), "--follow".into()]);
    if let Some(service) = service {
        args.push(service.into());
    }
    let (removed, environment) = docker_environment(manifest)?;
    let status = command::status_sanitized(
        "docker",
        &args,
        &manifest.worktree,
        &environment,
        removed.iter().map(String::as_str),
    )?;
    if !status.success() {
        anyhow::bail!("docker compose logs exited with {status}");
    }
    Ok(())
}

pub fn is_running(manifest: &StacksteadManifest) -> anyhow::Result<bool> {
    let mut args = base_args(manifest);
    args.extend([
        "ps".into(),
        "--status".into(),
        "running".into(),
        "--quiet".into(),
    ]);
    run_docker_compose(manifest, &args).map(|output| !output.stdout.is_empty())
}

pub fn service_observations(
    manifest: &StacksteadManifest,
) -> anyhow::Result<Vec<ServiceObservation>> {
    let mut args = base_args(manifest);
    args.extend([
        "ps".into(),
        "--all".into(),
        "--format".into(),
        "json".into(),
    ]);
    let output = run_docker_compose(manifest, &args)?;
    parse_service_observations(&output.stdout)
}

pub(super) fn parse_service_observations(output: &[u8]) -> anyhow::Result<Vec<ServiceObservation>> {
    let output = std::str::from_utf8(output)?.trim();
    if output.is_empty() {
        return Ok(vec![]);
    }
    let values = if output.starts_with('[') {
        serde_json::from_str::<Vec<ComposeServiceObservation>>(output)
    } else {
        output
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<Vec<ComposeServiceObservation>, _>>()
    }
    .map_err(|error| {
        anyhow::anyhow!("Docker Compose returned invalid service status JSON: {error}")
    })?;
    let mut observations = values
        .into_iter()
        .map(|value| {
            let state = value.state.to_ascii_lowercase();
            ServiceObservation {
                service: value.service,
                container: value.name,
                exit_code: value.exit_code.filter(|_| state == "exited"),
                state,
            }
        })
        .collect::<Vec<_>>();
    observations.sort_by(|left, right| {
        (&left.service, &left.container).cmp(&(&right.service, &right.container))
    });
    Ok(observations)
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ComposeServiceObservation {
    name: String,
    service: String,
    state: String,
    exit_code: Option<i64>,
}

pub fn service_is_running(manifest: &StacksteadManifest, service: &str) -> anyhow::Result<bool> {
    let args = service_running_args(manifest, service);
    run_docker_compose(manifest, &args).map(|output| running_service_output(&output.stdout))
}

pub(super) fn service_running_args(manifest: &StacksteadManifest, service: &str) -> Vec<String> {
    let mut args = base_args(manifest);
    args.extend([
        "ps".into(),
        "--status".into(),
        "running".into(),
        "--quiet".into(),
        service.into(),
    ]);
    args
}

pub(super) fn running_service_output(output: &[u8]) -> bool {
    !output.iter().all(u8::is_ascii_whitespace)
}

pub fn endpoint_is_published(
    manifest: &StacksteadManifest,
    service: &str,
    container_port: u16,
    host: &str,
    host_port: u16,
) -> anyhow::Result<bool> {
    let mut args = base_args(manifest);
    args.extend(["port".into(), service.into(), container_port.to_string()]);
    let output = run_docker_compose(manifest, &args)?;
    let published = String::from_utf8(output.stdout)?;
    Ok(published
        .lines()
        .any(|endpoint| endpoint_matches(endpoint, host, host_port)))
}

pub fn ensure_endpoint_published(
    manifest: &StacksteadManifest,
    service: &str,
    container_port: u16,
    host: &str,
    host_port: u16,
) -> anyhow::Result<()> {
    if endpoint_is_published(manifest, service, container_port, host, host_port)? {
        return Ok(());
    }
    anyhow::bail!(
        "Compose service `{service}` does not publish container port {container_port} on the manifest endpoint {host}:{host_port}"
    )
}

pub(super) fn endpoint_matches(endpoint: &str, host: &str, port: u16) -> bool {
    let Ok(published) = endpoint.trim().parse::<std::net::SocketAddr>() else {
        return false;
    };
    if published.port() != port {
        return false;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(expected) => published.ip() == expected,
        Err(_) if host.eq_ignore_ascii_case("localhost") => matches!(
            published.ip(),
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
                | std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
        ),
        Err(_) => false,
    }
}

#[cfg(test)]
pub(super) fn endpoint_port(endpoint: &str) -> Option<u16> {
    endpoint.trim().rsplit_once(':')?.1.parse().ok()
}

pub fn postgres_is_ready(
    manifest: &StacksteadManifest,
    service: &str,
    user: &str,
    database: &str,
) -> bool {
    let mut args = base_args(manifest);
    args.extend([
        "exec".into(),
        "-T".into(),
        service.into(),
        "pg_isready".into(),
        "-U".into(),
        user.into(),
        "-d".into(),
        database.into(),
    ]);
    run_docker_compose(manifest, &args).is_ok()
}
