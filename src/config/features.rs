use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use super::helpers::{
    default_agent_rules, default_context_file, default_env_file, default_health_interval_millis,
    default_health_status, default_health_timeout_seconds, default_postgres_database,
    default_postgres_service, default_postgres_user,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DependencyConfig {
    #[serde(default)]
    pub install: CommandConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub shell: bool,
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
    pub(super) fn entries(&self) -> [(&str, &[CommandConfig]); 4] {
        [
            ("post_create", &self.post_create),
            ("pre_up", &self.pre_up),
            ("post_up", &self.post_up),
            ("pre_destroy", &self.pre_destroy),
        ]
    }
}
