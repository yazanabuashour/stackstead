use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

use crate::template::{TemplateContext, render_template, template_keys};

use super::ConfigError;

pub(super) fn validate_url_template(
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

pub(super) fn validate_identifier(field: &str, value: &str) -> Result<(), ConfigError> {
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

pub(super) fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

pub fn reserved_process_env(name: &str) -> bool {
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

pub(super) fn validate_relative_file(field: &str, path: &Path) -> Result<(), ConfigError> {
    validate_safe_relative(field, path)?;
    if path.file_name().is_none() {
        return invalid(format!("{field} must name a file"));
    }
    Ok(())
}

pub(super) fn validate_safe_relative(field: &str, path: &Path) -> Result<(), ConfigError> {
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

pub(super) fn invalid<T>(message: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError::Validation(message.into()))
}

pub(super) fn default_base() -> String {
    "main".to_owned()
}

pub(super) fn default_state_root() -> PathBuf {
    PathBuf::from("../.stacksteads")
}

pub(super) fn default_compose_files() -> Vec<PathBuf> {
    vec![PathBuf::from("docker-compose.yml")]
}

pub(super) const fn default_port_base() -> u16 {
    39000
}

pub(super) const fn default_port_stride() -> u16 {
    50
}

pub(super) const fn default_health_timeout_seconds() -> u64 {
    60
}

pub(super) const fn default_health_interval_millis() -> u64 {
    500
}

pub(super) const fn default_health_status() -> u16 {
    200
}

pub(super) fn default_postgres_service() -> String {
    "postgres".to_owned()
}

pub(super) fn default_postgres_database() -> String {
    "app".to_owned()
}

pub(super) fn default_postgres_user() -> String {
    "app".to_owned()
}

pub(super) fn default_env_file() -> PathBuf {
    PathBuf::from(".stackstead/.env")
}

pub(super) fn default_context_file() -> PathBuf {
    PathBuf::from(".stackstead/AGENT_CONTEXT.md")
}

pub(super) fn default_agent_rules() -> Vec<String> {
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
