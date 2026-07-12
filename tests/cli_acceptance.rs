use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use assert_cmd::Command;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::TempDir;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SourceOwnership {
    Stackstead,
    External,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ComponentStatus {
    Created,
    Ready,
    Running,
    Stopped,
    Reachable,
    Unreachable,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestStatus {
    source: ComponentStatus,
    dependencies: ComponentStatus,
    runtime: ComponentStatus,
    database: ComponentStatus,
    health: ComponentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StacksteadManifest {
    kind: String,
    version: String,
    stackstead_id: String,
    slug: String,
    short_id: String,
    runtime_token: String,
    project: String,
    branch: String,
    base: String,
    source_ownership: SourceOwnership,
    repo_root: PathBuf,
    project_state_root: PathBuf,
    stackstead_root: PathBuf,
    worktree: PathBuf,
    state_dir: PathBuf,
    port_lease_state_dir: Option<PathBuf>,
    compose_project: String,
    compose_files: Vec<PathBuf>,
    ports: std::collections::BTreeMap<String, u16>,
    container_ports: std::collections::BTreeMap<String, u16>,
    urls: std::collections::BTreeMap<String, String>,
    env_file: PathBuf,
    agent_context: PathBuf,
    pointer_file: PathBuf,
    event_log: PathBuf,
    env_keys: Vec<String>,
    status: ManifestStatus,
    database: Option<Value>,
    created_at: String,
    updated_at: String,
}

impl StacksteadManifest {
    fn read(path: &Path) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(&fs::read(path).expect("read manifest fixture"))
    }

    fn save_atomic(&self) -> std::io::Result<()> {
        fs::write(
            self.manifest_path(),
            serde_json::to_vec_pretty(self).unwrap(),
        )
    }

    fn manifest_path(&self) -> PathBuf {
        self.state_dir.join("manifest.json")
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StacksteadPointer {
    kind: String,
    version: String,
    stackstead_id: String,
    manifest: PathBuf,
    project: String,
    repo_root: PathBuf,
    project_state_root: PathBuf,
    stackstead_root: PathBuf,
}

type StacksteadConfig = serde_yaml::Value;

fn load_config(path: &Path) -> StacksteadConfig {
    serde_yaml::from_slice(&fs::read(path).expect("read config fixture"))
        .expect("parse config fixture")
}

fn append_event(path: &Path, event_type: &str, status: &str) {
    use std::io::Write;

    let event = serde_json::json!({
        "kind": "StacksteadEvent",
        "version": "1",
        "timestamp": "2026-01-01T00:00:00Z",
        "type": event_type,
        "status": status,
    });
    writeln!(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap(),
        "{event}"
    )
    .unwrap();
}

#[derive(Deserialize)]
struct StacksteadChange {
    kind: String,
    version: String,
    action: String,
    stackstead: StacksteadChangeDetails,
}

#[derive(Deserialize)]
struct StacksteadChangeDetails {
    files: StacksteadFiles,
}

#[derive(Deserialize)]
struct StacksteadFiles {
    manifest: PathBuf,
}

fn changed_manifest(output: &[u8], action: &str) -> StacksteadManifest {
    let change: StacksteadChange = serde_json::from_slice(output).expect("parse stackstead change");
    assert_eq!(change.kind, "StacksteadChange");
    assert_eq!(change.version, "1");
    assert_eq!(change.action, action);
    StacksteadManifest::read(&change.stackstead.files.manifest).expect("read changed manifest")
}

struct Project {
    _temp: TempDir,
    repo: PathBuf,
}

impl Project {
    fn git_repo() -> Self {
        let temp = tempfile::tempdir().expect("create temporary project parent");
        let repo = temp.path().join("demo-project");
        fs::create_dir(&repo).expect("create temporary repository");
        let repo = repo
            .canonicalize()
            .expect("canonicalize temporary repository");
        git(&repo, &["init", "--initial-branch=main"]);
        git(&repo, &["config", "user.name", "Stackstead Tests"]);
        git(
            &repo,
            &["config", "user.email", "stackstead-tests@example.invalid"],
        );
        fs::write(repo.join("README.md"), "# Demo project\n").expect("write README");
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
        .expect("write Compose fixture");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "initial fixture"]);
        Self { _temp: temp, repo }
    }

    fn initialized() -> Self {
        let project = Self::git_repo();
        stackstead(&project.repo).arg("init").assert().success();
        git(&project.repo, &["add", "stackstead.yaml"]);
        git(&project.repo, &["commit", "-m", "configure stackstead"]);
        project
    }

    fn create(&self, name: &str) -> StacksteadManifest {
        let assert = stackstead(&self.repo)
            .args(["--json", "create", name])
            .assert()
            .success();
        changed_manifest(&assert.get_output().stdout, "created")
    }

    fn replace_config(&self, from: &str, to: &str) {
        let path = self.repo.join("stackstead.yaml");
        let original = fs::read_to_string(&path).expect("read fixture config");
        assert!(
            original.contains(from),
            "fixture config does not contain {from:?}"
        );
        fs::write(path, original.replacen(from, to, 1)).expect("update fixture config");
        git(&self.repo, &["add", "stackstead.yaml"]);
        git(&self.repo, &["commit", "-m", "adjust stackstead fixture"]);
    }

    fn write_config(&self, config: &StacksteadConfig, message: &str) {
        fs::write(
            self.repo.join("stackstead.yaml"),
            serde_yaml::to_string(config).expect("serialize fixture config"),
        )
        .expect("write fixture config");
        git(&self.repo, &["add", "stackstead.yaml"]);
        git(&self.repo, &["commit", "-m", message]);
    }

    fn project_state_dir(&self) -> PathBuf {
        self.repo
            .parent()
            .expect("repository has a parent")
            .join(".stacksteads/demo-project")
    }
}

fn test_state_home(cwd: &Path) -> PathBuf {
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

fn stackstead(cwd: &Path) -> Command {
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("stackstead");
    command
        .current_dir(cwd)
        .env("XDG_STATE_HOME", test_state_home(cwd))
        .env_remove("RUST_LOG");
    command
}

#[cfg(unix)]
fn stackstead_without_runtime(cwd: &Path) -> Command {
    let path = fake_docker_path(
        cwd.parent().expect("command directory has a parent"),
        "no-runtime-fake-docker-bin",
        "#!/bin/sh\n[ \"$#\" -eq 4 ] && [ \"$1 $2 $3 $4\" = 'volume ls --format {{.Name}}' ]\n",
    );
    let mut command = stackstead(cwd);
    command.env("PATH", path);
    command
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = ProcessCommand::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run Git fixture command");
    assert!(
        output.status.success(),
        "git {args:?} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Git output is UTF-8")
}

fn event_types(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .expect("read event log")
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .expect("parse event line")
                .get("type")
                .and_then(Value::as_str)
                .expect("event type")
                .to_owned()
        })
        .collect()
}

fn append_destroy_tombstone(manifest: &StacksteadManifest) {
    append_event(&manifest.event_log, "destroy", "started");
    append_event(&manifest.event_log, "runtime_remove", "succeeded");
    append_event(&manifest.event_log, "source_remove", "started");
}

fn state_stackstead_directories(project: &Project) -> Vec<PathBuf> {
    let state = project.project_state_dir();
    if !state.is_dir() {
        return vec![];
    }
    fs::read_dir(state)
        .expect("read project state")
        .map(|entry| entry.expect("read project state entry"))
        .filter(|entry| entry.file_type().expect("read entry type").is_dir())
        .map(|entry| entry.path())
        .collect()
}

fn output_text(output: &[u8]) -> &str {
    std::str::from_utf8(output).expect("command output is UTF-8")
}

#[cfg(unix)]
fn fake_docker_path(parent: &Path, directory: &str, script: &str) -> OsString {
    use std::os::unix::fs::PermissionsExt;

    let fake_bin = parent.join(directory);
    fs::create_dir(&fake_bin).expect("create fake Docker directory");
    let docker = fake_bin.join("docker");
    fs::write(&docker, script).expect("write fake Docker");
    fs::set_permissions(&docker, fs::Permissions::from_mode(0o755))
        .expect("make fake Docker executable");
    std::env::join_paths(std::iter::once(fake_bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .expect("construct command-local PATH")
}

#[cfg(unix)]
fn wait_for_file(path: &Path, attempts: usize, delay: std::time::Duration) -> bool {
    for _ in 0..attempts {
        if path.is_file() {
            return true;
        }
        std::thread::sleep(delay);
    }
    false
}

#[test]
fn help_exposes_the_complete_command_surface() {
    let directory = tempfile::tempdir().expect("create command directory");
    let assert = stackstead(directory.path())
        .arg("--help")
        .assert()
        .success();
    let help = String::from_utf8_lossy(&assert.get_output().stdout);
    for command in [
        "init", "compose", "create", "adopt", "up", "run", "launch", "ps", "inspect", "env",
        "logs", "context", "open", "db", "stop", "destroy", "doctor", "repair",
    ] {
        assert!(help.contains(command), "top-level help omits {command:?}");
    }

    for args in [
        vec!["init", "--help"],
        vec!["compose", "plan", "--help"],
        vec!["compose", "apply", "--help"],
        vec!["create", "--help"],
        vec!["adopt", "--help"],
        vec!["up", "--help"],
        vec!["run", "--help"],
        vec!["launch", "--help"],
        vec!["ps", "--help"],
        vec!["inspect", "--help"],
        vec!["env", "--help"],
        vec!["logs", "--help"],
        vec!["context", "--help"],
        vec!["open", "--help"],
        vec!["db", "status", "--help"],
        vec!["stop", "--help"],
        vec!["destroy", "--help"],
        vec!["doctor", "--help"],
        vec!["repair", "--help"],
    ] {
        stackstead(directory.path()).args(args).assert().success();
    }

    stackstead(directory.path())
        .args(["env", "demo", "--show-secrets"])
        .assert()
        .failure();
}

#[test]
fn init_writes_a_valid_config_and_refuses_to_overwrite_it() {
    let project = Project::git_repo();
    let config_path = project.repo.join("stackstead.yaml");
    let assert = stackstead(&project.repo)
        .args(["init", "--json"])
        .assert()
        .success();
    let initialized: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(initialized["kind"], "StacksteadInit");
    assert_eq!(initialized["version"], "1");
    assert_eq!(initialized["path"], config_path.to_string_lossy().as_ref());

    let original = fs::read(&config_path).expect("read initialized config");
    let config = load_config(&config_path);
    assert_eq!(config["version"], "1");
    assert_eq!(config["kind"], "StacksteadProject");
    assert_eq!(config["project"]["name"], "demo-project");
    assert_eq!(config["source"]["base"], "main");

    let assert = stackstead(&project.repo).arg("init").assert().failure();
    assert!(String::from_utf8_lossy(&assert.get_output().stderr).contains("refusing to overwrite"));
    assert_eq!(
        fs::read(&config_path).expect("reread initialized config"),
        original
    );
}

#[test]
fn human_init_recommends_but_does_not_edit_repository_instructions() {
    let project = Project::git_repo();
    let instructions = project.repo.join("AGENTS.md");
    fs::write(&instructions, "# Human-owned policy\n").unwrap();

    let assert = stackstead(&project.repo).arg("init").assert().success();
    let stdout = output_text(&assert.get_output().stdout);
    for expected in [
        "review, add, and commit this policy",
        "lifecycle commands instead of bare Docker Compose",
        "stackstead --json create <name>",
        "stackstead up <full-id>",
        "stackstead run <full-id> -- <agent-or-command>",
        "use only the ports, URLs, and database it provides",
        "Reuse an environment only when the user or manager supplies its exact full ID",
        "Stackstead does not edit human-owned agent instructions",
    ] {
        assert!(
            stdout.contains(expected),
            "init output omitted {expected:?}"
        );
    }
    assert!(!stdout.contains("stackstead --json ps"));
    assert_eq!(
        fs::read_to_string(&instructions).unwrap(),
        "# Human-owned policy\n"
    );

    assert!(!project.repo.join("CLAUDE.md").exists());
}

#[test]
fn create_refuses_a_runtime_contract_missing_from_the_configured_base() {
    let project = Project::git_repo();
    stackstead(&project.repo).arg("init").assert().success();

    let assert = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    let stderr = output_text(&assert.get_output().stderr);
    assert!(
        stderr.contains("not present on source.base commit")
            && stderr.contains("commit or merge stackstead.yaml"),
        "unexpected error: {stderr}"
    );
    assert!(state_stackstead_directories(&project).is_empty());
    assert!(
        git(&project.repo, &["branch", "--list", "feature-a"])
            .trim()
            .is_empty()
    );
}

#[test]
fn create_refuses_locally_modified_contract_files_without_allocating_state() {
    let project = Project::initialized();
    let compose = project.repo.join("docker-compose.yml");
    let mut contents = fs::read_to_string(&compose).expect("read Compose fixture");
    contents.push_str("# uncommitted runtime change\n");
    fs::write(&compose, contents).expect("modify Compose fixture");

    let assert = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr).contains("differs from source.base commit"));
    assert!(state_stackstead_directories(&project).is_empty());
    assert!(
        git(&project.repo, &["branch", "--list", "feature-a"])
            .trim()
            .is_empty()
    );
}

#[test]
fn create_compares_clean_contract_files_through_git_filters() {
    let project = Project::initialized();
    fs::write(
        project.repo.join(".gitattributes"),
        "stackstead.yaml text eol=crlf\ndocker-compose.yml text eol=crlf\n",
    )
    .expect("write CRLF attributes");
    git(&project.repo, &["add", ".gitattributes"]);
    git(&project.repo, &["add", "--renormalize", "."]);
    git(&project.repo, &["commit", "-m", "configure CRLF contracts"]);
    for file in ["stackstead.yaml", "docker-compose.yml"] {
        let path = project.repo.join(file);
        fs::remove_file(&path).expect("remove LF contract fixture");
        git(&project.repo, &["checkout", "--", file]);
        assert!(
            fs::read(&path)
                .expect("read filtered contract")
                .windows(2)
                .any(|bytes| bytes == b"\r\n"),
            "Git did not apply the CRLF checkout filter to {file}"
        );
    }
    let status = git(
        &project.repo,
        &[
            "status",
            "--short",
            "--",
            "stackstead.yaml",
            "docker-compose.yml",
        ],
    );
    assert!(
        status.trim().is_empty(),
        "Git did not consider the filtered contract clean: {status:?}"
    );
    let manifest = project.create("feature-a");
    assert!(manifest.worktree.is_dir());
}

#[test]
fn create_pins_the_configured_base_when_called_from_another_branch() {
    let project = Project::initialized();
    git(&project.repo, &["switch", "-c", "caller-branch"]);
    fs::write(project.repo.join("README.md"), "caller-only change\n")
        .expect("write caller branch change");
    git(&project.repo, &["add", "README.md"]);
    git(&project.repo, &["commit", "-m", "caller-only change"]);
    let main = git(&project.repo, &["rev-parse", "main"]);
    let caller = git(&project.repo, &["rev-parse", "caller-branch"]);

    let manifest = project.create("feature-a");
    assert_eq!(manifest.base, main.trim());
    assert_ne!(manifest.base, caller.trim());
    assert_eq!(
        fs::read_to_string(manifest.worktree.join("README.md")).expect("read created README"),
        "# Demo project\n"
    );
}

#[cfg(unix)]
#[test]
fn recreating_an_existing_branch_rejects_a_base_it_does_not_contain() {
    let project = Project::initialized();
    let first = project.create("feature-a");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "base-fake-docker-bin",
        "#!/bin/sh\nexit 0\n",
    );
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", &first.stackstead_id, "--yes"])
        .assert()
        .success();

    fs::write(project.repo.join("README.md"), "# Advanced base\n").expect("advance base file");
    git(&project.repo, &["add", "README.md"]);
    git(&project.repo, &["commit", "-m", "advance configured base"]);
    let assert = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr).contains("does not contain pinned source.base")
    );
    assert!(state_stackstead_directories(&project).is_empty());
}

