use std::{ffi::OsString, path::PathBuf};

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
    /// Run a host command from the exact stackstead worktree.
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
    /// Run a command inside a running Compose service.
    Exec {
        name: String,
        service: String,
        #[arg(
            required = true,
            num_args = 1..,
            last = true,
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
    /// Print the validated identity of the current generated worktree.
    Current,
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
    /// Locate or print `AGENT_CONTEXT.md`.
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

mod dispatch;
mod operations;
mod presentation;
mod queries;
mod runtime;

#[cfg(test)]
mod tests;
