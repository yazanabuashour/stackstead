mod agent;
mod cli;
mod command;
mod compose;
mod config;
mod context;
mod database;
mod discovery;
mod doctor;
mod envfile;
mod error;
mod events;
mod git;
mod health;
mod lease;
mod lifecycle;
mod lock;
mod manifest;
mod open;
mod output;
mod paths;
mod ports;
mod repair;
mod repository_policy;
mod slug;
mod state;
mod template;

use clap::Parser;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .without_time()
        .init();

    match cli::Cli::parse().run() {
        Ok(0) => {}
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("error: {error:#}");
            std::process::exit(1);
        }
    }
}