#[test]
fn normalized_compose_paths_survive_create_and_resolution() {
    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["runtime"]["files"] = serde_yaml::Value::Sequence(vec!["./docker-compose.yml".into()]);
    project.write_config(&config, "use an explicitly relative Compose path");

    let manifest = project.create("feature-a");
    assert_eq!(
        manifest.compose_files,
        [manifest.worktree.join("docker-compose.yml")]
    );
    stackstead(&project.repo)
        .args(["context", "feature-a", "--json"])
        .assert()
        .success();
}

#[test]
fn host_wide_port_leases_keep_stopped_projects_on_disjoint_ports() {
    let registry = tempfile::tempdir().unwrap();
    let first_project = Project::initialized();
    let second_project = Project::initialized();
    let mut second_config = load_config(&second_project.repo.join("stackstead.yaml"));
    second_config["project"]["name"] = "second-project".into();
    second_project.write_config(&second_config, "use a distinct project identity");
    let create = |project: &Project, name: &str| {
        let created = stackstead(&project.repo)
            .env("XDG_STATE_HOME", registry.path())
            .args(["--json", "create", name])
            .assert()
            .success();
        changed_manifest(&created.get_output().stdout, "created")
    };

    let first = create(&first_project, "first");
    let second = create(&second_project, "second");
    let first_ports = first.ports.values().copied().collect::<BTreeSet<_>>();
    let second_ports = second.ports.values().copied().collect::<BTreeSet<_>>();
    assert!(first_ports.is_disjoint(&second_ports));
    assert_eq!(first_ports.len(), 2);
    assert_eq!(second_ports.len(), 2);
    assert_eq!(
        first_ports.iter().next_back().unwrap() - first_ports.iter().next().unwrap(),
        1
    );
    assert_eq!(
        second_ports.iter().next_back().unwrap() - second_ports.iter().next().unwrap(),
        1
    );

    #[cfg(unix)]
    {
        let path = fake_docker_path(
            first_project.repo.parent().unwrap(),
            "lease-release-fake-bin",
            "#!/bin/sh\ncase \"$1 $2\" in 'container ls'|'network ls'|'volume ls') exit 0;; esac\nexit 97\n",
        );
        stackstead(&first_project.repo)
            .env("XDG_STATE_HOME", registry.path())
            .env("PATH", path)
            .args(["destroy", &first.stackstead_id, "--yes"])
            .assert()
            .success();
        let registry: Value = serde_json::from_slice(
            &fs::read(registry.path().join("stackstead/port-leases.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            registry["leases"]
                .as_array()
                .unwrap()
                .iter()
                .map(|lease| lease["port"].as_u64().unwrap() as u16)
                .collect::<BTreeSet<_>>(),
            second_ports
        );
    }
}

#[cfg(unix)]
#[test]
fn lifecycle_commands_reject_a_port_lease_that_no_longer_belongs_to_the_manifest() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let git_common_dir = PathBuf::from(
        git(
            &project.repo,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .trim(),
    );
    let registry_path = git_common_dir.join("stackstead-test-state/stackstead/port-leases.json");
    let mut registry: Value =
        serde_json::from_slice(&fs::read(&registry_path).expect("read port lease registry"))
            .expect("parse port lease registry");
    for lease in registry["leases"].as_array_mut().unwrap() {
        lease["owner"] = "ffffffffffffffffffffffffffffffff".into();
    }
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).unwrap(),
    )
    .expect("replace port lease owner");

    let marker = project.repo.parent().unwrap().join("lease-docker-ran");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "lease-mismatch-fake-bin",
        &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
    );
    for args in [
        vec!["up", &manifest.stackstead_id],
        vec!["stop", &manifest.stackstead_id],
        vec!["db", "status", &manifest.stackstead_id],
        vec!["run", &manifest.stackstead_id, "--", "true"],
        vec!["repair", &manifest.stackstead_id],
        vec!["destroy", &manifest.stackstead_id, "--yes"],
    ] {
        let rejected = stackstead(&project.repo)
            .env("PATH", &path)
            .args(args)
            .assert()
            .failure();
        assert!(
            output_text(&rejected.get_output().stderr).contains("port leases for owner"),
            "unexpected error: {}",
            output_text(&rejected.get_output().stderr)
        );
    }
    assert!(
        !marker.exists(),
        "Docker ran before lease ownership validation"
    );
    assert!(manifest.manifest_path().is_file());
    assert!(manifest.worktree.is_dir());
}

#[cfg(target_os = "linux")]
#[test]
fn open_refuses_a_stopped_runtime_before_invoking_the_browser() {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let marker = project.repo.parent().unwrap().join("browser-opened");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "stale-open-fake-bin",
        "#!/bin/sh\ncase \"$1 $2\" in 'container ls'|'network ls'|'volume ls') exit 0;; esac\nexit 97\n",
    );
    let fake_bin = std::env::split_paths(&path).next().unwrap();
    let opener = fake_bin.join("xdg-open");
    fs::write(
        &opener,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&opener, fs::Permissions::from_mode(0o755)).unwrap();

    let rejected = stackstead(&project.repo)
        .env("PATH", path)
        .args(["open", &manifest.stackstead_id, "web"])
        .assert()
        .failure();
    assert!(
        output_text(&rejected.get_output().stderr).contains("has no Stackstead ownership claim")
    );
    assert!(
        !marker.exists(),
        "browser launched for an unrelated listener"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn open_launches_only_after_owned_service_publication_is_proven() {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let marker = project.repo.parent().unwrap().join("owned-browser-url");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "owned-open-fake-bin",
        r#"#!/bin/sh
case "$1 $2" in
  "container ls"|"network ls") exit 0 ;;
  "volume ls") printf '%s\n' "$COMPOSE_PROJECT_NAME-stackstead-claim"; exit 0 ;;
  "volume inspect") printf '{"io.stackstead.runtime-token":"%s"}\n' "$EXPECTED_TOKEN"; exit 0 ;;
esac
for argument in "$@"; do
  case "$argument" in
    ps) printf 'owned-container\n'; exit 0 ;;
    port) printf '127.0.0.1:%s\n' "$EXPECTED_PORT"; exit 0 ;;
  esac
done
exit 97
"#,
    );
    let fake_bin = std::env::split_paths(&path).next().unwrap();
    let opener = fake_bin.join("xdg-open");
    fs::write(
        &opener,
        format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&opener, fs::Permissions::from_mode(0o755)).unwrap();

    stackstead(&project.repo)
        .env("PATH", path)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .env("EXPECTED_PORT", manifest.ports["web"].to_string())
        .args(["open", &manifest.stackstead_id, "web"])
        .assert()
        .success();
    assert!(wait_for_file(
        &marker,
        100,
        std::time::Duration::from_millis(10)
    ));
    assert_eq!(fs::read_to_string(marker).unwrap(), manifest.urls["web"]);
}

#[test]
fn compose_discovery_generates_config_and_rewrites_only_after_confirmation() {
    let project = Project::git_repo();
    let compose = project.repo.join("docker-compose.yml");
    fs::write(
        &compose,
        r#"services:
  web:
    image: nginx:alpine
    ports:
      - "3000:80"
  postgres:
    image: postgres:16-alpine
    ports:
      - "5432:5432"
"#,
    )
    .expect("write fixed-port Compose fixture");
    git(&project.repo, &["add", "docker-compose.yml"]);
    git(&project.repo, &["commit", "-m", "use fixed fixture ports"]);

    stackstead(&project.repo).arg("init").assert().success();
    let config = load_config(&project.repo.join("stackstead.yaml"));
    assert_eq!(
        config["resources"]["ports"]["expose"]["web"]["container"],
        80
    );
    assert_eq!(
        config["resources"]["ports"]["expose"]["postgres"]["container"],
        5432
    );
    assert_eq!(config["health"]["checks"].as_sequence().unwrap().len(), 1);
    assert_eq!(config["health"]["checks"][0]["name"], "web");

    let plan = stackstead(&project.repo)
        .args(["--json", "compose", "plan"])
        .assert()
        .success();
    let plan: Value =
        serde_json::from_slice(&plan.get_output().stdout).expect("parse Compose plan");
    assert_eq!(plan["kind"], "ComposePlan");
    assert_eq!(plan["version"], "1");
    assert_eq!(plan["file"], "docker-compose.yml");
    let ports = plan["ports"].as_array().expect("Compose plan ports");
    assert_eq!(
        ports
            .iter()
            .find(|port| port["name"] == "web")
            .expect("web plan")["current_host_port"],
        3000
    );
    assert_eq!(
        ports
            .iter()
            .find(|port| port["name"] == "postgres")
            .expect("Postgres plan")["current_host_port"],
        5432
    );

    let original = fs::read(&compose).expect("read original Compose fixture");
    stackstead(&project.repo)
        .args(["compose", "apply"])
        .assert()
        .failure();
    assert_eq!(
        fs::read(&compose).expect("reread Compose fixture"),
        original
    );

    stackstead(&project.repo)
        .args(["compose", "apply", "--yes"])
        .assert()
        .success();
    let rewritten = fs::read_to_string(&compose).expect("read rewritten Compose fixture");
    assert!(rewritten.contains("127.0.0.1:${WEB_PORT}:80"));
    assert!(rewritten.contains("127.0.0.1:${POSTGRES_PORT}:5432"));
}

#[test]
fn explicit_nested_compose_file_drives_init_plan_and_apply() {
    let project = Project::git_repo();
    git(&project.repo, &["rm", "docker-compose.yml"]);
    let nested = project.repo.join("infra/docker/compose.yml");
    fs::create_dir_all(nested.parent().unwrap()).expect("create nested Compose directory");
    fs::write(
        &nested,
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"3000:80\"\n",
    )
    .expect("write nested Compose file");
    git(&project.repo, &["add", "infra/docker/compose.yml"]);
    git(&project.repo, &["commit", "-m", "add nested Compose file"]);

    let missing = stackstead(&project.repo).arg("init").assert().failure();
    let error = output_text(&missing.get_output().stderr);
    assert!(error.contains("--compose-file"));
    assert!(error.contains("infra/docker/compose.yml"));

    stackstead(&project.repo)
        .args(["init", "--compose-file", "infra/docker/compose.yml"])
        .assert()
        .success();
    let config = load_config(&project.repo.join("stackstead.yaml"));
    assert_eq!(
        config["runtime"]["files"],
        serde_yaml::Value::Sequence(vec!["infra/docker/compose.yml".into()])
    );

    let plan = stackstead(&project.repo)
        .args(["compose", "plan", "--json"])
        .assert()
        .success();
    let plan: Value = serde_json::from_slice(&plan.get_output().stdout).expect("parse plan");
    assert_eq!(plan["file"], "infra/docker/compose.yml");

    let second = project.repo.join("infra/docker/admin-compose.yml");
    fs::write(
        &second,
        "services:\n  admin:\n    image: nginx:alpine\n    ports:\n      - \"4000:81\"\n",
    )
    .expect("write second Compose file");
    let second_before = fs::read(&second).unwrap();
    let mut config = config;
    config["runtime"]["files"]
        .as_sequence_mut()
        .unwrap()
        .push("infra/docker/admin-compose.yml".into());
    fs::write(
        project.repo.join("stackstead.yaml"),
        serde_yaml::to_string(&config).unwrap(),
    )
    .unwrap();

    stackstead(&project.repo)
        .args(["compose", "plan"])
        .assert()
        .failure();
    let explicit = stackstead(&project.repo)
        .args([
            "compose",
            "plan",
            "--compose-file",
            "infra/docker/compose.yml",
            "--json",
        ])
        .assert()
        .success();
    let explicit: Value = serde_json::from_slice(&explicit.get_output().stdout).unwrap();
    assert_eq!(explicit["file"], "infra/docker/compose.yml");

    stackstead(&project.repo)
        .args([
            "compose",
            "apply",
            "--compose-file",
            "infra/docker/compose.yml",
            "--yes",
        ])
        .assert()
        .success();
    assert!(
        fs::read_to_string(&nested)
            .expect("read rewritten nested Compose file")
            .contains("127.0.0.1:${WEB_PORT}:80")
    );
    assert_eq!(fs::read(&second).unwrap(), second_before);
}

#[test]
fn multi_file_config_keeps_the_conventional_root_plan_fallback() {
    let project = Project::initialized();
    fs::write(
        project.repo.join("compose.override.yml"),
        "services:\n  web:\n    environment:\n      TRIAL: yes\n",
    )
    .unwrap();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["runtime"]["files"]
        .as_sequence_mut()
        .unwrap()
        .push("compose.override.yml".into());
    fs::write(
        project.repo.join("stackstead.yaml"),
        serde_yaml::to_string(&config).unwrap(),
    )
    .unwrap();

    let plan = stackstead(&project.repo)
        .args(["compose", "plan", "--json"])
        .assert()
        .success();
    let plan: Value = serde_json::from_slice(&plan.get_output().stdout).unwrap();
    assert_eq!(plan["file"], "docker-compose.yml");
}

