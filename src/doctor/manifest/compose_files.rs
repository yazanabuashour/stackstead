use super::{Diagnostic, StacksteadManifest, files::check_file};
use crate::compose;

pub(super) fn diagnose(manifest: &StacksteadManifest, diagnostics: &mut Vec<Diagnostic>) {
    let label = &manifest.stackstead_id;
    for compose_file in &manifest.compose_files {
        check_file(
            "compose.worktree_file_missing",
            label,
            "worktree Compose file",
            compose_file,
            "restore the configured Compose file in the worktree",
            diagnostics,
        );
        diagnose_fixed_ports(compose_file, diagnostics);
        diagnose_unbound_ports(label, compose_file, diagnostics);
        diagnose_exposed_ports(label, compose_file, diagnostics);
    }
}

fn diagnose_fixed_ports(compose_file: &std::path::Path, diagnostics: &mut Vec<Diagnostic>) {
    if let Ok(fixed_ports) = compose::fixed_ports_in_file(compose_file) {
        for fixed in fixed_ports {
            diagnostics.push(Diagnostic::error(
                "compose.worktree_fixed_host_port",
                format!(
                    "fixed host port {} found in {}:{} (`{}`)",
                    fixed.host_port,
                    compose_file.display(),
                    fixed.file_line,
                    fixed.mapping
                ),
                "replace the fixed host port with a generated Stackstead env variable",
            ));
        }
    }
}

fn diagnose_unbound_ports(
    label: &str,
    compose_file: &std::path::Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Ok(unbound) = compose::unbound_ports_in_file(compose_file) {
        for (service, container) in unbound {
            diagnostics.push(Diagnostic::error(
                "compose.worktree_unbound_host_port",
                format!(
                    "{label} service `{service}` publishes container port {container} without a deterministic host port in {}",
                    compose_file.display()
                ),
                "add the manifest-generated host-port mapping before starting this stackstead",
            ));
        }
    }
}

fn diagnose_exposed_ports(
    label: &str,
    compose_file: &std::path::Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Ok(exposed) = compose::all_interface_ports_in_file(compose_file) {
        for (service, container) in exposed {
            diagnostics.push(Diagnostic::error(
                "compose.worktree_all_interfaces_host_port",
                format!(
                    "{label} Compose port `{service}` ({container}/tcp) binds all host interfaces in {}",
                    compose_file.display()
                ),
                "bind the generated port to 127.0.0.1",
            ));
        }
    }
}
