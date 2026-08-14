use std::{
    io::{self, Write},
    path::Path,
};

use super::{Cli, presentation::print_json};
use crate::{
    compose, database, lifecycle,
    lock::LockGuard,
    manifest::{SourceOwnership, StacksteadManifest},
    open, output,
};

impl Cli {
    pub(super) fn open(
        &self,
        cwd: &Path,
        name: &str,
        service: Option<&str>,
        print_only: bool,
    ) -> anyhow::Result<()> {
        let runtime = lifecycle::load_project(cwd)?;
        let manifest = runtime.resolve(name)?;
        let target = open::resolve(&manifest, service)?;
        let should_open = !print_only && !self.json;
        if should_open && let Some(launch) = open::launch_endpoint(&target, &manifest)? {
            let _run_lease = LockGuard::acquire_existing_shared(
                &manifest.state_dir.join("run.lock"),
                "stackstead browser launch",
            )?;
            lifecycle::validate_current_contract(&runtime, &manifest)?;
            lifecycle::verify_port_leases(&manifest)?;
            compose::verify_owned_runtime(&manifest)?;
            let compose_target = compose::resolve_port_target(
                &manifest.compose_files,
                &manifest.container_ports,
                &runtime.config.env.generate,
                &launch.contract_key,
            )?;
            if !compose::service_is_running(&manifest, &compose_target.service)? {
                anyhow::bail!(
                    "Compose service `{}` for port contract `{}` is not running",
                    compose_target.service,
                    launch.contract_key
                );
            }
            compose::ensure_endpoint_published(
                &manifest,
                &compose_target.service,
                compose_target.container_port,
                &launch.endpoint.host,
                launch.endpoint.port,
            )?;
            open::launch(&target.value)?;
        }
        if self.json {
            print_json(&output::OpenOutput::new(
                manifest.stackstead_id,
                target.value,
            ))?;
        } else {
            println!("{}", target.value);
            if !target.value.starts_with("http://") && !target.value.starts_with("https://") {
                eprintln!("note: this service exposes a raw port, not an HTTP URL");
            }
        }
        Ok(())
    }

    pub(super) fn db_status(&self, cwd: &Path, name: &str) -> anyhow::Result<()> {
        let runtime = lifecycle::load_project(cwd)?;
        let mut manifest = runtime.resolve(name)?;
        let _run_lease = LockGuard::acquire_existing_shared(
            &manifest.state_dir.join("run.lock"),
            "stackstead database status",
        )?;
        manifest = StacksteadManifest::read(&manifest.manifest_path())?;
        lifecycle::validate_manifest_binding(&runtime, &manifest)?;
        lifecycle::validate_current_contract(&runtime, &manifest)?;
        lifecycle::verify_port_leases(&manifest)?;
        let status = database::status(&manifest)?;
        let identity_status = database::identity_status(&manifest);
        if self.json {
            print_json(&output::DatabaseStatusOutput::new(
                status,
                identity_status.to_string(),
            ))?;
        } else {
            println!("Database: {}", status.database);
            println!("Strategy: {}", status.strategy);
            println!("Service:  {}", status.service);
            println!("Address:  {}:{}", status.host, status.port);
            println!("TCP listener: {}", status.reachable);
            println!("Identity:     {identity_status}");
            println!("Seed:      {}", status.seed_status);
            if let Some(last_seed_at) = status.last_seed_at {
                println!("Last seed: {last_seed_at}");
            }
        }
        Ok(())
    }

    pub(super) fn destroy(&self, cwd: &Path, name: &str, yes: bool) -> anyhow::Result<()> {
        if self.json && !yes {
            anyhow::bail!("--json destroy requires --yes so stdout remains machine-readable");
        }
        let manifest = lifecycle::resolve_destroy(cwd, name)?;
        if !yes {
            println!("This will destroy:");
            println!(
                "  Compose project and volumes: {}",
                manifest.compose_project
            );
            println!(
                "  Git worktree: {} ({})",
                manifest.worktree.display(),
                if manifest.source_ownership == SourceOwnership::Stackstead {
                    "removed"
                } else {
                    "preserved"
                }
            );
            println!("  Git branch: {} (preserved)", manifest.branch);
            println!("  Local Compose build images: removed when runtime resources exist");
            println!("  Project coordination lock: preserved");
            println!("  Stackstead state: {}", manifest.stackstead_root.display());
            print!("Continue? [y/N] ");
            io::stdout().flush()?;
            let mut answer = String::new();
            io::stdin().read_line(&mut answer)?;
            if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                anyhow::bail!("destroy cancelled");
            }
        }
        let destroyed = lifecycle::destroy(cwd, &manifest.stackstead_id)?;
        if self.json {
            print_json(&output::StacksteadChangeOutput::new(
                "destroyed",
                &destroyed,
            ))?;
        } else {
            println!("Destroyed {}", destroyed.stackstead_id);
            println!(
                "Preserved Git branch {} and the project coordination lock.",
                destroyed.branch
            );
        }
        Ok(())
    }
}
