use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use crate::{
    command,
    config::{HealthCheckConfig, HealthConfig},
    manifest::StacksteadManifest,
    template::render_template,
};

pub fn wait(
    config: &HealthConfig,
    manifest: &StacksteadManifest,
    environment: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    if config.checks.is_empty() {
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(config.timeout_seconds);
    loop {
        let failed = failed_checks(config, manifest, environment, deadline);
        if failed.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "health checks did not pass within {}s: {}",
                config.timeout_seconds,
                failed.join(", ")
            );
        }
        std::thread::sleep(
            Duration::from_millis(config.interval_millis)
                .min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

pub fn healthy_passive(
    config: &HealthConfig,
    manifest: &StacksteadManifest,
    environment: &BTreeMap<String, String>,
) -> Option<bool> {
    (!config.checks.is_empty() && config.checks.iter().all(|check| check.url.is_some())).then(
        || {
            failed_checks(
                config,
                manifest,
                environment,
                Instant::now() + Duration::from_secs(2),
            )
            .is_empty()
        },
    )
}

fn failed_checks(
    config: &HealthConfig,
    manifest: &StacksteadManifest,
    environment: &BTreeMap<String, String>,
    deadline: Instant,
) -> Vec<String> {
    config
        .checks
        .iter()
        .filter(|check| {
            !check_passes(
                check,
                manifest,
                environment,
                deadline.saturating_duration_since(Instant::now()),
            )
        })
        .map(|check| check.name.clone())
        .collect()
}

fn check_passes(
    check: &HealthCheckConfig,
    manifest: &StacksteadManifest,
    environment: &BTreeMap<String, String>,
    timeout: Duration,
) -> bool {
    if timeout.is_zero() {
        return false;
    }
    if let Some(url) = &check.url {
        let Ok(url) = render_template(url, &crate::lifecycle::template_context(manifest)) else {
            return false;
        };
        if !crate::open::is_loopback_url(&url) {
            return false;
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(timeout.min(Duration::from_secs(2)))
            .redirects(0)
            .build();
        return match agent.get(&url).call() {
            Ok(response) => response.status() == check.expect_status,
            Err(ureq::Error::Status(status, _)) => status == check.expect_status,
            Err(_) => false,
        };
    }
    matches!(
        command::configured_status_with_timeout(
            &check.command.command,
            check.command.shell,
            &manifest.worktree,
            environment,
            timeout,
        ),
        Ok(Some(status)) if status.success()
    )
}

#[cfg(test)]
mod tests {
    use crate::test_support::TestResultExt as _;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        thread,
    };

    use chrono::Utc;

    use crate::{
        config::{CommandConfig, HealthCheckConfig, HealthConfig},
        manifest::{ManifestStatus, SourceOwnership, StacksteadManifest},
    };

    use super::*;

    fn manifest(port: u16) -> StacksteadManifest {
        StacksteadManifest {
            kind: "StacksteadManifest".into(),
            version: crate::manifest::MANIFEST_VERSION.into(),
            stackstead_id: "feature-a-a111".into(),
            slug: "feature-a".into(),
            short_id: "a111".into(),
            runtime_token: "0123456789abcdef0123456789abcdef".into(),
            project: "demo".into(),
            branch: "feature-a".into(),
            base: "main".into(),
            source_ownership: SourceOwnership::Stackstead,
            repo_root: "/repo".into(),
            project_state_root: "/state".into(),
            stackstead_root: "/state/demo/feature-a-a111".into(),
            worktree: "/tmp".into(),
            state_dir: "/state/demo/feature-a-a111/state".into(),
            port_lease_state_dir: Some("/tmp/leases".into()),
            compose_project: "demo-feature-a-a111".into(),
            compose_files: vec![],
            ports: BTreeMap::from([("web".into(), port)]),
            container_ports: BTreeMap::from([("web".into(), 3000)]),
            urls: BTreeMap::from([("web".into(), format!("http://127.0.0.1:{port}"))]),
            env_file: "/tmp/.stackstead/.env".into(),
            agent_context: "/tmp/.stackstead/AGENT_CONTEXT.md".into(),
            pointer_file: "/tmp/.stackstead/stackstead.json".into(),
            event_log: "/tmp/events.jsonl".into(),
            env_keys: vec![],
            status: ManifestStatus::default(),
            database: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn respond(mut stream: TcpStream, response: &[u8]) -> anyhow::Result<()> {
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut buffer = [0; 1024];
            let read = stream.read(&mut buffer).test()?;
            assert_ne!(read, 0, "client closed before sending HTTP headers");
            request.extend_from_slice(&buffer[..read]);
        }
        stream.write_all(response).test()?;
        stream.flush().test()?;
        Ok(())
    }

    #[test]
    fn checks_loopback_http_status() -> anyhow::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").test()?;
        let port = listener.local_addr().test()?.port();
        let server = thread::spawn(move || -> anyhow::Result<()> {
            let (stream, _) = listener.accept().test()?;
            respond(
                stream,
                b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n",
            )
        });
        let config = HealthConfig {
            checks: vec![HealthCheckConfig {
                name: "web".into(),
                url: Some("{{ urls.web }}".into()),
                expect_status: 204,
                command: CommandConfig::default(),
            }],
            ..HealthConfig::default()
        };
        assert_eq!(
            healthy_passive(&config, &manifest(port), &BTreeMap::new()),
            Some(true)
        );
        server.join().test()??;
        Ok(())
    }

    #[test]
    fn checks_the_direct_redirect_status_without_following_it() -> anyhow::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").test()?;
        let port = listener.local_addr().test()?.port();
        let server = thread::spawn(move || -> anyhow::Result<()> {
            let (stream, _) = listener.accept().test()?;
            respond(
                stream,
                b"HTTP/1.1 302 Found\r\nLocation: https://example.invalid/\r\nContent-Length: 0\r\n\r\n",
            )
        });
        let config = HealthConfig {
            checks: vec![HealthCheckConfig {
                name: "login".into(),
                url: Some("{{ urls.web }}".into()),
                expect_status: 302,
                command: CommandConfig::default(),
            }],
            ..HealthConfig::default()
        };
        assert_eq!(
            healthy_passive(&config, &manifest(port), &BTreeMap::new()),
            Some(true)
        );
        server.join().test()??;
        Ok(())
    }

    #[test]
    fn passive_health_never_executes_command_checks() -> anyhow::Result<()> {
        let config = HealthConfig {
            checks: vec![HealthCheckConfig {
                name: "worker".into(),
                url: None,
                expect_status: 200,
                command: CommandConfig {
                    command: "stackstead-command-that-must-not-run".into(),
                    shell: false,
                },
            }],
            ..HealthConfig::default()
        };
        assert_eq!(
            healthy_passive(&config, &manifest(1), &BTreeMap::new()),
            None
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn polling_interval_is_clamped_to_the_overall_deadline() -> anyhow::Result<()> {
        let config = HealthConfig {
            timeout_seconds: 1,
            interval_millis: 60_000,
            checks: vec![HealthCheckConfig {
                name: "never".into(),
                url: None,
                expect_status: 200,
                command: CommandConfig {
                    command: "false".into(),
                    shell: false,
                },
            }],
        };
        let started = Instant::now();
        assert!(wait(&config, &manifest(1), &BTreeMap::new()).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
        Ok(())
    }
}
