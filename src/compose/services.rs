use crate::{command, manifest::StacksteadManifest};

use super::{
    docker::{base_args, docker_environment, run_docker_compose, sanitize_generated_error},
    model::ServiceObservation,
    ownership::verify_ownership_override,
    resources::owned_service_observations,
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

/// The optional deadline includes all independent claim, inventory, and ownership reads.
pub fn service_observations(
    manifest: &StacksteadManifest,
    deadline: Option<std::time::Instant>,
) -> anyhow::Result<Vec<ServiceObservation>> {
    let generated = manifest.validated_environment().map_err(|_error| {
        anyhow::anyhow!(
            "cannot validate generated Compose environment; run Stackstead repair; details withheld"
        )
    })?;
    verify_ownership_override(manifest)
        .and_then(|()| owned_service_observations(manifest, deadline))
        .map_err(|error| sanitize_generated_error(&error, &generated))
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
