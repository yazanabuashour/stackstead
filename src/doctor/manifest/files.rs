use std::path::Path;

use super::{Diagnostic, StacksteadManifest};
use crate::events;

pub(super) fn diagnose(manifest: &StacksteadManifest, diagnostics: &mut Vec<Diagnostic>) {
    let label = &manifest.stackstead_id;
    check_directory(
        "worktree.missing",
        label,
        &manifest.worktree,
        "restore the Git worktree or destroy the orphaned stackstead after review",
        diagnostics,
    );
    check_directory(
        "state.directory_missing",
        label,
        &manifest.state_dir,
        "run `stackstead repair` to recreate non-destructive state directories",
        diagnostics,
    );
    for (code, name, path, suggestion) in [
        (
            "pointer.missing",
            "pointer file",
            &manifest.pointer_file,
            "run `stackstead repair` to regenerate the pointer file",
        ),
        (
            "env.missing",
            "generated env file",
            &manifest.env_file,
            "run `stackstead repair` to regenerate the env file",
        ),
        (
            "context.missing",
            "agent context file",
            &manifest.agent_context,
            "run `stackstead repair` to regenerate agent context",
        ),
        (
            "events.missing",
            "event log",
            &manifest.event_log,
            "run `stackstead repair` to restore non-destructive state",
        ),
    ] {
        check_file(code, label, name, path, suggestion, diagnostics);
    }
    diagnose_events(manifest, diagnostics);
}

fn diagnose_events(manifest: &StacksteadManifest, diagnostics: &mut Vec<Diagnostic>) {
    if !manifest.event_log.is_file() {
        return;
    }
    let label = &manifest.stackstead_id;
    match events::read(&manifest.event_log) {
        Ok(log) if log.truncated_tail => diagnostics.push(Diagnostic::warning(
            "events.truncated_tail",
            format!("{label} event log has an unterminated final record"),
            "rerun the interrupted operation; only the incomplete final record is ignored",
        )),
        Ok(_) => diagnostics.push(Diagnostic::info(
            "events.valid",
            format!("{label} event log contains valid typed records"),
        )),
        Err(error) => diagnostics.push(Diagnostic::error(
            "events.invalid",
            format!("{label} event log is invalid: {error}"),
            "inspect the event journal before attempting recovery; completed malformed records are never ignored",
        )),
    }
}

pub(super) fn check_file(
    code: &str,
    label: &str,
    name: &str,
    path: &Path,
    suggestion: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !path.is_file() {
        diagnostics.push(Diagnostic::error(
            code,
            format!("{label} is missing {name}: {}", path.display()),
            suggestion,
        ));
    }
}

fn check_directory(
    code: &str,
    label: &str,
    path: &Path,
    suggestion: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !path.is_dir() {
        diagnostics.push(Diagnostic::error(
            code,
            format!("{label} is missing directory {}", path.display()),
            suggestion,
        ));
    }
}
