use std::{
    collections::BTreeMap,
    path::Path,
    process::{Command, ExitStatus, Output, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct ConfiguredCommand {
    program: String,
    args: Vec<String>,
}

impl ConfiguredCommand {
    pub fn parse(command: &str, shell: bool) -> anyhow::Result<Option<Self>> {
        if command.trim().is_empty() {
            return Ok(None);
        }
        if shell {
            #[cfg(windows)]
            let (program, args) = ("cmd".into(), vec!["/C".into(), command.into()]);
            #[cfg(not(windows))]
            let (program, args) = ("sh".into(), vec!["-c".into(), command.into()]);
            return Ok(Some(Self { program, args }));
        }
        let words = shell_words::split(command)
            .map_err(|error| anyhow::anyhow!("cannot parse configured command: {error}"))?;
        let (program, args) = words
            .split_first()
            .filter(|(program, _)| !program.is_empty())
            .ok_or_else(|| anyhow::anyhow!("configured command requires a nonempty executable"))?;
        Ok(Some(Self {
            program: program.clone(),
            args: args.to_vec(),
        }))
    }

    pub fn run(&self, cwd: &Path, env: &BTreeMap<String, String>) -> anyhow::Result<Output> {
        super::run(&self.program, &self.args, cwd, env)
    }

    pub fn status_with_timeout(
        &self,
        cwd: &Path,
        env: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> anyhow::Result<Option<ExitStatus>> {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .current_dir(cwd)
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            anyhow::anyhow!("command timeout exceeds the supported Instant range")
        })?;
        super::status_until(&mut command, deadline)
    }
}
