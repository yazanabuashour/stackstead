use super::*;

pub(super) struct Project {
    pub _temp: TempDir,
    pub repo: PathBuf,
}

impl Project {
    pub(super) fn git_repo() -> anyhow::Result<Self> {
        let temp = tempfile::tempdir().test_context("create temporary project parent")?;
        let repo = temp.path().join("demo-project");
        fs::create_dir(&repo).test_context("create temporary repository")?;
        let repo = repo
            .canonicalize()
            .test_context("canonicalize temporary repository")?;
        git(&repo, &["init", "--initial-branch=main"])?;
        git(&repo, &["config", "user.name", "Stackstead Tests"])?;
        git(
            &repo,
            &["config", "user.email", "stackstead-tests@example.invalid"],
        )?;
        fs::write(repo.join("README.md"), "# Demo project\n").test_context("write README")?;
        fs::write(
            repo.join("docker-compose.yml"),
            r#"services:
  web:
    image: nginx:alpine
    ports:
      - "127.0.0.1:${WEB_PORT}:80"
  postgres:
    image: postgres:16-alpine
    ports:
      - "127.0.0.1:${POSTGRES_PORT}:5432"
"#,
        )
        .test_context("write Compose fixture")?;
        git(&repo, &["add", "."])?;
        git(&repo, &["commit", "-m", "initial fixture"])?;
        Ok(Self { _temp: temp, repo })
    }

    pub(super) fn initialized() -> anyhow::Result<Self> {
        let project = Self::git_repo()?;
        stackstead(&project.repo).arg("init").assert().success();
        git(&project.repo, &["add", "stackstead.yaml"])?;
        git(&project.repo, &["commit", "-m", "configure stackstead"])?;
        Ok(project)
    }

    pub(super) fn create(&self, name: &str) -> anyhow::Result<StacksteadManifest> {
        let assert = stackstead(&self.repo)
            .args(["--json", "create", name])
            .assert()
            .success();
        changed_manifest(&assert.get_output().stdout, "created")
    }

    pub(super) fn replace_config(&self, from: &str, to: &str) -> anyhow::Result<()> {
        let path = self.repo.join("stackstead.yaml");
        let original = fs::read_to_string(&path).test_context("read fixture config")?;
        assert!(
            original.contains(from),
            "fixture config does not contain {from:?}"
        );
        fs::write(path, original.replacen(from, to, 1)).test_context("update fixture config")?;
        git(&self.repo, &["add", "stackstead.yaml"])?;
        git(&self.repo, &["commit", "-m", "adjust stackstead fixture"])?;
        Ok(())
    }

    pub(super) fn write_config(
        &self,
        config: &StacksteadConfig,
        message: &str,
    ) -> anyhow::Result<()> {
        fs::write(
            self.repo.join("stackstead.yaml"),
            serde_yaml::to_string(config).test_context("serialize fixture config")?,
        )
        .test_context("write fixture config")?;
        git(&self.repo, &["add", "stackstead.yaml"])?;
        git(&self.repo, &["commit", "-m", message])?;
        Ok(())
    }

    pub(super) fn project_state_dir(&self) -> anyhow::Result<PathBuf> {
        Ok(self
            .repo
            .parent()
            .test_context("repository has a parent")?
            .join(".stacksteads/demo-project"))
    }
}

pub(super) fn test_state_home(cwd: &Path) -> PathBuf {
    let git_common_dir = ProcessCommand::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|path| PathBuf::from(path.trim()));
    git_common_dir
        .unwrap_or_else(|| cwd.join(".git"))
        .join("stackstead-test-state")
}

pub(super) fn stackstead(cwd: &Path) -> Command {
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("stackstead");
    command
        .current_dir(cwd)
        .env("XDG_STATE_HOME", test_state_home(cwd))
        .env_remove("RUST_LOG");
    command
}

pub(super) fn git(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = ProcessCommand::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .test_context("run Git fixture command")?;
    assert!(
        output.status.success(),
        "git {args:?} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).test_context("Git output is UTF-8")
}

pub(super) fn event_types(path: &Path) -> anyhow::Result<Vec<String>> {
    fs::read_to_string(path)
        .test_context("read event log")?
        .lines()
        .map(|line| {
            Ok(serde_json::from_str::<Value>(line)
                .test_context("parse event line")?
                .get("type")
                .and_then(Value::as_str)
                .test_context("event type")?
                .to_owned())
        })
        .collect()
}

pub(super) fn state_stackstead_directories(project: &Project) -> anyhow::Result<Vec<PathBuf>> {
    let state = project.project_state_dir()?;
    if !state.is_dir() {
        return Ok(vec![]);
    }
    let mut directories = Vec::new();
    for entry in fs::read_dir(state).test_context("read project state")? {
        let entry = entry.test_context("read project state entry")?;
        if entry.file_type().test_context("read entry type")?.is_dir() {
            directories.push(entry.path());
        }
    }
    Ok(directories)
}

pub(super) fn output_text(output: &[u8]) -> anyhow::Result<&str> {
    std::str::from_utf8(output).test_context("command output is UTF-8")
}

#[cfg(unix)]
pub(super) fn fake_docker_path(
    parent: &Path,
    directory: &str,
    script: &str,
) -> anyhow::Result<OsString> {
    use std::os::unix::fs::PermissionsExt;

    let fake_bin = parent.join(directory);
    fs::create_dir(&fake_bin).test_context("create fake Docker directory")?;
    let docker = fake_bin.join("docker");
    fs::write(&docker, script).test_context("write fake Docker")?;
    fs::set_permissions(&docker, fs::Permissions::from_mode(0o755))
        .test_context("make fake Docker executable")?;
    std::env::join_paths(std::iter::once(fake_bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .test_context("construct command-local PATH")
}

#[cfg(unix)]
pub(super) fn wait_for_file(path: &Path, attempts: usize, delay: std::time::Duration) -> bool {
    for _ in 0..attempts {
        if path.is_file() {
            return true;
        }
        std::thread::sleep(delay);
    }
    false
}
