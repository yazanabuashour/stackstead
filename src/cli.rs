use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::{
    agent, compose, database, doctor, envfile, lifecycle,
    lock::LockGuard,
    manifest::{ComponentStatus, SourceOwnership, StacksteadManifest},
    open, output, repair, repository_policy,
};
use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "stackstead",
    version,
    about = "Isolated real application runtimes for parallel coding agents"
)]
pub struct Cli {
    #[arg(long, global = true, help = "Emit stable machine-readable JSON")]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Discover Compose and create a ready-to-review stackstead.yaml.
    Init {
        /// Use an explicit repository-relative Compose file.
        #[arg(long)]
        compose_file: Option<PathBuf>,
    },
    /// Inspect the Compose changes required for collision-free stacksteads.
    Compose {
        #[command(subcommand)]
        command: ComposeCommand,
    },
    /// Create a branch, worktree, ports, env, manifest, and agent context.
    Create { name: String },
    /// Bind an existing manager-owned worktree to an isolated runtime contract.
    Adopt {
        name: String,
        #[arg(long)]
        worktree: PathBuf,
    },
    /// Install dependencies and start the Compose runtime.
    Up { name: String },
    /// Run a command inside the exact stackstead source and runtime contract.
    Run {
        name: String,
        #[arg(
            required = true,
            num_args = 1..,
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        command: Vec<OsString>,
    },
    /// Create, start, and run a command in a new stackstead.
    Launch {
        name: String,
        #[arg(
            required = true,
            num_args = 1..,
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        command: Vec<OsString>,
    },
    /// List known stacksteads for this project.
    Ps,
    /// Show a durable contract plus computed live status.
    Inspect { name: String },
    /// Locate or print the generated environment.
    Env {
        name: String,
        #[arg(long)]
        print: bool,
        #[arg(long, requires = "print")]
        show_secrets: bool,
    },
    /// Delegate runtime logs to Docker Compose.
    Logs(LogsArgs),
    /// Locate or print AGENT_CONTEXT.md.
    Context {
        name: String,
        #[arg(long)]
        print: bool,
    },
    /// Open or print a configured local service URL.
    Open {
        name: String,
        service: Option<String>,
        #[arg(long)]
        print: bool,
    },
    /// Inspect stackstead-local database state.
    Db {
        #[command(subcommand)]
        command: DatabaseCommand,
    },
    /// Stop Compose services without deleting state.
    Stop { name: String },
    /// Delete the manifest-owned Compose project, volumes, worktree, and state.
    Destroy {
        name: String,
        #[arg(long)]
        yes: bool,
    },
    /// Run read-only diagnostics.
    Doctor {
        /// Exit with status 1 when any error diagnostic is present.
        #[arg(long)]
        fail_on_error: bool,
    },
    /// Conservatively regenerate contract files and dependency/link state.
    Repair { name: String },
}

#[derive(Debug, Args)]
struct LogsArgs {
    name: String,
    #[arg(long)]
    service: Option<String>,
    #[arg(long, default_value_t = 200)]
    tail: usize,
    #[arg(long)]
    follow: bool,
}

#[derive(Debug, Subcommand)]
enum DatabaseCommand {
    Status { name: String },
}

#[derive(Debug, Subcommand)]
enum ComposeCommand {
    /// Detect published services and propose deterministic port mappings.
    Plan {
        /// Inspect an explicit repository-relative Compose file.
        #[arg(long)]
        compose_file: Option<PathBuf>,
    },
    /// Rewrite common fixed host-port mappings to generated variables.
    Apply {
        /// Confirm writing the tracked Compose file.
        #[arg(long)]
        yes: bool,
        /// Rewrite an explicit repository-relative Compose file.
        #[arg(long)]
        compose_file: Option<PathBuf>,
    },
}

