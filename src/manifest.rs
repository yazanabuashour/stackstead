use std::{
    collections::BTreeMap,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const MANIFEST_VERSION: &str = "2";
pub const POINTER_VERSION: &str = "2";
const RUNTIME_TOKEN_LEN: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StacksteadManifest {
    pub kind: String,
    pub version: String,
    pub stackstead_id: String,
    pub slug: String,
    pub short_id: String,
    pub runtime_token: String,
    pub project: String,
    pub branch: String,
    pub base: String,
    pub source_ownership: SourceOwnership,
    pub repo_root: PathBuf,
    pub project_state_root: PathBuf,
    pub stackstead_root: PathBuf,
    pub worktree: PathBuf,
    pub state_dir: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port_lease_state_dir: Option<PathBuf>,
    pub compose_project: String,
    pub compose_files: Vec<PathBuf>,
    pub ports: BTreeMap<String, u16>,
    pub container_ports: BTreeMap<String, u16>,
    pub urls: BTreeMap<String, String>,
    pub env_file: PathBuf,
    pub agent_context: PathBuf,
    pub pointer_file: PathBuf,
    pub event_log: PathBuf,
    pub env_keys: Vec<String>,
    pub status: ManifestStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<DatabaseState>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SourceOwnership {
    Stackstead,
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestStatus {
    pub source: ComponentStatus,
    pub dependencies: ComponentStatus,
    pub runtime: ComponentStatus,
    pub database: ComponentStatus,
    pub health: ComponentStatus,
}

impl Default for ManifestStatus {
    fn default() -> Self {
        Self {
            source: ComponentStatus::Created,
            dependencies: ComponentStatus::Unknown,
            runtime: ComponentStatus::Stopped,
            database: ComponentStatus::Unknown,
            health: ComponentStatus::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ComponentStatus {
    Created,
    Ready,
    Running,
    Stopped,
    Reachable,
    Unreachable,
    Failed,
    Unknown,
}

impl std::fmt::Display for ComponentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Created => "created",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Reachable => "reachable",
            Self::Unreachable => "unreachable",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DatabaseState {
    pub strategy: String,
    pub service: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub seed_status: ComponentStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StacksteadPointer {
    pub kind: String,
    pub version: String,
    pub stackstead_id: String,
    pub manifest: PathBuf,
    pub project: String,
    pub repo_root: PathBuf,
    pub project_state_root: PathBuf,
    pub stackstead_root: PathBuf,
}

impl StacksteadManifest {
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let file = File::open(path)
            .map_err(|error| anyhow::anyhow!("cannot open manifest {}: {error}", path.display()))?;
        let value: serde_json::Value =
            serde_json::from_reader(BufReader::new(file)).map_err(|error| {
                anyhow::anyhow!("cannot parse manifest {}: {error}", path.display())
            })?;
        let kind = value.get("kind").and_then(serde_json::Value::as_str);
        let version = value.get("version").and_then(serde_json::Value::as_str);
        if kind == Some("StacksteadManifest") && version == Some("1") {
            anyhow::bail!(
                "unsupported manifest contract in {}: version 1 lacks a cryptographic runtime token; destroy it with a compatible older Stackstead binary, then recreate it with this version",
                path.display()
            );
        }
        if kind != Some("StacksteadManifest") || version != Some(MANIFEST_VERSION) {
            anyhow::bail!(
                "unsupported manifest contract in {}: kind={} version={}",
                path.display(),
                kind.unwrap_or("<missing>"),
                version.unwrap_or("<missing>")
            );
        }
        if value.get("source_ownership").is_none() {
            anyhow::bail!(
                "invalid manifest contract in {}: version {MANIFEST_VERSION} requires source_ownership",
                path.display()
            );
        }
        let runtime_token = value
            .get("runtime_token")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "invalid manifest contract in {}: version {MANIFEST_VERSION} requires a cryptographic runtime_token; recreate this stackstead before use",
                    path.display()
                )
            })?;
        if !valid_runtime_token(runtime_token) {
            anyhow::bail!(
                "invalid manifest contract in {}: runtime_token must contain exactly {RUNTIME_TOKEN_LEN} lowercase hexadecimal characters; recreate this stackstead before use",
                path.display()
            );
        }
        let ports_present = value
            .get("ports")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|ports| !ports.is_empty());
        let lease_state_dir = value
            .get("port_lease_state_dir")
            .and_then(serde_json::Value::as_str);
        if ports_present && lease_state_dir.is_none_or(|path| !Path::new(path).is_absolute()) {
            anyhow::bail!(
                "invalid manifest contract in {}: stacksteads with ports require an absolute port_lease_state_dir",
                path.display()
            );
        }
        serde_json::from_value(value)
            .map_err(|error| anyhow::anyhow!("cannot parse manifest {}: {error}", path.display()))
    }

    pub fn save_atomic(&mut self) -> anyhow::Result<()> {
        self.updated_at = Utc::now();
        write_json_atomic(&self.manifest_path(), self)
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.state_dir.join("manifest.json")
    }

    pub fn trusted_environment(
        &self,
        generated: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let mut environment = generated
            .iter()
            .filter(|(key, _)| !crate::config::reserved_process_env(key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        for (key, value) in [
            ("STACKSTEAD_ID", self.stackstead_id.clone()),
            ("STACKSTEAD_PROJECT", self.project.clone()),
            ("STACKSTEAD_WORKTREE", self.worktree.display().to_string()),
            (
                "STACKSTEAD_MANIFEST",
                self.manifest_path().display().to_string(),
            ),
            (
                "STACKSTEAD_CONTEXT",
                self.agent_context.display().to_string(),
            ),
            ("STACKSTEAD_ENV_FILE", self.env_file.display().to_string()),
            ("STACKSTEAD_COMPOSE_PROJECT", self.compose_project.clone()),
            ("COMPOSE_PROJECT_NAME", self.compose_project.clone()),
        ] {
            environment.insert(key.into(), value);
        }
        environment
    }

    pub fn validated_environment(&self) -> anyhow::Result<BTreeMap<String, String>> {
        let generated = crate::envfile::read(&self.env_file)?;
        let keys = generated.keys().cloned().collect::<Vec<_>>();
        if keys != self.env_keys {
            anyhow::bail!(
                "generated environment keys at {} do not match the manifest contract; run `stackstead repair {}`",
                self.env_file.display(),
                self.stackstead_id
            );
        }
        Ok(generated)
    }
}

pub fn new_runtime_token() -> anyhow::Result<String> {
    crate::slug::new_random_hex()
}

fn valid_runtime_token(value: &str) -> bool {
    value.len() == RUNTIME_TOKEN_LEN
        && value
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
}

mod persistence;
pub use persistence::{write_json_atomic, write_pointer};

#[cfg(test)]
mod tests;
