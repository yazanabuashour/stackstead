use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::Write,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{command, manifest::StacksteadManifest};

#[derive(Debug, Clone, PartialEq, Eq)]
enum HostBinding {
    Fixed(u16),
    Variable(String),
    Missing,
}

const COMPOSE_FILES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];
const OWNERSHIP_OVERRIDE: &str = ".stackstead/compose-ownership.yaml";
const OWNERSHIP_HELPER_IMAGE: &str =
    "alpine@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc";
const RUNTIME_TOKEN_LABEL: &str = "io.stackstead.runtime-token";
const COMPOSE_PROJECT_LABEL: &str = "com.docker.compose.project";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposePlan {
    pub file: PathBuf,
    pub ports: Vec<ComposePortPlan>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposePortPlan {
    pub name: String,
    pub service: String,
    pub container_port: u16,
    pub env: String,
    pub current_host_port: Option<u16>,
    pub replacement: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeApplyOutput {
    pub file: PathBuf,
    pub changed_lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposePortTarget {
    pub service: String,
    pub container_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceObservation {
    pub service: String,
    pub container: String,
    pub state: String,
    pub exit_code: Option<i64>,
}

impl ServiceObservation {
    pub fn status(&self) -> String {
        match (self.state.as_str(), self.exit_code) {
            ("exited", Some(0)) => "completed (0)".into(),
            ("exited", Some(code)) => format!("exited ({code})"),
            _ => self.state.clone(),
        }
    }
}

#[cfg(test)]
pub fn plan(repo_root: &Path) -> anyhow::Result<ComposePlan> {
    plan_at(repo_root, None)
}

pub fn plan_at(repo_root: &Path, requested: Option<&Path>) -> anyhow::Result<ComposePlan> {
    let repo_root = std::fs::canonicalize(repo_root)?;
    let file = resolve_compose_file(&repo_root, requested)?;
    plan_file(&repo_root, &file)
}

fn resolve_compose_file(repo_root: &Path, requested: Option<&Path>) -> anyhow::Result<PathBuf> {
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

fn plan_contents(repo_root: &Path, file: &Path, contents: &str) -> anyhow::Result<ComposePlan> {
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
        .map(Path::to_owned)
        .unwrap_or_else(|_| file.to_owned());
    Ok(ComposePlan {
        file: relative,
        ports: discovered,
        warnings,
    })
}

#[derive(Debug)]
struct PortDeclaration {
    name: String,
    service: String,
    container_port: u16,
    host_binding: HostBinding,
    protocol: String,
    host_ip: Option<String>,
}

fn port_declarations(
    document: &serde_yaml::Value,
    file: &Path,
) -> anyhow::Result<Vec<PortDeclaration>> {
    let mut document = document.clone();
    document.apply_merge().map_err(|error| {
        anyhow::anyhow!(
            "cannot resolve YAML merge keys in Compose file {}: {error}",
            file.display()
        )
    })?;
    if yaml_field(&document, "include").is_some() {
        anyhow::bail!(
            "Compose `include` is not supported in {}; list each file explicitly in runtime.files",
            file.display()
        );
    }
    let Some(services_value) = yaml_field(&document, "services") else {
        return Ok(vec![]);
    };
    let services = services_value.as_mapping().ok_or_else(|| {
        anyhow::anyhow!(
            "{} has a services value that is not a mapping",
            file.display()
        )
    })?;
    let mut declarations = Vec::new();
    let mut service_counts = HashMap::new();
    for (service, value) in services {
        let service = service.as_str().ok_or_else(|| {
            anyhow::anyhow!(
                "{} contains a non-string Compose service name",
                file.display()
            )
        })?;
        if yaml_field(value, "extends").is_some() {
            anyhow::bail!(
                "Compose service `{service}` in {} uses unsupported `extends`; list each Compose file explicitly in runtime.files",
                file.display()
            );
        }
        let Some(raw_ports) = yaml_field(value, "ports") else {
            continue;
        };
        let ports = raw_ports.as_sequence().ok_or_else(|| {
            anyhow::anyhow!(
                "Compose service `{service}` has a ports value that is not a YAML sequence in {}",
                file.display()
            )
        })?;
        for value in ports {
            let (container_port, host_binding, protocol, host_ip) = parse_compose_port(value).ok_or_else(|| {
                anyhow::anyhow!(
                    "unsupported or ambiguous port mapping for Compose service `{service}` in {}: {value:?}",
                    file.display()
                )
            })?;
            if !matches!(protocol.as_str(), "" | "/tcp") {
                anyhow::bail!(
                    "Compose service `{service}` publishes unsupported protocol `{}` in {}; Stackstead exposes TCP ports only",
                    protocol.trim_start_matches('/'),
                    file.display()
                );
            }
            let count = service_counts.entry(service.to_owned()).or_insert(0usize);
            *count += 1;
            let name = if *count == 1 {
                service.to_owned()
            } else {
                format!("{service}-{container_port}")
            };
            declarations.push(PortDeclaration {
                name,
                service: service.to_owned(),
                container_port,
                host_binding,
                protocol,
                host_ip,
            });
        }
    }
    Ok(declarations)
}

pub fn validate_port_contract(
    files: &[PathBuf],
    expected_containers: &BTreeMap<String, u16>,
    generated_environment: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    let mut actual = BTreeMap::new();
    let mut variable_owners = BTreeMap::new();
    for file in files {
        let document: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(file)?)
            .map_err(|error| anyhow::anyhow!("cannot parse {}: {error}", file.display()))?;
        for declaration in port_declarations(&document, file)? {
            let loopback = declaration.host_ip.as_deref().is_some_and(|host| {
                host == "localhost"
                    || host
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|address| address.is_loopback())
            });
            let variable = match declaration.host_binding {
                HostBinding::Variable(variable) => variable,
                HostBinding::Fixed(port) => anyhow::bail!(
                    "fixed host port {port} found for Compose port `{}` in {}; use a generated environment variable",
                    declaration.name,
                    file.display()
                ),
                HostBinding::Missing => anyhow::bail!(
                    "Compose port `{}` in {} has no deterministic host binding",
                    declaration.name,
                    file.display()
                ),
            };
            if !loopback {
                anyhow::bail!(
                    "Compose port `{}` in {} must bind a loopback host such as 127.0.0.1",
                    declaration.name,
                    file.display()
                );
            }
            let generated = generated_environment.get(&variable).ok_or_else(|| {
                anyhow::anyhow!(
                    "Compose port `{}` uses `${{{variable}}}`, but env.generate does not define `{variable}`",
                    declaration.name
                )
            })?;
            let contract_name = port_template_name(generated).ok_or_else(|| {
                anyhow::anyhow!(
                    "env.generate.{variable} must be exactly a `{{{{ ports.<name> }}}}` allocation"
                )
            })?;
            if let Some(owner) = variable_owners.insert(variable.clone(), contract_name.clone()) {
                anyhow::bail!(
                    "Compose ports `{owner}` and `{contract_name}` both use `{variable}`; every published port needs its own host variable"
                );
            }
            let contract = (declaration.container_port, variable, file.clone());
            if actual.insert(contract_name.clone(), contract).is_some() {
                anyhow::bail!(
                    "Compose port `{contract_name}` is declared more than once across runtime files"
                );
            }
        }
    }
    if actual.keys().ne(expected_containers.keys()) {
        anyhow::bail!(
            "Compose published-port names do not match the durable Stackstead contract; expected {:?}, found {:?}",
            expected_containers.keys().collect::<Vec<_>>(),
            actual.keys().collect::<Vec<_>>()
        );
    }
    for (name, expected_container) in expected_containers {
        let (container, variable, file) = &actual[name];
        if container != expected_container {
            anyhow::bail!(
                "Compose port `{name}` publishes container port {container}, expected {expected_container} in {}",
                file.display()
            );
        }
        debug_assert_eq!(
            port_template_name(&generated_environment[variable]).as_deref(),
            Some(name.as_str())
        );
    }
    Ok(())
}

pub fn resolve_port_target(
    files: &[PathBuf],
    expected_containers: &BTreeMap<String, u16>,
    generated_environment: &BTreeMap<String, String>,
    contract_key: &str,
) -> anyhow::Result<ComposePortTarget> {
    let expected_container = expected_containers.get(contract_key).ok_or_else(|| {
        anyhow::anyhow!("port contract has no `{contract_key}` container mapping")
    })?;
    validate_port_contract(files, expected_containers, generated_environment)?;
    let mut target = None;
    for file in files {
        let document: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(file)?)
            .map_err(|error| anyhow::anyhow!("cannot parse {}: {error}", file.display()))?;
        for declaration in port_declarations(&document, file)? {
            let HostBinding::Variable(variable) = declaration.host_binding else {
                continue;
            };
            if generated_environment
                .get(&variable)
                .and_then(|value| port_template_name(value))
                .as_deref()
                != Some(contract_key)
            {
                continue;
            }
            let candidate = ComposePortTarget {
                service: declaration.service,
                container_port: declaration.container_port,
            };
            if target.replace(candidate).is_some() {
                anyhow::bail!("port contract `{contract_key}` maps to more than one Compose port");
            }
        }
    }
    let target = target.ok_or_else(|| {
        anyhow::anyhow!("port contract `{contract_key}` has no direct Compose port mapping")
    })?;
    if target.container_port != *expected_container {
        anyhow::bail!(
            "port contract `{contract_key}` maps to container port {}, expected {expected_container}",
            target.container_port
        );
    }
    Ok(target)
}

fn port_template_name(value: &str) -> Option<String> {
    value
        .trim()
        .strip_prefix("{{")?
        .strip_suffix("}}")?
        .trim()
        .strip_prefix("ports.")
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
pub fn apply(repo_root: &Path) -> anyhow::Result<ComposeApplyOutput> {
    apply_at(repo_root, None)
}

pub fn apply_at(repo_root: &Path, requested: Option<&Path>) -> anyhow::Result<ComposeApplyOutput> {
    let repo_root = std::fs::canonicalize(repo_root)?;
    let path = resolve_compose_file(&repo_root, requested)?;
    let contents = std::fs::read_to_string(&path)?;
    let plan = plan_contents(&repo_root, &path, &contents)?;
    let document: serde_yaml::Value = serde_yaml::from_str(&contents)?;
    let declarations = port_declarations(&document, &path)?;
    let fixed = detect_fixed_host_ports(&contents);
    let planned_fixed = plan
        .ports
        .iter()
        .filter(|port| port.current_host_port.is_some())
        .count();
    if fixed.len() != planned_fixed {
        anyhow::bail!(
            "cannot safely rewrite all fixed host ports in {}; use one port mapping per YAML line",
            plan.file.display()
        );
    }
    let mut changed_lines = 0usize;
    let mut output = Vec::new();

    for (index, line) in contents.lines().enumerate() {
        let Some(fixed) = fixed.iter().find(|fixed| fixed.file_line == index + 1) else {
            output.push(line.to_owned());
            continue;
        };
        let matches = plan
            .ports
            .iter()
            .filter(|port| port.current_host_port == Some(fixed.host_port))
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            anyhow::bail!(
                "cannot safely rewrite host port {} on line {}: expected one discovered service, found {}",
                fixed.host_port,
                fixed.file_line,
                matches.len()
            );
        }
        let port = matches[0];
        let replacement = if fixed.mapping.starts_with("published:") {
            format!("published: \"${{{}}}\"", port.env)
        } else {
            port.replacement.clone()
        };
        let updated = line.replacen(&fixed.mapping, &replacement, 1);
        if updated == line {
            anyhow::bail!(
                "could not safely locate the fixed host-port text on line {}",
                fixed.file_line
            );
        }
        output.push(updated);
        if fixed.mapping.starts_with("published:")
            && declarations.iter().any(|declaration| {
                declaration.host_binding == HostBinding::Fixed(fixed.host_port)
                    && declaration.host_ip.is_none()
            })
        {
            let indentation = line.strip_suffix(line.trim_start()).unwrap_or_default();
            output.push(format!("{indentation}host_ip: \"127.0.0.1\""));
        }
        changed_lines += 1;
    }

    if changed_lines > 0 {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Compose path has no parent"))?;
        let permissions = std::fs::metadata(&path)?.permissions();
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        use std::io::Write;
        temporary.write_all(output.join("\n").as_bytes())?;
        if contents.ends_with('\n') {
            temporary.write_all(b"\n")?;
        }
        temporary.as_file().set_permissions(permissions)?;
        temporary.as_file().sync_all()?;
        if std::fs::read(&path)? != contents.as_bytes() {
            anyhow::bail!(
                "{} changed after Compose planning; no edits were applied",
                plan.file.display()
            );
        }
        temporary.persist(&path).map_err(|error| error.error)?;
    }
    Ok(ComposeApplyOutput {
        file: plan.file,
        changed_lines,
    })
}

fn yaml_field<'a>(value: &'a serde_yaml::Value, key: &str) -> Option<&'a serde_yaml::Value> {
    value
        .as_mapping()?
        .get(serde_yaml::Value::String(key.to_owned()))
}

