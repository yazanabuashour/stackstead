use std::path::Path;

use super::{
    Cli, LogsArgs,
    presentation::{next_actions, print_json, print_runtime, print_urls},
};
use crate::{compose, envfile, lifecycle, output};

impl Cli {
    pub(super) fn ps(&self, cwd: &Path) -> anyhow::Result<()> {
        let runtime = lifecycle::load_project(cwd)?;
        let stacksteads = runtime
            .paths
            .manifests()?
            .into_iter()
            .map(|manifest| {
                lifecycle::validate_manifest_binding(&runtime, &manifest)?;
                let observation = lifecycle::observe_runtime(&manifest);
                Ok(output::StacksteadSummaryOutput::new(manifest, &observation))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let output = output::StacksteadListOutput::new(stacksteads);
        if self.json {
            print_json(&output)?;
        } else if output.stacksteads().is_empty() {
            println!("No stacksteads. Create one with `stackstead create <name>`.");
        } else {
            println!(
                "{:<28} {:<24} {:<10} {:<12} PORTS",
                "STACKSTEAD", "BRANCH", "ACTIVITY", "READINESS"
            );
            for item in output.stacksteads() {
                let ports = item
                    .ports()
                    .iter()
                    .map(|(service, port)| format!("{service}={port}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                println!(
                    "{:<28} {:<24} {:<10} {:<12} {}",
                    item.stackstead_id(),
                    item.branch(),
                    item.runtime(),
                    item.readiness(),
                    ports
                );
                for service in item.service_statuses() {
                    println!("  {service}");
                }
                for issue in item.issues() {
                    println!("  - {issue}");
                }
            }
        }
        Ok(())
    }

    pub(super) fn inspect(&self, cwd: &Path, name: &str) -> anyhow::Result<()> {
        let output = lifecycle::inspect(cwd, name)?;
        if self.json {
            return print_json(&crate::output::StacksteadInspectionOutput::new(&output));
        }
        let manifest = &output.manifest;
        println!("Stackstead: {}\n", manifest.stackstead_id);
        println!("Source:        {}", manifest.status.source);
        println!("Dependencies:  {}", manifest.status.dependencies);
        println!(
            "Recorded:      runtime={} database={} health={}",
            manifest.status.runtime, manifest.status.database, manifest.status.health
        );
        print_runtime(&output);
        println!(
            "Database:      {}",
            output
                .live
                .database_status
                .map_or_else(|| "not configured".into(), |status| status.to_string())
        );
        println!(
            "Application health: {} ({})\n",
            output.effective.health.status, output.effective.health.basis
        );
        println!("Branch:        {}", manifest.branch);
        println!("Worktree:      {}", manifest.worktree.display());
        println!("Compose:       {}\n", manifest.compose_project);
        print_urls(&manifest.urls);
        println!("\nPorts:");
        for (service, port) in &manifest.ports {
            let target = manifest
                .container_ports
                .get(service)
                .copied()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "manifest port contract for {} has no container port for `{service}`",
                        manifest.stackstead_id
                    )
                })?;
            println!("  {service:<14} {port} -> {target}");
        }
        println!("\nFiles:");
        println!("  manifest:     {}", manifest.manifest_path().display());
        println!("  env:          {}", manifest.env_file.display());
        println!("  context:      {}", manifest.agent_context.display());
        println!("  events:       {}", manifest.event_log.display());
        println!("\nWarnings:");
        if output.warnings.is_empty() {
            println!("  none");
        } else {
            for warning in output.warnings {
                println!("  - {warning}");
            }
        }
        println!("\nNext:");
        for action in next_actions(&manifest.stackstead_id, output.live.runtime.status()) {
            println!("  {action}");
        }
        Ok(())
    }

    pub(super) fn env(
        &self,
        cwd: &Path,
        name: &str,
        print: bool,
        show_secrets: bool,
    ) -> anyhow::Result<()> {
        let runtime = lifecycle::load_project(cwd)?;
        let manifest = runtime.resolve(name)?;
        let values = if show_secrets {
            envfile::read(&manifest.env_file)?
        } else {
            envfile::redacted_summary(&manifest.env_file)?
        };
        if self.json {
            return print_json(&output::EnvironmentOutput::new(
                manifest.stackstead_id,
                manifest.env_file,
                values,
            ));
        }
        if print {
            println!("{}", envfile::rendered(&manifest.env_file, show_secrets)?);
        } else {
            println!("Environment: {}", manifest.env_file.display());
            for (key, value) in values {
                println!("  {key}={value}");
            }
        }
        Ok(())
    }

    pub(super) fn context(&self, cwd: &Path, name: &str, print: bool) -> anyhow::Result<()> {
        let runtime = lifecycle::load_project(cwd)?;
        let manifest = runtime.resolve(name)?;
        let content = print
            .then(|| std::fs::read_to_string(&manifest.agent_context))
            .transpose()?;
        if self.json {
            return print_json(&output::ContextOutput::new(
                manifest.stackstead_id,
                manifest.agent_context,
                content,
            ));
        }
        if let Some(content) = content {
            print!("{content}");
        } else {
            println!("Agent context: {}", manifest.agent_context.display());
        }
        Ok(())
    }

    pub(super) fn logs(&self, cwd: &Path, args: &LogsArgs) -> anyhow::Result<()> {
        let runtime = lifecycle::load_project(cwd)?;
        let manifest = runtime.resolve(&args.name)?;
        if args.follow {
            if self.json {
                anyhow::bail!("--json cannot be combined with --follow because logs are streaming");
            }
            return compose::follow_logs(&manifest, args.service.as_deref(), args.tail);
        }
        let content = compose::logs(&manifest, args.service.as_deref(), args.tail)?;
        if self.json {
            print_json(&output::LogsOutput::new(
                manifest.stackstead_id,
                args.service.clone(),
                args.tail,
                content,
            ))?;
        } else {
            print!("{content}");
        }
        Ok(())
    }
}
