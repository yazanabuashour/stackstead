use super::*;

#[cfg(unix)]
#[test]
fn inspect_passively_checks_http_health_only_for_a_running_runtime() -> anyhow::Result<()> {
    use std::thread;

    let project = Project::initialized()?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["resources"]["ports"]["base"] = serde_yaml::Value::Number(50_000.into());
    project.write_config(&config, "isolate passive health test ports")?;
    let manifest = project.create("feature-a")?;
    let listener = TcpListener::bind(("127.0.0.1", manifest.ports["web"]))
        .test_context("bind allocated web port")?;
    let server = thread::spawn(move || -> anyhow::Result<()> {
        for status in ["200 OK", "500 Internal Server Error"] {
            respond_to_health_request(&listener, status)?;
        }
        Ok(())
    });
    let path = health_docker_path(&project, &manifest, "running", "127.0.0.1")?;

    for expected in [true, false] {
        let inspected = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["inspect", "feature-a", "--json"])
            .assert()
            .success();
        let value: Value = serde_json::from_slice(&inspected.get_output().stdout).test()?;
        assert_eq!(value["version"], "4");
        assert_eq!(value["live"]["runtime"]["running"], true);
        assert_eq!(value["live"]["health"]["healthy"], expected);
        assert_eq!(value["effective"]["health"]["basis"], "live");
    }
    server.join().test()??;
    Ok(())
}