fn parse_compose_port(
    value: &serde_yaml::Value,
) -> Option<(u16, HostBinding, String, Option<String>)> {
    if let Some(mapping) = value.as_mapping() {
        let container = yaml_u16(mapping.get(serde_yaml::Value::String("target".into()))?)?;
        let host = match mapping.get(serde_yaml::Value::String("published".into())) {
            None => HostBinding::Missing,
            Some(value) => match yaml_u16(value) {
                Some(port) => HostBinding::Fixed(port),
                None => HostBinding::Variable(compose_variable(value.as_str()?)?),
            },
        };
        let protocol = mapping
            .get(serde_yaml::Value::String("protocol".into()))
            .and_then(serde_yaml::Value::as_str)
            .filter(|protocol| *protocol != "tcp")
            .map_or_else(String::new, |protocol| format!("/{protocol}"));
        let host_ip = mapping
            .get(serde_yaml::Value::String("host_ip".into()))
            .and_then(serde_yaml::Value::as_str);
        if host_ip.is_some_and(|host_ip| !safe_host_ip(host_ip)) {
            return None;
        }
        return Some((container, host, protocol, host_ip.map(str::to_owned)));
    }
    let value = value.as_str()?;
    let (mapping, protocol) = value
        .split_once('/')
        .map_or((value, String::new()), |(mapping, protocol)| {
            (mapping, format!("/{protocol}"))
        });
    let Some((host, container)) = rsplit_port_separator(mapping) else {
        return Some((mapping.parse().ok()?, HostBinding::Missing, protocol, None));
    };
    let mut host_ip = None;
    let host = if host.starts_with("${") {
        host
    } else if let Some((address, published)) = rsplit_port_separator(host) {
        let address = address.trim_matches(['[', ']']);
        if safe_host_ip(address) {
            host_ip = Some(address.to_owned());
            published
        } else {
            return None;
        }
    } else {
        host
    };
    let host = host.parse().map_or_else(
        |_| compose_variable(host).map(HostBinding::Variable),
        |port| Some(HostBinding::Fixed(port)),
    )?;
    Some((container.parse().ok()?, host, protocol, host_ip))
}

fn rsplit_port_separator(value: &str) -> Option<(&str, &str)> {
    let mut braces = 0usize;
    let mut brackets = 0usize;
    for (index, character) in value.char_indices().rev() {
        match character {
            '}' => braces += 1,
            '{' => braces = braces.saturating_sub(1),
            ']' => brackets += 1,
            '[' => brackets = brackets.saturating_sub(1),
            ':' if braces == 0 && brackets == 0 => {
                return Some((&value[..index], &value[index + 1..]));
            }
            _ => {}
        }
    }
    None
}

fn compose_variable(value: &str) -> Option<String> {
    let raw = value.strip_prefix('$')?;
    let (candidate, suffix) = if let Some(braced) = raw.strip_prefix('{') {
        let inner = braced.strip_suffix('}')?;
        let name_len = inner
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .count();
        (&inner[..name_len], &inner[name_len..])
    } else {
        (raw, "")
    };
    let name = candidate
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect::<String>();
    let safe_suffix = suffix.is_empty()
        || suffix
            .strip_prefix(":-")
            .or_else(|| suffix.strip_prefix('-'))
            .is_some_and(|default| {
                !default.is_empty() && default.chars().all(|c| c.is_ascii_digit())
            });
    (name == candidate
        && !name.is_empty()
        && safe_suffix
        && name
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic() || character == '_'))
    .then_some(name)
}

fn safe_host_ip(value: &str) -> bool {
    value == "localhost"
        || value
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback() || address.is_unspecified())
}

fn yaml_u16(value: &serde_yaml::Value) -> Option<u16> {
    value
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .or_else(|| value.as_str()?.parse().ok())
}

fn env_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn http_port(service: &str, container: u16, protocol: &str) -> bool {
    matches!(protocol, "" | "/tcp")
        && matches!(container, 80 | 3000 | 5173 | 8000 | 8080)
        && ["app", "api", "backend", "frontend", "server", "web"]
            .iter()
            .any(|candidate| service.eq_ignore_ascii_case(candidate))
}

pub fn base_args(manifest: &StacksteadManifest) -> Vec<String> {
    let mut args = vec![
        "compose".into(),
        "-p".into(),
        manifest.compose_project.clone(),
        "--env-file".into(),
        manifest.env_file.display().to_string(),
    ];
    for file in &manifest.compose_files {
        args.push("-f".into());
        args.push(file.display().to_string());
    }
    args.push("-f".into());
    args.push(ownership_override_path(manifest).display().to_string());
    args
}