impl Cli {
    pub fn run(self) -> anyhow::Result<i32> {
        let cwd = std::env::current_dir()?;
        match &self.command {
            Commands::Init { compose_file } => {
                let path = lifecycle::init_with_compose_file(&cwd, compose_file.as_deref())?;
                if self.json {
                    print_json(&output::PathOutput::initialized(path))?;
                } else {
                    println!("Created {}", path.display());
                    print_compose_plan(&lifecycle::compose_plan(&cwd)?);
                    println!(
                        "\nNext: review, add, and commit this policy in AGENTS.md, CLAUDE.md, \
                         or your repository's equivalent instruction file:\n\n{}\n{}\n\n\
                         Stackstead may read recognized root instruction files during `doctor`, \
                         but it does not edit human-owned agent instructions.",
                        repository_policy::marker(),
                        repository_policy::TEXT
                    );
                }
            }
            Commands::Compose { command } => match command {
                ComposeCommand::Plan { compose_file } => {
                    let plan = lifecycle::compose_plan_with_file(&cwd, compose_file.as_deref())?;
                    if self.json {
                        print_json(&output::ComposePlanOutput::from(&plan))?;
                    } else {
                        print_compose_plan(&plan);
                    }
                }
                ComposeCommand::Apply { yes, compose_file } => {
                    if !yes {
                        anyhow::bail!(
                            "compose apply writes the tracked Compose file; review `stackstead compose plan` and rerun with --yes"
                        );
                    }
                    let output = lifecycle::compose_apply_with_file(&cwd, compose_file.as_deref())?;
                    if self.json {
                        print_json(&crate::output::ComposeApplyOutput::from(&output))?;
                    } else if output.changed_lines == 0 {
                        println!("No fixed host-port mappings needed changes.");
                    } else {
                        println!(
                            "Updated {} fixed host-port mapping(s) in {}. Review the Git diff before creating a stackstead.",
                            output.changed_lines,
                            output.file.display()
                        );
                    }
                }
            },
            Commands::Create { name } => {
                let manifest = lifecycle::create(&cwd, name)?;
                if self.json {
                    print_json(&output::StacksteadChangeOutput::new("created", &manifest))?;
                } else {
                    println!("Created {}", manifest.stackstead_id);
                    println!("Worktree: {}", manifest.worktree.display());
                    println!("Manifest: {}", manifest.manifest_path().display());
                }
            }
            Commands::Adopt { name, worktree } => {
                let manifest = lifecycle::adopt(&cwd, name, worktree)?;
                if self.json {
                    print_json(&output::StacksteadChangeOutput::new("adopted", &manifest))?;
                } else {
                    println!("Adopted {}", manifest.stackstead_id);
                    println!("External worktree: {}", manifest.worktree.display());
                    println!("Manifest: {}", manifest.manifest_path().display());
                }
            }
            Commands::Up { name } => {
                let lifecycle::UpOutcome {
                    manifest, timings, ..
                } = lifecycle::up(&cwd, name)?;
                if self.json {
                    print_json(&output::StacksteadChangeOutput::new("started", &manifest))?;
                } else {
                    println!(
                        "Running {} ({})",
                        manifest.stackstead_id, manifest.compose_project
                    );
                    print_urls(&manifest.urls);
                    print_up_timings(&timings);
                }
            }
            Commands::Run { name, command } => {
                if self.json {
                    anyhow::bail!(
                        "--json cannot be combined with run because child output is inherited"
                    );
                }
                let (program, args) = command
                    .split_first()
                    .ok_or_else(|| anyhow::anyhow!("run requires a command after `--`"))?;
                return agent::run(&cwd, name, program, args).map(agent::exit_code);
            }
            Commands::Launch { name, command } => {
                if self.json {
                    anyhow::bail!(
                        "--json cannot be combined with launch because child output is inherited"
                    );
                }
                let (program, args) = command
                    .split_first()
                    .ok_or_else(|| anyhow::anyhow!("launch requires a command after `--`"))?;
                let created = lifecycle::create_for_launch(&cwd, name)?;
                println!("Created {}", created.manifest.stackstead_id);
                let stackstead_id = created.manifest.stackstead_id.clone();
                let outcome =
                    lifecycle::up_after_create(&cwd, &stackstead_id, created.mutation_lock)?;
                println!(
                    "Running {} ({})",
                    outcome.manifest.stackstead_id, outcome.manifest.compose_project
                );
                print_urls(&outcome.manifest.urls);
                print_up_timings(&outcome.timings);
                return agent::run_after_up(
                    &cwd,
                    &outcome.manifest.stackstead_id,
                    program,
                    args,
                    outcome.mutation_lock,
                    outcome.run_lease,
                )
                .map(agent::exit_code);
            }
            Commands::Ps => self.ps(&cwd)?,
            Commands::Inspect { name } => self.inspect(&cwd, name)?,
            Commands::Env {
                name,
                print,
                show_secrets,
            } => self.env(&cwd, name, *print, *show_secrets)?,
            Commands::Logs(args) => self.logs(&cwd, args)?,
            Commands::Context { name, print } => self.context(&cwd, name, *print)?,
            Commands::Open {
                name,
                service,
                print,
            } => self.open(&cwd, name, service.as_deref(), *print)?,
            Commands::Db { command } => match command {
                DatabaseCommand::Status { name } => self.db_status(&cwd, name)?,
            },
            Commands::Stop { name } => {
                let manifest = lifecycle::stop(&cwd, name)?;
                if self.json {
                    print_json(&output::StacksteadChangeOutput::new("stopped", &manifest))?;
                } else {
                    println!("Stopped {}", manifest.stackstead_id);
                }
            }
            Commands::Destroy { name, yes } => self.destroy(&cwd, name, *yes)?,
            Commands::Doctor { fail_on_error } => {
                let diagnostics = doctor::run(&cwd)?;
                let report = output::DoctorOutput::new(&diagnostics);
                if self.json {
                    print_json(&report)?;
                } else {
                    for diagnostic in diagnostics {
                        println!(
                            "{:<7} {:<28} {}",
                            diagnostic.severity, diagnostic.code, diagnostic.message
                        );
                        if let Some(suggestion) = diagnostic.suggestion {
                            println!("        suggestion: {suggestion}");
                        }
                    }
                }
                if *fail_on_error && report.has_errors() {
                    return Ok(1);
                }
            }
            Commands::Repair { name } => {
                let manifest = repair::run(&cwd, name)?;
                if self.json {
                    print_json(&output::StacksteadChangeOutput::new("repaired", &manifest))?;
                } else {
                    println!("Repaired {}", manifest.stackstead_id);
                }
            }
        }
        Ok(0)
    }

