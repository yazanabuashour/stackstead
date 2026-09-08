use super::*;

#[cfg(unix)]
#[test]
fn exec_targets_one_owned_running_service_and_preserves_command_arguments() -> anyhow::Result<()> {
    let project = Project::initialized()?;
    let manifest = project.create("feature-a")?;
    let parent = project.repo.parent().test()?;
    let fake_state = parent.join("service-exec-state");
    let path = service_exec_docker_path(parent, "service-exec-bin")?;
    assert_exec_requires_command_boundary(&project, &manifest)?;
    let executed = stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_PROJECT", &manifest.compose_project)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .env("EXEC_ASSERT_ENV", "1")
        .env("EXEC_ASSERT_FOREGROUND", "1")
        .env("EXEC_EXIT_CODE", "23")
        .env("WEB_PORT", "inherited-spoof")
        .arg("exec")
        .arg(&manifest.stackstead_id)
        .arg("web")
        .arg("--")
        .arg("program with spaces")
        .arg("--flag")
        .arg("two words")
        .assert()
        .code(23);
    assert_eq!(
        output_text(&executed.get_output().stdout)?,
        "service=<web>\nargument=<program with spaces>\nargument=<--flag>\nargument=<two words>\n"
    );
    assert!(fake_state.join("exec-ran").is_file());
    fs::remove_file(fake_state.join("exec-ran")).test()?;
    assert_exec_rejects_json(&project, &manifest)?;
    assert_exec_rejects_invalid_targets(&project, &manifest, &path, &fake_state)?;
    assert_exec_rejects_tampered_ownership(&project, &manifest, &path, &fake_state)
}

#[cfg(unix)]
fn assert_exec_requires_command_boundary(
    project: &Project,
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    let missing_boundary = stackstead(&project.repo)
        .args([
            "exec",
            &manifest.stackstead_id,
            "web",
            "program-without-boundary",
        ])
        .assert()
        .failure();
    assert!(
        output_text(&missing_boundary.get_output().stderr)?.contains("-- <COMMAND>"),
        "exec accepted a command without the required `--` boundary"
    );
    Ok(())
}

#[cfg(unix)]
fn assert_exec_rejects_json(
    project: &Project,
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    let rejected_json = stackstead(&project.repo)
        .args([
            "--json",
            "exec",
            &manifest.stackstead_id,
            "web",
            "--",
            "true",
        ])
        .assert()
        .failure();
    assert!(
        output_text(&rejected_json.get_output().stderr)?
            .contains("--json cannot be combined with exec"),
        "exec accepted JSON output"
    );
    Ok(())
}

#[cfg(unix)]
fn assert_exec_rejects_invalid_targets(
    project: &Project,
    manifest: &StacksteadManifest,
    path: &OsString,
    fake_state: &Path,
) -> anyhow::Result<()> {
    let unknown = stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", fake_state)
        .env("EXPECTED_PROJECT", &manifest.compose_project)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["exec", &manifest.stackstead_id, "missing", "--", "true"])
        .assert()
        .failure();
    let stderr = output_text(&unknown.get_output().stderr)?;
    assert!(
        stderr.contains("is not configured") && stderr.contains("postgres, web"),
        "exec target validation broke its contract"
    );
    assert!(
        !fake_state.join("exec-ran").exists(),
        "exec target validation broke its contract"
    );

    let stopped = stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", fake_state)
        .env("EXPECTED_PROJECT", &manifest.compose_project)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .env("SERVICE_RUNNING", "0")
        .args(["exec", &manifest.stackstead_id, "web", "--", "true"])
        .assert()
        .failure();
    assert!(
        output_text(&stopped.get_output().stderr)?.contains("is not running"),
        "exec target validation broke its contract"
    );
    assert!(
        !fake_state.join("exec-ran").exists(),
        "exec target validation broke its contract"
    );

    let foreign = stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", fake_state)
        .env("EXPECTED_PROJECT", &manifest.compose_project)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .env("DOCKER_TOKEN", "foreign-runtime-token")
        .args(["exec", &manifest.stackstead_id, "web", "--", "true"])
        .assert()
        .failure();
    assert!(
        output_text(&foreign.get_output().stderr)?.contains("ownership label"),
        "exec target validation broke its contract"
    );
    assert!(
        !fake_state.join("exec-ran").exists(),
        "exec target validation broke its contract"
    );
    Ok(())
}

#[cfg(unix)]
fn assert_exec_rejects_tampered_ownership(
    project: &Project,
    manifest: &StacksteadManifest,
    path: &OsString,
    fake_state: &Path,
) -> anyhow::Result<()> {
    use std::io::Write as _;

    fs::OpenOptions::new()
        .append(true)
        .open(manifest.worktree.join(".stackstead/compose-ownership.yaml"))
        .test()?
        .write_all(b"# tampered\n")
        .test()?;
    let tampered = stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", fake_state)
        .env("EXPECTED_PROJECT", &manifest.compose_project)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["exec", &manifest.stackstead_id, "web", "--", "true"])
        .assert()
        .failure();
    assert!(
        output_text(&tampered.get_output().stderr)?
            .contains("generated Compose ownership override"),
        "exec accepted a tampered ownership contract"
    );
    assert!(
        !fake_state.join("exec-ran").exists(),
        "exec accepted a tampered ownership contract"
    );
    Ok(())
}

#[cfg(unix)]
pub(super) fn service_exec_docker_path(parent: &Path, directory: &str) -> anyhow::Result<OsString> {
    fake_docker_path(
        parent,
        directory,
        r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
printf '%s\n' "$*" >> "$FAKE_STATE/commands"
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$1 $2" in
  "container ls"|"network ls") exit 0 ;;
  "volume ls") printf '%s\n' "$claim"; exit 0 ;;
  "volume inspect")
    printf '{"io.stackstead.runtime-token":"%s"}\n' "${DOCKER_TOKEN:-$EXPECTED_TOKEN}"
    exit 0
    ;;
  "compose -p")
    case " $* " in
      *" ps --status running --quiet "*)
        test "${SERVICE_RUNNING:-1}" = 0 || printf '%s\n' fake-container-id
        exit 0
        ;;
      *" exec "*)
        test "$COMPOSE_PROJECT_NAME" = "$EXPECTED_PROJECT"
        if test -n "${EXEC_ASSERT_ENV-}"; then
          test "${WEB_PORT+x}" != x
        fi
        if test -n "${EXEC_ASSERT_FOREGROUND-}"; then
          test "$(ps -o pgid= -p $$ | tr -d ' ')" = "$(ps -o pgid= -p $PPID | tr -d ' ')"
        fi
        while test "$1" != exec; do shift; done
        shift
        test "${1-}" != -T || shift
        printf 'service=<%s>\n' "$1"
        shift
        for argument in "$@"; do printf 'argument=<%s>\n' "$argument"; done
        : > "$FAKE_STATE/exec-ran"
        test -z "${EXEC_READY-}" || : > "$EXEC_READY"
        while test -n "${EXEC_RELEASE-}" && test -e "$EXEC_RELEASE"; do sleep 0.02; done
        exit "${EXEC_EXIT_CODE:-0}"
        ;;
    esac
    ;;
esac
exit 0
"#,
    )
}