pub fn write_ownership_override(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    let rendered = render_ownership_override(manifest)?;
    let path = ownership_override_path(manifest);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Compose ownership override has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(rendered.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&path)
        .map_err(|error| anyhow::anyhow!("cannot replace {}: {}", path.display(), error.error))?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn ownership_override_path(manifest: &StacksteadManifest) -> PathBuf {
    manifest.worktree.join(OWNERSHIP_OVERRIDE)
}

fn verify_ownership_override(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    let path = ownership_override_path(manifest);
    let expected = render_ownership_override(manifest)?;
    let actual = std::fs::read_to_string(&path).map_err(|error| {
        anyhow::anyhow!(
            "cannot read generated Compose ownership override {}: {error}; run `stackstead repair {}`",
            path.display(),
            manifest.stackstead_id
        )
    })?;
    if actual != expected {
        anyhow::bail!(
            "generated Compose ownership override {} does not match runtime token {}; run `stackstead repair {}`",
            path.display(),
            manifest.runtime_token,
            manifest.stackstead_id
        );
    }
    Ok(())
}

fn render_ownership_override(manifest: &StacksteadManifest) -> anyhow::Result<String> {
    let model = ownership_model(&manifest.compose_files)?;
    let labels = serde_json::json!({RUNTIME_TOKEN_LABEL: manifest.runtime_token});
    let entries = |names: &BTreeSet<String>| {
        names
            .iter()
            .map(|name| (name.clone(), serde_json::json!({"labels": labels})))
            .collect::<serde_json::Map<_, _>>()
    };
    let document = serde_json::json!({
        "services": entries(&model.services),
        "networks": entries(&model.networks),
        "volumes": entries(&model.volumes),
    });
    let mut output = String::from("# Generated by Stackstead. Do not edit by hand.\n");
    output.push_str(&serde_yaml::to_string(&document)?);
    Ok(output)
}

#[derive(Default)]
struct OwnershipModel {
    services: BTreeSet<String>,
    networks: BTreeSet<String>,
    volumes: BTreeSet<String>,
}

fn ownership_model(files: &[PathBuf]) -> anyhow::Result<OwnershipModel> {
    let mut model = OwnershipModel::default();
    let mut documents = Vec::new();
    let mut network_states = BTreeMap::new();
    let mut volume_states = BTreeMap::new();
    for file in files {
        let mut document: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(file)?)
            .map_err(|error| anyhow::anyhow!("cannot parse {}: {error}", file.display()))?;
        document.apply_merge().map_err(|error| {
            anyhow::anyhow!(
                "cannot resolve YAML merge keys in Compose file {}: {error}",
                file.display()
            )
        })?;
        if yaml_field(&document, "include").is_some() {
            anyhow::bail!(
                "Compose `include` is not supported in {}; Stackstead cannot attest included resources",
                file.display()
            );
        }
        collect_resource_states(&document, file, "networks", &mut network_states)?;
        collect_resource_states(&document, file, "volumes", &mut volume_states)?;
        documents.push((file, document));
    }
    model.networks.extend(
        network_states
            .iter()
            .filter(|(_, managed)| **managed)
            .map(|(name, _)| name.clone()),
    );
    model.volumes.extend(
        volume_states
            .iter()
            .filter(|(_, managed)| **managed)
            .map(|(name, _)| name.clone()),
    );
    let declared_volumes = volume_states.keys().cloned().collect();
    for (file, document) in documents {
        let Some(services_value) = yaml_field(&document, "services") else {
            continue;
        };
        let services = services_value.as_mapping().ok_or_else(|| {
            anyhow::anyhow!(
                "{} has a services value that is not a mapping",
                file.display()
            )
        })?;
        for (name, value) in services {
            let name = name.as_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "{} contains a non-string Compose service name",
                    file.display()
                )
            })?;
            let service = value.as_mapping().ok_or_else(|| {
                anyhow::anyhow!(
                    "Compose service `{name}` in {} is not a mapping",
                    file.display()
                )
            })?;
            if service.contains_key(serde_yaml::Value::String("extends".into())) {
                anyhow::bail!(
                    "Compose service `{name}` in {} uses unsupported `extends`; Stackstead cannot attest inherited resources",
                    file.display()
                );
            }
            if let Some(container_name) =
                service.get(serde_yaml::Value::String("container_name".into()))
                && !is_null_or_tagged_null(container_name)
            {
                let container_name = container_name.as_str().ok_or_else(|| {
                    anyhow::anyhow!(
                        "Compose service `{name}` in {} has a non-string container_name",
                        file.display()
                    )
                })?;
                if contains_interpolation(container_name) {
                    anyhow::bail!(
                        "Compose service `{name}` in {} uses interpolation in container_name; Stackstead requires a literal name for ownership attestation",
                        file.display()
                    );
                }
            }
            validate_service_volumes(name, service, &declared_volumes, file)?;
            model.services.insert(name.into());
        }
    }
    if !model.services.is_empty() && !network_states.contains_key("default") {
        model.networks.insert("default".into());
    }
    Ok(model)
}

fn collect_resource_states(
    document: &serde_yaml::Value,
    file: &Path,
    field: &str,
    output: &mut BTreeMap<String, bool>,
) -> anyhow::Result<()> {
    let Some(value) = yaml_field(document, field) else {
        return Ok(());
    };
    let values = value.as_mapping().ok_or_else(|| {
        anyhow::anyhow!("Compose `{field}` in {} is not a mapping", file.display())
    })?;
    for (name, value) in values {
        let name = name.as_str().ok_or_else(|| {
            anyhow::anyhow!(
                "{} contains a non-string Compose {field} name",
                file.display()
            )
        })?;
        let mapping = value.as_mapping();
        if mapping.is_none() && !value.is_null() {
            anyhow::bail!(
                "Compose {field} `{name}` in {} is not a mapping",
                file.display()
            );
        }
        let external = resource_is_external(mapping, field, name, file)?;
        let custom_name = mapping
            .and_then(|mapping| mapping.get(serde_yaml::Value::String("name".into())))
            .filter(|value| !is_null_or_tagged_null(value))
            .map(|value| {
                value.as_str().ok_or_else(|| {
                    anyhow::anyhow!(
                        "Compose {field} `{name}` in {} has a non-string name",
                        file.display()
                    )
                })
            })
            .transpose()?;
        if custom_name.is_some_and(contains_interpolation) {
            anyhow::bail!(
                "Compose {field} `{name}` in {} uses interpolation in name; Stackstead requires a literal name for ownership attestation",
                file.display()
            );
        }
        if !external && custom_name.is_some() {
            anyhow::bail!(
                "Compose managed {field} `{name}` in {} uses a global custom name; Stackstead requires project-scoped managed resource names",
                file.display()
            );
        }
        if output.contains_key(name) {
            anyhow::bail!(
                "Compose {field} `{name}` is declared in multiple Compose files; consolidate the declaration so Stackstead can attest the effective resource name"
            );
        }
        output.insert(name.into(), !external);
    }
    Ok(())
}

fn validate_service_volumes(
    service_name: &str,
    service: &serde_yaml::Mapping,
    declared_volumes: &BTreeSet<String>,
    file: &Path,
) -> anyhow::Result<()> {
    let Some(volumes) = service
        .get(serde_yaml::Value::String("volumes".into()))
        .and_then(serde_yaml::Value::as_sequence)
    else {
        return Ok(());
    };
    for volume in volumes {
        let source = if let Some(value) = volume.as_str() {
            match value.split_once(':') {
                Some((source, _)) if !source.is_empty() && !source.starts_with(['.', '/']) => {
                    Some(source)
                }
                Some(_) => None,
                None => anyhow::bail!(
                    "Compose service `{service_name}` in {} uses an anonymous volume; declare a named top-level volume so Stackstead can attest it",
                    file.display()
                ),
            }
        } else if let Some(mapping) = volume.as_mapping() {
            let kind = match mapping.get(serde_yaml::Value::String("type".into())) {
                Some(value) => value.as_str().ok_or_else(|| {
                    anyhow::anyhow!(
                        "Compose service `{service_name}` in {} has a non-string volume type",
                        file.display()
                    )
                })?,
                None => "volume",
            };
            if kind != "volume" {
                None
            } else {
                Some(
                    mapping
                        .get(serde_yaml::Value::String("source".into()))
                        .and_then(serde_yaml::Value::as_str)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Compose service `{service_name}` in {} uses an anonymous volume; declare a named top-level volume so Stackstead can attest it",
                                file.display()
                            )
                        })?,
                )
            }
        } else {
            anyhow::bail!(
                "Compose service `{service_name}` in {} has an unsupported volume declaration",
                file.display()
            );
        };
        if let Some(source) = source
            && !declared_volumes.contains(source)
        {
            anyhow::bail!(
                "Compose service `{service_name}` in {} uses named volume `{source}` without a top-level declaration",
                file.display()
            );
        }
    }
    Ok(())
}

fn docker_environment(
    manifest: &StacksteadManifest,
) -> anyhow::Result<(Vec<String>, BTreeMap<String, String>)> {
    let generated = manifest.validated_environment()?;
    let removed = generated
        .keys()
        .filter(|key| !crate::config::reserved_process_env(key))
        .cloned()
        .collect();
    let environment = BTreeMap::from([(
        "COMPOSE_PROJECT_NAME".into(),
        manifest.compose_project.clone(),
    )]);
    Ok((removed, environment))
}

fn run_docker_compose(
    manifest: &StacksteadManifest,
    args: &[String],
) -> anyhow::Result<std::process::Output> {
    verify_ownership_override(manifest)?;
    run_docker(manifest, args)
}

fn run_docker(
    manifest: &StacksteadManifest,
    args: &[String],
) -> anyhow::Result<std::process::Output> {
    let (removed, environment) = docker_environment(manifest)?;
    command::run_sanitized(
        "docker",
        args,
        &manifest.worktree,
        &environment,
        removed.iter().map(String::as_str),
    )
}

fn run_docker_control(
    manifest: &StacksteadManifest,
    args: &[String],
) -> anyhow::Result<std::process::Output> {
    let removed = manifest
        .env_keys
        .iter()
        .filter(|key| !crate::config::reserved_process_env(key))
        .map(String::as_str);
    let environment = BTreeMap::from([(
        "COMPOSE_PROJECT_NAME".into(),
        manifest.compose_project.clone(),
    )]);
    let cwd = if manifest.worktree.is_dir() {
        &manifest.worktree
    } else {
        &manifest.repo_root
    };
    command::run_sanitized("docker", args, cwd, &environment, removed)
}

pub fn up(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    let resources_present = verify_runtime_resources(manifest)?;
    if runtime_claim_exists(manifest)? {
        verify_runtime_claim(manifest)?;
    } else if resources_present {
        anyhow::bail!(
            "Compose namespace `{}` has runtime resources but no Stackstead ownership claim",
            manifest.compose_project
        );
    } else {
        ensure_runtime_claim(manifest)?;
    }
    verify_runtime_claim(manifest)?;
    let _ = verify_runtime_resources(manifest)?;
    let mut args = base_args(manifest);
    args.extend(["up".into(), "-d".into()]);
    run_docker_compose(manifest, &args)?;
    let _ = verify_runtime_resources(manifest)?;
    Ok(())
}