#[cfg(unix)]
#[test]
fn up_rejects_every_structurally_unsafe_or_disconnected_port_contract() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "port-fake-docker-bin",
        "#!/bin/sh\necho docker-must-not-run >&2\nexit 97\n",
    );

    for (mapping, expected) in [
        ("3000", "unsupported"),
        ("\"80\"", "no deterministic host binding"),
        ("\"3000-3002:80-82\"", "unsupported"),
        ("\"3000:80\"", "fixed host port"),
        (
            "\"127.0.0.1:${APP_PORT}:80\"",
            "env.generate does not define `APP_PORT`",
        ),
    ] {
        fs::write(
            &manifest.compose_files[0],
            format!(
                "services:\n  web:\n    image: nginx\n    ports: [{mapping}]\n  postgres:\n    image: postgres:16\n    ports: [\"127.0.0.1:${{POSTGRES_PORT}}:5432\"]\n"
            ),
        )
        .expect("write unsafe Compose fixture");
        let assert = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["up", &manifest.stackstead_id])
            .assert()
            .failure();
        let stderr = output_text(&assert.get_output().stderr);
        assert!(
            stderr.contains(expected),
            "unexpected error for {mapping}: {stderr}"
        );
        assert!(!stderr.contains("docker-must-not-run"));
    }

    fs::write(
        &manifest.compose_files[0],
        "services:\n  web:\n    ports: [\"127.0.0.1:${WEB_PORT}:80\"]\n  postgres:\n    ports: [\"127.0.0.1:${POSTGRES_PORT}:5432\"]\n",
    )
    .unwrap();
    project.replace_config(
        "    WEB_PORT: '{{ ports.web }}'\n",
        "    WEB_PORT: '39000'\n",
    );
    let literal = stackstead(&project.repo)
        .env("PATH", &path)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    assert!(output_text(&literal.get_output().stderr).contains("ports.<name>"));
    assert!(!output_text(&literal.get_output().stderr).contains("docker-must-not-run"));
}

#[cfg(unix)]
#[test]
fn run_pins_stackstead_identity_and_preserves_the_child_exit_code() {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let script = project.repo.parent().unwrap().join("agent-probe");
    fs::write(
        &script,
        r#"#!/bin/sh
test "$PWD" = "$1" || exit 91
test "$STACKSTEAD_ID" = "$2" || exit 92
test "$STACKSTEAD_COMPOSE_PROJECT" = "$3" || exit 93
test "$COMPOSE_PROJECT_NAME" = "$3" || exit 94
test "$STACKSTEAD_WORKTREE" = "$1" || exit 95
printf '%s|%s\n' "$STACKSTEAD_ID" "$4"
exit 23
"#,
    )
    .expect("write agent probe");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
        .expect("make agent probe executable");

    let assert = stackstead(&project.repo)
        .env("STACKSTEAD_ID", "spoofed")
        .env("COMPOSE_PROJECT_NAME", "shared")
        .arg("run")
        .arg("feature-a")
        .arg("--")
        .arg(&script)
        .arg(&manifest.worktree)
        .arg(&manifest.stackstead_id)
        .arg(&manifest.compose_project)
        .arg("argument with spaces")
        .assert()
        .code(23);
    assert_eq!(
        output_text(&assert.get_output().stdout),
        format!("{}|argument with spaces\n", manifest.stackstead_id)
    );

    let json_run = stackstead(&project.repo)
        .args(["--json", "run", "feature-a", "--", "true"])
        .assert()
        .failure();
    assert!(
        output_text(&json_run.get_output().stderr).contains("--json cannot be combined with run")
    );
}

#[cfg(unix)]
#[test]
fn launch_creates_starts_and_runs_with_the_full_stackstead_identity() {
    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["database"]["postgres"] = serde_yaml::Value::Null;
    config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
    project.write_config(&config, "configure launch fixture");
    let fake_state = project.repo.parent().unwrap().join("launch-docker-state");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "launch-fake-docker-bin",
        r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$1 $2" in
  "volume ls") test ! -f "$FAKE_STATE/claim" || printf '%s\n' "$claim" ;;
  "volume create")
    for argument in "$@"; do
      case "$argument" in
        io.stackstead.runtime-token=*) printf '%s' "${argument#*=}" > "$FAKE_STATE/token" ;;
      esac
    done
    : > "$FAKE_STATE/claim"
    ;;
  "volume inspect") printf '{"io.stackstead.runtime-token":"%s"}\n' "$(cat "$FAKE_STATE/token")" ;;
esac
exit 0
"#,
    );

    let launched = stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", fake_state)
        .args([
            "launch",
            "feature-a",
            "--",
            "sh",
            "-c",
            "printf 'child:%s|%s\\n' \"$STACKSTEAD_ID\" \"$PWD\"; exit 23",
        ])
        .assert()
        .code(23);

    let directories = state_stackstead_directories(&project);
    assert_eq!(directories.len(), 1);
    let manifest = StacksteadManifest::read(&directories[0].join("state/manifest.json")).unwrap();
    assert_eq!(manifest.status.runtime, ComponentStatus::Running);
    let stdout = output_text(&launched.get_output().stdout);
    assert!(stdout.contains(&format!("Created {}", manifest.stackstead_id)));
    assert!(stdout.contains("Timings:"));
    assert!(stdout.contains(&format!(
        "child:{}|{}",
        manifest.stackstead_id,
        manifest.worktree.display()
    )));
}

#[cfg(unix)]
#[test]
fn active_launch_blocks_destroy_without_blocking_another_run() {
    use std::time::Duration;

    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["database"]["postgres"] = serde_yaml::Value::Null;
    config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
    project.write_config(&config, "configure launch lease fixture");
    let fake_state = project
        .repo
        .parent()
        .unwrap()
        .join("launch-lease-docker-state");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "launch-lease-fake-docker-bin",
        r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$1 $2" in
  "volume ls") test ! -f "$FAKE_STATE/claim" || printf '%s\n' "$claim" ;;
  "volume create")
    for argument in "$@"; do
      case "$argument" in
        io.stackstead.runtime-token=*) printf '%s' "${argument#*=}" > "$FAKE_STATE/token" ;;
      esac
    done
    : > "$FAKE_STATE/claim"
    ;;
  "volume inspect") printf '{"io.stackstead.runtime-token":"%s"}\n' "$(cat "$FAKE_STATE/token")" ;;
esac
exit 0
"#,
    );
    let ready = project.repo.parent().unwrap().join("launch-agent-ready");
    let mut launch = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .env("PATH", path)
        .env("FAKE_STATE", fake_state)
        .args(["launch", "feature-a", "--", "sh", "-c"])
        .arg("touch \"$1\"; while test -e \"$1\"; do sleep 0.05; done")
        .arg("stackstead-launch-lease")
        .arg(&ready)
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("start launch command");
    assert!(
        wait_for_file(&ready, 100, Duration::from_millis(20)),
        "launched child did not start"
    );

    let directories = state_stackstead_directories(&project);
    assert_eq!(directories.len(), 1);
    let manifest = StacksteadManifest::read(&directories[0].join("state/manifest.json")).unwrap();
    stackstead(&project.repo)
        .args(["run", &manifest.stackstead_id, "--", "true"])
        .assert()
        .success();
    let blocked = stackstead(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .failure();
    assert!(output_text(&blocked.get_output().stderr).contains("active stackstead agent"));
    assert!(manifest.worktree.is_dir());

    fs::remove_file(&ready).expect("release launched child");
    assert!(launch.wait().expect("wait for launch command").success());
}

#[test]
fn launch_preserves_the_created_cell_when_up_fails() {
    let project = Project::initialized();
    project.replace_config(
        "    command: ''\n    shell: false\n",
        "    command: stackstead-launch-dependency-that-does-not-exist\n    shell: false\n",
    );
    let child_marker = project.repo.parent().unwrap().join("launch-child-ran");

    let rejected = stackstead(&project.repo)
        .arg("launch")
        .arg("feature-a")
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg(format!("touch '{}'", child_marker.display()))
        .assert()
        .failure();

    let directories = state_stackstead_directories(&project);
    assert_eq!(directories.len(), 1);
    let manifest = StacksteadManifest::read(&directories[0].join("state/manifest.json")).unwrap();
    assert_eq!(manifest.status.dependencies, ComponentStatus::Failed);
    assert!(
        output_text(&rejected.get_output().stdout)
            .contains(&format!("Created {}", manifest.stackstead_id))
    );
    assert!(!child_marker.exists());
}

#[test]
fn launch_refuses_to_reuse_an_existing_cell() {
    let project = Project::initialized();
    let existing = project.create("feature-a");
    let child_marker = project
        .repo
        .parent()
        .unwrap()
        .join("duplicate-launch-child-ran");

    let rejected = stackstead(&project.repo)
        .arg("launch")
        .arg("feature-a")
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg(format!("touch '{}'", child_marker.display()))
        .assert()
        .failure();

    assert!(output_text(&rejected.get_output().stderr).contains("already exists"));
    assert_eq!(state_stackstead_directories(&project).len(), 1);
    assert!(existing.manifest_path().is_file());
    assert!(!child_marker.exists());
}

#[test]
fn launch_rejects_json_before_creating_state() {
    let project = Project::initialized();

    let rejected = stackstead(&project.repo)
        .args(["--json", "launch", "feature-a", "--", "true"])
        .assert()
        .failure();

    assert!(
        output_text(&rejected.get_output().stderr)
            .contains("--json cannot be combined with launch")
    );
    assert!(state_stackstead_directories(&project).is_empty());
}

#[test]
fn create_rejects_a_slug_that_matches_an_existing_full_id() {
    let project = Project::initialized();
    let existing = project.create("feature-a");
    let before = state_stackstead_directories(&project);

    let assert = stackstead(&project.repo)
        .args(["create", &existing.stackstead_id, "--json"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr).contains("already exists"));
    assert_eq!(state_stackstead_directories(&project), before);
}

#[test]
fn generated_environment_cannot_add_process_control_keys() {
    use std::io::Write;

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(&manifest.env_file)
            .unwrap(),
        "PATH=/attacker/bin"
    )
    .unwrap();
    let rejected = stackstead(&project.repo)
        .args(["run", "feature-a", "--", "true"])
        .assert()
        .failure();
    assert!(output_text(&rejected.get_output().stderr).contains("do not match the manifest"));
}

#[cfg(unix)]
#[test]
fn active_agent_run_blocks_destroy_until_the_child_exits() {
    use std::time::Duration;

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let ready = project.repo.parent().unwrap().join("agent-run-ready");
    let mut child = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .args(["run", "feature-a", "--", "sh", "-c"])
        .arg("touch \"$1\"; while test -e \"$1\"; do sleep 0.05; done")
        .arg("stackstead-agent-lease")
        .arg(&ready)
        .spawn()
        .expect("start leased agent command");
    assert!(
        wait_for_file(&ready, 100, Duration::from_millis(20)),
        "agent child did not start"
    );

    for args in [
        vec!["up", "feature-a"],
        vec!["stop", "feature-a"],
        vec!["repair", "feature-a"],
        vec!["destroy", "feature-a", "--yes"],
    ] {
        let blocked = stackstead(&project.repo).args(args).assert().failure();
        assert!(output_text(&blocked.get_output().stderr).contains("active stackstead agent"));
    }
    assert!(manifest.manifest_path().is_file());
    assert!(manifest.worktree.is_dir());

    fs::remove_file(&ready).expect("release agent probe");
    assert!(child.wait().expect("wait for agent command").success());
}

#[cfg(target_os = "linux")]
#[test]
fn normal_agent_completion_terminates_background_descendants() {
    use std::{thread, time::Duration};

    let project = Project::initialized();
    project.create("feature-a");
    let pid_file = project.repo.parent().unwrap().join("background-agent.pid");
    stackstead(&project.repo)
        .args(["run", "feature-a", "--", "sh", "-c"])
        .arg("sleep 30 & echo $! > \"$1\"")
        .arg("stackstead-background-agent")
        .arg(&pid_file)
        .assert()
        .success();
    let pid = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    for _ in 0..100 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("background agent descendant {pid} survived normal wrapper completion");
}

#[cfg(unix)]
#[test]
fn agent_child_keeps_the_destroy_lease_after_the_cli_is_killed() {
    use std::{os::unix::fs::PermissionsExt, thread, time::Duration};

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let parent = project.repo.parent().unwrap();
    let pid_file = parent.join("orphaned-agent.pid");
    let script = parent.join("orphaned-agent");
    fs::write(&script, "#!/bin/sh\necho $$ > \"$1\"\nexec sleep 30\n").expect("write agent script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
        .expect("make agent script executable");
    let mut wrapper = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .args(["run", "feature-a", "--"])
        .arg(&script)
        .arg(&pid_file)
        .spawn()
        .expect("start stackstead wrapper");
    assert!(wait_for_file(&pid_file, 100, Duration::from_millis(20)));
    let agent_pid = fs::read_to_string(&pid_file)
        .expect("agent wrote PID")
        .trim()
        .parse::<i32>()
        .expect("parse agent PID");
    assert_eq!(unsafe { libc::kill(wrapper.id() as i32, libc::SIGKILL) }, 0);
    wrapper.wait().expect("reap killed wrapper");

    let blocked = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--yes"])
        .assert()
        .failure();
    assert!(output_text(&blocked.get_output().stderr).contains("active stackstead agent"));
    assert_eq!(unsafe { libc::kill(agent_pid, libc::SIGKILL) }, 0);

    let path = fake_docker_path(parent, "orphan-lease-fake-bin", "#!/bin/sh\nexit 0\n");
    for _ in 0..100 {
        let output = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["destroy", "feature-a", "--yes"])
            .output()
            .expect("retry destroy");
        if output.status.success() {
            assert!(!manifest.stackstead_root.exists());
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("destroy lease did not release after the orphaned agent exited");
}

#[cfg(unix)]
#[test]
fn missing_lock_contract_is_rejected_without_recreation() {
    let project = Project::initialized();
    let run_cell = project.create("run-legacy");
    fs::remove_file(run_cell.state_dir.join("lock")).expect("remove legacy mutation lock");
    fs::remove_file(run_cell.state_dir.join("run.lock")).expect("remove legacy run lock");
    let diagnosed = stackstead(&project.repo)
        .args(["doctor", "--json", "--fail-on-error"])
        .assert()
        .code(1);
    let report: Value = serde_json::from_slice(&diagnosed.get_output().stdout).unwrap();
    let codes = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["code"].as_str())
        .collect::<BTreeSet<_>>();
    assert!(codes.contains("lock.stackstead.missing"));
    assert!(codes.contains("lock.run.missing"));
    stackstead(&project.repo)
        .args(["run", "run-legacy", "--", "true"])
        .assert()
        .failure();
    assert!(!run_cell.state_dir.join("lock").exists());
    assert!(!run_cell.state_dir.join("run.lock").exists());

    let destroy_cell = project.create("destroy-legacy");
    fs::remove_file(destroy_cell.state_dir.join("lock")).expect("remove legacy mutation lock");
    fs::remove_file(destroy_cell.state_dir.join("run.lock")).expect("remove legacy run lock");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "legacy-lock-fake-bin",
        "#!/bin/sh\nexit 0\n",
    );
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", "destroy-legacy", "--yes"])
        .assert()
        .failure();
    assert!(destroy_cell.stackstead_root.exists());
    assert!(!destroy_cell.state_dir.join("lock").exists());
    assert!(!destroy_cell.state_dir.join("run.lock").exists());
}

