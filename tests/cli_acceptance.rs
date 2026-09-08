#![expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing,
    clippy::panic_in_result_fn,
    clippy::redundant_clone,
    clippy::redundant_pub_crate,
    reason = "acceptance tests return Result for setup while assertions and direct fixture access report failures"
)]

use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use assert_cmd::Command;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::TempDir;

#[path = "../src/test_support.rs"]
mod test_support;
use test_support::{TestResultErrorExt, TestResultExt};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SourceOwnership {
    Stackstead,
    External,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ComponentStatus {
    Created,
    Ready,
    Running,
    Stopped,
    Reachable,
    Unreachable,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestStatus {
    source: ComponentStatus,
    dependencies: ComponentStatus,
    runtime: ComponentStatus,
    database: ComponentStatus,
    health: ComponentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StacksteadManifest {
    kind: String,
    version: String,
    stackstead_id: String,
    slug: String,
    short_id: String,
    runtime_token: String,
    project: String,
    branch: String,
    base: String,
    source_ownership: SourceOwnership,
    repo_root: PathBuf,
    project_state_root: PathBuf,
    stackstead_root: PathBuf,
    worktree: PathBuf,
    state_dir: PathBuf,
    port_lease_state_dir: Option<PathBuf>,
    compose_project: String,
    compose_files: Vec<PathBuf>,
    readiness: Value,
    ports: std::collections::BTreeMap<String, u16>,
    container_ports: std::collections::BTreeMap<String, u16>,
    urls: std::collections::BTreeMap<String, String>,
    env_file: PathBuf,
    agent_context: PathBuf,
    pointer_file: PathBuf,
    event_log: PathBuf,
    env_keys: Vec<String>,
    status: ManifestStatus,
    database: Option<Value>,
    created_at: String,
    updated_at: String,
}

impl StacksteadManifest {
    fn read(path: &Path) -> anyhow::Result<Self> {
        serde_json::from_slice(&fs::read(path).test_context("read manifest fixture")?)
            .test_context("parse manifest fixture")
    }

    fn write_fixture(&self) -> anyhow::Result<()> {
        fs::write(
            self.manifest_path(),
            serde_json::to_vec_pretty(self).test()?,
        )?;
        Ok(())
    }

    fn manifest_path(&self) -> PathBuf {
        self.state_dir.join("manifest.json")
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StacksteadPointer {
    kind: String,
    version: String,
    stackstead_id: String,
    manifest: PathBuf,
    project: String,
    repo_root: PathBuf,
    project_state_root: PathBuf,
    stackstead_root: PathBuf,
}

type StacksteadConfig = serde_yaml::Value;

fn load_config(path: &Path) -> anyhow::Result<StacksteadConfig> {
    serde_yaml::from_slice(&fs::read(path).test_context("read config fixture")?)
        .test_context("parse config fixture")
}

fn has_diagnostic(report: &Value, code: &str, severity: &str) -> bool {
    report["diagnostics"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["code"] == code && item["severity"] == severity)
    })
}

#[derive(Deserialize)]
struct StacksteadChange {
    kind: String,
    version: String,
    action: String,
    stackstead: StacksteadChangeDetails,
}

#[derive(Deserialize)]
struct StacksteadChangeDetails {
    files: StacksteadFiles,
}

#[derive(Deserialize)]
struct StacksteadFiles {
    manifest: PathBuf,
}

fn changed_manifest(output: &[u8], action: &str) -> anyhow::Result<StacksteadManifest> {
    let change: StacksteadChange =
        serde_json::from_slice(output).test_context("parse stackstead change")?;
    assert_eq!(
        change.kind, "StacksteadChange",
        "Stackstead change output broke its transport contract"
    );
    assert_eq!(
        change.version, "1",
        "Stackstead change output broke its transport contract"
    );
    assert_eq!(
        change.action, action,
        "Stackstead change output broke its transport contract"
    );
    StacksteadManifest::read(&change.stackstead.files.manifest)
        .test_context("read changed manifest")
}

#[path = "cli_acceptance/fixtures.rs"]
mod fixtures;
use fixtures::*;

#[path = "cli_acceptance/command_help.rs"]
mod command_help;
#[path = "cli_acceptance/compose_config.rs"]
mod compose_config;
#[path = "cli_acceptance/compose_ports.rs"]
mod compose_ports;
#[path = "cli_acceptance/create_config.rs"]
mod create_config;
#[path = "cli_acceptance/current_and_pointer.rs"]
mod current_and_pointer;
#[path = "cli_acceptance/dependencies.rs"]
mod dependencies;
#[path = "cli_acceptance/destroy.rs"]
mod destroy;
#[path = "cli_acceptance/destroy_retry.rs"]
mod destroy_retry;
#[path = "cli_acceptance/doctor.rs"]
mod doctor;
#[path = "cli_acceptance/environment.rs"]
mod environment;
#[path = "cli_acceptance/exec.rs"]
mod exec;
#[path = "cli_acceptance/exec_leases.rs"]
mod exec_leases;
#[path = "cli_acceptance/health_inspection.rs"]
mod health_inspection;
#[path = "cli_acceptance/health_lifecycle.rs"]
mod health_lifecycle;
#[path = "cli_acceptance/identity_guards.rs"]
mod identity_guards;
#[path = "cli_acceptance/init.rs"]
mod init;
#[path = "cli_acceptance/inspect.rs"]
mod inspect;
#[path = "cli_acceptance/launch.rs"]
mod launch;
#[path = "cli_acceptance/lifecycle_locks.rs"]
mod lifecycle_locks;
#[path = "cli_acceptance/open.rs"]
mod open;
#[path = "cli_acceptance/port_leases.rs"]
mod port_leases;
#[cfg(unix)]
#[path = "cli_acceptance/readiness.rs"]
mod readiness;
#[path = "cli_acceptance/repair_and_recovery.rs"]
mod repair_and_recovery;
#[path = "cli_acceptance/run.rs"]
mod run;
#[path = "cli_acceptance/runtime_cleanup.rs"]
mod runtime_cleanup;
#[path = "cli_acceptance/runtime_ownership.rs"]
mod runtime_ownership;
#[path = "cli_acceptance/source_adoption.rs"]
mod source_adoption;
#[path = "cli_acceptance/source_contract.rs"]
mod source_contract;
#[path = "cli_acceptance/source_creation.rs"]
mod source_creation;
#[path = "cli_acceptance/source_recovery.rs"]
mod source_recovery;
#[path = "cli_acceptance/state_safety.rs"]
mod state_safety;
#[cfg(unix)]
#[path = "cli_acceptance/supervision.rs"]
mod supervision;
