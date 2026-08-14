use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use super::{
    CONFIG_VERSION, ConfigError,
    features::DependencyProvider,
    helpers::{
        invalid, reserved_process_env, valid_env_name, validate_identifier, validate_relative_file,
        validate_safe_relative, validate_url_template,
    },
    model::StacksteadConfig,
};
use crate::template::validate_template_keys;

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
        let exposed_offset =
            u32::try_from(ports.expose.len().saturating_sub(1)).map_err(|error| {
                ConfigError::Validation(format!("too many exposed services: {error}"))
            })?;
        if u32::from(ports.base)
            .checked_add(exposed_offset)
            .is_none_or(|last| last > u32::from(u16::MAX))
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

        for (hook_name, commands) in self.hooks.entries() {
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