#[cfg(unix)]
#[test]
fn changed_worktree_branch_is_reported_and_rejected_before_agent_or_teardown() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    git(&manifest.worktree, &["switch", "-c", "unexpected-source"]);

    let inspected = stackstead(&project.repo)
        .args(["--json", "inspect", "feature-a"])
        .assert()
        .success();
    let inspected: Value =
        serde_json::from_slice(&inspected.get_output().stdout).expect("parse inspect output");
    assert!(inspected["warnings"].as_array().is_some_and(|warnings| {
        warnings.iter().any(|warning| {
            warning
                .as_str()
                .is_some_and(|warning| warning.contains("unexpected-source"))
        })
    }));

    for args in [
        vec!["run", "feature-a", "--", "true"],
        vec!["up", "feature-a"],
        vec!["stop", "feature-a"],
        vec!["repair", "feature-a"],
        vec!["destroy", "feature-a", "--yes"],
    ] {
        let assert = stackstead(&project.repo).args(args).assert().failure();
        let stderr = output_text(&assert.get_output().stderr);
        assert!(
            stderr.contains("unexpected-source")
                && stderr.contains("refusing to use the wrong source"),
            "unexpected source-binding error: {stderr}"
        );
    }
    assert!(manifest.manifest_path().is_file());
    assert!(manifest.worktree.is_dir());
}

#[test]
fn all_resolved_commands_reject_redirected_manifest_contract_fields() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let secret = project.repo.parent().unwrap().join("must-not-read.env");
    fs::write(&secret, "PRIVATE_VALUE=must-not-leak\n").expect("write outside env fixture");

    let mut tampered = manifest.clone();
    tampered.env_file = secret;
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&tampered).expect("serialize redirected manifest"),
    )
    .expect("write redirected manifest");
    let assert = stackstead(&project.repo)
        .args(["env", "feature-a", "--print", "--show-secrets"])
        .assert()
        .failure();
    assert!(!output_text(&assert.get_output().stdout).contains("must-not-leak"));
    assert!(output_text(&assert.get_output().stderr).contains("escapes worktree"));

    tampered = manifest.clone();
    tampered.compose_project = "unrelated-valid-project".into();
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&tampered).expect("serialize redirected Compose identity"),
    )
    .expect("write redirected Compose identity");
    let assert = stackstead(&project.repo)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)
            .contains("manifest Compose project does not match the durable stackstead identity")
    );

    tampered = manifest.clone();
    tampered.short_id = "ffffffffffffffffffffffffffffffff".into();
    tampered.compose_project = format!("{}-feature-a-{}", tampered.project, tampered.short_id);
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&tampered).expect("serialize forged redundant identity"),
    )
    .expect("write forged redundant identity");
    let assert = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--yes"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)
            .contains("manifest stackstead ID does not match its slug and short ID")
    );
}

#[test]
fn adopted_manifests_cannot_cross_bind_or_delete_another_checkout() {
    let project = Project::initialized();
    let parent = project.repo.parent().unwrap();
    let first_path = parent.join("manager-first");
    let second_path = parent.join("manager-second");
    for (branch, path) in [
        ("manager-first", &first_path),
        ("manager-second", &second_path),
    ] {
        git(
            &project.repo,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                path.to_str().expect("UTF-8 fixture path"),
                "main",
            ],
        );
    }
    let adopt = |name: &str, path: &Path| {
        let assert = stackstead(&project.repo)
            .arg("--json")
            .arg("adopt")
            .arg(name)
            .arg("--worktree")
            .arg(path)
            .assert()
            .success();
        changed_manifest(&assert.get_output().stdout, "adopted")
    };
    let first = adopt("manager-first", &first_path);
    let second = adopt("manager-second", &second_path);
    let mut redirected = first.clone();
    redirected.worktree = second.worktree.clone();
    redirected.branch = second.branch.clone();
    redirected.compose_files = second.compose_files.clone();
    redirected.env_file = second.env_file.clone();
    redirected.agent_context = second.agent_context.clone();
    redirected.pointer_file = second.pointer_file.clone();
    redirected
        .save_atomic()
        .expect("redirect first manifest to second checkout");

    for args in [
        vec!["run", &first.stackstead_id, "--", "true"],
        vec!["repair", &first.stackstead_id],
        vec!["destroy", &first.stackstead_id, "--yes"],
    ] {
        let assert = stackstead(&project.repo).args(args).assert().failure();
        assert!(
            output_text(&assert.get_output().stderr).contains("reciprocal pointer"),
            "unexpected cross-binding failure: {}",
            output_text(&assert.get_output().stderr)
        );
    }
    assert!(second.pointer_file.is_file());
    assert!(second.manifest_path().is_file());
    assert!(second.worktree.is_dir());
    assert!(first_path.join(".stackstead/stackstead.json").is_file());
}

#[cfg(unix)]
#[test]
fn post_create_holds_the_cell_lock_after_manifest_publication() {
    use std::time::Duration;

    let project = Project::initialized();
    let ready = project.repo.parent().unwrap().join("post-create-ready");
    let release = project.repo.parent().unwrap().join("post-create-release");
    fs::write(&release, "wait\n").expect("create hook release gate");
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["hooks"]["post_create"] = serde_yaml::to_value([serde_json::json!({
        "command": format!(
            "touch '{}'; while test -e '{}'; do sleep 0.02; done",
            ready.display(),
            release.display()
        ),
        "shell": true,
    })])
    .unwrap();
    project.write_config(&config, "add blocking post-create hook");

    let mut create = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("stackstead"))
        .current_dir(&project.repo)
        .env("XDG_STATE_HOME", test_state_home(&project.repo))
        .args(["create", "feature-a"])
        .spawn()
        .expect("spawn blocked create");
    assert!(
        wait_for_file(&ready, 200, Duration::from_millis(10)),
        "post-create hook did not start"
    );
    let manifests = state_stackstead_directories(&project)
        .into_iter()
        .map(|root| root.join("state/manifest.json"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    assert_eq!(manifests.len(), 1, "manifest was not published during hook");
    let manifest = StacksteadManifest::read(&manifests[0]).expect("read published manifest");
    let assert = stackstead(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr).contains("lock"));
    assert!(manifest.worktree.is_dir());
    fs::remove_file(&release).expect("release post-create hook");
    assert!(create.wait().expect("wait for create").success());
}

#[test]
fn create_generates_the_durable_runtime_contract() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");

    assert_eq!(manifest.kind, "StacksteadManifest");
    assert_eq!(manifest.version, "2");
    assert_eq!(manifest.project, "demo-project");
    assert_eq!(manifest.slug, "feature-a");
    assert_eq!(manifest.branch, "feature-a");
    assert_eq!(
        manifest.base,
        git(&project.repo, &["rev-parse", "main"]).trim()
    );
    assert!(manifest.stackstead_id.starts_with("feature-a-"));
    assert_eq!(manifest.stackstead_id.len(), "feature-a-".len() + 32);
    assert_eq!(manifest.runtime_token.len(), 32);
    assert!(
        manifest
            .runtime_token
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    );
    assert_eq!(
        manifest.compose_project,
        format!("demo-project-{}", manifest.stackstead_id)
    );
    assert!(manifest.worktree.is_dir());
    assert!(manifest.state_dir.is_dir());
    assert!(manifest.state_dir.join("lock").is_file());
    assert!(manifest.state_dir.join("run.lock").is_file());
    assert!(manifest.manifest_path().is_file());
    assert!(manifest.pointer_file.is_file());
    assert!(manifest.env_file.is_file());
    assert!(manifest.agent_context.is_file());
    assert!(manifest.event_log.is_file());

    let persisted = StacksteadManifest::read(&manifest.manifest_path()).expect("read manifest");
    assert_eq!(persisted.stackstead_id, manifest.stackstead_id);
    assert_eq!(persisted.ports, manifest.ports);
    assert_eq!(persisted.urls, manifest.urls);

    let pointer: StacksteadPointer =
        serde_json::from_slice(&fs::read(&manifest.pointer_file).expect("read generated pointer"))
            .expect("parse generated pointer");
    assert_eq!(pointer.kind, "StacksteadPointer");
    assert_eq!(pointer.version, "2");
    assert_eq!(pointer.stackstead_id, manifest.stackstead_id);
    assert_eq!(pointer.manifest, manifest.manifest_path());
    assert_eq!(pointer.repo_root, project.repo);
    assert_eq!(pointer.stackstead_root, manifest.stackstead_root);

    let environment = fs::read_to_string(&manifest.env_file).expect("read generated env");
    assert!(environment.contains("# Generated by Stackstead. Do not edit by hand."));
    assert!(environment.contains(&format!("# Stackstead: {}", manifest.stackstead_id)));
    assert!(environment.contains(&format!("STACKSTEAD_ID={}", manifest.stackstead_id)));
    assert!(environment.contains(&format!("WEB_PORT={}", manifest.ports["web"])));
    assert!(environment.contains(&format!("POSTGRES_PORT={}", manifest.ports["postgres"])));

    let context = fs::read_to_string(&manifest.agent_context).expect("read agent context");
    assert!(context.contains(&format!("# Stackstead: {}", manifest.stackstead_id)));
    assert!(context.contains(manifest.manifest_path().to_string_lossy().as_ref()));
    for command in [
        format!("stackstead inspect {}", manifest.stackstead_id),
        format!("stackstead context {} --print", manifest.stackstead_id),
        format!("stackstead logs {} --tail 200", manifest.stackstead_id),
        format!("stackstead db status {}", manifest.stackstead_id),
        format!("stackstead open {} web --print", manifest.stackstead_id),
        format!("stackstead up {}", manifest.stackstead_id),
        format!("stackstead repair {}", manifest.stackstead_id),
        format!("stackstead stop {}", manifest.stackstead_id),
        format!("stackstead destroy {} --yes", manifest.stackstead_id),
    ] {
        assert!(
            context.lines().any(|line| line == command),
            "missing exact context command: {command}"
        );
    }
    assert!(
        !context
            .lines()
            .any(|line| line == "stackstead inspect feature-a")
    );
    assert!(context.contains(&format!(
        "Endpoint: 127.0.0.1:{}",
        manifest.ports["postgres"]
    )));
    assert!(context.contains("Database: app"));
    assert!(!context.contains("postgres://app:app"));
    assert!(context.contains("it is not a security sandbox"));

    git(
        &manifest.worktree,
        &["check-ignore", "--quiet", ".stackstead/stackstead.json"],
    );
    assert!(
        git(
            &manifest.worktree,
            &["status", "--short", "--untracked-files=all"]
        )
        .trim()
        .is_empty(),
        "generated contract appears in Git status"
    );
    assert_eq!(
        git(&project.repo, &["branch", "--list", "feature-a"]).trim(),
        "+ feature-a"
    );
    assert_eq!(
        event_types(&manifest.event_log),
        [
            "create",
            "pointer_generate",
            "environment_generate",
            "context_generate"
        ]
    );
}

#[test]
fn two_stacksteads_have_distinct_runtime_identity_and_state() {
    let project = Project::initialized();
    let first = project.create("feature-a");
    let second = project.create("feature-b");

    assert_ne!(first.stackstead_id, second.stackstead_id);
    assert_ne!(first.worktree, second.worktree);
    assert_ne!(first.stackstead_root, second.stackstead_root);
    assert_ne!(first.compose_project, second.compose_project);
    assert_ne!(first.env_file, second.env_file);
    assert_ne!(first.agent_context, second.agent_context);
    assert_ne!(first.manifest_path(), second.manifest_path());
    assert_ne!(first.pointer_file, second.pointer_file);
    let first_ports = first.ports.values().copied().collect::<BTreeSet<_>>();
    let second_ports = second.ports.values().copied().collect::<BTreeSet<_>>();
    assert!(first_ports.is_disjoint(&second_ports));
    assert_eq!(
        first.ports.keys().collect::<Vec<_>>(),
        second.ports.keys().collect::<Vec<_>>()
    );

    let assert = stackstead(&project.repo)
        .args(["ps", "--json"])
        .assert()
        .success();
    let listed: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("parse stackstead list");
    assert_eq!(listed["kind"], "StacksteadList");
    assert_eq!(listed["version"], "1");
    let listed = listed["stacksteads"]
        .as_array()
        .expect("stackstead list items");
    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed
            .iter()
            .filter_map(|item| item["stackstead_id"].as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([first.stackstead_id.as_str(), second.stackstead_id.as_str()])
    );
    assert!(
        listed
            .iter()
            .all(|item| matches!(item["runtime"].as_str(), Some("stopped" | "unknown")))
    );
}

#[cfg(unix)]
#[test]
fn adopted_worktree_is_bound_but_preserved_on_destroy() {
    let project = Project::initialized();
    let external = project
        .repo
        .parent()
        .expect("repository has parent")
        .join("manager-owned");
    git(
        &project.repo,
        &[
            "worktree",
            "add",
            "-b",
            "manager-feature",
            external.to_str().expect("UTF-8 fixture path"),
            "main",
        ],
    );
    let adopted = stackstead(&project.repo)
        .args([
            "--json",
            "adopt",
            "manager-feature",
            "--worktree",
            external.to_str().expect("UTF-8 fixture path"),
        ])
        .assert()
        .success();
    let manifest = changed_manifest(&adopted.get_output().stdout, "adopted");
    assert_eq!(manifest.source_ownership, SourceOwnership::External);
    assert_eq!(manifest.worktree, external);
    assert!(manifest.pointer_file.is_file());

    stackstead(&project.repo)
        .arg("adopt")
        .arg("duplicate-manager-feature")
        .arg("--worktree")
        .arg(&external)
        .assert()
        .failure();

    assert!(manifest.pointer_file.is_file());
    assert!(manifest.manifest_path().is_file());

    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "adopt-fake-docker-bin",
        "#!/bin/sh\nexit 0\n",
    );
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", "manager-feature", "--yes"])
        .assert()
        .success();

    assert!(external.is_dir(), "manager-owned worktree was removed");
    assert!(!external.join(".stackstead").exists());
    assert!(!manifest.stackstead_root.exists());
    assert_eq!(
        git(&external, &["branch", "--show-current"]).trim(),
        "manager-feature"
    );
}

