use crate::template::{TemplateContext, render_template, template_keys, validate_template_keys};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const CONFIG_FILE: &str = "stackstead.yaml";
pub const CONFIG_VERSION: &str = "1";

#[derive(Debug)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse(serde_yaml::Error),
    Validation(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "failed to read config {}: {source}", path.display())
            }
            Self::Parse(source) => write!(f, "invalid Stackstead config: {source}"),
            Self::Validation(message) => write!(f, "invalid Stackstead config: {message}"),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse(source) => Some(source),
            Self::Validation(_) => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StacksteadConfig {
    pub version: String,
    pub kind: ConfigKind,
    #[serde(default)]
    pub project: ProjectConfig,
    #[serde(default)]
    pub source: SourceConfig,
    #[serde(default)]
    pub state: StateConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub resources: ResourcesConfig,
    #[serde(default)]
    pub dependencies: DependencyConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub env: EnvConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub hooks: HooksConfig,
    #[serde(default)]
    pub health: HealthConfig,
}

impl Default for StacksteadConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION.to_owned(),
            kind: ConfigKind::default(),
            project: ProjectConfig::default(),
            source: SourceConfig::default(),
            state: StateConfig::default(),
            runtime: RuntimeConfig::default(),
            resources: ResourcesConfig::default(),
            dependencies: DependencyConfig::default(),
            database: DatabaseConfig::default(),
            env: EnvConfig::default(),
            agent: AgentConfig::default(),
            hooks: HooksConfig::default(),
            health: HealthConfig::default(),
        }
    }
}

impl StacksteadConfig {
    pub fn from_yaml(yaml: &str) -> Result<Self, ConfigError> {
        let config: Self = serde_yaml::from_str(yaml).map_err(ConfigError::Parse)?;
        config.validate()?;
        Ok(config)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        Self::from_yaml(&contents)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != CONFIG_VERSION {
            return invalid(format!(
                "unsupported version `{}`; expected `{CONFIG_VERSION}`",
                self.version
            ));
        }
        if self.project.name.trim().is_empty() {
            return invalid("project.name is required");
        }
        validate_identifier("project.name", &self.project.name)?;
        if self.source.base.trim().is_empty() {
            return invalid("source.base cannot be empty");
        }
        if self.runtime.files.is_empty() {
            return invalid("runtime.files must contain at least one Compose file");
        }
        for file in &self.runtime.files {
            validate_relative_file("runtime.files", file)?;
        }
        if self.state.root.as_os_str().is_empty() || self.state.root == Path::new("/") {
            return invalid("state.root must not be empty or the filesystem root");
        }

        self.validate_ports()?;
        validate_relative_file("env.file", &self.env.file)?;
        validate_relative_file("agent.context_file", &self.agent.context_file)?;

        for name in self.env.generate.keys() {
            if !valid_env_name(name) {
                return invalid(format!("invalid env var name `{name}`"));
            }
            if reserved_process_env(name) {
                return invalid(format!(
                    "env.generate cannot define process- or Docker-control variable `{name}`"
                ));
            }
        }

        self.validate_commands()?;
        self.validate_database()?;
        self.validate_templates()?;
        Ok(())
    }

    pub fn validate_for_repo(&self, repo_root: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        if !repo_root.is_dir() {
            return invalid(format!(
                "repo root does not exist or is not a directory: {}",
                repo_root.display()
            ));
        }
        self.validated_state_root(repo_root)?;
        for file in &self.runtime.files {
            let path = repo_root.join(file);
            if !path.is_file() {
                return invalid(format!("Compose file does not exist: {}", path.display()));
            }
        }
        Ok(())
    }

    pub fn validated_state_root(&self, repo_root: &Path) -> Result<PathBuf, ConfigError> {
        let state_root = crate::paths::absolute_from(repo_root, &self.state.root)
            .and_then(|path| crate::paths::resolve_existing_ancestor(&path))
            .map_err(|error| ConfigError::Validation(format!("invalid state.root: {error}")))?;
        let repo_root = crate::paths::resolve_existing_ancestor(repo_root)
            .map_err(|error| ConfigError::Validation(format!("invalid repo root: {error}")))?;
        if state_root.parent().is_none() {
            return invalid("state.root must not resolve to the filesystem root");
        }
        if state_root.starts_with(&repo_root) {
            return invalid("state.root must resolve outside the repository");
        }
        Ok(state_root)
    }

