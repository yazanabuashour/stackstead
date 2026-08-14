use super::{Cli, Commands, DatabaseCommand};

impl Cli {
    pub fn run(self) -> anyhow::Result<i32> {
        let cwd = std::env::current_dir()?;
        match &self.command {
            Commands::Init { compose_file } => self.init(&cwd, compose_file.as_deref())?,
            Commands::Compose { command } => self.compose(&cwd, command)?,
            Commands::Create { name } => self.create(&cwd, name)?,
            Commands::Adopt { name, worktree } => self.adopt(&cwd, name, worktree)?,
            Commands::Up { name } => self.up(&cwd, name)?,
            Commands::Run { name, command } => return self.run_agent(&cwd, name, command),
            Commands::Exec {
                name,
                service,
                command,
            } => return self.exec(&cwd, name, service, command),
            Commands::Launch { name, command } => return self.launch(&cwd, name, command),
            Commands::Ps => self.ps(&cwd)?,
            Commands::Current => self.current(&cwd)?,
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
            Commands::Stop { name } => self.stop(&cwd, name)?,
            Commands::Destroy { name, yes } => self.destroy(&cwd, name, *yes)?,
            Commands::Doctor { fail_on_error } => return self.doctor(&cwd, *fail_on_error),
            Commands::Repair { name } => self.repair(&cwd, name)?,
        }
        Ok(0)
    }
}
