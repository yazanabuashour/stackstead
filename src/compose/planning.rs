use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use crate::command;

use super::{
    model::{ComposePlan, ComposePortPlan, HostBinding},
    yaml::{env_name, http_port, port_declarations},
};

const COMPOSE_FILES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];

#[cfg(test)]
pub fn plan(repo_root: &Path) -> anyhow::Result<ComposePlan> {
    plan_at(repo_root, None)
}

pub fn plan_at(repo_root: &Path, requested: Option<&Path>) -> anyhow::Result<ComposePlan> {
    let repo_root = std::fs::canonicalize(repo_root)?;
    let file = resolve_compose_file(&repo_root, requested)?;
    plan_file(&repo_root, &file)
}

pub(super) fn resolve_compose_file(
    repo_root: &Path,
    requested: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    if let Some(requested) = requested {
        if requested.is_absolute() {
            anyhow::bail!("--compose-file must be relative to the repository root");
        }
        let canonical_root = std::fs::canonicalize(repo_root)?;
        let canonical_file = std::fs::canonicalize(repo_root.join(requested)).map_err(|error| {
            anyhow::anyhow!(
                "cannot access Compose file {}: {error}",
                requested.display()
            )
        })?;
        if !canonical_file.starts_with(&canonical_root) || !canonical_file.is_file() {
            anyhow::bail!(
                "Compose file {} must resolve to a file inside the repository",
                requested.display()
            );
        }
        return Ok(canonical_file);
    }
    if let Some(file) = COMPOSE_FILES
        .iter()
        .map(|name| repo_root.join(name))
        .find(|path| path.is_file())
    {
        return Ok(file);
    }
    let candidates = tracked_compose_candidates(repo_root);
    let hint = if candidates.is_empty() {
        String::new()
    } else {
        format!(
            "; tracked nested candidate(s): {}",
            candidates
                .iter()
                .take(5)
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    anyhow::bail!(
        "no root Compose file found; expected one of {}; pass --compose-file <repository-relative-path>{hint}",
        COMPOSE_FILES.join(", ")
    )
}

fn tracked_compose_candidates(repo_root: &Path) -> Vec<PathBuf> {
    let Ok(output) = command::run("git", &["ls-files".into()], repo_root, &BTreeMap::new()) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(PathBuf::from)
        .filter(|path| {
            path.parent()
                .is_some_and(|parent| !parent.as_os_str().is_empty())
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| COMPOSE_FILES.contains(&name))
        })
        .collect()
}

pub fn plan_file(repo_root: &Path, file: &Path) -> anyhow::Result<ComposePlan> {
    let contents = std::fs::read_to_string(file)?;
    plan_contents(repo_root, file, &contents)
}

pub(super) fn plan_contents(
    repo_root: &Path,
    file: &Path,
    contents: &str,
) -> anyhow::Result<ComposePlan> {
    let document: serde_yaml::Value = serde_yaml::from_str(contents)
        .map_err(|error| anyhow::anyhow!("cannot parse {}: {error}", file.display()))?;
    let mut discovered = Vec::new();
    let mut warnings = Vec::new();
    let mut env_owners = HashMap::new();

    for declaration in port_declarations(&document, file)? {
        let service = declaration.service;
        let container_port = declaration.container_port;
        let host_binding = declaration.host_binding;
        let protocol = declaration.protocol;
        let name = declaration.name;
        let host_ip = declaration.host_ip;
        let generated_env = format!("{}_PORT", env_name(&name));
        let env = match &host_binding {
            HostBinding::Missing => anyhow::bail!(
                "service `{service}` publishes container port {container_port} without a deterministic host binding; change it to `127.0.0.1:${{{generated_env}}}:{container_port}` before initializing Stackstead"
            ),
            HostBinding::Variable(env) => env.clone(),
            HostBinding::Fixed(_) => generated_env,
        };
        if let Some(owner) = env_owners.insert(env.clone(), name.clone()) {
            anyhow::bail!(
                "Compose ports `{owner}` and `{name}` both use environment variable `{env}`; every published port needs its own variable"
            );
        }
        let url = http_port(&service, container_port, &protocol)
            .then(|| format!("http://127.0.0.1:{{{{ ports.{name} }}}}"));
        let loopback_host = host_ip.as_deref().filter(|host| {
            *host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
        if host_ip.is_some() && loopback_host.is_none() {
            anyhow::bail!(
                "Compose port `{name}` explicitly binds all host interfaces; bind `127.0.0.1:${{{env}}}:{container_port}`"
            );
        }
        if loopback_host.is_none() && matches!(host_binding, HostBinding::Variable(_)) {
            anyhow::bail!(
                "Compose port `{name}` uses a generated host port but binds all host interfaces; bind `127.0.0.1:${{{env}}}:{container_port}`"
            );
        }
        if loopback_host.is_none() {
            warnings.push(format!(
                "Compose port `{name}` currently binds all host interfaces; `compose apply` will bind it to 127.0.0.1"
            ));
        }
        let replacement_host = host_ip.as_deref().unwrap_or("127.0.0.1");
        let replacement_host = if replacement_host.contains(':') {
            format!("[{replacement_host}]")
        } else {
            replacement_host.to_owned()
        };
        discovered.push(ComposePortPlan {
            name,
            service,
            container_port,
            env: env.clone(),
            current_host_port: match &host_binding {
                HostBinding::Fixed(port) => Some(*port),
                HostBinding::Variable(_) | HostBinding::Missing => None,
            },
            replacement: format!("{replacement_host}:${{{env}}}:{container_port}{protocol}"),
            url,
        });
    }

    let relative = file
        .strip_prefix(repo_root)
        .map_or_else(|_| file.to_owned(), Path::to_owned);
    Ok(ComposePlan {
        file: relative,
        ports: discovered,
        warnings,
    })
}
