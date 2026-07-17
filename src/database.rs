use std::{
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

use crate::{
    compose,
    manifest::{ComponentStatus, StacksteadManifest},
};
use chrono::{DateTime, Utc};

const PROBE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct DatabaseStatusOutput {
    pub stackstead_id: String,
    pub strategy: String,
    pub service: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub reachable: bool,
    pub seed_status: ComponentStatus,
    pub last_seed_at: Option<DateTime<Utc>>,
}

pub fn status(manifest: &StacksteadManifest) -> anyhow::Result<DatabaseStatusOutput> {
    let database = manifest
        .database
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("stackstead has no configured Postgres database"))?;
    Ok(DatabaseStatusOutput {
        stackstead_id: manifest.stackstead_id.clone(),
        strategy: database.strategy.clone(),
        service: database.service.clone(),
        host: database.host.clone(),
        port: database.port,
        database: database.database.clone(),
        reachable: reachable(&database.host, database.port, PROBE_INTERVAL),
        seed_status: database.seed_status,
        last_seed_at: database.last_seed_at,
    })
}

pub fn identity_status(manifest: &StacksteadManifest) -> ComponentStatus {
    let runtime_status = match compose::is_running(manifest) {
        Ok(true) => ComponentStatus::Running,
        Ok(false) => ComponentStatus::Stopped,
        Err(_) => ComponentStatus::Unknown,
    };
    live_status(manifest, runtime_status)
}

pub fn live_status(
    manifest: &StacksteadManifest,
    runtime_status: ComponentStatus,
) -> ComponentStatus {
    let Some(database) = &manifest.database else {
        return ComponentStatus::Unknown;
    };
    let Some(container_port) = manifest.container_ports.get(&database.service).copied() else {
        return ComponentStatus::Unknown;
    };
    classify_live_status(
        runtime_status,
        || {
            compose::endpoint_is_published(
                manifest,
                &database.service,
                container_port,
                &database.host,
                database.port,
            )
        },
        || reachable(&database.host, database.port, PROBE_INTERVAL),
    )
}

fn classify_live_status(
    runtime_status: ComponentStatus,
    published: impl FnOnce() -> anyhow::Result<bool>,
    tcp_reachable: impl FnOnce() -> bool,
) -> ComponentStatus {
    if runtime_status == ComponentStatus::Unknown {
        return ComponentStatus::Unknown;
    }
    if runtime_status != ComponentStatus::Running {
        return ComponentStatus::Unreachable;
    }
    match published() {
        Ok(true) if tcp_reachable() => ComponentStatus::Reachable,
        Ok(_) => ComponentStatus::Unreachable,
        Err(_) => ComponentStatus::Unknown,
    }
}

pub fn reachable(host: &str, port: u16, timeout: Duration) -> bool {
    format!("{host}:{port}")
        .parse::<SocketAddr>()
        .is_ok_and(|address| TcpStream::connect_timeout(&address, timeout).is_ok())
}

pub fn wait_until_postgres_ready<F>(
    host: &str,
    port: u16,
    timeout: Duration,
    mut probe: F,
) -> anyhow::Result<()>
where
    F: FnMut() -> bool,
{
    if wait_until_ready(
        timeout,
        || reachable(host, port, PROBE_INTERVAL),
        &mut probe,
    ) {
        return Ok(());
    }
    anyhow::bail!(
        "Postgres did not become reachable at {host}:{port} and report ready within {timeout:?}"
    )
}

fn wait_until_ready<H, P>(timeout: Duration, mut host_reachable: H, mut probe: P) -> bool
where
    H: FnMut() -> bool,
    P: FnMut() -> bool,
{
    let start = Instant::now();
    while start.elapsed() < timeout {
        if host_reachable() && probe() {
            return true;
        }
        std::thread::sleep(PROBE_INTERVAL);
    }
    false
}

#[cfg(test)]
mod tests {
    use crate::test_support::TestResultExt as _;
    use std::cell::Cell;
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn postgres_readiness_requires_the_recorded_host_port() -> anyhow::Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).test()?;
        let port = listener.local_addr().test()?.port();
        assert!(
            wait_until_postgres_ready("127.0.0.1", port, Duration::from_millis(10), || true)
                .is_ok()
        );

        let probed = Cell::new(false);
        assert!(!wait_until_ready(
            Duration::from_millis(10),
            || false,
            || {
                probed.set(true);
                false
            }
        ));
        assert!(
            !probed.get(),
            "Postgres probe ran without a reachable host port"
        );
        Ok(())
    }

    #[test]
    fn live_status_requires_runtime_publication_and_tcp_reachability() -> anyhow::Result<()> {
        let invoked = Cell::new(false);
        assert_eq!(
            classify_live_status(
                ComponentStatus::Stopped,
                || {
                    invoked.set(true);
                    Ok(false)
                },
                || {
                    invoked.set(true);
                    false
                },
            ),
            ComponentStatus::Unreachable
        );
        assert!(!invoked.replace(false));
        assert_eq!(
            classify_live_status(
                ComponentStatus::Unknown,
                || {
                    invoked.set(true);
                    Ok(false)
                },
                || {
                    invoked.set(true);
                    false
                },
            ),
            ComponentStatus::Unknown
        );
        assert!(!invoked.replace(false));
        assert_eq!(
            classify_live_status(
                ComponentStatus::Running,
                || Ok(false),
                || {
                    invoked.set(true);
                    false
                }
            ),
            ComponentStatus::Unreachable
        );
        assert!(!invoked.replace(false));
        assert_eq!(
            classify_live_status(
                ComponentStatus::Running,
                || Err(anyhow::anyhow!("Docker unavailable")),
                || {
                    invoked.set(true);
                    false
                }
            ),
            ComponentStatus::Unknown
        );
        assert!(!invoked.get());
        assert_eq!(
            classify_live_status(ComponentStatus::Running, || Ok(true), || false),
            ComponentStatus::Unreachable
        );
        assert_eq!(
            classify_live_status(ComponentStatus::Running, || Ok(true), || true),
            ComponentStatus::Reachable
        );
        Ok(())
    }
}
