use std::{fs, path::Path, process::Command};

use super::{Diagnostic, ToolStatus};
use crate::{discovery::Discovery, git, repository_policy};

pub(super) fn diagnose_tools(diagnostics: &mut Vec<Diagnostic>) -> ToolStatus {
    let git = command_succeeds("git", &["--version"]);
    diagnostics.push(tool_diagnostic(
        "git",
        git,
        "install Git and ensure it is available on PATH",
    ));

    let docker = command_succeeds("docker", &["--version"]);
    diagnostics.push(tool_diagnostic(
        "docker",
        docker,
        "install Docker and ensure it is available on PATH",
    ));

    let compose = docker && command_succeeds("docker", &["compose", "version"]);
    diagnostics.push(tool_diagnostic(
        "docker_compose",
        compose,
        "install the Docker Compose plugin (`docker compose`)",
    ));

    let docker_daemon =
        docker && command_succeeds("docker", &["info", "--format", "{{.ServerVersion}}"]);
    diagnostics.push(if docker_daemon {
        Diagnostic::info("tool.docker_daemon.available", "Docker daemon is reachable")
    } else {
        Diagnostic::warning(
            "tool.docker_daemon.unavailable",
            "Docker daemon is not reachable; runtime project checks were skipped",
            "start Docker and rerun `stackstead doctor`",
        )
    });

    ToolStatus {
        git,
        docker,
        compose,
        docker_daemon,
    }
}

fn command_succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn tool_diagnostic(tool: &str, available: bool, suggestion: &str) -> Diagnostic {
    if available {
        Diagnostic::info(
            format!("tool.{tool}.available"),
            format!("{} is available", tool.replace('_', " ")),
        )
    } else {
        Diagnostic::error(
            format!("tool.{tool}.missing"),
            format!("{} is not available", tool.replace('_', " ")),
            suggestion,
        )
    }
}

pub(super) fn diagnose_initial_discovery(discovery: &Discovery, diagnostics: &mut Vec<Diagnostic>) {
    match discovery {
        Discovery::Project {
            repo_root,
            config_path,
        } => diagnostics.push(Diagnostic::info(
            "discovery.project",
            format!(
                "project root {} discovered through {}",
                repo_root.display(),
                config_path.display()
            ),
        )),
        Discovery::Stackstead {
            pointer_path,
            manifest,
            ..
        } => diagnostics.push(Diagnostic::info(
            "discovery.stackstead",
            format!(
                "stackstead {} discovered through {}",
                manifest.stackstead_id,
                pointer_path.display()
            ),
        )),
    }
}

pub(super) fn diagnose_repository(
    repo_root: &Path,
    git_available: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !git_available {
        return;
    }
    match git::repo_root(repo_root) {
        Ok(detected) if detected == repo_root => diagnostics.push(Diagnostic::info(
            "git.repo_root.valid",
            format!("Git repository root is {}", repo_root.display()),
        )),
        Ok(detected) => diagnostics.push(Diagnostic::error(
            "git.repo_root.mismatch",
            format!(
                "Stackstead project root {} differs from Git root {}",
                repo_root.display(),
                detected.display()
            ),
            "move stackstead.yaml to the canonical Git repository root",
        )),
        Err(error) => diagnostics.push(Diagnostic::error(
            "git.repo_root.unavailable",
            format!("cannot resolve Git repository root: {error}"),
            "run Stackstead from a valid Git repository",
        )),
    }
}

#[expect(
    clippy::comparison_chain,
    reason = "the equal/older/newer policy branches read directly in version order"
)]
pub(super) fn diagnose_repository_policy(repo_root: &Path, diagnostics: &mut Vec<Diagnostic>) {
    let mut found = false;
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let path = repo_root.join(name);
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                found = true;
                diagnostics.push(Diagnostic::warning(
                    "repository_policy.unreadable",
                    format!(
                        "cannot read repository policy file {}: {error}",
                        path.display()
                    ),
                    "make the instruction file readable and rerun `stackstead doctor`",
                ));
                continue;
            }
        };
        match policy_marker_version(&contents) {
            Some(Ok(version)) => {
                found = true;
                if version == repository_policy::VERSION {
                    diagnostics.push(Diagnostic::info(
                        "repository_policy.current",
                        format!("repository policy is current in {}", path.display()),
                    ));
                } else if version < repository_policy::VERSION {
                    diagnostics.push(Diagnostic::warning(
                        "repository_policy.outdated",
                        format!(
                            "repository policy version {version} in {} is older than version {} required by this Stackstead binary",
                            path.display(),
                            repository_policy::VERSION
                        ),
                        format!("update the policy from {}", repository_policy::GUIDE_URL),
                    ));
                } else {
                    diagnostics.push(Diagnostic::warning(
                        "repository_policy.binary_outdated",
                        format!(
                            "repository policy version {version} in {} is newer than version {} understood by this Stackstead binary",
                            path.display(),
                            repository_policy::VERSION
                        ),
                        "upgrade Stackstead before relying on this repository policy",
                    ));
                }
            }
            Some(Err(error)) => {
                found = true;
                diagnostics.push(Diagnostic::warning(
                    "repository_policy.invalid",
                    format!(
                        "repository policy marker is invalid in {}: {error}",
                        path.display()
                    ),
                    format!(
                        "replace it with the current policy from {}",
                        repository_policy::GUIDE_URL
                    ),
                ));
            }
            None if contents.contains("## Stackstead")
                && contents.contains("$STACKSTEAD_CONTEXT") =>
            {
                found = true;
                diagnostics.push(Diagnostic::warning(
                    "repository_policy.unversioned",
                    format!(
                        "Stackstead repository policy in {} has no version marker",
                        path.display()
                    ),
                    format!("update the policy from {}", repository_policy::GUIDE_URL),
                ));
            }
            None => {}
        }
    }
    if !found {
        diagnostics.push(Diagnostic::warning(
            "repository_policy.missing",
            "no current Stackstead repository policy was found in AGENTS.md or CLAUDE.md",
            format!("add the policy from {}", repository_policy::GUIDE_URL),
        ));
    }
}

fn policy_marker_version(contents: &str) -> Option<anyhow::Result<u64>> {
    contents.lines().find_map(|line| {
        line.trim()
            .strip_prefix(repository_policy::MARKER_PREFIX)
            .map(|value| {
                value
                    .strip_suffix(repository_policy::MARKER_SUFFIX)
                    .ok_or_else(|| anyhow::anyhow!("policy marker has no closing delimiter"))?
                    .trim()
                    .parse()
                    .map_err(Into::into)
            })
    })
}