    fn ps(&self, cwd: &Path) -> anyhow::Result<()> {
        let runtime = lifecycle::load_project(cwd)?;
        let stacksteads = runtime
            .paths
            .manifests()?
            .into_iter()
            .map(|manifest| {
                let status = match compose::is_running(&manifest) {
                    Ok(true) => "running".into(),
                    Ok(false) => "stopped".into(),
                    Err(_) => "unknown".into(),
                };
                output::StacksteadSummaryOutput::new(manifest, status)
            })
            .collect::<Vec<_>>();
        let output = output::StacksteadListOutput::new(stacksteads);
        if self.json {
            print_json(&output)?;
        } else if output.stacksteads().is_empty() {
            println!("No stacksteads. Create one with `stackstead create <name>`.");
        } else {
            println!(
                "{:<28} {:<24} {:<10} PORTS",
                "STACKSTEAD", "BRANCH", "STATUS"
            );
            for item in output.stacksteads() {
                let ports = item
                    .ports()
                    .iter()
                    .map(|(service, port)| format!("{service}={port}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                println!(
                    "{:<28} {:<24} {:<10} {}",
                    item.stackstead_id(),
                    item.branch(),
                    item.runtime(),
                    ports
                );
            }
        }
        Ok(())
    }

    fn inspect(&self, cwd: &Path, name: &str) -> anyhow::Result<()> {
        let output = lifecycle::inspect(cwd, name)?;
        if self.json {
            return print_json(&crate::output::StacksteadInspectionOutput::new(&output));
        }
        let manifest = &output.manifest;
        let database_status = manifest
            .database
            .as_ref()
            .map(|_| database::live_status(manifest, output.live.runtime_status));
        println!("Stackstead: {}\n", manifest.stackstead_id);
        println!("Source:        {}", manifest.status.source);
        println!("Dependencies:  {}", manifest.status.dependencies);
        println!("Runtime:       {}", output.live.runtime_status);
        println!("Services:");
        if output.live.services.is_empty() {
            println!("  none");
        } else {
            for service in &output.live.services {
                println!("  {:<14} {}", service.service, service.status());
            }
        }
        println!(
            "Database:      {}",
            database_status.map_or_else(|| "not configured".into(), |status| status.to_string())
        );
        println!(
            "Health:        {}\n",
            output.live.health_healthy.map_or_else(
                || manifest.status.health.to_string(),
                |healthy| if healthy { "ready" } else { "failed" }.into()
            )
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
        for action in next_actions(&manifest.stackstead_id, output.live.runtime_status) {
            println!("  {action}");
        }
        Ok(())
    }

    fn env(&self, cwd: &Path, name: &str, print: bool, show_secrets: bool) -> anyhow::Result<()> {
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

    fn context(&self, cwd: &Path, name: &str, print: bool) -> anyhow::Result<()> {
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

    fn logs(&self, cwd: &Path, args: &LogsArgs) -> anyhow::Result<()> {
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

    fn open(
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

    fn db_status(&self, cwd: &Path, name: &str) -> anyhow::Result<()> {
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

    fn destroy(&self, cwd: &Path, name: &str, yes: bool) -> anyhow::Result<()> {
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

fn next_actions(stackstead_id: &str, runtime_status: ComponentStatus) -> [String; 2] {
    let runtime = match runtime_status {
        ComponentStatus::Running => format!("stackstead logs {stackstead_id} --tail 200"),
        ComponentStatus::Stopped => format!("stackstead up {stackstead_id}"),
        _ => "stackstead doctor".into(),
    };
    [
        runtime,
        format!("stackstead context {stackstead_id} --print"),
    ]
}

fn print_up_timings(timings: &lifecycle::UpTimings) {
    println!("\nTimings:");
    print_timing("Dependencies", timings.dependencies);
    print_timing("Runtime start", timings.runtime);
    for (label, duration) in [
        ("DB readiness", timings.database),
        ("Seed", timings.seed),
        ("Hooks", timings.hooks),
        ("Health checks", timings.health),
    ] {
        if let Some(duration) = duration {
            print_timing(label, duration);
        }
    }
    print_timing("Total", timings.total);
}

fn print_timing(label: &str, duration: std::time::Duration) {
    let elapsed = if duration.as_millis() == 0 {
        "<1ms".into()
    } else if duration.as_secs() == 0 {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{:.1}s", duration.as_secs_f64())
    };
    println!("  {label:<14} {elapsed:>8}");
}

fn print_urls(urls: &BTreeMap<String, String>) {
    if !urls.is_empty() {
        println!("URLs:");
        for (service, url) in urls {
            println!("  {service:<14} {url}");
        }
    }
}

fn print_compose_plan(plan: &compose::ComposePlan) {
    println!("Compose: {}", plan.file.display());
    if plan.ports.is_empty() {
        println!("No published service ports detected.");
    } else {
        println!("Detected isolation contract:");
        for port in &plan.ports {
            println!(
                "  {:<16} container {:<5} env {:<24} mapping {}",
                port.name, port.container_port, port.env, port.replacement
            );
        }
    }
    let fixed = plan
        .ports
        .iter()
        .filter_map(|port| port.current_host_port.map(|host| (port, host)))
        .collect::<Vec<_>>();
    if !fixed.is_empty() {
        println!("Required Compose edits before `stackstead up`:");
        for (port, host_port) in fixed {
            println!(
                "  {}: replace fixed host port {} with `{}`",
                port.service, host_port, port.replacement
            );
        }
        println!(
            "Run `stackstead compose apply --yes` to make these narrow edits, then review the Git diff."
        );
    }
    for warning in &plan.warnings {
        println!("Warning: {warning}");
    }
}

fn print_json<T: output::CliOutput>(value: &T) -> anyhow::Result<()> {
    serde_json::to_writer_pretty(io::stdout().lock(), value)?;
    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_required_command_surface() {
        for command in [
            vec!["stackstead", "init"],
            vec!["stackstead", "compose", "plan"],
            vec!["stackstead", "compose", "apply", "--yes"],
            vec!["stackstead", "create", "feature-a"],
            vec![
                "stackstead",
                "adopt",
                "feature-a",
                "--worktree",
                "/tmp/feature-a",
            ],
            vec!["stackstead", "up", "feature-a"],
            vec!["stackstead", "run", "feature-a", "--", "agent", "--flag"],
            vec!["stackstead", "launch", "feature-a", "--", "agent", "--flag"],
            vec!["stackstead", "ps"],
            vec!["stackstead", "inspect", "feature-a"],
            vec!["stackstead", "env", "feature-a", "--print"],
            vec!["stackstead", "logs", "feature-a", "--tail", "10"],
            vec!["stackstead", "context", "feature-a"],
            vec!["stackstead", "open", "feature-a", "web", "--print"],
            vec!["stackstead", "db", "status", "feature-a"],
            vec!["stackstead", "stop", "feature-a"],
            vec!["stackstead", "destroy", "feature-a", "--yes"],
            vec!["stackstead", "doctor"],
            vec!["stackstead", "repair", "feature-a"],
        ] {
            Cli::try_parse_from(command).unwrap();
        }
    }

    #[test]
    fn json_is_global_after_subcommand() {
        assert!(
            Cli::try_parse_from(["stackstead", "ps", "--json"])
                .unwrap()
                .json
        );
    }

    #[test]
    fn inspect_actions_use_the_full_id_and_runtime_state() {
        for (status, runtime_action) in [
            (
                ComponentStatus::Stopped,
                "stackstead up demo-feature-a-b123",
            ),
            (
                ComponentStatus::Running,
                "stackstead logs demo-feature-a-b123 --tail 200",
            ),
            (ComponentStatus::Unknown, "stackstead doctor"),
        ] {
            assert_eq!(
                next_actions("demo-feature-a-b123", status),
                [
                    runtime_action,
                    "stackstead context demo-feature-a-b123 --print",
                ]
            );
        }
    }
}
