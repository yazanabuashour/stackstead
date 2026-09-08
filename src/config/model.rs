use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::readiness::Role;

use super::{
    CONFIG_VERSION,
    features::{
        AgentConfig, DatabaseConfig, DependencyConfig, EnvConfig, HealthConfig, HooksConfig,
    },
    helpers::{
        default_base, default_compose_files, default_port_base, default_port_stride,
        default_state_root,
    },
};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<ReadinessConfig>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            provider: RuntimeProvider::default(),
            files: default_compose_files(),
            readiness: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReadinessConfig {
    pub required: BTreeMap<String, Role>,
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