#[cfg(unix)]
#[test]
fn destroy_recovery_revalidates_an_adopted_worktree_before_cleanup() {
    let project = Project::initialized();
    let external = project
        .repo
        .parent()
        .unwrap()
        .join("recovery-manager-owned");
    git(
        &project.repo,
        &[
            "worktree",
            "add",
            "-b",
            "recovery-manager-feature",
            external.to_str().unwrap(),
            "main",
        ],
    );
    let adopted = stackstead(&project.repo)
        .arg("--json")
        .arg("adopt")
        .arg("recovery-manager-feature")
        .arg("--worktree")
        .arg(&external)
        .assert()
        .success();
    let manifest = changed_manifest(&adopted.get_output().stdout, "adopted");
    append_destroy_tombstone(&manifest);

    let dirty = external.join("unreviewed-replacement");
    fs::write(&dirty, "must survive\n").unwrap();
    let refused = stackstead(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .failure();
    assert!(
        output_text(&refused.get_output().stderr).contains("uncommitted or untracked"),
        "unexpected error: {}",
        output_text(&refused.get_output().stderr)
    );
    assert!(dirty.is_file());
    assert!(manifest.pointer_file.is_file());
    assert!(manifest.manifest_path().is_file());

    fs::remove_file(dirty).unwrap();
    stackstead_without_runtime(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(external.is_dir());
    assert!(!external.join(".stackstead").exists());
    assert!(!manifest.stackstead_root.exists());
}

#[test]
fn adoption_rejects_a_manager_worktree_that_does_not_contain_the_pinned_base() {
    let project = Project::initialized();
    let external = project.repo.parent().unwrap().join("stale-manager-owned");
    git(
        &project.repo,
        &[
            "worktree",
            "add",
            "-b",
            "stale-manager-feature",
            external.to_str().unwrap(),
            "main",
        ],
    );
    fs::write(project.repo.join("README.md"), "# Advanced base\n").unwrap();
    git(&project.repo, &["add", "README.md"]);
    git(
        &project.repo,
        &["commit", "-m", "advance base before adoption"],
    );

    let rejected = stackstead(&project.repo)
        .args(["adopt", "stale-manager-feature", "--worktree"])
        .arg(&external)
        .assert()
        .failure();
    assert!(output_text(&rejected.get_output().stderr).contains("not based on pinned commit"));
    assert!(!external.join(".stackstead").exists());
    assert!(state_stackstead_directories(&project).is_empty());
}

#[test]
fn adoption_rejects_nested_unrelated_and_detached_checkouts_without_state() {
    let project = Project::initialized();
    let parent = project.repo.parent().expect("repository parent");

    let registered = parent.join("registered-manager-worktree");
    git(
        &project.repo,
        &[
            "worktree",
            "add",
            "-b",
            "registered-manager",
            registered.to_str().expect("UTF-8 fixture path"),
            "main",
        ],
    );
    let nested = registered.join("nested");
    fs::create_dir(&nested).expect("create nested checkout path");
    stackstead(&project.repo)
        .arg("adopt")
        .arg("nested")
        .arg("--worktree")
        .arg(&nested)
        .assert()
        .failure();

    let mut stale_compose =
        fs::read_to_string(registered.join("docker-compose.yml")).expect("read manager Compose");
    stale_compose.push_str("# stale manager contract\n");
    fs::write(registered.join("docker-compose.yml"), stale_compose)
        .expect("change manager Compose contract");
    stackstead(&project.repo)
        .arg("adopt")
        .arg("stale-manager")
        .arg("--worktree")
        .arg(&registered)
        .assert()
        .failure();

    let unrelated = parent.join("unrelated-repository");
    fs::create_dir(&unrelated).expect("create unrelated repository");
    git(&unrelated, &["init", "--initial-branch=other"]);
    git(&unrelated, &["config", "user.name", "Stackstead Tests"]);
    git(
        &unrelated,
        &["config", "user.email", "stackstead-tests@example.invalid"],
    );
    fs::write(unrelated.join("README.md"), "unrelated\n").expect("write unrelated fixture");
    git(&unrelated, &["add", "."]);
    git(&unrelated, &["commit", "-m", "unrelated fixture"]);
    stackstead(&project.repo)
        .arg("adopt")
        .arg("unrelated")
        .arg("--worktree")
        .arg(&unrelated)
        .assert()
        .failure();

    let detached = parent.join("detached-manager-worktree");
    git(
        &project.repo,
        &[
            "worktree",
            "add",
            "--detach",
            detached.to_str().expect("UTF-8 fixture path"),
            "main",
        ],
    );
    stackstead(&project.repo)
        .arg("adopt")
        .arg("detached")
        .arg("--worktree")
        .arg(&detached)
        .assert()
        .failure();

    assert!(registered.is_dir());
    assert!(unrelated.is_dir());
    assert!(detached.is_dir());
    assert!(state_stackstead_directories(&project).is_empty());
    assert!(!registered.join(".stackstead").exists());
    assert!(!unrelated.join(".stackstead").exists());
    assert!(!detached.join(".stackstead").exists());
}

#[test]
fn create_rejects_a_compose_template_that_omits_the_durable_identity() {
    let project = Project::initialized();
    project.replace_config(
        "{{ project.name }}-{{ stackstead.id }}",
        "{{ project.name }}",
    );
    let assert = stackstead(&project.repo)
        .args(["create", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr).contains("must render the durable identity"));
    assert!(state_stackstead_directories(&project).is_empty());
    assert!(
        git(&project.repo, &["branch", "--list", "feature-a"])
            .trim()
            .is_empty()
    );
}

#[test]
fn nested_worktree_commands_use_the_pointer_before_the_copied_config() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let nested = manifest.worktree.join("scratch/deeply/nested");
    fs::create_dir_all(&nested).expect("create nested worktree directory");

    let assert = stackstead(&nested)
        .args(["context", "feature-a", "--json"])
        .assert()
        .success();
    let output: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("parse context output");
    assert_eq!(output["stackstead_id"], manifest.stackstead_id);
    assert_eq!(
        output["path"],
        manifest.agent_context.to_string_lossy().as_ref()
    );
}

#[test]
fn pointer_state_root_cannot_normalize_to_the_filesystem_root() {
    let project = Project::initialized();
    let mut manifest = project.create("feature-a");
    let mut pointer: StacksteadPointer =
        serde_json::from_slice(&fs::read(&manifest.pointer_file).expect("read pointer"))
            .expect("parse pointer");
    manifest.project_state_root = PathBuf::from("/tmp/..");
    pointer.project_state_root = manifest.project_state_root.clone();
    manifest.save_atomic().expect("write tampered manifest");
    fs::write(
        &manifest.pointer_file,
        serde_json::to_vec_pretty(&pointer).expect("serialize pointer"),
    )
    .expect("write tampered pointer");

    let assert = stackstead(&manifest.worktree)
        .args(["context", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr).contains("filesystem root"));
}

#[test]
fn legacy_pointer_v1_discovers_normally_and_repair_rewrites_v2() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let mut pointer: StacksteadPointer =
        serde_json::from_slice(&fs::read(&manifest.pointer_file).unwrap()).unwrap();
    pointer.version = "1".into();
    fs::write(
        &manifest.pointer_file,
        serde_json::to_vec_pretty(&pointer).unwrap(),
    )
    .unwrap();
    stackstead(&manifest.worktree)
        .args(["context", "feature-a", "--json"])
        .assert()
        .success();
    stackstead(&project.repo)
        .args(["repair", "feature-a", "--json"])
        .assert()
        .success();
    let rewritten: StacksteadPointer =
        serde_json::from_slice(&fs::read(&manifest.pointer_file).unwrap()).unwrap();
    assert_eq!(rewritten.version, "2");
}

#[test]
fn destroy_recovers_a_persisted_prepublication_create() {
    let project = Project::initialized();
    let mut manifest = project.create("feature-a");
    fs::remove_file(&manifest.pointer_file).unwrap();
    fs::remove_file(&manifest.event_log).unwrap();
    manifest.status.source = ComponentStatus::Created;
    manifest.save_atomic().unwrap();
    stackstead(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(!manifest.stackstead_root.exists());
    let registry: Value = serde_json::from_slice(
        &fs::read(test_state_home(&project.repo).join("stackstead/port-leases.json")).unwrap(),
    )
    .unwrap();
    assert!(registry["leases"].as_array().unwrap().is_empty());
}

#[test]
fn runtime_probe_failure_is_reported_without_breaking_inspect_json() {
    let project = Project::initialized();
    project.create("feature-a");

    let ps = stackstead(&project.repo)
        .env("PATH", "")
        .args(["ps", "--json"])
        .assert()
        .success();
    let listed: Value = serde_json::from_slice(&ps.get_output().stdout).expect("parse ps output");
    assert_eq!(listed["stacksteads"][0]["runtime"], "unknown");

    let inspect = stackstead(&project.repo)
        .env("PATH", "")
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value =
        serde_json::from_slice(&inspect.get_output().stdout).expect("parse inspect output");
    assert_eq!(inspected["live"]["runtime"]["running"], false);
    assert_eq!(inspected["live"]["runtime"]["status"], "unknown");
    assert_eq!(inspected["live"]["database"]["status"], "unknown");
    assert!(
        inspected["warnings"]
            .as_array()
            .is_some_and(|warnings| !warnings.is_empty())
    );
}

#[test]
fn inspect_human_output_ends_with_full_id_actions() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");

    let inspect = stackstead(&project.repo)
        .env("PATH", "")
        .args(["inspect", "feature-a"])
        .assert()
        .success();
    let stdout = output_text(&inspect.get_output().stdout);

    assert!(stdout.contains(&format!(
        "\nNext:\n  stackstead doctor\n  stackstead context {} --print\n",
        manifest.stackstead_id
    )));
}

#[cfg(unix)]
#[test]
fn database_status_requires_the_exact_compose_port_publication() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let port = manifest.ports["postgres"];
    let _listener = TcpListener::bind(("127.0.0.1", port)).expect("bind unrelated listener");

    let wrong_path = fake_docker_path(
        project.repo.parent().unwrap(),
        "wrong-database-publication-fake-bin",
        &format!(
            "#!/bin/sh\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    ps) printf 'container-id\\n'; exit 0 ;;\n    port) printf '127.0.0.2:{port}\\n'; exit 0 ;;\n  esac\ndone\nexit 0\n"
        ),
    );
    let status = stackstead(&project.repo)
        .env("PATH", &wrong_path)
        .args(["db", "status", "feature-a", "--json"])
        .assert()
        .success();
    let status: Value = serde_json::from_slice(&status.get_output().stdout).unwrap();
    for key in [
        "stackstead_id",
        "strategy",
        "service",
        "host",
        "port",
        "database",
        "reachable",
        "identity_status",
        "seed_status",
        "last_seed_at",
    ] {
        assert!(status.get(key).is_some(), "db status JSON omitted {key}");
    }
    assert_eq!(status["reachable"], true);
    assert_eq!(status["identity_status"], "unreachable");

    let inspected = stackstead(&project.repo)
        .env("PATH", &wrong_path)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout).unwrap();
    assert_eq!(inspected["live"]["database"]["reachable"], true);
    assert_eq!(inspected["live"]["database"]["status"], "unreachable");

    let exact_path = fake_docker_path(
        project.repo.parent().unwrap(),
        "exact-database-publication-fake-bin",
        &format!(
            "#!/bin/sh\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    ps) printf 'container-id\\n'; exit 0 ;;\n    port) printf '127.0.0.1:{port}\\n'; exit 0 ;;\n  esac\ndone\nexit 0\n"
        ),
    );
    let status = stackstead(&project.repo)
        .env("PATH", &exact_path)
        .args(["db", "status", "feature-a", "--json"])
        .assert()
        .success();
    let status: Value = serde_json::from_slice(&status.get_output().stdout).unwrap();
    assert_eq!(status["reachable"], true);
    assert_eq!(status["identity_status"], "reachable");

    let inspected = stackstead(&project.repo)
        .env("PATH", exact_path)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .success();
    let inspected: Value = serde_json::from_slice(&inspected.get_output().stdout).unwrap();
    assert_eq!(inspected["live"]["database"]["reachable"], true);
    assert_eq!(inspected["live"]["database"]["status"], "reachable");
}

#[test]
fn v2_manifest_requires_explicit_source_ownership() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let mut value: Value =
        serde_json::from_slice(&fs::read(manifest.manifest_path()).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("source_ownership");
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();

    let rejected = stackstead(&project.repo)
        .args(["inspect", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(rejected.get_output().stdout.is_empty());
    assert!(
        output_text(&rejected.get_output().stderr).contains("requires source_ownership"),
        "unexpected error: {}",
        output_text(&rejected.get_output().stderr)
    );
    assert!(manifest.worktree.is_dir());
    assert!(manifest.stackstead_root.is_dir());
}

#[test]
fn repair_regenerates_missing_contract_files_without_docker() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    fs::remove_file(&manifest.env_file).expect("remove generated env");
    fs::remove_file(&manifest.agent_context).expect("remove generated context");
    fs::remove_file(&manifest.pointer_file).expect("remove generated pointer");

    let assert = stackstead(&project.repo)
        .args(["repair", "feature-a", "--json"])
        .assert()
        .success();
    let repaired = changed_manifest(&assert.get_output().stdout, "repaired");
    assert_eq!(repaired.stackstead_id, manifest.stackstead_id);
    assert!(repaired.env_file.is_file());
    assert!(repaired.agent_context.is_file());
    assert!(repaired.pointer_file.is_file());
    assert_eq!(
        event_types(&repaired.event_log).last().map(String::as_str),
        Some("repair")
    );

    let repaired_pointer: StacksteadPointer =
        serde_json::from_slice(&fs::read(&repaired.pointer_file).expect("read repaired pointer"))
            .expect("parse repaired pointer");
    assert_eq!(repaired_pointer.manifest, repaired.manifest_path());
}

#[test]
fn json_destroy_requires_yes_without_writing_a_prompt_to_stdout() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");

    let assert = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(
        assert.get_output().stdout.is_empty(),
        "JSON failure was contaminated by: {}",
        output_text(&assert.get_output().stdout)
    );
    assert!(output_text(&assert.get_output().stderr).contains("--yes"));
    assert!(manifest.stackstead_root.is_dir());
    assert!(manifest.worktree.is_dir());
}

#[test]
fn unexposed_database_service_fails_without_publishing_partial_state() {
    let project = Project::initialized();
    project.replace_config("    service: postgres\n", "    service: missing-postgres\n");

    let assert = stackstead(&project.repo)
        .args(["create", "feature-a", "--json"])
        .assert()
        .failure();
    let stderr = output_text(&assert.get_output().stderr);
    assert!(
        stderr.contains("resources.ports.expose") || stderr.contains("must be present"),
        "unexpected error: {stderr}"
    );
    assert!(state_stackstead_directories(&project).is_empty());
    assert!(
        git(&project.repo, &["branch", "--list", "feature-a"])
            .trim()
            .is_empty()
    );
}

