use std::path::Path;

use super::Diagnostic;
use crate::{compose, config::StacksteadConfig};

pub(super) fn diagnose_compose_files(
    repo_root: &Path,
    config: &StacksteadConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let files = config
        .runtime
        .files
        .iter()
        .map(|relative| repo_root.join(relative))
        .collect::<Vec<_>>();
    for path in &files {
        match compose::fixed_ports_in_file(path) {
            Ok(fixed_ports) => {
                diagnostics.push(Diagnostic::info(
                    "compose.file.readable",
                    format!("Compose file is readable: {}", path.display()),
                ));
                for fixed in fixed_ports {
                    diagnostics.push(Diagnostic::error(
                        "compose.fixed_host_port",
                        format!(
                            "fixed host port {} found in {}:{} (`{}`)",
                            fixed.host_port,
                            path.display(),
                            fixed.file_line,
                            fixed.mapping
                        ),
                        "replace the fixed host port with a generated Stackstead env variable",
                    ));
                }
                if let Ok(unbound) = compose::unbound_ports_in_file(path) {
                    for (service, container) in unbound {
                        diagnostics.push(Diagnostic::error(
                            "compose.unbound_host_port",
                            format!(
                                "service `{service}` publishes container port {container} without a deterministic host port in {}",
                                path.display()
                            ),
                            "add a generated host-port mapping such as `127.0.0.1:${WEB_PORT}:80`",
                        ));
                    }
                }
                if let Ok(exposed) = compose::all_interface_ports_in_file(path) {
                    for (service, container) in exposed {
                        diagnostics.push(Diagnostic::error(
                            "compose.all_interfaces_host_port",
                            format!(
                                "Compose port `{service}` ({container}/tcp) binds all host interfaces in {}",
                                path.display()
                            ),
                            "bind `127.0.0.1:${PORT}:<container-port>`",
                        ));
                    }
                }
            }
            Err(error) => diagnostics.push(Diagnostic::error(
                "compose.file.unreadable",
                format!("cannot inspect {}: {error}", path.display()),
                "make the configured Compose file readable",
            )),
        }
    }
    let expected = config
        .resources
        .ports
        .expose
        .iter()
        .map(|(name, exposure)| (name.clone(), exposure.container))
        .collect();
    if let Err(error) = compose::validate_port_contract(&files, &expected, &config.env.generate) {
        diagnostics.push(Diagnostic::error(
            "compose.isolation_contract.invalid",
            format!("Compose isolation contract is unsafe or disconnected: {error}"),
            "make every published host port consume its matching env.generate allocation",
        ));
    }
}

pub(super) fn diagnose_state_root(state_root: &Path, diagnostics: &mut Vec<Diagnostic>) {
    if state_root.exists() && !state_root.is_dir() {
        diagnostics.push(Diagnostic::error(
            "state.root.not_directory",
            format!("state root is not a directory: {}", state_root.display()),
            "choose a writable directory for state.root",
        ));
        return;
    }

    let nearest = state_root.ancestors().find(|candidate| candidate.is_dir());
    match nearest {
        Some(directory)
            if !std::fs::metadata(directory).is_ok_and(|meta| meta.permissions().readonly()) =>
        {
            diagnostics.push(Diagnostic::info(
                "state.root.writable",
                format!(
                    "state root has a writable existing ancestor: {}",
                    directory.display()
                ),
            ));
        }
        Some(directory) => diagnostics.push(Diagnostic::error(
            "state.root.read_only",
            format!("state root ancestor is read-only: {}", directory.display()),
            "choose a writable state.root or update its filesystem permissions",
        )),
        None => diagnostics.push(Diagnostic::error(
            "state.root.unreachable",
            format!(
                "state root has no existing ancestor: {}",
                state_root.display()
            ),
            "choose a reachable state.root",
        )),
    }
}