#[cfg(unix)]
#[test]
fn inspect_preserves_passive_health_for_a_static_loopback_url() -> anyhow::Result<()> {
    use std::thread;

    let project = Project::initialized()?;
    let listener = TcpListener::bind(("127.0.0.1", 0)).test()?;
    let port = listener.local_addr().test()?.port();
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["health"]["checks"][0]["url"] = format!("http://127.0.0.1:{port}").into();
    project.write_config(&config, "use a static loopback health target")?;
    let manifest = project.create("feature-a")?;
    assert!(!manifest.ports.values().any(|allocated| *allocated == port));
    let server = thread::spawn(move || respond_to_health_request(&listener, "200 OK"));
    let path = health_docker_path(&project, &manifest, "running", "127.0.0.1")?;

    let inspected = stackstead(&project.repo)
        .env("PATH", path)
        .args(["inspect", &manifest.stackstead_id, "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout).test()?;
    assert_eq!(inspected["version"], "4");
    assert_eq!(inspected["live"]["health"]["healthy"], true);
    assert_eq!(inspected["effective"]["health"]["basis"], "live");
    server.join().test()??;
    Ok(())
}

#[cfg(unix)]
#[test]
fn inspect_reports_recorded_ready_but_live_failed_for_a_stopped_health_target() -> anyhow::Result<()>
{
    let project = Project::initialized()?;
    let mut manifest = project.create("feature-a")?;
    manifest.status.runtime = ComponentStatus::Running;
    manifest.status.health = ComponentStatus::Ready;
    manifest.write_fixture().test()?;
    let path = health_docker_path(&project, &manifest, "exited", "127.0.0.1")?;
    let inspect = || -> anyhow::Result<Value> {
        let output = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["inspect", &manifest.stackstead_id, "--json"])
            .assert()
            .success();
        serde_json::from_slice(&output.get_output().stdout).test()
    };

    let mut first = inspect()?;
    let mut second = inspect()?;
    assert_eq!(first["version"], "4");
    assert_eq!(first["stackstead"]["status"]["health"], "ready");
    assert_eq!(first["live"]["health"]["healthy"], false);
    assert_eq!(first["effective"]["health"]["status"], "failed");
    assert_eq!(first["effective"]["health"]["basis"], "live");
    assert!(first["warnings"].as_array().is_some_and(|warnings| {
        warnings.iter().any(|warning| {
            warning
                .as_str()
                .is_some_and(|warning| warning.contains("health recorded=ready effective=failed"))
        })
    }));
    first["effective"]["observed_at"] = Value::Null;
    second["effective"]["observed_at"] = Value::Null;
    assert_eq!(first, second);
    Ok(())
}

#[cfg(unix)]
#[test]
fn inspect_validates_later_mapped_targets_before_any_passive_request() -> anyhow::Result<()> {
    for corrupt_mapping in [false, true] {
        let project = Project::initialized()?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).test()?;
        let port = listener.local_addr().test()?.port();
        let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
        let mut uncorrelated = config["health"]["checks"][0].clone();
        uncorrelated["name"] = "uncorrelated".into();
        uncorrelated["url"] = format!("http://127.0.0.1:{port}").into();
        config["health"]["checks"]
            .as_sequence_mut()
            .test()?
            .insert(0, uncorrelated);
        project.write_config(&config, "check uncorrelated URL before mapped target")?;
        let manifest = project.create("feature-a")?;
        assert!(!manifest.ports.values().any(|allocated| *allocated == port));
        let mapped_listener = TcpListener::bind(("127.0.0.1", manifest.ports["web"])).test()?;
        if corrupt_mapping {
            let compose_file = manifest.compose_files.first().test()?;
            let compose = fs::read_to_string(compose_file).test()?;
            assert!(compose.contains("${WEB_PORT}:80"));
            fs::write(
                compose_file,
                compose.replace("${WEB_PORT}:80", "${WEB_PORT}:81"),
            )
            .test()?;
        }
        let path = health_docker_path(&project, &manifest, "running", "127.0.0.2")?;
        let output = stackstead(&project.repo)
            .env("PATH", path)
            .args(["inspect", &manifest.stackstead_id, "--json"])
            .assert()
            .success();
        let inspected: Value = serde_json::from_slice(&output.get_output().stdout).test()?;
        assert_eq!(inspected["version"], "4");
        assert_eq!(inspected["live"]["runtime"]["running"], true);
        if corrupt_mapping {
            assert!(inspected["live"]["health"]["healthy"].is_null());
            assert!(
                inspected["warnings"]
                    .as_array()
                    .test()?
                    .iter()
                    .any(|warning| {
                        warning.as_str().is_some_and(|warning| {
                            warning.contains("could not inspect configured health targets")
                                && warning.contains("publishes container port 81, expected 80")
                        })
                    })
            );
        } else {
            assert_eq!(inspected["live"]["health"]["healthy"], false);
            assert_eq!(inspected["effective"]["health"]["basis"], "live");
        }
        for listener in [listener, mapped_listener] {
            listener.set_nonblocking(true).test()?;
            assert_eq!(
                listener.accept().test_err()?.kind(),
                std::io::ErrorKind::WouldBlock,
                "passive request preceded completion of endpoint ownership validation"
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn respond_to_health_request(listener: &TcpListener, status: &str) -> anyhow::Result<()> {
    use std::io::{Read, Write};

    let (mut stream, _) = listener.accept().test()?;
    let mut request = Vec::new();
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let mut buffer = [0; 1024];
        let read = stream.read(&mut buffer).test()?;
        assert_ne!(read, 0, "client closed before sending HTTP headers");
        request.extend_from_slice(&buffer[..read]);
    }
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .test()?;
    stream.flush().test()?;
    Ok(())
}

#[cfg(unix)]
fn health_docker_path(
    project: &Project,
    manifest: &StacksteadManifest,
    web_state: &str,
    published_host: &str,
) -> anyhow::Result<OsString> {
    use std::fmt::Write as _;
    let labels = serde_json::json!({
        "com.docker.compose.project": manifest.compose_project,
        "io.stackstead.runtime-token": manifest.runtime_token,
    });
    let mut ids = Vec::new();
    let mut names = Vec::new();
    let mut metadata_cases = String::new();
    for (service, state, digit) in [("web", web_state, "a"), ("postgres", "running", "b")] {
        let id = digit.repeat(64);
        let name = format!("{}-{service}-1", manifest.compose_project);
        let metadata = serde_json::json!({
            "id": id, "container": format!("/{name}"),
            "project": manifest.compose_project, "runtime_token": manifest.runtime_token,
            "service": service, "state": state, "exit_code": i32::from(state == "exited"),
            "health": null, "healthcheck_enabled": false, "oneoff": "False",
            "container_number": "1", "config_hash": "c".repeat(64),
        });
        writeln!(metadata_cases, "  {id}) printf '%s\\n' '{metadata}' ;;").test()?;
        ids.push(id);
        names.push(name);
    }
    let script = format!(
        r#"#!/bin/sh
for argument in "$@"; do last="$argument"; done
case " $* " in
  *'"container_number":'*)
    case "$last" in
{metadata_cases}      *) exit 1 ;;
    esac ;;
  *" inspect --format "*) printf '%s\n' '{labels}' ;;
  *" container ls --all --filter "*) printf '%s\n' '{ids}' ;;
  *" container ls --all --format "*) printf '%s\n' '{names}' ;;
  *" volume ls "*) printf '%s\n' '{claim}' ;;
  *" network ls "*) ;;
  *" port web 80 "*) printf '%s\n' '{published_host}:{port}' ;;
  *) exit 1 ;;
esac
"#,
        ids = ids.join("\n"),
        names = names.join("\n"),
        claim = format_args!("{}-stackstead-claim", manifest.compose_project),
        port = manifest.ports["web"],
    );
    fake_docker_path(project.repo.parent().test()?, "inspect-health-bin", &script)
}