#[test]
fn pointer_project_identity_fields_are_checked_against_the_manifest() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let original: Value =
        serde_json::from_slice(&fs::read(&manifest.pointer_file).expect("read generated pointer"))
            .expect("parse generated pointer");

    for (field, value) in [
        ("project", Value::String("different-project".into())),
        (
            "project_state_root",
            Value::String("/definitely/not/the/stackstead/state".into()),
        ),
    ] {
        let mut tampered = original.clone();
        tampered[field] = value;
        fs::write(
            &manifest.pointer_file,
            serde_json::to_vec_pretty(&tampered).expect("serialize tampered pointer"),
        )
        .expect("write tampered pointer");

        let assert = stackstead(&manifest.worktree)
            .args(["context", "feature-a", "--json"])
            .assert()
            .failure();
        let stderr = output_text(&assert.get_output().stderr);
        assert!(
            stderr.contains("does not match its manifest"),
            "tampered {field} was not rejected during discovery: {stderr}"
        );
    }

    fs::write(
        &manifest.pointer_file,
        serde_json::to_vec_pretty(&original).expect("serialize original pointer"),
    )
    .expect("restore original pointer");
}

#[cfg(unix)]
#[test]
fn copied_pointer_rejects_every_affected_command_before_external_mutation() {
    let victim = Project::initialized();
    let manifest = victim.create("victim");
    let caller = Project::initialized();
    let copied_pointer = caller.repo.join(".stackstead/stackstead.json");
    fs::create_dir_all(copied_pointer.parent().unwrap()).unwrap();
    fs::copy(&manifest.pointer_file, &copied_pointer).unwrap();
    let manifest_before = fs::read(manifest.manifest_path()).unwrap();
    let events_before = fs::read(&manifest.event_log).unwrap();
    let docker_marker = caller
        .repo
        .parent()
        .unwrap()
        .join("copied-pointer-docker-ran");
    let probe = caller
        .repo
        .parent()
        .unwrap()
        .join("copied-pointer-probe-ran");
    let path = fake_docker_path(
        caller.repo.parent().unwrap(),
        "copied-pointer-fake-bin",
        &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", docker_marker.display()),
    );
    let caller_path = caller.repo.to_str().unwrap();
    let probe_command = format!("touch '{}'", probe.display());

    for args in [
        vec!["create", "redirected"],
        vec!["adopt", "redirected", "--worktree", caller_path],
        vec!["up", &manifest.stackstead_id],
        vec![
            "run",
            &manifest.stackstead_id,
            "--",
            "sh",
            "-c",
            &probe_command,
        ],
        vec!["stop", &manifest.stackstead_id],
        vec!["destroy", &manifest.stackstead_id, "--yes"],
        vec!["repair", &manifest.stackstead_id],
    ] {
        let rejected = stackstead(&caller.repo)
            .env("PATH", &path)
            .args(args)
            .assert()
            .failure();
        assert!(
            output_text(&rejected.get_output().stderr).contains("does not match its manifest"),
            "unexpected copied-pointer error: {}",
            output_text(&rejected.get_output().stderr)
        );
    }
    assert!(!docker_marker.exists());
    assert!(!probe.exists());
    assert_eq!(fs::read(manifest.manifest_path()).unwrap(), manifest_before);
    assert_eq!(fs::read(&manifest.event_log).unwrap(), events_before);
    assert!(manifest.worktree.is_dir());
}

#[cfg(unix)]
#[test]
fn failed_git_worktree_add_leaves_no_published_manifest_or_stackstead_root() {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized();
    let fake_bin = project
        .repo
        .parent()
        .expect("repository has parent")
        .join("fake-bin");
    fs::create_dir(&fake_bin).expect("create fake binary directory");
    let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join("git"))
        .find(|candidate| candidate.is_file())
        .expect("find real Git executable");
    let wrapper = fake_bin.join("git");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ \"$1\" = worktree ] && [ \"$2\" = add ]; then\n  echo 'intentional worktree failure' >&2\n  exit 19\nfi\nexec '{}' \"$@\"\n",
            real_git.display().to_string().replace('\'', "'\"'\"'")
        ),
    )
    .expect("write Git wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
        .expect("make Git wrapper executable");
    let path = std::env::join_paths(std::iter::once(fake_bin.clone()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .expect("construct command-local PATH");

    let mut command = stackstead(&project.repo);
    command.env("PATH", path);
    let assert = command
        .args(["create", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr).contains("intentional worktree failure"));
    assert!(state_stackstead_directories(&project).is_empty());
    let registry: Value = serde_json::from_slice(
        &fs::read(test_state_home(&project.repo).join("stackstead/port-leases.json"))
            .expect("read rolled-back port lease registry"),
    )
    .expect("parse rolled-back port lease registry");
    assert!(registry["leases"].as_array().unwrap().is_empty());
    assert!(
        git(&project.repo, &["branch", "--list", "feature-a"])
            .trim()
            .is_empty()
    );
}

#[test]
fn dependency_failure_is_persisted_without_starting_compose() {
    let project = Project::initialized();
    project.replace_config(
        "    command: ''\n    shell: false\n",
        "    command: stackstead-command-that-does-not-exist\n    shell: false\n",
    );
    let manifest = project.create("feature-a");

    let assert = stackstead(&project.repo)
        .args(["up", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr).contains("stackstead-command-that-does-not-exist")
    );

    let persisted = StacksteadManifest::read(&manifest.manifest_path()).expect("read failed state");
    assert_eq!(
        serde_json::to_value(persisted.status.dependencies).expect("serialize status"),
        Value::String("failed".into())
    );
    let events = event_types(&persisted.event_log);
    assert!(events.contains(&"dependencies_install".into()));
    assert!(!events.contains(&"runtime_start".into()));
}

#[cfg(unix)]
#[test]
fn dependency_and_yarn_logs_redact_structured_and_environment_secrets() {
    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["dependencies"]["provider"] = "yarn-classic".into();
    config["dependencies"]["install"]["command"] = "printf 'Authorization: Bearer dependency-header-marker\\nhttps://user:dependency-url-marker@example.invalid/repo\\n%s\\nordinary dependency output\\n' \"$API_TOKEN\"".into();
    config["dependencies"]["install"]["shell"] = true.into();
    config["dependencies"]["link"] = serde_yaml::to_value(serde_json::json!({
        "enabled": true,
        "link_folder": ".stackstead/yarn-links",
        "command": "printf 'Proxy-Authorization: Basic yarn-header-marker\\nhttps://user:yarn-url-marker@example.invalid/repo\\n%s\\nordinary yarn output\\n' \"$API_TOKEN\"",
        "shell": true,
    }))
    .unwrap();
    config["env"]["generate"]["API_TOKEN"] = "known-environment-marker".into();
    project.write_config(&config, "configure secret-emitting dependency fixtures");
    let manifest = project.create("feature-a");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "redaction-fake-docker-bin",
        "#!/bin/sh\nexit 19\n",
    );

    stackstead(&project.repo)
        .env("PATH", path)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    for (name, ordinary) in [
        ("dependencies.log", "ordinary dependency output"),
        ("yarn-link.log", "ordinary yarn output"),
    ] {
        let log = fs::read_to_string(manifest.state_dir.join("logs").join(name)).unwrap();
        assert!(log.contains(ordinary));
        assert!(log.contains("[REDACTED]"));
        for marker in [
            "dependency-header-marker",
            "dependency-url-marker",
            "yarn-header-marker",
            "yarn-url-marker",
            "known-environment-marker",
        ] {
            assert!(!log.contains(marker), "{name} leaked {marker}");
        }
    }
}

#[test]
fn failed_dependency_diagnostics_and_events_share_structured_redaction() {
    let project = Project::initialized();
    project.replace_config(
        "    command: ''\n    shell: false\n",
        "    command: \"printf 'Authorization: Bearer event-header-marker\\\\n' >&2; exit 7\"\n    shell: true\n",
    );
    let manifest = project.create("feature-a");

    let rejected = stackstead(&project.repo)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    assert!(!output_text(&rejected.get_output().stderr).contains("event-header-marker"));
    let events = fs::read_to_string(&manifest.event_log).unwrap();
    assert!(events.contains("[REDACTED]"));
    assert!(!events.contains("event-header-marker"));
}

#[test]
fn pre_up_failure_preserves_completed_dependency_status() {
    let project = Project::initialized();
    project.replace_config(
        "  pre_up: []\n",
        "  pre_up:\n  - command: stackstead-pre-up-command-that-does-not-exist\n    shell: false\n",
    );
    let mut manifest = project.create("feature-a");
    manifest.status.database = ComponentStatus::Reachable;
    manifest.status.health = ComponentStatus::Ready;
    manifest.save_atomic().unwrap();

    stackstead(&project.repo)
        .args(["up", "feature-a", "--json"])
        .assert()
        .failure();

    let persisted = StacksteadManifest::read(&manifest.manifest_path()).expect("read failed state");
    assert_eq!(persisted.status.dependencies, ComponentStatus::Ready);
    assert_eq!(persisted.status.database, ComponentStatus::Unknown);
    assert_eq!(persisted.status.health, ComponentStatus::Unknown);
    assert!(!event_types(&persisted.event_log).contains(&"runtime_start".into()));
}

#[cfg(unix)]
#[test]
fn up_revalidates_contract_mutations_after_pre_and_post_hooks() {
    for post_up in [false, true] {
        let project = Project::initialized();
        let mut config = load_config(&project.repo.join("stackstead.yaml"));
        config["database"]["postgres"] = serde_yaml::Value::Null;
        config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
        let mutation = serde_json::json!({
            "command": "printf 'services:\\n  web:\\n    ports: [\"80\"]\\n' > docker-compose.yml",
            "shell": true,
        });
        if post_up {
            config["hooks"]["post_up"] = serde_yaml::to_value([mutation]).unwrap();
        } else {
            config["hooks"]["pre_up"] = serde_yaml::to_value([mutation]).unwrap();
        }
        project.write_config(&config, "configure contract mutation hook");
        let manifest = project.create("feature-a");
        let marker = project.repo.parent().unwrap().join(if post_up {
            "post-up-docker-ran"
        } else {
            "pre-up-docker-ran"
        });
        let fake_state = project
            .repo
            .parent()
            .unwrap()
            .join("contract-hook-docker-state");
        let docker_script = format!(
            r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$1 $2" in
  "container ls"|"network ls") exit 0 ;;
  "volume ls") test ! -f "$FAKE_STATE/claim" || printf '%s\n' "$claim"; exit 0 ;;
  "volume create") printf '%s' "$EXPECTED_TOKEN" > "$FAKE_STATE/claim"; exit 0 ;;
  "volume inspect") printf '{{"io.stackstead.runtime-token":"%s"}}\n' "$(cat "$FAKE_STATE/claim")"; exit 0 ;;
  "compose -p") touch '{}'; exit 0 ;;
esac
exit 0
"#,
            marker.display()
        );
        let path = fake_docker_path(
            project.repo.parent().unwrap(),
            if post_up {
                "post-up-contract-fake-bin"
            } else {
                "pre-up-contract-fake-bin"
            },
            &docker_script,
        );
        let rejected = stackstead(&project.repo)
            .env("PATH", path)
            .env("FAKE_STATE", fake_state)
            .env("EXPECTED_TOKEN", &manifest.runtime_token)
            .args(["up", &manifest.stackstead_id])
            .assert()
            .failure();
        assert!(output_text(&rejected.get_output().stderr).contains("deterministic host binding"));
        assert_eq!(marker.exists(), post_up, "Docker stage ordering changed");
    }
}

#[cfg(unix)]
#[test]
fn command_health_persists_ready_failed_and_stop_reset_states() {
    let project = Project::git_repo();
    fs::write(
        project.repo.join("docker-compose.yml"),
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"127.0.0.1:${WEB_PORT}:80\"\n",
    )
    .expect("write web-only Compose fixture");
    git(&project.repo, &["add", "docker-compose.yml"]);
    git(
        &project.repo,
        &["commit", "-m", "use web-only health fixture"],
    );
    stackstead(&project.repo).arg("init").assert().success();

    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["health"]["timeout_seconds"] = 1.into();
    config["health"]["interval_millis"] = 10.into();
    config["health"]["checks"] = serde_yaml::to_value([serde_json::json!({
        "name": "worker",
        "url": null,
        "expect_status": 200,
        "command": {
            "command": "test -f README.md && test \"$COMPOSE_PROJECT_NAME\" = \"demo-project-$STACKSTEAD_ID\"",
            "shell": true,
        },
    })])
    .unwrap();
    config["hooks"]["pre_up"] = serde_yaml::to_value([serde_json::json!({
        "command": "true",
        "shell": false,
    })])
    .unwrap();
    config["hooks"]["post_up"] = serde_yaml::to_value([serde_json::json!({
        "command": "true",
        "shell": false,
    })])
    .unwrap();
    project.write_config(&config, "configure command health fixture");
    let manifest = project.create("feature-a");

    let fake_state = project.repo.parent().unwrap().join("health-docker-state");
    let docker_script = format!(
        r#"#!/bin/sh
set -eu
test -z "${{WEB_PORT+x}}" || exit 90
test "$COMPOSE_PROJECT_NAME" = "{}" || exit 92
mkdir -p "$FAKE_STATE"
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$1 $2" in
  "container ls"|"network ls") exit 0 ;;
  "volume ls") test ! -f "$FAKE_STATE/claim" || printf '%s\n' "$claim"; exit 0 ;;
  "volume create") printf '%s' "$EXPECTED_TOKEN" > "$FAKE_STATE/claim"; exit 0 ;;
  "volume inspect") printf '{{"io.stackstead.runtime-token":"%s"}}\n' "$(cat "$FAKE_STATE/claim")"; exit 0 ;;
esac
while [ "$#" -gt 0 ]; do
  if [ "$1" = --env-file ]; then shift; env_file=$1; break; fi
  shift
