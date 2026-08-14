use super::*;

#[cfg(unix)]
#[test]
fn inspect_passively_checks_http_health_only_for_a_running_runtime() -> anyhow::Result<()> {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let project = Project::initialized()?;
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["resources"]["ports"]["base"] = serde_yaml::Value::Number(50_000.into());
    project.write_config(&config, "isolate passive health test ports")?;
    let manifest = project.create("feature-a")?;
    let listener = TcpListener::bind(("127.0.0.1", manifest.ports["web"]))
        .test_context("bind allocated web port")?;
    let server = thread::spawn(move || -> anyhow::Result<()> {
        for status in ["200 OK", "500 Internal Server Error"] {
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
        }
        Ok(())
    });
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "inspect-health-fake-bin",
        &format!(
            "#!/bin/sh\ncase \" $* \" in\n  *\" ps --all --format json \"*) printf '%s\\n' '[{{\"Name\":\"demo-web-1\",\"Service\":\"web\",\"State\":\"running\",\"ExitCode\":0}}]' ;;\n  *\" port web 80 \"*) printf '127.0.0.1:{}\\n' ;;\nesac\n",
            manifest.ports["web"]
        ),
    )?;

    for expected in [true, false] {
        let inspected = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["inspect", "feature-a", "--json"])
            .assert()
            .success();
        let value: Value = serde_json::from_slice(&inspected.get_output().stdout).test()?;
        assert_eq!(value["version"], "3", "test contract values differ");
        assert_eq!(
            value["live"]["runtime"]["running"], true,
            "test contract values differ"
        );
        assert_eq!(
            value["live"]["health"]["healthy"], expected,
            "test contract values differ"
        );
        assert_eq!(
            value["effective"]["health"]["basis"], "live",
            "test contract values differ"
        );
    }
    server.join().test()??;
    Ok(())
}

#[cfg(unix)]
#[test]
fn inspect_preserves_passive_health_for_a_static_loopback_url() -> anyhow::Result<()> {
    use std::{
        io::{Read, Write},
        thread,
    };

    let project = Project::initialized()?;
    let listener = TcpListener::bind(("127.0.0.1", 0)).test()?;
    let port = listener.local_addr().test()?.port();
    let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
    config["health"]["checks"][0]["url"] = format!("http://127.0.0.1:{port}").into();
    project.write_config(&config, "use a static loopback health target")?;
    let manifest = project.create("feature-a")?;
    let server = thread::spawn(move || -> anyhow::Result<()> {
        let (mut stream, _) = listener.accept().test()?;
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut buffer = [0; 1024];
            let read = stream.read(&mut buffer).test()?;
            assert_ne!(read, 0, "client closed before sending HTTP headers");
            request.extend_from_slice(&buffer[..read]);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .test()?;
        Ok(())
    });
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "inspect-static-health-bin",
        "#!/bin/sh\ncase \" $* \" in *\" ps --all --format json \"*) printf '%s\\n' '[{\"Name\":\"demo-web-1\",\"Service\":\"web\",\"State\":\"running\",\"ExitCode\":0}]';; esac\n",
    )?;

    let inspected = stackstead(&project.repo)
        .env("PATH", path)
        .args(["inspect", &manifest.stackstead_id, "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout).test()?;
    assert_eq!(
        inspected["live"]["health"]["healthy"], true,
        "test contract values differ"
    );
    assert_eq!(
        inspected["effective"]["health"]["basis"], "live",
        "test contract values differ"
    );
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
    manifest.save_atomic().test()?;
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "inspect-stopped-target-bin",
        r#"#!/bin/sh
case " $* " in
  *" ps --all --format json "*)
    printf '%s\n' '[{"Name":"demo-postgres-1","Service":"postgres","State":"running","ExitCode":0},{"Name":"demo-web-1","Service":"web","State":"exited","ExitCode":1}]'
    ;;
esac
"#,
    )?;
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
    assert_eq!(first["version"], "3", "test contract values differ");
    assert_eq!(
        first["stackstead"]["status"]["health"], "ready",
        "test contract values differ"
    );
    assert_eq!(
        first["live"]["health"]["healthy"], false,
        "test contract values differ"
    );
    assert_eq!(
        first["effective"]["health"]["status"], "failed",
        "test contract values differ"
    );
    assert_eq!(
        first["effective"]["health"]["basis"], "live",
        "test contract values differ"
    );
    assert!(
        first["warnings"].as_array().is_some_and(|warnings| {
            warnings.iter().any(|warning| {
                warning.as_str().is_some_and(|warning| {
                    warning.contains("health recorded=ready effective=failed")
                })
            })
        }),
        "test contract condition failed"
    );
    first["effective"]["observed_at"] = Value::Null;
    second["effective"]["observed_at"] = Value::Null;
    assert_eq!(first, second, "test contract values differ");
    Ok(())
}
