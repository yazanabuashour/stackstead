use std::{collections::BTreeMap, path::PathBuf};

use serde::Serialize;

use crate::manifest::StacksteadManifest;

#[derive(Debug, Serialize)]
pub(super) struct StacksteadOutput {
    stackstead_id: String,
    slug: String,
    project: String,
    branch: String,
    base: String,
    source_ownership: crate::manifest::SourceOwnership,
    repo_root: PathBuf,
    worktree: PathBuf,
    compose_project: String,
    compose_files: Vec<PathBuf>,
    ports: BTreeMap<String, u16>,
    container_ports: BTreeMap<String, u16>,
    urls: BTreeMap<String, String>,
    files: StacksteadFilesOutput,
    status: StacksteadStatusOutput,
    database: Option<StacksteadDatabaseOutput>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
struct StacksteadFilesOutput {
    manifest: PathBuf,
    environment: PathBuf,
    context: PathBuf,
    events: PathBuf,
    pointer: PathBuf,
}

#[derive(Debug, Serialize)]
struct StacksteadStatusOutput {
    source: String,
    dependencies: String,
    runtime: String,
    database: String,
    health: String,
}

#[derive(Debug, Serialize)]
struct StacksteadDatabaseOutput {
    strategy: String,
    service: String,
    host: String,
    port: u16,
    database: String,
    seed_status: String,
    last_seed_at: Option<String>,
}

impl From<&StacksteadManifest> for StacksteadOutput {
    fn from(manifest: &StacksteadManifest) -> Self {
        let mut compose_files = manifest.compose_files.clone();
        compose_files.push(manifest.worktree.join(".stackstead/compose-ownership.yaml"));
        Self {
            stackstead_id: manifest.stackstead_id.clone(),
            slug: manifest.slug.clone(),
            project: manifest.project.clone(),
            branch: manifest.branch.clone(),
            base: manifest.base.clone(),
            source_ownership: manifest.source_ownership,
            repo_root: manifest.repo_root.clone(),
            worktree: manifest.worktree.clone(),
            compose_project: manifest.compose_project.clone(),
            compose_files,
            ports: manifest.ports.clone(),
            container_ports: manifest.container_ports.clone(),
            urls: manifest.urls.clone(),
            files: StacksteadFilesOutput {
                manifest: manifest.manifest_path(),
                environment: manifest.env_file.clone(),
                context: manifest.agent_context.clone(),
                events: manifest.event_log.clone(),
                pointer: manifest.pointer_file.clone(),
            },
            status: StacksteadStatusOutput {
                source: manifest.status.source.to_string(),
                dependencies: manifest.status.dependencies.to_string(),
                runtime: manifest.status.runtime.to_string(),
                database: manifest.status.database.to_string(),
                health: manifest.status.health.to_string(),
            },
            database: manifest
                .database
                .as_ref()
                .map(|database| StacksteadDatabaseOutput {
                    strategy: database.strategy.clone(),
                    service: database.service.clone(),
                    host: database.host.clone(),
                    port: database.port,
                    database: database.database.clone(),
                    seed_status: database.seed_status.to_string(),
                    last_seed_at: database.last_seed_at.map(|value| value.to_rfc3339()),
                }),
            created_at: manifest.created_at.to_rfc3339(),
            updated_at: manifest.updated_at.to_rfc3339(),
        }
    }
}