    pub fn service_names(&self) -> Vec<String> {
        self.resources.ports.expose.keys().cloned().collect()
    }

    fn validate_ports(&self) -> Result<(), ConfigError> {
        let ports = &self.resources.ports;
        if ports.base == 0 {
            return invalid("resources.ports.base must be greater than zero");
        }
        if ports.stride == 0 {
            return invalid("resources.ports.stride must be greater than zero");
        }
        if usize::from(ports.stride) < ports.expose.len() {
            return invalid(format!(
                "resources.ports.stride {} is smaller than exposed service count {}",
                ports.stride,
                ports.expose.len()
            ));
        }
        if u32::from(ports.base) + ports.expose.len().saturating_sub(1) as u32 > u32::from(u16::MAX)
        {
            return invalid("the first deterministic port slot exceeds port 65535");
        }
        for (service, exposure) in &ports.expose {
            validate_identifier("exposed service name", service)?;
            if exposure.container == 0 {
                return invalid(format!(
                    "resources.ports.expose.{service}.container must be greater than zero"
                ));
            }
        }
        Ok(())
    }

    fn validate_commands(&self) -> Result<(), ConfigError> {
        if self.dependencies.provider == DependencyProvider::YarnClassic {
            if let Some(link) = &self.dependencies.link {
                validate_safe_relative("dependencies.link.link_folder", &link.link_folder)?;
                if link.enabled && link.command.trim().is_empty() {
                    return invalid(
                        "dependencies.link.command cannot be empty when linking is enabled",
                    );
                }
            }
        } else if self
            .dependencies
            .link
            .as_ref()
            .is_some_and(|link| link.enabled)
        {
            return invalid("dependencies.link requires provider `yarn-classic`");
        }

        for (hook_name, commands) in self.hooks.iter() {
            for command in commands {
                if command.command.trim().is_empty() {
                    return invalid(format!("hooks.{hook_name} contains an empty command"));
                }
            }
        }
        if self.health.timeout_seconds == 0
            || self.health.timeout_seconds > 86_400
            || self.health.interval_millis == 0
            || self.health.interval_millis > 60_000
        {
            return invalid(
                "health timeout_seconds must be 1..=86400 and interval_millis must be 1..=60000",
            );
        }
        for check in &self.health.checks {
            validate_identifier("health check name", &check.name)?;
            if check.url.as_ref().is_some_and(|url| url.trim().is_empty()) {
                return invalid(format!("health check `{}` URL cannot be blank", check.name));
            }
            let has_url = check.url.as_ref().is_some_and(|url| !url.trim().is_empty());
            let has_command = !check.command.command.trim().is_empty();
            if has_url == has_command {
                return invalid(format!(
                    "health check `{}` must configure exactly one of `url` or `command.command`",
                    check.name
                ));
            }
            if !(100..=599).contains(&check.expect_status) {
                return invalid(format!(
                    "health check `{}` expect_status must be between 100 and 599",
                    check.name
                ));
            }
        }
        Ok(())
    }

    fn validate_database(&self) -> Result<(), ConfigError> {
        let Some(postgres) = &self.database.postgres else {
            return Ok(());
        };
        for (field, value) in [
            ("service", postgres.service.as_str()),
            ("database", postgres.database.as_str()),
            ("user", postgres.user.as_str()),
        ] {
            if value.trim().is_empty() {
                return invalid(format!("database.postgres.{field} cannot be empty"));
            }
        }
        validate_identifier("database.postgres.service", &postgres.service)?;
        validate_identifier("database.postgres.database", &postgres.database)?;
        validate_identifier("database.postgres.user", &postgres.user)?;
        if !self.resources.ports.expose.contains_key(&postgres.service) {
            return invalid(format!(
                "database.postgres.service `{}` must also be configured under resources.ports.expose",
                postgres.service
            ));
        }
        Ok(())
    }