done
. "$env_file"
test "$WEB_PORT" = "{}" || exit 91
exit 0
"#,
        manifest.compose_project, manifest.ports["web"]
    );
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "health-fake-docker-bin",
        &docker_script,
    );
    let docker = std::env::split_paths(&path).next().unwrap().join("docker");

    let ready = stackstead(&project.repo)
        .env("PATH", &path)
        .env("WEB_PORT", "9")
        .env("STACKSTEAD_ID", "spoofed")
        .env("COMPOSE_PROJECT_NAME", "shared")
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["--json", "up", "feature-a"])
        .assert()
        .success();
    assert!(!output_text(&ready.get_output().stdout).contains("Timings"));
    let ready = changed_manifest(&ready.get_output().stdout, "started");
    assert_eq!(ready.status.health, ComponentStatus::Ready);
    let human = stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", "feature-a"])
        .assert()
        .success();
    let human = output_text(&human.get_output().stdout);
    for phase in [
        "Timings:",
        "Dependencies",
        "Runtime start",
        "Hooks",
        "Health checks",
        "Total",
    ] {
        assert!(
            human.contains(phase),
            "human output omitted {phase:?}: {human}"
        );
    }
    for omitted in ["DB readiness", "Seed"] {
        assert!(
            !human.contains(omitted),
            "human output included unconfigured phase {omitted:?}: {human}"
        );
    }
    let inspected = stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["--json", "inspect", "feature-a"])
        .assert()
        .success();
    let inspected: Value =
        serde_json::from_slice(&inspected.get_output().stdout).expect("parse inspect output");
    assert!(inspected["live"]["health"].is_null());
    assert_eq!(inspected["stackstead"]["status"]["health"], "ready");

    fs::write(&docker, "#!/bin/sh\nexit 19\n").expect("make Compose fail");
    stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", "feature-a"])
        .assert()
        .failure();
    let compose_failed =
        StacksteadManifest::read(&manifest.manifest_path()).expect("read Compose failure state");
    assert_eq!(compose_failed.status.health, ComponentStatus::Unknown);
    fs::write(&docker, &docker_script).expect("restore fake Docker");

    let stopped = stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["--json", "stop", "feature-a"])
        .assert()
        .success();
    let stopped = changed_manifest(&stopped.get_output().stdout, "stopped");
    assert_eq!(stopped.status.health, ComponentStatus::Unknown);

    config["health"]["checks"][0]["command"]["command"] = "false".into();
    project.write_config(&config, "make command health fail");
    stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", fake_state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", "feature-a"])
        .assert()
        .failure();
    let failed = StacksteadManifest::read(&manifest.manifest_path()).expect("read failed manifest");
    assert_eq!(failed.status.health, ComponentStatus::Failed);
    let health_error = fs::read_to_string(&failed.event_log)
        .expect("read health events")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("parse health event"))
        .any(|event| event["type"] == "health_wait" && event["status"] == "failed");
    assert!(health_error);
}

#[cfg(unix)]
#[test]
fn inspect_passively_checks_http_health_only_for_a_running_runtime() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let listener =
        TcpListener::bind(("127.0.0.1", manifest.ports["web"])).expect("bind allocated web port");
    let server = thread::spawn(move || {
        for status in ["200 OK", "500 Internal Server Error"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let read = stream.read(&mut buffer).unwrap();
                assert_ne!(read, 0, "client closed before sending HTTP headers");
                request.extend_from_slice(&buffer[..read]);
            }
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            stream.flush().unwrap();
        }
    });
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "inspect-health-fake-bin",
        "#!/bin/sh\necho running\n",
    );

    for expected in [true, false] {
        let inspected = stackstead(&project.repo)
            .env("PATH", &path)
            .args(["inspect", "feature-a", "--json"])
            .assert()
            .success();
        let value: Value = serde_json::from_slice(&inspected.get_output().stdout).unwrap();
        assert_eq!(value["live"]["runtime"]["running"], true);
        assert_eq!(value["live"]["health"]["healthy"], expected);
    }
    server.join().unwrap();
}

#[test]
fn in_repo_state_is_rejected_before_creating_state() {
    let project = Project::initialized();
    project.replace_config("root: ../.stacksteads", "root: .stacksteads");
    let rejected = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    assert!(
        output_text(&rejected.get_output().stderr)
            .contains("state.root must resolve outside the repository")
    );
    assert!(!project.repo.join(".stacksteads").exists());
}

#[cfg(unix)]
#[test]
fn create_rejects_a_project_lock_symlink_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let project = Project::initialized();
    let marker = project.repo.parent().unwrap().join("lock-marker");
    fs::write(&marker, "unchanged\n").unwrap();
    let lock = project
        .repo
        .parent()
        .unwrap()
        .join(".stacksteads/demo-project/project.lock");
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    symlink(&marker, &lock).unwrap();

    stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .failure();
    assert_eq!(fs::read_to_string(&marker).unwrap(), "unchanged\n");
    assert!(
        fs::symlink_metadata(&lock)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::read_dir(lock.parent().unwrap())
            .unwrap()
            .all(|entry| !entry.unwrap().file_type().unwrap().is_dir())
    );

    fs::remove_file(&lock).unwrap();
    let manifest = project.create("feature-a");
    assert!(manifest.manifest_path().is_file());
    assert_eq!(fs::read_to_string(marker).unwrap(), "unchanged\n");
}

#[cfg(unix)]
#[test]
fn state_parent_symlinks_are_resolved_to_safe_external_targets() {
    use std::os::unix::fs::symlink;

    let project = Project::initialized();
    project.replace_config("root: ../.stacksteads", "root: .stacksteads");
    let outside = project.repo.parent().unwrap().join("outside-state-root");
    let project_target = outside.join("demo-project");
    fs::create_dir_all(&project_target).unwrap();
    let link = project.repo.join(".stacksteads");
    let lock_target = project_target.join("project.lock");
    fs::write(&lock_target, "unchanged\n").unwrap();
    symlink(&outside, &link).unwrap();
    git(&project.repo, &["add", ".stacksteads"]);
    git(&project.repo, &["commit", "-m", "add external state alias"]);

    let created = stackstead(&project.repo)
        .args(["create", "feature-a"])
        .assert()
        .success();
    assert!(!created.get_output().stdout.is_empty());
    assert!(
        fs::read_to_string(lock_target)
            .unwrap()
            .contains("acquired_at=")
    );
}

#[test]
fn doctor_scans_branch_local_compose_files_for_fixed_ports() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    fs::write(
        &manifest.compose_files[0],
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"3000:80\"\n",
    )
    .expect("write branch-local Compose change");

    let assert = stackstead(&project.repo)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let diagnostics: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("parse diagnostics");
    assert!(
        diagnostics["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["code"]
                == "compose.worktree_fixed_host_port"
                && item["message"].as_str().is_some_and(|message| {
                    message.contains("3000") && message.contains("docker-compose.yml:5")
                })))
    );
    assert!(
        diagnostics["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| {
                item["code"] == "compose.worktree_all_interfaces_host_port"
                    && item["severity"] == "error"
                    && item["message"].as_str().is_some_and(|message| {
                        message.contains("web")
                            && message.contains("80/tcp")
                            && message
                                .contains(manifest.compose_files[0].to_string_lossy().as_ref())
                    })
            }))
    );

    fs::write(
        &manifest.compose_files[0],
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"127.0.0.1:${WEB_PORT}:80\"\n",
    )
    .unwrap();
    let loopback = stackstead(&project.repo)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let loopback: Value = serde_json::from_slice(&loopback.get_output().stdout).unwrap();
    assert!(loopback["diagnostics"].as_array().is_some_and(|items| {
        items
            .iter()
            .all(|item| item["code"] != "compose.worktree_all_interfaces_host_port")
    }));
}

#[test]
fn doctor_reports_project_worktree_and_pointer_contract_failures_together() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    fs::write(
        project.repo.join("docker-compose.yml"),
        "services:\n  web:\n    ports: [\"80\"]\n",
    )
    .unwrap();
    fs::write(
        &manifest.compose_files[0],
        "services:\n  web:\n    ports: [\"${APP_PORT}:80\"]\n",
    )
    .unwrap();
    let mut pointer: Value =
        serde_json::from_slice(&fs::read(&manifest.pointer_file).unwrap()).expect("parse pointer");
    pointer["stackstead_id"] = Value::String("copied-pointer-a123".into());
    fs::write(
        &manifest.pointer_file,
        serde_json::to_vec_pretty(&pointer).unwrap(),
    )
    .unwrap();

    let output = stackstead(&project.repo)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let diagnostics: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    let codes = diagnostics["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["code"].as_str())
        .collect::<BTreeSet<_>>();
    for expected in [
        "compose.unbound_host_port",
        "compose.isolation_contract.invalid",
        "compose.worktree_isolation_contract.invalid",
        "pointer.binding.invalid",
    ] {
        assert!(
            codes.contains(expected),
            "missing diagnostic {expected}: {codes:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn doctor_fail_on_error_keeps_complete_json_and_ignores_warnings() {
    let project = Project::initialized();
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "doctor-ci-fake-bin",
        "#!/bin/sh\ntest \"${1-}\" != info\n",
    );

    let warning_only = stackstead(&project.repo)
        .env("PATH", &path)
        .args(["doctor", "--json", "--fail-on-error"])
        .assert()
        .success();
    let warning_report: Value = serde_json::from_slice(&warning_only.get_output().stdout).unwrap();
    assert_eq!(warning_report["kind"], "DoctorReport");
    assert_eq!(warning_report["version"], "1");
    assert_eq!(warning_report["error_count"], 0);
    assert!(warning_report["warning_count"].as_u64().unwrap() > 0);

    fs::write(
        project.repo.join("docker-compose.yml"),
        "services:\n  web:\n    ports: [\"80\"]\n",
    )
    .unwrap();
    stackstead(&project.repo)
        .env("PATH", &path)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let failed = stackstead(&project.repo)
        .env("PATH", path)
        .args(["doctor", "--json", "--fail-on-error"])
        .assert()
        .code(1);
    let error_report: Value = serde_json::from_slice(&failed.get_output().stdout).unwrap();
    assert_eq!(error_report["ok"], false);
    assert!(error_report["error_count"].as_u64().unwrap() > 0);
    assert!(!error_report["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn env_outputs_redact_credentials_and_generation_is_deterministic() {
    let project = Project::initialized();
    project.replace_config(
        "    DATABASE_URL: postgres://app:app@127.0.0.1:{{ ports.postgres }}/app\n",
        "    Z_LAST: \"value#hash\"\n    SERVICE_DSN: postgresql://worker:dnspass@127.0.0.1:{{ ports.postgres }}/app\n    DATABASE_URL: postgres://app:app@127.0.0.1:{{ ports.postgres }}/app\n    A_FIRST: \"hello world\"\n",
    );
    let manifest = project.create("feature-a");

    let env = fs::read_to_string(&manifest.env_file).expect("read generated env");
    assert!(env.contains("A_FIRST=\"hello world\""));
    assert!(env.contains("Z_LAST=\"value#hash\""));
    let keys = env
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split_once('=').expect("env assignment").0)
        .collect::<Vec<_>>();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted, "generated env assignments are not sorted");

    for args in [
        vec!["env", "feature-a"],
        vec!["env", "feature-a", "--json"],
        vec!["env", "feature-a", "--print"],
    ] {
        let assert = stackstead(&project.repo).args(args).assert().success();
        let stdout = output_text(&assert.get_output().stdout);
        assert!(
            !stdout.contains("postgres://app:app@"),
            "DATABASE_URL leaked: {stdout}"
        );
        assert!(
            !stdout.contains("worker:dnspass@"),
            "SERVICE_DSN leaked: {stdout}"
        );
        assert!(stdout.contains("DATABASE_URL"));
        assert!(stdout.contains("SERVICE_DSN"));
        assert!(stdout.contains("[REDACTED]"));
    }
}

#[test]
fn tampered_manifest_destroy_fails_before_external_mutation() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let mut tampered = manifest.clone();
    tampered.repo_root = project.repo.join("different-repository");
    fs::write(
        manifest.manifest_path(),
        serde_json::to_vec_pretty(&tampered).expect("serialize tampered manifest"),
    )
    .expect("write tampered manifest");

    let assert = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--yes", "--json"])
        .assert()
        .failure();
    assert!(
        output_text(&assert.get_output().stderr)
            .contains("project identity does not match the discovered project")
    );
    assert!(assert.get_output().stdout.is_empty());
    assert!(manifest.stackstead_root.is_dir());
    assert!(manifest.worktree.is_dir());
    assert!(!event_types(&manifest.event_log).contains(&"destroyed".into()));
}

#[cfg(unix)]
#[test]
fn repair_rejects_a_generated_directory_symlink_escape() {
    use std::os::unix::fs::symlink;

    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let generated = manifest.worktree.join(".stackstead");
    fs::remove_dir_all(&generated).expect("remove generated contract directory");
    let outside = project
        .repo
        .parent()
        .expect("repository has parent")
        .join("escape-target");
    fs::create_dir(&outside).expect("create escape target");
    symlink(&outside, &generated).expect("create generated-directory symlink");

    let assert = stackstead(&project.repo)
        .args(["repair", "feature-a", "--json"])
        .assert()
        .failure();
    let stderr = output_text(&assert.get_output().stderr);
    assert!(
        stderr.contains("symlink") || stderr.contains("escapes") || stderr.contains("unsafe"),
        "unexpected symlink error: {stderr}"
    );
    assert!(!outside.join(".env").exists());
    assert!(!outside.join("AGENT_CONTEXT.md").exists());
    assert!(!outside.join("stackstead.json").exists());
    assert!(manifest.stackstead_root.is_dir());
}

#[test]
fn destroy_refuses_a_dirty_worktree_before_touching_runtime_state() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    fs::write(manifest.worktree.join("README.md"), "dirty\n").expect("dirty tracked file");

    let assert = stackstead(&project.repo)
        .args(["destroy", "feature-a", "--yes", "--json"])
        .assert()
        .failure();
    assert!(
        String::from_utf8_lossy(&assert.get_output().stderr)
            .contains("uncommitted or untracked changes")
    );

    assert!(manifest.manifest_path().is_file());
    assert!(manifest.worktree.is_dir());
}

#[cfg(unix)]
#[test]
fn destroy_uses_the_durable_manifest_after_non_destructive_config_path_changes() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["env"]["file"] = ".stackstead-next/.env".into();
    config["agent"]["context_file"] = ".stackstead-next/AGENT_CONTEXT.md".into();
    project.write_config(&config, "change future generated paths");

    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "cleanup-fake-docker-bin",
        "#!/bin/sh\nexit 0\n",
    );
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", "feature-a", "--yes"])
        .assert()
        .success();

    assert!(!manifest.stackstead_root.exists());
    assert!(!manifest.worktree.exists());
}

#[test]
fn repair_rejects_changed_generated_paths_without_writing_them() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["env"]["file"] = ".stackstead-next/.env".into();
    config["agent"]["context_file"] = ".stackstead-next/AGENT_CONTEXT.md".into();
    project.write_config(&config, "change future repair paths");

    stackstead(&project.repo)
        .args(["repair", "feature-a", "--json"])
        .assert()
        .failure();
    assert!(!manifest.worktree.join(".stackstead-next").exists());
    assert!(manifest.env_file.is_file());
    assert!(manifest.agent_context.is_file());
}

