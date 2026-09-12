use std::{collections::BTreeMap, path::Path};

use super::Diagnostic;
use crate::{
    lock,
    manifest::{MANIFEST_VERSION, StacksteadManifest},
};

pub(super) fn read_manifests(
    project_state_dir: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<StacksteadManifest> {
    if !project_state_dir.exists() {
        diagnostics.push(Diagnostic::info(
            "state.no_stacksteads",
            format!("no stacksteads found under {}", project_state_dir.display()),
        ));
        return Vec::new();
    }

    let entries = match std::fs::read_dir(project_state_dir) {
        Ok(entries) => entries,
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                "state.unreadable",
                format!("cannot read {}: {error}", project_state_dir.display()),
                "make the project state directory readable",
            ));
            return Vec::new();
        }
    };

    let mut manifests = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                diagnostics.push(Diagnostic::error(
                    "state.entry_unreadable",
                    format!("cannot read project state entry: {error}"),
                    "check project state directory permissions",
                ));
                continue;
            }
        };
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }

        let manifest_path = entry.path().join("state/manifest.json");
        if !manifest_path.is_file() {
            diagnostics.push(Diagnostic::error(
                "manifest.missing",
                format!("stackstead directory has no manifest: {}", entry.path().display()),
                "remove the orphan only after verifying it is Stackstead-owned, or restore its manifest",
            ));
            continue;
        }
        match StacksteadManifest::read(&manifest_path) {
            Ok(manifest) => manifests.push(manifest),
            Err(error) => diagnostics.push(Diagnostic::error(
                "manifest.unreadable",
                format!("cannot read {}: {error}", manifest_path.display()),
                format!("restore a valid StacksteadManifest version {MANIFEST_VERSION} file; destroy unsupported stacksteads with the binary that created them, then recreate them with this version"),
            )),
        }
    }
    manifests.sort_by(|left, right| left.stackstead_id.cmp(&right.stackstead_id));
    diagnostics.push(Diagnostic::info(
        "manifest.count",
        format!("{} readable stackstead manifest(s) found", manifests.len()),
    ));
    manifests
}

pub(super) fn diagnose_duplicate_ports(
    manifests: &[StacksteadManifest],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut owners: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for manifest in manifests {
        for (service, port) in &manifest.ports {
            owners
                .entry(*port)
                .or_default()
                .push(format!("{}:{service}", manifest.stackstead_id));
        }
    }
    for (port, owners) in owners {
        if owners.len() > 1 {
            diagnostics.push(Diagnostic::error(
                "ports.duplicate_allocation",
                format!("host port {port} is allocated to {}", owners.join(", ")),
                "stop conflicting runtimes and repair or recreate one of the stacksteads",
            ));
        }
    }
}

pub(super) fn diagnose_duplicate_compose_projects(
    manifests: &[StacksteadManifest],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut owners: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for manifest in manifests {
        owners
            .entry(&manifest.compose_project)
            .or_default()
            .push(&manifest.stackstead_id);
    }
    for (project, owners) in owners {
        if owners.len() > 1 {
            diagnostics.push(Diagnostic::error(
                "compose.duplicate_project",
                format!(
                    "Compose project `{project}` is shared by {}",
                    owners.join(", ")
                ),
                "recreate one stackstead with a unique Compose project identity",
            ));
        }
    }
}

pub(super) fn diagnose_project_lock(project_state_dir: &Path, diagnostics: &mut Vec<Diagnostic>) {
    if !project_state_dir.is_dir() {
        return;
    }
    let path = lock::project_lock_path(project_state_dir);
    if !path.is_file() {
        diagnostics.push(Diagnostic::error(
            "lock.project.missing",
            format!("project lock is missing: {}", path.display()),
            "recreate the affected stacksteads; Stackstead will not infer or recreate missing lock ownership state",
        ));
        return;
    }
    diagnostics.push(if lock::LockGuard::can_acquire(&path) {
        Diagnostic::info(
            "lock.project.available",
            format!("project lock is available: {}", path.display()),
        )
    } else {
        Diagnostic::warning(
            "lock.project.busy",
            format!("project lock cannot be acquired: {}", path.display()),
            "wait for the active Stackstead operation to finish; do not infer staleness from the lock file alone",
        )
    });
}