pub fn stop(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    let resources_present = verify_runtime_resources(manifest)?;
    if !runtime_claim_exists(manifest)? {
        if resources_present {
            anyhow::bail!(
                "Compose namespace `{}` has runtime resources but no Stackstead ownership claim",
                manifest.compose_project
            );
        }
        return Ok(());
    }
    verify_runtime_claim(manifest)?;
    if !resources_present {
        return Ok(());
    }
    let mut args = base_args(manifest);
    args.push("stop".into());
    run_docker_compose(manifest, &args)?;
    Ok(())
}

pub fn down_volumes(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    let resources_present = verify_runtime_resources(manifest)?;
    if !runtime_claim_exists(manifest)? {
        if resources_present {
            anyhow::bail!(
                "Compose namespace `{}` has runtime resources but no Stackstead ownership claim",
                manifest.compose_project
            );
        }
        return Ok(());
    }
    verify_runtime_claim(manifest)?;
    if !resources_present {
        return Ok(());
    }
    let mut args = base_args(manifest);
    args.extend([
        "down".into(),
        "-v".into(),
        "--remove-orphans".into(),
        "--rmi".into(),
        "local".into(),
    ]);
    run_docker_compose(manifest, &args)?;
    if verify_runtime_resources(manifest)? {
        remove_labeled_runtime_resources(manifest)?;
    }
    if verify_runtime_resources(manifest)? {
        anyhow::bail!(
            "Compose namespace `{}` still has Stackstead runtime resources after teardown; retaining recovery state and ownership claim",
            manifest.compose_project
        );
    }
    Ok(())
}

pub fn prepare_owned_source_removal(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if manifest.source_ownership != crate::manifest::SourceOwnership::Stackstead {
        return Ok(());
    }
    if !runtime_claim_exists(manifest)? {
        return Ok(());
    }
    verify_runtime_claim(manifest)?;
    if !manifest.worktree.is_dir() {
        anyhow::bail!(
            "managed worktree is missing at {}",
            manifest.worktree.display()
        );
    }
    let source = manifest
        .worktree
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("managed worktree path is not UTF-8"))?;
    let helper = format!("{}-stackstead-owner", manifest.compose_project);
    let list = vec![
        "container".into(),
        "ls".into(),
        "--all".into(),
        "--filter".into(),
        format!("name=^/{helper}$"),
        "--filter".into(),
        format!("label={COMPOSE_PROJECT_LABEL}={}", manifest.compose_project),
        "--filter".into(),
        format!("label={RUNTIME_TOKEN_LABEL}={}", manifest.runtime_token),
        "--format".into(),
        "{{.ID}}".into(),
    ];
    let existing = String::from_utf8(run_docker_control(manifest, &list)?.stdout)?;
    let existing = existing
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    if existing.len() > 1 {
        anyhow::bail!(
            "more than one exact ownership helper exists for {}",
            manifest.stackstead_id
        );
    }
    if let Some(identifier) = existing.first() {
        verify_resource_label(manifest, "container", identifier, ".Config.Labels")?;
        run_docker_control(
            manifest,
            &[
                "container".into(),
                "rm".into(),
                "--force".into(),
                (*identifier).into(),
            ],
        )?;
    }
    #[cfg(unix)]
    let (uid, gid) = {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(&manifest.worktree)?;
        (metadata.uid(), metadata.gid())
    };
    #[cfg(not(unix))]
    let (uid, gid) = (0_u32, 0_u32);
    let mut args = vec![
        "run".into(),
        "--rm".into(),
        "--pull=missing".into(),
        "--name".into(),
        helper,
        "--label".into(),
        format!("{COMPOSE_PROJECT_LABEL}={}", manifest.compose_project),
        "--label".into(),
        format!("{RUNTIME_TOKEN_LABEL}={}", manifest.runtime_token),
        "--user".into(),
        "0:0".into(),
    ];
    #[cfg(target_os = "linux")]
    args.push("--userns=host".into());
    args.extend([
        "--mount".into(),
        ownership_bind_mount(source),
        OWNERSHIP_HELPER_IMAGE.into(),
        "sh".into(),
        "-ceu".into(),
        "chown -R \"$1:$2\" /stackstead-source; chmod -R u+rwX /stackstead-source".into(),
        "stackstead-owner".into(),
        uid.to_string(),
        gid.to_string(),
    ]);
    run_docker_control(manifest, &args)?;
    Ok(())
}

fn ownership_bind_mount(source: &str) -> String {
    format!(
        "type=bind,\"src={}\",dst=/stackstead-source",
        source.replace('"', "\"\"")
    )
}

fn runtime_claim_name(manifest: &StacksteadManifest) -> String {
    format!("{}-stackstead-claim", manifest.compose_project)
}

fn ensure_runtime_claim(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    let args = vec![
        "volume".into(),
        "create".into(),
        "--label".into(),
        format!("{COMPOSE_PROJECT_LABEL}={}", manifest.compose_project),
        "--label".into(),
        format!("{RUNTIME_TOKEN_LABEL}={}", manifest.runtime_token),
        runtime_claim_name(manifest),
    ];
    run_docker_control(manifest, &args)?;
    Ok(())
}

fn verify_runtime_claim(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    verify_resource_label(manifest, "volume", &runtime_claim_name(manifest), ".Labels").map_err(
        |error| {
            anyhow::anyhow!(
                "Compose namespace `{}` is not owned by runtime token {}: {error}",
                manifest.compose_project,
                manifest.runtime_token
            )
        },
    )
}

pub fn verify_owned_runtime(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if !runtime_claim_exists(manifest)? {
        anyhow::bail!(
            "Compose namespace `{}` has no Stackstead ownership claim",
            manifest.compose_project
        );
    }
    verify_runtime_claim(manifest)?;
    let _ = verify_runtime_resources(manifest)?;
    Ok(())
}

pub(crate) fn remove_runtime_claim(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if !runtime_claim_exists(manifest)? {
        return Ok(());
    }
    verify_runtime_claim(manifest)?;
    if verify_labeled_runtime_resources(manifest)? {
        anyhow::bail!(
            "Compose namespace `{}` still has Stackstead runtime resources; refusing to remove its ownership claim",
            manifest.compose_project
        );
    }
    let args = vec!["volume".into(), "rm".into(), runtime_claim_name(manifest)];
    run_docker_control(manifest, &args)?;
    Ok(())
}

fn runtime_claim_exists(manifest: &StacksteadManifest) -> anyhow::Result<bool> {
    let args = vec![
        "volume".into(),
        "ls".into(),
        "--format".into(),
        "{{.Name}}".into(),
    ];
    let output = run_docker_control(manifest, &args)?;
    let names = String::from_utf8(output.stdout)?;
    Ok(names
        .lines()
        .map(str::trim)
        .any(|name| name == runtime_claim_name(manifest)))
}

fn verify_runtime_resources(manifest: &StacksteadManifest) -> anyhow::Result<bool> {
    let expected = verify_expected_runtime_names(manifest)?;
    let labeled = verify_labeled_runtime_resources(manifest)?;
    Ok(expected || labeled)
}

fn remove_labeled_runtime_resources(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    for (kind, list_args, identifier_template, labels_template, remove_args) in [
        (
            "container",
            vec!["container", "ls", "--all"],
            "{{.ID}}",
            ".Config.Labels",
            vec!["container", "rm", "--force"],
        ),
        (
            "network",
            vec!["network", "ls"],
            "{{.ID}}",
            ".Labels",
            vec!["network", "rm"],
        ),
        (
            "volume",
            vec!["volume", "ls"],
            "{{.Name}}",
            ".Labels",
            vec!["volume", "rm"],
        ),
    ] {
        let mut args = list_args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        args.extend([
            "--filter".into(),
            format!("label={COMPOSE_PROJECT_LABEL}={}", manifest.compose_project),
            "--format".into(),
            identifier_template.into(),
        ]);
        let output = run_docker_control(manifest, &args)?;
        for identifier in String::from_utf8(output.stdout)?
            .lines()
            .map(str::trim)
            .filter(|identifier| !identifier.is_empty())
        {
            if kind == "volume" && identifier == runtime_claim_name(manifest) {
                continue;
            }
            verify_resource_label(manifest, kind, identifier, labels_template)?;
            let mut args = remove_args
                .iter()
                .map(|value| (*value).into())
                .collect::<Vec<_>>();
            args.push(identifier.into());
            run_docker_control(manifest, &args)?;
        }
    }
    Ok(())
}

fn verify_labeled_runtime_resources(manifest: &StacksteadManifest) -> anyhow::Result<bool> {
    let mut present = false;
    for (kind, list_args, identifier_template, labels_template) in [
        (
            "container",
            vec!["container", "ls", "--all"],
            "{{.ID}}",
            ".Config.Labels",
        ),
        ("network", vec!["network", "ls"], "{{.ID}}", ".Labels"),
        ("volume", vec!["volume", "ls"], "{{.Name}}", ".Labels"),
    ] {
        let mut args = list_args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        args.extend([
            "--filter".into(),
            format!("label={COMPOSE_PROJECT_LABEL}={}", manifest.compose_project),
            "--format".into(),
            identifier_template.into(),
        ]);
        let output = run_docker_control(manifest, &args)?;
        let identifiers = String::from_utf8(output.stdout)?;
        for identifier in identifiers
            .lines()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            if kind == "volume" && identifier == runtime_claim_name(manifest) {
                continue;
            }
            present = true;
            verify_resource_label(manifest, kind, identifier, labels_template)?;
        }
    }
    Ok(present)
}

