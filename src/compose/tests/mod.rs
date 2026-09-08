use std::{collections::BTreeMap, path::Path};

use super::*;
use crate::{
    manifest::StacksteadManifest,
    test_support::{TestResultErrorExt as _, TestResultExt as _},
};

fn manifest() -> anyhow::Result<StacksteadManifest> {
    serde_json::from_value(serde_json::json!({
        "kind":"StacksteadManifest","version":"3","stackstead_id":"a-b123","slug":"a","short_id":"b123",
        "runtime_token":"0123456789abcdef0123456789abcdef",
        "readiness":{"configuration":"unconfigured"},
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

mod apply;
mod contract;
mod ownership_override;
mod ownership_resources;
mod planning;
mod ports;
mod services;