#[cfg(unix)]
#[test]
fn custom_compose_project_contract_is_rejected() {
    let project = Project::initialized();
    let mut manifest = project.create("feature-a");
    manifest.compose_project = format!(
        "{}_{}_{}",
        manifest.project, manifest.slug, manifest.short_id
    );
    manifest.save_atomic().expect("write legacy manifest");
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["runtime"]["project_name_template"] =
        "{{ project.name }}_{{ stackstead.slug }}_{{ stackstead.short_id }}".into();
    project.write_config(&config, "preserve legacy Compose identity");

    let marker = project.repo.parent().unwrap().join("docker-ran");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "legacy-project-fake-bin",
        &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
    );
    stackstead(&project.repo)
        .env("PATH", path)
        .args(["destroy", "feature-a", "--yes"])
        .assert()
        .failure();
    assert!(manifest.stackstead_root.exists());
    assert!(!marker.exists());
}

#[test]
fn destroy_retries_only_state_after_recorded_source_cleanup() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    git(
        &project.repo,
        &["worktree", "remove", manifest.worktree.to_str().unwrap()],
    );
    append_destroy_tombstone(&manifest);
    append_event(&manifest.event_log, "source_remove", "succeeded");

    stackstead_without_runtime(&project.repo)
        .args(["destroy", "feature-a", "--yes"])
        .assert()
        .success();
    assert!(!manifest.stackstead_root.exists());
}

#[test]
fn destroy_resumes_source_cleanup_from_the_persisted_tombstone() {
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    append_destroy_tombstone(&manifest);
    stackstead_without_runtime(&project.repo)
        .args(["destroy", "feature-a", "--yes"])
        .assert()
        .success();
    assert!(!manifest.stackstead_root.exists());
    assert!(!manifest.worktree.exists());
}

#[test]
fn destroy_recovery_revalidates_a_dirty_owned_worktree() {
    let project = Project::initialized();
    let manifest = project.create("dirty-recovery");
    append_destroy_tombstone(&manifest);
    fs::write(manifest.worktree.join("README.md"), "dirty\n").expect("dirty tracked file");

    let assert = stackstead(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .failure();
    assert!(output_text(&assert.get_output().stderr).contains("uncommitted or untracked changes"));
    assert!(manifest.worktree.is_dir());
    assert!(manifest.manifest_path().is_file());
}

#[cfg(unix)]
#[test]
fn destroy_retry_removes_a_git_unregistered_permission_blocked_remainder() {
    use std::os::unix::fs::PermissionsExt;

    let project = Project::initialized();
    let manifest = project.create("permission-remainder");
    let blocked = manifest.worktree.join(".stackstead/container-owned");
    fs::create_dir(&blocked).expect("create ignored container-owned directory");
    fs::write(blocked.join("artifact"), "generated\n").expect("write generated artifact");
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o555))
        .expect("make generated directory non-writable");
    append_destroy_tombstone(&manifest);

    let first = stackstead(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .failure();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o755))
        .expect("restore generated directory ownership-equivalent permissions");
    assert!(output_text(&first.get_output().stderr).contains("container-created files"));
    assert!(manifest.worktree.is_dir());
    assert!(
        !git(&project.repo, &["worktree", "list", "--porcelain"])
            .contains(manifest.worktree.to_string_lossy().as_ref())
    );

    stackstead_without_runtime(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(!manifest.stackstead_root.exists());
}

#[test]
fn destroy_retry_preserves_a_still_registered_locked_worktree() {
    let project = Project::initialized();
    let manifest = project.create("locked-remainder");
    git(
        &project.repo,
        &["worktree", "lock", manifest.worktree.to_str().unwrap()],
    );
    append_destroy_tombstone(&manifest);

    stackstead(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .failure();
    assert!(manifest.worktree.is_dir());
    assert!(
        git(&project.repo, &["worktree", "list", "--porcelain"])
            .contains(manifest.worktree.to_string_lossy().as_ref())
    );

    git(
        &project.repo,
        &["worktree", "unlock", manifest.worktree.to_str().unwrap()],
    );
    stackstead_without_runtime(&project.repo)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(!manifest.stackstead_root.exists());
}

#[test]
fn destroy_rejects_untyped_legacy_recovery_events() {
    use std::io::Write;

    let project = Project::initialized();
    let manifest = project.create("legacy-cleanup");
    manifest.save_atomic().unwrap();
    git(
        &project.repo,
        &["worktree", "remove", manifest.worktree.to_str().unwrap()],
    );
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(&manifest.event_log)
            .unwrap(),
        "{{\"type\":\"destroyed\"}}"
    )
    .unwrap();

    stackstead(&project.repo)
        .args(["destroy", "legacy-cleanup", "--yes"])
        .assert()
        .failure();
    assert!(manifest.stackstead_root.exists());
}

#[cfg(unix)]
#[test]
fn compose_runtime_ownership_rejects_foreign_resources_and_preserves_owned_lifecycle() {
    const DOCKER: &str = r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
printf '%s\n' "$*" >> "$FAKE_STATE/commands"
kind=${1-}
verb=${2-}
last=
for argument in "$@"; do last=$argument; done
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$kind $verb" in
  "container ls")
    test "${FOREIGN_KIND-}" = container && printf '%s\n' "$FOREIGN_NAME"
    ;;
  "network ls")
    test "${FOREIGN_KIND-}" = network && printf '%s\n' "$FOREIGN_NAME"
    ;;
  "volume ls")
    test -f "$FAKE_STATE/claim-token" && printf '%s\n' "$claim"
    test "${FOREIGN_KIND-}" = volume && printf '%s\n' "$FOREIGN_NAME"
    ;;
  "volume create")
    test -f "$FAKE_STATE/claim-token" || printf '%s' "$EXPECTED_TOKEN" > "$FAKE_STATE/claim-token"
    printf '%s\n' "$claim"
    ;;
  "volume rm")
    test "$last" = "$claim"
    rm "$FAKE_STATE/claim-token"
    ;;
  "container inspect"|"network inspect"|"volume inspect")
    if test "$last" = "$claim"; then
      test -f "$FAKE_STATE/claim-token" || exit 41
      token=$(cat "$FAKE_STATE/claim-token")
    elif test "$last" = "${FOREIGN_NAME-}"; then
      token=foreign-runtime-token
    else
      exit 42
    fi
    printf '{"io.stackstead.runtime-token":"%s"}\n' "$token"
    ;;
  "compose -p")
    touch "$FAKE_STATE/compose-ran"
    ;;
esac
exit 0
"#;

    for foreign_kind in ["container", "network", "volume", "claim"] {
        let project = Project::initialized();
        fs::write(
            project.repo.join("docker-compose.yml"),
            r#"services:
  web:
    image: nginx:alpine
    ports: ["127.0.0.1:${WEB_PORT}:80"]
    volumes: [cache:/cache]
  postgres:
    image: postgres:16-alpine
    ports: ["127.0.0.1:${POSTGRES_PORT}:5432"]
volumes:
  cache: {}
"#,
        )
        .unwrap();
        git(&project.repo, &["add", "docker-compose.yml"]);
        git(
            &project.repo,
            &["commit", "-m", "add managed volume fixture"],
        );
        let mut config = load_config(&project.repo.join("stackstead.yaml"));
        config["database"]["postgres"] = serde_yaml::Value::Null;
        config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
        project.write_config(&config, "disable runtime readiness fixture");
        let manifest = project.create("feature-a");
        let state = project.repo.parent().unwrap().join("fake-docker-state");
        fs::create_dir(&state).unwrap();
        fs::write(state.join("foreign-resource"), foreign_kind).unwrap();
        let foreign_name = match foreign_kind {
            "container" => format!("{}-web-1", manifest.compose_project),
            "network" => format!("{}_default", manifest.compose_project),
            "volume" => format!("{}_cache", manifest.compose_project),
            "claim" => String::new(),
            _ => unreachable!(),
        };
        if foreign_kind == "claim" {
            fs::write(state.join("claim-token"), "foreign-runtime-token").unwrap();
        }
        let path = fake_docker_path(project.repo.parent().unwrap(), "ownership-bin", DOCKER);
        let rejected = stackstead(&project.repo)
            .env("PATH", path)
            .env("FAKE_STATE", &state)
            .env("EXPECTED_TOKEN", &manifest.runtime_token)
            .env("FOREIGN_KIND", foreign_kind)
            .env("FOREIGN_NAME", &foreign_name)
            .args(["up", &manifest.stackstead_id])
            .assert()
            .failure();
        let error = output_text(&rejected.get_output().stderr);
        assert!(
            error.contains("foreign") || error.contains("not owned"),
            "unexpected {foreign_kind} error: {error}"
        );
        assert!(!state.join("compose-ran").exists());
        assert_eq!(
            fs::read_to_string(state.join("foreign-resource")).unwrap(),
            foreign_kind
        );
        if foreign_kind == "container" {
            assert!(
                fs::read_to_string(state.join("commands"))
                    .unwrap()
                    .contains("container ls --all --format {{.Names}}"),
                "stopped containers must be included in exact-name ownership checks"
            );
        }
        if foreign_kind == "claim" {
            assert_eq!(
                fs::read_to_string(state.join("claim-token")).unwrap(),
                "foreign-runtime-token"
            );
        }
    }

    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["database"]["postgres"] = serde_yaml::Value::Null;
    config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
    project.write_config(&config, "disable runtime readiness fixture");
    let manifest = project.create("feature-a");
    let state = project.repo.parent().unwrap().join("owned-docker-state");
    let path = fake_docker_path(project.repo.parent().unwrap(), "owned-bin", DOCKER);
    for _ in 0..2 {
        stackstead(&project.repo)
            .env("PATH", &path)
            .env("FAKE_STATE", &state)
            .env("EXPECTED_TOKEN", &manifest.runtime_token)
            .args(["up", &manifest.stackstead_id])
            .assert()
            .success();
    }
    assert!(state.join("claim-token").is_file());
    stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(!state.join("claim-token").exists());
    let commands = fs::read_to_string(state.join("commands")).unwrap();
    assert_eq!(commands.matches("compose -p").count(), 2);
    assert!(commands.contains("up -d"));
    assert!(!commands.contains("down -v --remove-orphans"));
}

#[cfg(unix)]
#[test]
fn ownership_checks_exact_and_project_labeled_inventories_without_short_circuiting() {
    const DOCKER: &str = r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
printf '%s\n' "$*" >> "$FAKE_STATE/commands"
last=
for argument in "$@"; do last=$argument; done
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "${1-} ${2-}" in
  "container ls")
    case " $* " in *" --filter "*) printf '%s\n' orphan-id;; *) printf '%s\n' "$COMPOSE_PROJECT_NAME-web-1";; esac
    ;;
  "container inspect")
    if test "$last" = orphan-id; then token=foreign; else token="$EXPECTED_TOKEN"; fi
    printf '{"io.stackstead.runtime-token":"%s"}\n' "$token"
    ;;
  "network ls") ;;
  "volume ls") printf '%s\n' "$claim" ;;
  "volume inspect") printf '{"io.stackstead.runtime-token":"%s"}\n' "$EXPECTED_TOKEN" ;;
  "compose -p") touch "$FAKE_STATE/compose-ran" ;;
esac
"#;
    let project = Project::initialized();
    let manifest = project.create("feature-a");
    let state = project.repo.parent().unwrap().join("dual-inventory-state");
    fs::create_dir(&state).unwrap();
    let path = fake_docker_path(project.repo.parent().unwrap(), "dual-inventory-bin", DOCKER);
    let rejected = stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .failure();
    assert!(output_text(&rejected.get_output().stderr).contains("foreign"));
    assert!(!state.join("compose-ran").exists());
}

#[cfg(unix)]
#[test]
fn destroy_removes_reverified_residual_owned_resources() {
    const DOCKER: &str = r#"#!/bin/sh
set -eu
mkdir -p "$FAKE_STATE"
kind=${1-}
verb=${2-}
last=
for argument in "$@"; do last=$argument; done
claim="$COMPOSE_PROJECT_NAME-stackstead-claim"
case "$kind $verb" in
  "container ls"|"network ls") ;;
  "volume ls")
    test -f "$FAKE_STATE/claim" && printf '%s\n' "$claim"
    test -f "$FAKE_STATE/residual" && printf '%s\n' "$COMPOSE_PROJECT_NAME-retired"
    ;;
  "volume create")
    : > "$FAKE_STATE/claim"
    printf '%s\n' "$claim"
    ;;
  "volume inspect")
    printf '{"io.stackstead.runtime-token":"%s"}\n' "$EXPECTED_TOKEN"
    ;;
  "volume rm")
    if test "$last" = "$claim"; then rm -f "$FAKE_STATE/claim"; else rm -f "$FAKE_STATE/residual"; fi
    ;;
  "compose -p")
    case " $* " in
      *" down -v --remove-orphans "*) : > "$FAKE_STATE/residual" ;;
    esac
    ;;
esac
exit 0
"#;

    let project = Project::initialized();
    let mut config = load_config(&project.repo.join("stackstead.yaml"));
    config["database"]["postgres"] = serde_yaml::Value::Null;
    config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
    project.write_config(&config, "disable runtime readiness fixture");
    let manifest = project.create("feature-a");
    let state = project.repo.parent().unwrap().join("residual-docker-state");
    let path = fake_docker_path(project.repo.parent().unwrap(), "residual-bin", DOCKER);
    stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["up", &manifest.stackstead_id])
        .assert()
        .success();

    stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", &state)
        .env("EXPECTED_TOKEN", &manifest.runtime_token)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(!manifest.stackstead_root.exists());
    assert!(!state.join("claim").exists());
}

#[cfg(unix)]
#[test]
fn stop_and_destroy_without_runtime_resources_skip_compose_and_claim_removal() {
    let project = Project::initialized();
    let manifest = project.create("never-started");
    let state = project.repo.parent().unwrap().join("empty-docker-state");
    let path = fake_docker_path(
        project.repo.parent().unwrap(),
        "empty-runtime-bin",
        "#!/bin/sh\ncase \"$1 $2\" in 'container ls'|'network ls'|'volume ls') exit 0;; esac\nexit 97\n",
    );
    stackstead(&project.repo)
        .env("PATH", &path)
        .env("FAKE_STATE", &state)
        .args(["stop", &manifest.stackstead_id])
        .assert()
        .success();
    stackstead(&project.repo)
        .env("PATH", path)
        .env("FAKE_STATE", state)
        .args(["destroy", &manifest.stackstead_id, "--yes"])
        .assert()
        .success();
    assert!(!manifest.stackstead_root.exists());
}