fn verify_expected_runtime_names(manifest: &StacksteadManifest) -> anyhow::Result<bool> {
    let mut present = false;
    for (kind, format, labels_template, expected) in expected_runtime_names(manifest)? {
        let mut args = vec![kind.clone(), "ls".into()];
        if kind == "container" {
            args.push("--all".into());
        }
        args.extend(["--format".into(), format]);
        let output = run_docker_control(manifest, &args)?;
        let names = String::from_utf8(output.stdout)?;
        let actual = names
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect::<BTreeSet<_>>();
        for name in expected
            .iter()
            .filter(|name| actual.contains(name.as_str()))
        {
            present = true;
            verify_resource_label(manifest, &kind, name, &labels_template)?;
        }
    }
    Ok(present)
}

type ExpectedRuntimeNames = Vec<(String, String, String, BTreeSet<String>)>;

fn expected_runtime_names(manifest: &StacksteadManifest) -> anyhow::Result<ExpectedRuntimeNames> {
    let mut services = BTreeMap::<String, Option<String>>::new();
    let mut networks = BTreeMap::<String, (bool, Option<String>)>::new();
    let mut volumes = BTreeMap::<String, (bool, Option<String>)>::new();
    for file in &manifest.compose_files {
        let mut document: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(file)?)?;
        document.apply_merge()?;
        collect_runtime_names(&document, "networks", &mut networks, file)?;
        collect_runtime_names(&document, "volumes", &mut volumes, file)?;
        let Some(values) = yaml_field(&document, "services") else {
            continue;
        };
        let values = values.as_mapping().ok_or_else(|| {
            anyhow::anyhow!(
                "{} has a services value that is not a mapping",
                file.display()
            )
        })?;
        for (name, value) in values {
            let name = name.as_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "{} contains a non-string Compose service name",
                    file.display()
                )
            })?;
            let value = value.as_mapping().ok_or_else(|| {
                anyhow::anyhow!(
                    "Compose service `{name}` in {} is not a mapping",
                    file.display()
                )
            })?;
            let custom = match value.get(serde_yaml::Value::String("container_name".into())) {
                None => services.get(name).cloned().flatten(),
                Some(value) if is_null_or_tagged_null(value) => None,
                Some(value) => Some(value.as_str().map(str::to_owned).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Compose service `{name}` in {} has a non-string container_name",
                        file.display()
                    )
                })?),
            };
            if custom.as_deref().is_some_and(contains_interpolation) {
                anyhow::bail!(
                    "Compose service `{name}` in {} uses interpolation in container_name; Stackstead requires a literal name for ownership attestation",
                    file.display()
                );
            }
            services.insert(name.into(), custom);
        }
    }
    let mut container_names = BTreeSet::new();
    for (service, custom) in services {
        if let Some(custom) = custom {
            container_names.insert(custom);
        } else {
            container_names.insert(format!("{}-{service}-1", manifest.compose_project));
            container_names.insert(format!("{}_{service}_1", manifest.compose_project));
        }
    }
    let mut network_names = BTreeSet::from([
        format!("{}_default", manifest.compose_project),
        format!("{}-default", manifest.compose_project),
    ]);
    for (name, (managed, custom)) in networks {
        if managed {
            network_names
                .insert(custom.unwrap_or_else(|| format!("{}_{name}", manifest.compose_project)));
        }
    }
    let volume_names = volumes
        .into_iter()
        .filter(|(_, (managed, _))| *managed)
        .map(|(name, (_, custom))| {
            custom.unwrap_or_else(|| format!("{}_{name}", manifest.compose_project))
        })
        .collect();
    Ok(vec![
        (
            "container".into(),
            "{{.Names}}".into(),
            ".Config.Labels".into(),
            container_names,
        ),
        (
            "network".into(),
            "{{.Name}}".into(),
            ".Labels".into(),
            network_names,
        ),
        (
            "volume".into(),
            "{{.Name}}".into(),
            ".Labels".into(),
            volume_names,
        ),
    ])
}

fn collect_runtime_names(
    document: &serde_yaml::Value,
    field: &str,
    output: &mut BTreeMap<String, (bool, Option<String>)>,
    file: &Path,
) -> anyhow::Result<()> {
    let Some(values) = yaml_field(document, field) else {
        return Ok(());
    };
    let values = values.as_mapping().ok_or_else(|| {
        anyhow::anyhow!("Compose `{field}` in {} is not a mapping", file.display())
    })?;
    for (name, value) in values {
        let name = name.as_str().ok_or_else(|| {
            anyhow::anyhow!(
                "{} contains a non-string Compose {field} name",
                file.display()
            )
        })?;
        let mapping = value.as_mapping();
        if mapping.is_none() && !value.is_null() {
            anyhow::bail!(
                "Compose {field} `{name}` in {} is not a mapping",
                file.display()
            );
        }
        let managed = !resource_is_external(mapping, field, name, file)?;
        let custom = mapping
            .and_then(|mapping| mapping.get(serde_yaml::Value::String("name".into())))
            .filter(|value| !is_null_or_tagged_null(value))
            .map(|value| {
                value.as_str().map(str::to_owned).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Compose {field} `{name}` in {} has a non-string name",
                        file.display()
                    )
                })
            })
            .transpose()?;
        if custom.as_deref().is_some_and(contains_interpolation) {
            anyhow::bail!(
                "Compose {field} `{name}` in {} uses interpolation in name; Stackstead requires a literal name for ownership attestation",
                file.display()
            );
        }
        if managed && custom.is_some() {
            anyhow::bail!(
                "Compose managed {field} `{name}` in {} uses a global custom name; Stackstead requires project-scoped managed resource names",
                file.display()
            );
        }
        if output.contains_key(name) {
            anyhow::bail!(
                "Compose {field} `{name}` is declared in multiple Compose files; consolidate the declaration so Stackstead can attest the effective resource name"
            );
        }
        output.insert(name.into(), (managed, custom));
    }
    Ok(())
}

fn contains_interpolation(value: &str) -> bool {
    value.contains('$')
}

fn is_null_or_tagged_null(value: &serde_yaml::Value) -> bool {
    match value {
        serde_yaml::Value::Null => true,
        serde_yaml::Value::Tagged(value) => is_null_or_tagged_null(&value.value),
        _ => false,
    }
}

fn resource_is_external(
    mapping: Option<&serde_yaml::Mapping>,
    field: &str,
    name: &str,
    file: &Path,
) -> anyhow::Result<bool> {
    let Some(value) =
        mapping.and_then(|mapping| mapping.get(serde_yaml::Value::String("external".into())))
    else {
        return Ok(false);
    };
    value.as_bool().ok_or_else(|| {
        anyhow::anyhow!(
            "Compose {field} `{name}` in {} uses a non-boolean external value; use `external: true` with a top-level literal `name` if needed",
            file.display()
        )
    })
}

fn verify_resource_label(
    manifest: &StacksteadManifest,
    kind: &str,
    identifier: &str,
    labels_template: &str,
) -> anyhow::Result<()> {
    let args = vec![
        kind.into(),
        "inspect".into(),
        "--format".into(),
        format!("{{{{json {labels_template}}}}}"),
        identifier.into(),
    ];
    let output = run_docker_control(manifest, &args)?;
    let labels: serde_json::Value =
        serde_json::from_slice(output.stdout.trim_ascii()).map_err(|error| {
            anyhow::anyhow!("Docker returned invalid labels for {kind} `{identifier}`: {error}")
        })?;
    let token = labels
        .as_object()
        .and_then(|labels| labels.get(RUNTIME_TOKEN_LABEL))
        .and_then(serde_json::Value::as_str);
    if token != Some(manifest.runtime_token.as_str()) {
        anyhow::bail!(
            "refusing to target foreign {kind} `{identifier}` in Compose project `{}`: ownership label is missing or mismatched",
            manifest.compose_project
        );
    }
    Ok(())
}

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