    fn validate_templates(&self) -> Result<(), ConfigError> {
        let allowed = self.template_keys();
        let validate = |name: &str, value: &str| {
            validate_template_keys(value, allowed.iter().map(String::as_str))
                .map_err(|error| ConfigError::Validation(format!("{name}: {error}")))
        };

        validate(
            "runtime.project_name_template",
            &self.runtime.project_name_template,
        )?;
        for (service, exposure) in &self.resources.ports.expose {
            if let Some(url) = &exposure.url {
                validate(&format!("resources.ports.expose.{service}.url"), url)?;
                validate_url_template(service, url, &allowed)?;
            }
        }
        for (name, value) in &self.env.generate {
            validate(&format!("env.generate.{name}"), value)?;
        }
        for check in &self.health.checks {
            if let Some(url) = &check.url {
                validate(&format!("health.checks.{}.url", check.name), url)?;
                validate_url_template(&check.name, url, &allowed)?;
            }
        }
        Ok(())
    }

    fn template_keys(&self) -> BTreeSet<String> {
        let mut keys: BTreeSet<String> = [
            "project.name",
            "stackstead.id",
            "stackstead.slug",
            "stackstead.short_id",
            "paths.repo_root",
            "paths.stackstead_root",
            "paths.worktree",
            "paths.state_dir",
        ]
        .map(str::to_owned)
        .into_iter()
        .collect();
        for (service, exposure) in &self.resources.ports.expose {
            keys.insert(format!("ports.{service}"));
            if exposure.url.is_some() {
                keys.insert(format!("urls.{service}"));
            }
        }
        keys
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConfigKind {
    #[default]
    StacksteadProject,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    #[serde(default)]
    pub provider: SourceProvider,
    #[serde(default = "default_base")]
    pub base: String,
}

impl Default for SourceConfig {
    fn default() -> Self {
        Self {
            provider: SourceProvider::default(),
            base: default_base(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SourceProvider {
    #[default]
    GitWorktree,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StateConfig {
    #[serde(default = "default_state_root")]
    pub root: PathBuf,
}

impl Default for StateConfig {
    fn default() -> Self {
        Self {
            root: default_state_root(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub provider: RuntimeProvider,
    #[serde(default = "default_compose_files")]
    pub files: Vec<PathBuf>,
    #[serde(default = "default_project_name_template")]
    pub project_name_template: String,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            provider: RuntimeProvider::default(),
            files: default_compose_files(),
            project_name_template: default_project_name_template(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeProvider {
    #[default]
    DockerCompose,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourcesConfig {
    #[serde(default)]
    pub ports: PortsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PortsConfig {
    #[serde(default)]
    pub strategy: PortStrategy,
    #[serde(default = "default_port_base")]
    pub base: u16,
    #[serde(default = "default_port_stride")]
    pub stride: u16,
    #[serde(default)]
    pub expose: BTreeMap<String, PortExposure>,
}

impl Default for PortsConfig {
    fn default() -> Self {
        Self {
            strategy: PortStrategy::default(),
            base: default_port_base(),
            stride: default_port_stride(),
            expose: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PortStrategy {
    #[default]
    Deterministic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PortExposure {
    pub container: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DependencyConfig {
    #[serde(default)]
    pub provider: DependencyProvider,
    #[serde(default)]
    pub install: CommandConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkConfig>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyProvider {
    #[default]
    Command,
    YarnClassic,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub shell: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LinkConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_link_folder")]
    pub link_folder: PathBuf,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub shell: bool,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            link_folder: default_link_folder(),
            command: String::new(),
            shell: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postgres: Option<PostgresConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PostgresConfig {
    #[serde(default)]
    pub strategy: PostgresStrategy,
    #[serde(default = "default_postgres_service")]
    pub service: String,
    #[serde(default = "default_postgres_database")]
    pub database: String,
    #[serde(default = "default_postgres_user")]
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub seed: CommandConfig,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PostgresStrategy {
    #[default]
    ComposeVolume,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnvConfig {
    #[serde(default = "default_env_file")]
    pub file: PathBuf,
    #[serde(default)]
    pub generate: BTreeMap<String, String>,
}

impl Default for EnvConfig {
    fn default() -> Self {
        Self {
            file: default_env_file(),
            generate: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    #[serde(default = "default_context_file")]
    pub context_file: PathBuf,
    #[serde(default = "default_agent_rules")]
    pub rules: Vec<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            context_file: default_context_file(),
            rules: default_agent_rules(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HooksConfig {
    #[serde(default)]
    pub post_create: Vec<CommandConfig>,
    #[serde(default)]
    pub pre_up: Vec<CommandConfig>,
    #[serde(default)]
    pub post_up: Vec<CommandConfig>,
    #[serde(default)]
    pub pre_destroy: Vec<CommandConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HealthConfig {
    #[serde(default = "default_health_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_health_interval_millis")]
    pub interval_millis: u64,
    #[serde(default)]
    pub checks: Vec<HealthCheckConfig>,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: default_health_timeout_seconds(),
            interval_millis: default_health_interval_millis(),
            checks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HealthCheckConfig {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default = "default_health_status")]
    pub expect_status: u16,
    #[serde(default)]
    pub command: CommandConfig,
}

impl HooksConfig {
    fn iter(&self) -> [(&str, &[CommandConfig]); 4] {
        [
            ("post_create", &self.post_create),
            ("pre_up", &self.pre_up),
            ("post_up", &self.post_up),
            ("pre_destroy", &self.pre_destroy),
        ]
    }
}

fn validate_url_template(
    service: &str,
    template: &str,
    allowed: &BTreeSet<String>,
) -> Result<(), ConfigError> {
    let referenced = template_keys(template)
        .map_err(|error| ConfigError::Validation(format!("URL for `{service}`: {error}")))?;
    let mut context = TemplateContext::new();
    for key in allowed {
        let value = if key.starts_with("ports.") {
            "12345"
        } else if key.starts_with("urls.") {
            "http://127.0.0.1:12345"
        } else {
            "value"
        };
        context.insert(key.clone(), value.to_owned());
    }
    for key in referenced {
        context.entry(key).or_insert_with(|| "value".to_owned());
    }
    let rendered = render_template(template, &context)
        .map_err(|error| ConfigError::Validation(format!("URL for `{service}`: {error}")))?;
    if rendered.chars().any(char::is_whitespace) || !crate::open::is_loopback_url(&rendered) {
        return invalid(format!(
            "URL template for `{service}` must render a loopback http:// or https:// URL without credentials or whitespace"
        ));
    }
    Ok(())
}

fn validate_identifier(field: &str, value: &str) -> Result<(), ConfigError> {
    let mut chars = value.chars();
    let valid = chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'
                || character == '_'
        });
    if !valid {
        return invalid(format!(
            "{field} `{value}` must contain only lowercase ASCII letters, digits, `-`, or `_` and start with a letter or digit"
        ));
    }
    Ok(())
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

pub(crate) fn reserved_process_env(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    matches!(
        name.as_str(),
        "PATH"
            | "PATHEXT"
            | "HOME"
            | "XDG_STATE_HOME"
            | "SHELL"
            | "PWD"
            | "CDPATH"
            | "COMSPEC"
            | "SYSTEMROOT"
    ) || ["LD_", "DYLD_", "DOCKER_", "COMPOSE_"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

fn validate_relative_file(field: &str, path: &Path) -> Result<(), ConfigError> {
    validate_safe_relative(field, path)?;
    if path.file_name().is_none() {
        return invalid(format!("{field} must name a file"));
    }
    Ok(())
}

fn validate_safe_relative(field: &str, path: &Path) -> Result<(), ConfigError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return invalid(format!("{field} must be a non-empty relative path"));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return invalid(format!("{field} cannot escape the worktree"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError::Validation(message.into()))
}

fn default_base() -> String {
    "main".to_owned()
}

fn default_state_root() -> PathBuf {
    PathBuf::from("../.stacksteads")
}

fn default_compose_files() -> Vec<PathBuf> {
    vec![PathBuf::from("docker-compose.yml")]
}

fn default_project_name_template() -> String {
    "{{ project.name }}-{{ stackstead.id }}".to_owned()
}

fn default_port_base() -> u16 {
    39000
}

fn default_port_stride() -> u16 {
    50
}

fn default_health_timeout_seconds() -> u64 {
    60
}

fn default_health_interval_millis() -> u64 {
    500
}

fn default_health_status() -> u16 {
    200
}

fn default_link_folder() -> PathBuf {
    PathBuf::from(".stackstead/yarn-links")
}

fn default_postgres_service() -> String {
    "postgres".to_owned()
}

fn default_postgres_database() -> String {
    "app".to_owned()
}

fn default_postgres_user() -> String {
    "app".to_owned()
}

fn default_env_file() -> PathBuf {
    PathBuf::from(".stackstead/.env")
}

fn default_context_file() -> PathBuf {
    PathBuf::from(".stackstead/AGENT_CONTEXT.md")
}

fn default_agent_rules() -> Vec<String> {
    [
        "Use only the generated ports in this stackstead.",
        "Do not connect to the shared development database.",
        "Run stackstead inspect before debugging service failures.",
        "Run stackstead logs before changing service startup code.",
        "Run stackstead db status before applying migrations.",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    const SAMPLE: &str = r#"
version: "1"
kind: StacksteadProject
project:
  name: loan-platform
source:
  provider: git-worktree
  base: main
state:
  root: ../.stacksteads
runtime:
  provider: docker-compose
  files: [docker-compose.yml]
resources:
  ports:
    strategy: deterministic
    base: 39000
    stride: 50
    expose:
      web:
        container: 3000
        url: "http://127.0.0.1:{{ ports.web }}"
      postgres:
        container: 5432
dependencies:
  provider: command
  install:
    command: ""
    shell: false
database:
  postgres:
    strategy: compose-volume
    service: postgres
    database: app
    user: app
    password: app
env:
  file: .stackstead/.env
  generate:
    WEB_PORT: "{{ ports.web }}"
    DATABASE_URL: "postgres://app:app@127.0.0.1:{{ ports.postgres }}/app"
agent:
  context_file: .stackstead/AGENT_CONTEXT.md
hooks:
  post_create: []
"#;

    #[test]
    fn parses_full_config() {
        let config = StacksteadConfig::from_yaml(SAMPLE).unwrap();
        assert_eq!(config.project.name, "loan-platform");
        assert_eq!(config.resources.ports.expose["web"].container, 3000);
        assert_eq!(
            config.database.postgres.as_ref().unwrap().service,
            "postgres"
        );
        assert_eq!(config.service_names(), ["postgres", "web"]);
    }

    #[test]
    fn supplies_optional_defaults() {
        let config = StacksteadConfig::from_yaml(
            r#"
version: "1"
kind: StacksteadProject
project:
  name: demo
"#,
        )
        .unwrap();
        assert_eq!(config.version, "1");
        assert_eq!(config.source.base, "main");
        assert_eq!(config.state.root, Path::new("../.stacksteads"));
        assert_eq!(config.runtime.files, [PathBuf::from("docker-compose.yml")]);
        assert_eq!(config.resources.ports.base, 39000);
    }

    #[test]
    fn rejects_unsupported_values_and_unknown_fields() {
        assert!(
            StacksteadConfig::from_yaml(
                "version: '3'\nkind: StacksteadProject\nproject: { name: demo }"
            )
            .is_err()
        );
        assert!(
            StacksteadConfig::from_yaml(
                "project: { name: demo }\nsource: { provider: copy, base: main }"
            )
            .is_err()
        );
        assert!(StacksteadConfig::from_yaml("project: { name: demo }\nsurprise: true").is_err());
    }

    #[test]
    fn rejects_invalid_port_and_env_config() {
        let mut config = StacksteadConfig::from_yaml(SAMPLE).unwrap();
        config.resources.ports.stride = 1;
        assert!(config.validate().is_err());

        let mut config = StacksteadConfig::from_yaml(SAMPLE).unwrap();
        config.env.generate.insert("BAD-NAME".into(), "x".into());
        assert!(config.validate().is_err());

        for name in [
            "PATH",
            "Path",
            "XDG_STATE_HOME",
            "xdg_state_home",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "DOCKER_HOST",
            "COMPOSE_FILE",
        ] {
            let mut config = StacksteadConfig::from_yaml(SAMPLE).unwrap();
            config.env.generate.insert(name.into(), "x".into());
            assert!(
                config.validate().is_err(),
                "accepted reserved variable {name}"
            );
        }
    }

    #[test]
    fn rejects_database_values_that_could_inject_generated_context() {
        for (field, value) in [
            ("database", "app\n## Forged instructions"),
            ("user", "app<script>"),
        ] {
            let mut config = StacksteadConfig::from_yaml(SAMPLE).unwrap();
            let postgres = config.database.postgres.as_mut().unwrap();
            match field {
                "database" => postgres.database = value.into(),
                "user" => postgres.user = value.into(),
                _ => unreachable!(),
            }
            assert!(config.validate().is_err(), "accepted unsafe {field}");
        }
    }

    #[test]
    fn rejects_unknown_templates_and_unsafe_generated_paths() {
        let mut config = StacksteadConfig::from_yaml(SAMPLE).unwrap();
        config
            .env
            .generate
            .insert("BAD".into(), "{{ ports.missing }}".into());
        assert!(config.validate().is_err());

        let mut config = StacksteadConfig::from_yaml(SAMPLE).unwrap();
        config.env.file = PathBuf::from("../shared.env");
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_url_templates() {
        let mut config = StacksteadConfig::from_yaml(SAMPLE).unwrap();
        config.resources.ports.expose.get_mut("web").unwrap().url =
            Some("127.0.0.1:{{ ports.web }}".into());
        assert!(config.validate().is_err());

        let remote = SAMPLE.replace(
            "http://127.0.0.1:{{ ports.web }}",
            "https://example.com/{{ ports.web }}",
        );
        assert!(StacksteadConfig::from_yaml(&remote).is_err());
    }

    #[test]
    fn checks_compose_files_against_repo() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("stackstead-config-{suffix}"));
        fs::create_dir_all(&root).unwrap();
        let config = StacksteadConfig::from_yaml(SAMPLE).unwrap();
        assert!(config.validate_for_repo(&root).is_err());
        fs::write(root.join("docker-compose.yml"), "services: {}").unwrap();
        assert!(config.validate_for_repo(&root).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_state_root_that_normalizes_to_filesystem_root() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("docker-compose.yml"), "services: {}").unwrap();
        let mut config = StacksteadConfig::from_yaml(SAMPLE).unwrap();
        config.state.root = PathBuf::from("/tmp/..");
        assert!(config.validate_for_repo(root.path()).is_err());

        config.state.root = PathBuf::from(".");
        assert!(config.validate_for_repo(root.path()).is_err());

        let inside = root.path().join("inside-state");
        let alias = root.path().parent().unwrap().join(format!(
            "{}-state-link",
            root.path().file_name().unwrap().to_string_lossy()
        ));
        fs::create_dir(&inside).unwrap();
        std::os::unix::fs::symlink(&inside, &alias).unwrap();
        config.state.root = alias;
        assert!(config.validate_for_repo(root.path()).is_err());
        fs::remove_file(&config.state.root).unwrap();
    }

    #[test]
    fn validates_yarn_link_shape() {
        let mut config = StacksteadConfig::from_yaml(SAMPLE).unwrap();
        config.dependencies.provider = DependencyProvider::YarnClassic;
        config.dependencies.link = Some(LinkConfig {
            enabled: true,
            ..LinkConfig::default()
        });
        assert!(config.validate().is_err());
        config.dependencies.link.as_mut().unwrap().command = "./scripts/link-packages.sh".into();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validates_http_and_command_health_checks() {
        let mut config = StacksteadConfig::from_yaml(SAMPLE).unwrap();
        config.health.checks = vec![
            HealthCheckConfig {
                name: "web".into(),
                url: Some("{{ urls.web }}/health".into()),
                expect_status: 200,
                command: CommandConfig::default(),
            },
            HealthCheckConfig {
                name: "worker".into(),
                url: None,
                expect_status: 200,
                command: CommandConfig {
                    command: "true".into(),
                    shell: false,
                },
            },
        ];
        assert!(config.validate().is_ok());

        config.health.checks[0].command.command = "true".into();
        assert!(config.validate().is_err());

        config.health.checks[0].command.command.clear();
        config.health.checks[0].url = Some(" \t".into());
        assert!(config.validate().is_err());

        config.health.checks[0].url = Some("{{ urls.web }}/health".into());
        config.health.timeout_seconds = u64::MAX;
        assert!(config.validate().is_err());
        config.health.timeout_seconds = 60;
        config.health.interval_millis = 60_001;
        assert!(config.validate().is_err());
    }
}
