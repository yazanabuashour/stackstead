use std::{ffi::OsString, path::Path};

use super::{
    Cli, ComposeCommand,
    presentation::{print_compose_plan, print_json, print_up_timings, print_urls},
};
use crate::{agent, doctor, lifecycle, output, repair, repository_policy};

impl Cli {
    pub(super) fn init(&self, cwd: &Path, compose_file: Option<&Path>) -> anyhow::Result<()> {
        let path = lifecycle::init_with_compose_file(cwd, compose_file)?;
        if self.json {
            print_json(&output::PathOutput::initialized(path))?;
        } else {
            println!("Created {}", path.display());
            print_compose_plan(&lifecycle::compose_plan(cwd)?);
            println!(
                "\nNext: review, add, and commit this policy in AGENTS.md, CLAUDE.md, \
                 or your repository's equivalent instruction file:\n\n{}\n{}\n\n\
                 Stackstead may read recognized root instruction files during `doctor`, \
                 but it does not edit human-owned agent instructions.",
                repository_policy::marker(),
                repository_policy::TEXT
            );
        }
        Ok(())
    }

    pub(super) fn compose(&self, cwd: &Path, command: &ComposeCommand) -> anyhow::Result<()> {
        match command {
            ComposeCommand::Plan { compose_file } => {
                let plan = lifecycle::compose_plan_with_file(cwd, compose_file.as_deref())?;
                if self.json {
                    print_json(&output::ComposePlanOutput::from(&plan))?;
                } else {
                    print_compose_plan(&plan);
                }
            }
            ComposeCommand::Apply { yes, compose_file } => {
                self.compose_apply(cwd, *yes, compose_file.as_deref())?;
            }
        }
        Ok(())
    }

    fn compose_apply(&self, cwd: &Path, yes: bool, file: Option<&Path>) -> anyhow::Result<()> {
        if !yes {
            anyhow::bail!(
                "compose apply writes the tracked Compose file; review `stackstead compose plan` and rerun with --yes"
            );
        }
        let applied = lifecycle::compose_apply_with_file(cwd, file)?;
        if self.json {
            print_json(&output::ComposeApplyOutput::from(&applied))?;
        } else if applied.changed_lines == 0 {
            println!("No fixed host-port mappings needed changes.");
        } else {
            println!(
                "Updated {} fixed host-port mapping(s) in {}. Review the Git diff before creating a stackstead.",
                applied.changed_lines,
                applied.file.display()
            );
        }
        Ok(())
    }

    pub(super) fn create(&self, cwd: &Path, name: &str) -> anyhow::Result<()> {
        let manifest = lifecycle::create(cwd, name)?;
        if self.json {
            print_json(&output::StacksteadChangeOutput::new("created", &manifest))?;
        } else {
            println!("Created {}", manifest.stackstead_id);
            println!("Worktree: {}", manifest.worktree.display());
            println!("Manifest: {}", manifest.manifest_path().display());
        }
        Ok(())
    }

    pub(super) fn adopt(&self, cwd: &Path, name: &str, worktree: &Path) -> anyhow::Result<()> {
        let manifest = lifecycle::adopt(cwd, name, worktree)?;
        if self.json {
            print_json(&output::StacksteadChangeOutput::new("adopted", &manifest))?;
        } else {
            println!("Adopted {}", manifest.stackstead_id);
            println!("External worktree: {}", manifest.worktree.display());
            println!("Manifest: {}", manifest.manifest_path().display());
        }
        Ok(())
    }

    pub(super) fn up(&self, cwd: &Path, name: &str) -> anyhow::Result<()> {
        let lifecycle::UpOutcome {
            manifest, timings, ..
        } = lifecycle::up(cwd, name)?;
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
        Ok(())
    }

    pub(super) fn run_agent(
        &self,
        cwd: &Path,
        name: &str,
        command: &[OsString],
    ) -> anyhow::Result<i32> {
        self.reject_json_stream("run", "child output is inherited")?;
        let (program, args) = command
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("run requires a command after `--`"))?;
        agent::run(cwd, name, program, args).map(agent::exit_code)
    }

    pub(super) fn exec(
        &self,
        cwd: &Path,
        name: &str,
        service: &str,
        command: &[OsString],
    ) -> anyhow::Result<i32> {
        self.reject_json_stream("exec", "child output is inherited")?;
        let (program, args) = command
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("exec requires a command after `--`"))?;
        agent::exec(cwd, name, service, program, args).map(agent::exit_code)
    }

    pub(super) fn launch(
        &self,
        cwd: &Path,
        name: &str,
        command: &[OsString],
    ) -> anyhow::Result<i32> {
        self.reject_json_stream("launch", "child output is inherited")?;
        let (program, args) = command
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("launch requires a command after `--`"))?;
        let created = lifecycle::create_for_launch(cwd, name)?;
        println!("Created {}", created.manifest.stackstead_id);
        let stackstead_id = created.manifest.stackstead_id.clone();
        let outcome = lifecycle::up_after_create(cwd, &stackstead_id, created.mutation_lock)?;
        println!(
            "Running {} ({})",
            outcome.manifest.stackstead_id, outcome.manifest.compose_project
        );
        print_urls(&outcome.manifest.urls);
        print_up_timings(&outcome.timings);
        agent::run_after_up(
            cwd,
            &outcome.manifest.stackstead_id,
            program,
            args,
            outcome.mutation_lock,
            outcome.run_lease,
        )
        .map(agent::exit_code)
    }

    fn reject_json_stream(&self, command: &str, reason: &str) -> anyhow::Result<()> {
        if self.json {
            anyhow::bail!("--json cannot be combined with {command} because {reason}");
        }
        Ok(())
    }

    pub(super) fn current(&self, cwd: &Path) -> anyhow::Result<()> {
        let current = lifecycle::current(cwd)?;
        if self.json {
            print_json(&output::StacksteadCurrentOutput::from(&current))?;
        } else {
            println!("{}", current.stackstead_id);
        }
        Ok(())
    }

    pub(super) fn stop(&self, cwd: &Path, name: &str) -> anyhow::Result<()> {
        let manifest = lifecycle::stop(cwd, name)?;
        if self.json {
            print_json(&output::StacksteadChangeOutput::new("stopped", &manifest))?;
        } else {
            println!("Stopped {}", manifest.stackstead_id);
        }
        Ok(())
    }

    pub(super) fn doctor(&self, cwd: &Path, fail_on_error: bool) -> anyhow::Result<i32> {
        let diagnostics = doctor::run(cwd);
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
        Ok(i32::from(fail_on_error && report.has_errors()))
    }

    pub(super) fn repair(&self, cwd: &Path, name: &str) -> anyhow::Result<()> {
        let manifest = repair::run(cwd, name)?;
        if self.json {
            print_json(&output::StacksteadChangeOutput::new("repaired", &manifest))?;
        } else {
            println!("Repaired {}", manifest.stackstead_id);
        }
        Ok(())
    }
}
