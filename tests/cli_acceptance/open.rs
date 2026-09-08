use super::*;

#[cfg(target_os = "linux")]
#[test]
fn open_refuses_a_stopped_runtime_before_invoking_the_browser() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let marker = project.repo.parent().test()?.join("browser-opened");
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "stale-open-fake-bin",
        "#!/bin/sh\ncase \"$1 $2\" in 'container ls'|'network ls'|'volume ls') exit 0;; esac\nexit 97\n",
    )?;
    let fake_bin = std::env::split_paths(&path).next().test()?;
    let opener = fake_bin.join("xdg-open");
    fs::write(
        &opener,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .test()?;
    fs::set_permissions(&opener, fs::Permissions::from_mode(0o755)).test()?;

    let rejected = stackstead(&project.repo)
        .env("PATH", path)
        .args(["open", &manifest.stackstead_id, "web"])
        .assert()
        .failure();
    assert!(
        output_text(&rejected.get_output().stderr)?.contains("has no Stackstead ownership claim")
    );
    assert!(
        !marker.exists(),
        "browser launched for an unrelated listener"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn open_launches_only_after_owned_service_publication_is_proven() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let marker = project.repo.parent().test()?.join("owned-browser-url");
    let path = fake_docker_path(
        project.repo.parent().test()?,
        "owned-open-fake-bin",
        r#"#!/bin/sh
case "$1 $2" in
  "container ls"|"network ls") exit 0 ;;
  "volume ls") printf '%s\n' "$COMPOSE_PROJECT_NAME-stackstead-claim"; exit 0 ;;
  "volume inspect") printf '{"io.stackstead.runtime-token":"%s"}\n' "$EXPECTED_TOKEN"; exit 0 ;;
esac
for argument in "$@"; do
  case "$argument" in
    ps) printf 'owned-container\n'; exit 0 ;;
    port) printf '127.0.0.1:%s\n' "$EXPECTED_PORT"; exit 0 ;;
  esac
done
exit 97
"#,
    )?;
    let fake_bin = std::env::split_paths(&path).next().test()?;
    let opener = fake_bin.join("xdg-open");
    fs::write(
        &opener,
        format!(
            "#!/bin/sh\n: > '{}'\nsleep 0.05\nprintf '%s' \"$1\" > '{}'\n",
            marker.display(),
            marker.display()
        ),
    )
    .test()?;
    fs::set_permissions(&opener, fs::Permissions::from_mode(0o755)).test()?;

    stackstead(&project.repo)
        .env("PATH", path)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .env("EXPECTED_PORT", manifest.ports["web"].to_string())
        .args(["open", &manifest.stackstead_id, "web"])
        .assert()
        .success();
    let mut opened_url = String::new();
    for _ in 0..100 {
        match fs::read_to_string(&marker) {
            Ok(url) => opened_url = url,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if opened_url == manifest.urls["web"] {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(opened_url, manifest.urls["web"]);
    Ok(())
}