fn parse_service_observations(output: &[u8]) -> anyhow::Result<Vec<ServiceObservation>> {
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

fn service_running_args(manifest: &StacksteadManifest, service: &str) -> Vec<String> {
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

fn running_service_output(output: &[u8]) -> bool {
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

fn endpoint_matches(endpoint: &str, host: &str, port: u16) -> bool {
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
fn endpoint_port(endpoint: &str) -> Option<u16> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedPort {
    pub file_line: usize,
    pub host_port: u16,
    pub mapping: String,
}

pub fn detect_fixed_host_ports(contents: &str) -> Vec<FixedPort> {
    contents
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("published:") {
                let raw = value.trim();
                let published = raw.trim_matches(['\'', '"']);
                return published.parse::<u16>().ok().map(|host_port| FixedPort {
                    file_line: index + 1,
                    host_port,
                    mapping: format!("published: {raw}"),
                });
            }
            let mapping = trimmed.strip_prefix('-')?.trim().trim_matches(['\'', '"']);
            if mapping.contains("${") || mapping.contains("{{") {
                return None;
            }
            let parts = mapping.split(':').collect::<Vec<_>>();
            let host = match parts.as_slice() {
                [host, container] if container_port(container).is_some() => *host,
                [ip, host, container]
                    if (ip.parse::<std::net::IpAddr>().is_ok() || *ip == "localhost")
                        && container_port(container).is_some() =>
                {
                    *host
                }
                _ => return None,
            };
            host.parse::<u16>().ok().map(|host_port| FixedPort {
                file_line: index + 1,
                host_port,
                mapping: mapping.to_string(),
            })
        })
        .collect()
}

fn container_port(value: &str) -> Option<u16> {
    value.split('/').next()?.parse().ok()
}

pub fn fixed_ports_in_file(path: &Path) -> anyhow::Result<Vec<FixedPort>> {
    Ok(detect_fixed_host_ports(&std::fs::read_to_string(path)?))
}

pub fn unbound_ports_in_file(path: &Path) -> anyhow::Result<Vec<(String, u16)>> {
    let document: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(path)?)?;
    let Some(services) = yaml_field(&document, "services").and_then(serde_yaml::Value::as_mapping)
    else {
        return Ok(vec![]);
    };
    let mut unbound = Vec::new();
    for (service, value) in services {
        let Some(service) = service.as_str() else {
            continue;
        };
        let Some(ports) = yaml_field(value, "ports").and_then(serde_yaml::Value::as_sequence)
        else {
            continue;
        };
        for value in ports {
            if let Some((container, HostBinding::Missing, _, _)) = parse_compose_port(value) {
                unbound.push((service.into(), container));
            }
        }
    }
    Ok(unbound)
}

pub fn all_interface_ports_in_file(path: &Path) -> anyhow::Result<Vec<(String, u16)>> {
    let document: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(path)?)?;
    let mut exposed = Vec::new();
    for declaration in port_declarations(&document, path)? {
        let all_interfaces = declaration.host_ip.as_deref().is_none_or(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_unspecified())
        });
        if all_interfaces {
            exposed.push((declaration.name, declaration.container_port));
        }
    }
    Ok(exposed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};

    fn manifest() -> anyhow::Result<StacksteadManifest> {
        serde_json::from_value(serde_json::json!({
            "kind":"StacksteadManifest","version":"2","stackstead_id":"a-b123","slug":"a","short_id":"b123",
            "runtime_token":"0123456789abcdef0123456789abcdef",
            "project":"demo","branch":"a","base":"main","repo_root":"/repo","project_state_root":"/state",
            "source_ownership":"stackstead",
            "stackstead_root":"/state/demo/a-b123","worktree":"/state/demo/a-b123/source","state_dir":"/state/demo/a-b123/state",
            "compose_project":"demo-a-b123","compose_files":["/state/demo/a-b123/source/compose.yml"],
            "ports":{},"container_ports":{},"urls":{},"env_file":"/state/demo/a-b123/source/.stackstead/.env",
            "agent_context":"/x","pointer_file":"/y","event_log":"/z","env_keys":[],
            "status":{"source":"created","dependencies":"unknown","runtime":"stopped","database":"unknown","health":"unknown"},
            "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"
        }))
        .test_context("parse manifest fixture")
    }

    #[test]
    fn compose_arguments_use_manifest_contract() -> anyhow::Result<()> {
        let manifest = manifest()?;
        let args = base_args(&manifest);
        assert_eq!(args[0], "compose");
        assert!(args.contains(&"demo-a-b123".to_string()));
        assert!(args.contains(&"/state/demo/a-b123/source/compose.yml".to_string()));
        assert!(
            args.contains(
                &"/state/demo/a-b123/source/.stackstead/compose-ownership.yaml".to_string()
            )
        );
        Ok(())
    }

    #[test]
    fn ownership_mount_quotes_valid_commas_and_quotes() -> anyhow::Result<()> {
        assert_eq!(
            ownership_bind_mount("/tmp/source,\"quoted\""),
            "type=bind,\"src=/tmp/source,\"\"quoted\"\"\",dst=/stackstead-source"
        );
        Ok(())
    }

    #[test]
    fn parses_array_and_line_delimited_service_observations() -> anyhow::Result<()> {
        let array = br#"[
          {"Name":"demo-web-1","Service":"web","State":"running","ExitCode":0},
          {"Name":"demo-init-1","Service":"init","State":"exited","ExitCode":0},
          {"Name":"demo-migrate-1","Service":"migrate","State":"exited","ExitCode":7}
        ]"#;
        let observations = parse_service_observations(array).test()?;
        assert_eq!(
            observations
                .iter()
                .map(|service| (service.service.as_str(), service.status()))
                .collect::<Vec<_>>(),
            [
                ("init", "completed (0)".into()),
                ("migrate", "exited (7)".into()),
                ("web", "running".into()),
            ]
        );

        let lines = br#"{"Name":"demo-web-1","Service":"web","State":"running","ExitCode":0}
{"Name":"demo-init-1","Service":"init","State":"exited","ExitCode":0}"#;
        assert_eq!(parse_service_observations(lines).test()?.len(), 2);
        Ok(())
    }

    #[test]
    fn ownership_override_attests_every_direct_managed_resource() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let compose = directory.path().join("compose.yaml");
        std::fs::write(
            &compose,
            r#"services:
  web:
    image: nginx
    networks: [backend]
    volumes: [cache:/cache]
networks:
  backend: {}
  upstream:
    external: true
volumes:
  cache: {}
  external-data:
    external: true
"#,
        )
        .test()?;
        let mut manifest = manifest()?;
        manifest.worktree = directory.path().into();
        manifest.compose_files = vec![compose];
        let rendered = render_ownership_override(&manifest).test()?;
        let document: serde_yaml::Value = serde_yaml::from_str(&rendered).test()?;
        for (field, names) in [
            ("services", &["web"][..]),
            ("networks", &["backend", "default"][..]),
            ("volumes", &["cache"][..]),
        ] {
            let values = yaml_field(&document, field)
                .and_then(serde_yaml::Value::as_mapping)
                .test()?;
            assert_eq!(values.len(), names.len());
            for name in names {
                let token = values
                    .get(serde_yaml::Value::String((*name).into()))
                    .and_then(|resource| yaml_field(resource, "labels"))
                    .and_then(|labels| yaml_field(labels, RUNTIME_TOKEN_LABEL))
                    .and_then(serde_yaml::Value::as_str);
                assert_eq!(token, Some(manifest.runtime_token.as_str()));
            }
        }
        assert!(!rendered.contains("upstream:"));
        assert!(!rendered.contains("external-data:"));
        Ok(())
    }

    #[test]
    fn ownership_override_rejects_unattestable_compose_shapes() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let compose = directory.path().join("compose.yaml");
        let mut manifest = manifest()?;
        manifest.compose_files = vec![compose.clone()];
        for (contents, expected) in [
            (
                "include: shared.yaml\nservices:\n  web: {image: nginx}\n",
                "`include`",
            ),
            ("services:\n  web:\n    extends: base\n", "`extends`"),
            (
                "services:\n  web:\n    volumes: [/cache]\n",
                "anonymous volume",
            ),
            (
                "services:\n  web:\n    volumes: [cache:/cache]\n",
                "without a top-level declaration",
            ),
        ] {
            std::fs::write(&compose, contents).test()?;
            let error = render_ownership_override(&manifest).test_err()?.to_string();
            assert!(error.contains(expected), "unexpected error: {error}");
        }
        Ok(())
    }

    #[test]
    fn ownership_override_rejects_non_string_optional_fields() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let compose = directory.path().join("compose.yaml");
        let mut manifest = manifest()?;
        manifest.compose_files = vec![compose.clone()];
        for (contents, subject, expected) in [
            (
                "services:\n  web:\n    container_name: 7\n",
                "service `web`",
                "non-string container_name",
            ),
            (
                "services:\n  web:\n    volumes:\n      - type: 7\n        source: data\n        target: /data\nvolumes:\n  data: {}\n",
                "service `web`",
                "non-string volume type",
            ),
            (
                "services:\n  web: {}\nvolumes:\n  data:\n    name: 7\n",
                "volumes `data`",
                "non-string name",
            ),
        ] {
            std::fs::write(&compose, contents).test()?;
            let error = render_ownership_override(&manifest).test_err()?.to_string();
            assert!(error.contains(subject), "unexpected error: {error}");
            assert!(error.contains(expected), "unexpected error: {error}");
            assert!(
                error.contains(&compose.display().to_string()),
                "unexpected error: {error}"
            );
        }
        Ok(())
    }

    #[test]
    fn ownership_and_runtime_names_accept_null_name_resets() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let primary = directory.path().join("compose.yaml");
        let overlay = directory.path().join("compose.reset.yaml");
        std::fs::write(
            &primary,
            "services:\n  web:\n    container_name: custom-web\n",
        )
        .test()?;
        std::fs::write(
            &overlay,
            "services:\n  web:\n    container_name: !reset null\nvolumes:\n  data:\n    name: null\nnetworks:\n  backend:\n    name: !reset null\n",
        )
        .test()?;
        let mut manifest = manifest()?;
        manifest.compose_files = vec![primary, overlay];

        assert!(render_ownership_override(&manifest).is_ok());
        let runtime_names = expected_runtime_names(&manifest).test()?;
        let names = |kind: &str| {
            runtime_names
                .iter()
                .find(|(candidate, ..)| candidate == kind)
                .map(|(_, _, _, names)| names)
                .test()
        };
        assert!(names("container")?.contains("demo-a-b123-web-1"));
        assert!(!names("container")?.contains("custom-web"));
        assert!(names("volume")?.contains("demo-a-b123_data"));
        assert!(names("network")?.contains("demo-a-b123_backend"));
        Ok(())
    }

    #[test]
    fn ownership_override_defaults_an_omitted_volume_type() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let compose = directory.path().join("compose.yaml");
        std::fs::write(
            &compose,
            "services:\n  web:\n    volumes:\n      - source: data\n        target: /data\nvolumes:\n  data: {}\n",
        )
        .test()?;
        let mut manifest = manifest()?;
        manifest.compose_files = vec![compose];

        assert!(render_ownership_override(&manifest).is_ok());
        Ok(())
    }

    #[test]
    fn ownership_override_supports_later_volume_overlays_and_external_volumes() -> anyhow::Result<()>
    {
        let directory = tempfile::tempdir().test()?;
        let primary = directory.path().join("compose.yaml");
        let overlay = directory.path().join("compose.volumes.yaml");
        std::fs::write(
            &primary,
            "services:\n  web:\n    volumes: [cache:/cache, shared:/shared]\n",
        )
        .test()?;
        std::fs::write(
            &overlay,
            "volumes:\n  cache:\n  shared:\n    external: true\nnetworks:\n  default:\n    external: true\n",
        )
        .test()?;
        let mut manifest = manifest()?;
        manifest.compose_files = vec![primary, overlay];
        let rendered = render_ownership_override(&manifest).test()?;
        assert!(rendered.contains("cache:"));
        assert!(!rendered.contains("shared:"));
        assert!(!rendered.contains("default:"));
        Ok(())
    }

    #[test]
    fn ownership_override_rejects_interpolated_and_redeclared_resource_names() -> anyhow::Result<()>
    {
        let directory = tempfile::tempdir().test()?;
        let primary = directory.path().join("compose.yaml");
        let overlay = directory.path().join("compose.overlay.yaml");
        let mut manifest = manifest()?;
        manifest.compose_files = vec![primary.clone()];

        std::fs::write(
            &primary,
            "services:\n  web: {image: nginx}\nvolumes:\n  data:\n    name: ${GLOBAL_DATA}\n",
        )
        .test()?;
        let error = render_ownership_override(&manifest).test_err()?.to_string();
        assert!(error.contains("requires a literal name"), "{error}");

        std::fs::write(
            &primary,
            "services:\n  web: {image: nginx}\nvolumes:\n  data: {}\n",
        )
        .test()?;
        std::fs::write(&overlay, "volumes:\n  data:\n    driver: local\n").test()?;
        manifest.compose_files.push(overlay);
        let error = render_ownership_override(&manifest).test_err()?.to_string();
        assert!(
            error.contains("declared in multiple Compose files"),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn selected_service_running_check_is_manifest_scoped() -> anyhow::Result<()> {
        let manifest = manifest()?;
        let args = service_running_args(&manifest, "frontend");
        assert_eq!(
            &args[args.len() - 5..],
            ["ps", "--status", "running", "--quiet", "frontend"]
        );
        assert!(args.windows(2).any(|args| args == ["-p", "demo-a-b123"]));
        assert!(
            args.windows(2)
                .any(|args| { args == ["-f", "/state/demo/a-b123/source/compose.yml"] })
        );
        assert!(running_service_output(b"frontend-container\n"));
        assert!(!running_service_output(b" \n\t"));
        Ok(())
    }

    #[test]
    fn resolves_contract_key_to_its_actual_compose_service() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let file = directory.path().join("compose.yaml");
        std::fs::write(
            &file,
            "services:\n  frontend:\n    image: nginx\n    ports: [\"127.0.0.1:${WEB_PORT}:3000\"]\n",
        )
        .test()?;
        let target = resolve_port_target(
            &[file],
            &BTreeMap::from([("dashboard".into(), 3000)]),
            &BTreeMap::from([("WEB_PORT".into(), "{{ ports.dashboard }}".into())]),
            "dashboard",
        )
        .test()?;
        assert_eq!(
            target,
            ComposePortTarget {
                service: "frontend".into(),
                container_port: 3000
            }
        );
        Ok(())
    }

    #[test]
    fn finds_common_fixed_port_forms_but_not_variables() -> anyhow::Result<()> {
        let ports = detect_fixed_host_ports(
            r#"
              - "3000:3000"
              - '127.0.0.1:4000:4000'
              - 5000:5000
              - "${WEB_PORT}:3000"
              - "127.0.0.1:${API_PORT}:4000"
              - target: 6000
                published: 6000
            "#,
        );
        assert_eq!(
            ports.iter().map(|port| port.host_port).collect::<Vec<_>>(),
            [3000, 4000, 5000, 6000]
        );
        Ok(())
    }

    #[test]
    fn parses_compose_port_endpoints() -> anyhow::Result<()> {
        assert_eq!(endpoint_port("0.0.0.0:39000"), Some(39000));
        assert_eq!(endpoint_port("[::]:39001"), Some(39001));
        assert_eq!(endpoint_port("not-an-endpoint"), None);
        assert!(endpoint_matches("127.0.0.1:39000", "127.0.0.1", 39000));
        assert!(!endpoint_matches("0.0.0.0:39000", "127.0.0.1", 39000));
        assert!(!endpoint_matches("127.0.0.2:39000", "127.0.0.1", 39000));
        assert!(!endpoint_matches("[::]:39000", "127.0.0.1", 39000));
        assert!(endpoint_matches("127.0.0.1:39000", "localhost", 39000));
        assert!(endpoint_matches("[::1]:39000", "localhost", 39000));
        assert!(!endpoint_matches("127.0.0.2:39000", "localhost", 39000));
        assert!(!endpoint_matches("127.0.0.1:39001", "localhost", 39000));
        Ok(())
    }

    #[test]
    fn plans_isolation_from_common_compose_ports() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let file = directory.path().join("compose.yaml");
        std::fs::write(
            &file,
            r#"
services:
  web:
    image: nginx
    ports:
      - "3000:80"
      - target: 8080
        published: 4000
  postgres:
    image: postgres:16
    ports:
      - "127.0.0.1:${POSTGRES_PORT}:5432"
"#,
        )
        .test()?;

        let plan = plan(directory.path()).test()?;
        assert_eq!(plan.file, Path::new("compose.yaml"));
        assert_eq!(plan.ports.len(), 3);
        assert_eq!(plan.ports[0].env, "WEB_PORT");
        assert_eq!(plan.ports[0].current_host_port, Some(3000));
        assert_eq!(plan.ports[0].replacement, "127.0.0.1:${WEB_PORT}:80");
        assert_eq!(plan.ports[1].name, "web-8080");
        assert_eq!(plan.ports[1].env, "WEB_8080_PORT");
        assert_eq!(plan.ports[2].current_host_port, None);
        assert_eq!(plan.ports[2].env, "POSTGRES_PORT");
        assert_eq!(plan.warnings.len(), 2);
        assert!(
            plan.warnings
                .iter()
                .all(|warning| warning.contains("compose apply"))
        );
        Ok(())
    }

    #[test]
    fn explicit_compose_paths_cannot_escape_the_repository() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let repo = directory.path().join("repo");
        std::fs::create_dir(&repo).test()?;
        let contents = "services:\n  web:\n    ports:\n      - \"3000:80\"\n";
        std::fs::write(repo.join("inside.yml"), contents).test()?;
        let outside = directory.path().join("outside-compose.yml");
        std::fs::write(&outside, contents).test()?;

        assert_eq!(
            plan_at(&repo.join("."), Some(Path::new("inside.yml")))
                .test()?
                .file,
            Path::new("inside.yml")
        );

        assert!(plan_at(&repo, Some(&outside)).is_err());
        assert!(plan_at(&repo, Some(Path::new("../outside-compose.yml"))).is_err());
        assert!(apply_at(&repo, Some(Path::new("../outside-compose.yml"))).is_err());
        assert_eq!(std::fs::read_to_string(&outside).test()?, contents);

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, repo.join("compose.yml")).test()?;
            assert!(apply_at(&repo, Some(Path::new("compose.yml"))).is_err());
            assert_eq!(std::fs::read_to_string(&outside).test()?, contents);
        }
        Ok(())
    }

    #[test]
    fn reuses_the_variable_already_consumed_by_compose() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        std::fs::write(
            directory.path().join("compose.yaml"),
            "services:\n  web:\n    ports: [\"127.0.0.1:${APP_PORT:-3000}:80\", \"127.0.0.1:$ADMIN_PORT:81\"]\n",
        )
        .test()?;
        let plan = plan(directory.path()).test()?;
        assert_eq!(plan.ports[0].env, "APP_PORT");
        assert_eq!(plan.ports[0].current_host_port, None);
        assert_eq!(plan.ports[1].env, "ADMIN_PORT");
        Ok(())
    }

    #[test]
    fn rejects_generated_ports_on_non_loopback_interfaces() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let file = directory.path().join("compose.yaml");
        for mapping in [
            "${MISSING_PORT}:80",
            "0.0.0.0:${IPV4_PORT}:81",
            "[::]:${IPV6_PORT}:82",
        ] {
            std::fs::write(
                &file,
                format!("services:\n  web:\n    ports: [\"{mapping}\"]\n"),
            )
            .test()?;
            assert!(plan(directory.path()).is_err(), "accepted {mapping}");
        }
        std::fs::write(
            &file,
            "services:\n  web:\n    ports: [\"127.0.0.1:${WEB_PORT}:80\"]\n",
        )
        .test()?;
        assert!(plan(directory.path()).test()?.warnings.is_empty());
        Ok(())
    }

    #[test]
    fn applies_only_unambiguous_fixed_port_edits() -> anyhow::Result<()> {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().test()?;
        let file = directory.path().join("compose.yaml");
        std::fs::write(
            &file,
            "services:\n  web:\n    ports:\n      - \"127.0.0.1:3000:80/tcp\"\n  postgres:\n    ports:\n      - target: 5432\n        published: \"5432\"\n",
        )
        .test()?;
        #[cfg(unix)]
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).test()?;

        let output = apply(directory.path()).test()?;
        assert_eq!(output.changed_lines, 2);
        let updated = std::fs::read_to_string(file).test()?;
        assert!(updated.contains("\"127.0.0.1:${WEB_PORT}:80/tcp\""));
        assert!(updated.contains("published: \"${POSTGRES_PORT}\""));
        assert!(updated.contains("host_ip: \"127.0.0.1\""));
        assert!(plan(directory.path()).test()?.warnings.is_empty());
        let document: serde_yaml::Value = serde_yaml::from_str(&updated).test()?;
        let postgres = yaml_field(&document, "services")
            .and_then(serde_yaml::Value::as_mapping)
            .and_then(|services| services.get(serde_yaml::Value::String("postgres".into())))
            .and_then(|service| yaml_field(service, "ports"))
            .and_then(serde_yaml::Value::as_sequence)
            .and_then(|ports| ports.first())
            .and_then(serde_yaml::Value::as_mapping)
            .test()?;
        assert_eq!(
            postgres
                .get(serde_yaml::Value::String("host_ip".into()))
                .and_then(serde_yaml::Value::as_str),
            Some("127.0.0.1")
        );
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(directory.path().join("compose.yaml"))
                .test()?
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        Ok(())
    }

    #[test]
    fn apply_rejects_an_explicit_all_interface_binding() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let file = directory.path().join("compose.yaml");
        std::fs::write(
            &file,
            "services:\n  web:\n    ports:\n      - \"0.0.0.0:3000:80\"\n",
        )
        .test()?;
        assert!(plan(directory.path()).is_err());
        assert!(apply(directory.path()).is_err());
        assert!(
            std::fs::read_to_string(file)
                .test()?
                .contains("0.0.0.0:3000:80")
        );
        Ok(())
    }

    #[test]
    fn reports_ports_exposed_on_all_host_interfaces() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let file = directory.path().join("compose.yaml");
        std::fs::write(
            &file,
            "services:\n  web:\n    ports: [\"${WEB_PORT}:80\", \"127.0.0.1:${ADMIN_PORT}:81\"]\n  db:\n    ports:\n      - target: 5432\n        published: ${DB_PORT}\n        host_ip: 0.0.0.0\n",
        )
        .test()?;
        assert_eq!(
            all_interface_ports_in_file(&file).test()?,
            [("web".into(), 80), ("db".into(), 5432)]
        );
        Ok(())
    }

    #[test]
    fn rejects_container_only_ports_instead_of_inventing_a_host_url() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        std::fs::write(
            directory.path().join("compose.yaml"),
            "services:\n  web:\n    image: nginx\n    ports: [\"80\"]\n",
        )
        .test()?;
        let error = plan(directory.path()).test_err()?.to_string();
        assert!(error.contains("without a deterministic host binding"));
        assert!(error.contains("${WEB_PORT}:80"));
        Ok(())
    }

    #[test]
    fn rejects_unsupported_ports_and_generated_environment_collisions() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let file = directory.path().join("compose.yaml");
        std::fs::write(
            &file,
            "services:\n  web:\n    image: nginx\n    ports: [3000]\n",
        )
        .test()?;
        assert!(
            plan_file(directory.path(), &file)
                .test_err()?
                .to_string()
                .contains("unsupported")
        );

        std::fs::write(
            &file,
            "services:\n  web:\n    image: nginx\n    ports: [\"3000-3002:80-82\"]\n",
        )
        .test()?;
        assert!(
            plan_file(directory.path(), &file)
                .test_err()?
                .to_string()
                .contains("unsupported")
        );

        std::fs::write(
            &file,
            "services:\n  foo-bar:\n    image: nginx\n    ports: [\"3000:80\"]\n  foo_bar:\n    image: nginx\n    ports: [\"4000:80\"]\n",
        )
        .test()?;
        assert!(
            plan_file(directory.path(), &file)
                .test_err()?
                .to_string()
                .contains("FOO_BAR_PORT")
        );

        for compose in [
            "services:\n  web:\n    ports: [\"192.168.1.8:3000:80\"]\n",
            "services:\n  web:\n    ports:\n      - target: 80\n        published: 3000\n        host_ip: 192.168.1.8\n",
            "services:\n  web:\n    ports: [\"${WEB_PORT}:80/udp\"]\n",
            "services:\n  web:\n    ports:\n      - target: 80\n        published: ${WEB_PORT}\n        protocol: udp\n",
            "services:\n  web:\n    ports: [\"${WEB_PORT:+3000}:80\"]\n",
        ] {
            std::fs::write(&file, compose).test()?;
            assert!(
                plan_file(directory.path(), &file).is_err(),
                "accepted {compose}"
            );
        }
        Ok(())
    }

    #[test]
    fn validates_the_exact_structural_port_environment_contract() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let file = directory.path().join("compose.yaml");
        std::fs::write(
            &file,
            "services:\n  web:\n    image: nginx\n    ports: [\"127.0.0.1:${APP_PORT:-3000}:80\"]\n",
        )
        .test()?;
        let containers = BTreeMap::from([("web".into(), 80)]);
        let environment = BTreeMap::from([("APP_PORT".into(), "{{ ports.web }}".into())]);
        validate_port_contract(std::slice::from_ref(&file), &containers, &environment).test()?;

        let wrong_environment = BTreeMap::from([("WORKER_PORT".into(), "{{ ports.web }}".into())]);
        assert!(
            validate_port_contract(std::slice::from_ref(&file), &containers, &wrong_environment)
                .test_err()?
                .to_string()
                .contains("APP_PORT")
        );

        std::fs::write(
            &file,
            "services:\n  web:\n    image: nginx\n    ports: [\"3000:80\"]\n",
        )
        .test()?;
        assert!(
            validate_port_contract(&[file], &containers, &environment)
                .test_err()?
                .to_string()
                .contains("fixed host port")
        );
        Ok(())
    }

    #[test]
    fn validates_port_names_from_generated_contract_across_override_files() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let primary = directory.path().join("compose.yaml");
        let override_file = directory.path().join("compose.override.yaml");
        std::fs::write(
            &primary,
            "services:\n  frontend:\n    ports: [\"127.0.0.1:${WEB_PORT}:80\"]\n",
        )
        .test()?;
        std::fs::write(&override_file, "volumes:\n  cache: {}\n").test()?;
        validate_port_contract(
            &[primary, override_file],
            &BTreeMap::from([("web".into(), 80)]),
            &BTreeMap::from([("WEB_PORT".into(), "{{ ports.web }}".into())]),
        )
        .test()?;
        Ok(())
    }

    #[test]
    fn rejects_direct_and_merge_hidden_compose_includes() -> anyhow::Result<()> {
        let file = Path::new("compose.yaml");
        for contents in [
            "include: compose.shared.yaml\nservices: {}\n",
            "x-root: &root\n  include: compose.shared.yaml\n<<: *root\nservices: {}\n",
        ] {
            let document: serde_yaml::Value = serde_yaml::from_str(contents).test()?;
            let error = port_declarations(&document, file).test_err()?.to_string();
            assert!(error.contains("`include`"), "unexpected error: {error}");
            assert!(error.contains("explicitly"), "unexpected error: {error}");
        }
        Ok(())
    }

    #[test]
    fn rejects_direct_and_merge_hidden_compose_extends() -> anyhow::Result<()> {
        let file = Path::new("compose.yaml");
        for contents in [
            "services:\n  web:\n    extends:\n      file: compose.shared.yaml\n      service: web\n",
            "x-service: &base\n  extends:\n    file: compose.shared.yaml\n    service: web\nservices:\n  web:\n    <<: *base\n",
        ] {
            let document: serde_yaml::Value = serde_yaml::from_str(contents).test()?;
            let error = port_declarations(&document, file).test_err()?.to_string();
            assert!(error.contains("`extends`"), "unexpected error: {error}");
            assert!(error.contains("web"), "unexpected error: {error}");
        }
        Ok(())
    }

    #[test]
    fn explicit_runtime_file_overlays_remain_supported() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let primary = directory.path().join("compose.yaml");
        let overlay = directory.path().join("compose.stackstead.yaml");
        std::fs::write(
            &primary,
            "services:\n  web:\n    image: nginx\n    ports: [\"127.0.0.1:${WEB_PORT}:80\"]\n",
        )
        .test()?;
        std::fs::write(
            &overlay,
            "services:\n  web:\n    environment:\n      STACKSTEAD: \"true\"\n",
        )
        .test()?;

        validate_port_contract(
            &[primary, overlay],
            &BTreeMap::from([("web".into(), 80)]),
            &BTreeMap::from([("WEB_PORT".into(), "{{ ports.web }}".into())]),
        )
        .test()?;
        Ok(())
    }

    #[test]
    fn duplicate_fixed_host_ports_fail_before_the_file_is_written() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let file = directory.path().join("compose.yaml");
        let original = "services:\n  web:\n    ports:\n      - \"3000:80\"\n  api:\n    ports:\n      - \"3000:8080\"\n";
        std::fs::write(&file, original).test()?;
        let error = apply(directory.path()).test_err()?.to_string();
        assert!(error.contains("cannot safely rewrite host port 3000"));
        assert_eq!(std::fs::read_to_string(file).test()?, original);
        Ok(())
    }

    #[test]
    fn inline_fixed_mapping_is_never_rewritten() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let file = directory.path().join("compose.yaml");
        let original = "services:\n  web:\n    ports: [\"3000:80\"]\n";
        std::fs::write(&file, original).test()?;
        let error = apply(directory.path()).test_err()?.to_string();
        assert!(error.contains("one port mapping per YAML line"));
        assert_eq!(std::fs::read_to_string(file).test()?, original);
        Ok(())
    }
}
