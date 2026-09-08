use super::super::*;
use std::fmt::Write as _;

pub(crate) const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

pub(crate) struct OwnedDocker {
    pub state: PathBuf,
    pub path: OsString,
}

impl OwnedDocker {
    pub fn new(manifest: &StacksteadManifest) -> anyhow::Result<Self> {
        let parent = manifest.repo_root.parent().test()?;
        let state = parent.join("owned-observations");
        fs::create_dir(&state).test()?;
        fs::write(state.join("claim"), &manifest.runtime_token).test()?;
        fs::write(state.join("containers"), "").test()?;
        fs::write(state.join("expected-profiles"), "<absent>").test()?;
        let base = format!(
            "compose -p {} --env-file {} {} -f {}",
            manifest.compose_project,
            manifest.env_file.display(),
            manifest
                .compose_files
                .iter()
                .map(|file| format!("-f {}", file.display()))
                .collect::<Vec<_>>()
                .join(" "),
            manifest
                .worktree
                .join(".stackstead/compose-ownership.yaml")
                .display(),
        );
        let script = include_str!("readiness_docker.sh")
            .replace("@COMPOSE_BASE@", &base)
            .replace("@PROJECT@", &manifest.compose_project)
            .replace("@TOKEN@", &manifest.runtime_token)
            .replace("@STATE@", &state.display().to_string())
            .replace("@METADATA_TEMPLATE@", METADATA_TEMPLATE)
            .replace(
                "@DATABASE_PORT@",
                &manifest
                    .ports
                    .get("postgres")
                    .copied()
                    .unwrap_or_default()
                    .to_string(),
            );
        let path = fake_docker_path(parent, "owned-observations-bin", &script)?;
        Ok(Self { state, path })
    }

    pub fn command(&self, manifest: &StacksteadManifest) -> Command {
        let mut command = stackstead(&manifest.repo_root);
        command
            .env("PATH", &self.path)
            .env("FAKE_STATE", &self.state)
            .env("FAKE_MANIFEST", manifest.manifest_path())
            .env_remove("COMPOSE_PROFILES");
        command
    }

    pub fn rows(&self, rows: &[Value]) -> anyhow::Result<()> {
        let mut inventory = String::new();
        for row in rows {
            let id = row["id"].as_str().test()?;
            let name = row["container"].as_str().test()?.trim_start_matches('/');
            let project = row["project"].as_str().unwrap_or_default();
            writeln!(inventory, "{id} {name} {project}").test()?;
            let mut labels =
                serde_json::json!({"io.stackstead.runtime-token": row["runtime_token"]});
            if let Some(project) = row.get("project") {
                labels["com.docker.compose.project"] = project.clone();
            }
            fs::write(
                self.state.join(format!("{id}.labels")),
                serde_json::to_vec(&labels).test()?,
            )
            .test()?;
            fs::write(self.state.join(id), serde_json::to_vec(row).test()?).test()?;
        }
        fs::write(self.state.join("containers"), inventory).test()?;
        let running = rows
            .iter()
            .filter(|row| row["state"] == "running")
            .map(|row| row["id"].as_str().test().map(|id| format!("{id}\n")))
            .collect::<anyhow::Result<String>>()?;
        fs::write(self.state.join("running"), running).test()?;
        Ok(())
    }

    pub fn model(&self, model: &Value) -> anyhow::Result<()> {
        fs::write(
            self.state.join("model.json"),
            serde_json::to_vec(model).test()?,
        )
        .test()?;
        let mut hashes = String::new();
        for service in model["services"].as_object().test()?.keys() {
            writeln!(hashes, "{service} {HASH}").test()?;
        }
        fs::write(self.state.join("hashes"), hashes).test()?;
        Ok(())
    }

    pub fn assert_supported(&self) -> anyhow::Result<()> {
        let unexpected = self.state.join("unexpected");
        assert!(
            !unexpected.try_exists().test()?,
            "unsupported Docker invocation: {}",
            fs::read_to_string(unexpected).unwrap_or_else(|error| error.to_string())
        );
        Ok(())
    }
}

pub(crate) fn row(
    manifest: &StacksteadManifest,
    service: &str,
    number: u64,
    identity: u64,
    state: &str,
    exit: i64,
) -> Value {
    serde_json::json!({
        "id": format!("{identity:064x}"),
        "container": format!("/{}-{service}-{number}", manifest.compose_project),
        "project": manifest.compose_project,
        "runtime_token": manifest.runtime_token,
        "service": service,
        "oneoff": "False",
        "container_number": number.to_string(),
        "config_hash": HASH,
        "state": state,
        "exit_code": exit,
        "health": null,
        "healthcheck_enabled": false,
    })
}

// Match the collector's projection, rather than accepting arbitrary Docker inspect templates.
const METADATA_TEMPLATE: &str = concat!(
    r#"{"id":{{json .Id}},"container":{{json .Name}},"#,
    r#""project":{{json (index .Config.Labels "com.docker.compose.project")}},"#,
    r#""runtime_token":{{json (index .Config.Labels "io.stackstead.runtime-token")}},"#,
    r#""service":{{json (index .Config.Labels "com.docker.compose.service")}},"#,
    r#""oneoff":{{json (index .Config.Labels "com.docker.compose.oneoff")}},"#,
    r#""container_number":{{json (index .Config.Labels "com.docker.compose.container-number")}},"#,
    r#""config_hash":{{json (index .Config.Labels "com.docker.compose.config-hash")}},"#,
    r#""state":{{with index . "State"}}{{json (index . "Status")}}{{else}}null{{end}},"#,
    r#""exit_code":{{with index . "State"}}{{json (index . "ExitCode")}}{{else}}null{{end}},"#,
    r#""health":{{with index . "State"}}{{with index . "Health"}}"#,
    r#"{{json (index . "Status")}}{{else}}null{{end}}{{else}}null{{end}},"#,
    r#""healthcheck_enabled":{{with index . "Config"}}{{with index . "Healthcheck"}}"#,
    r#"{{with index . "Test"}}{{if eq (index . 0) "NONE"}}false"#,
    r#"{{else if or (eq (index . 0) "CMD") (eq (index . 0) "CMD-SHELL")}}true"#,
    r#"{{else}}null{{end}}{{else}}false{{end}}{{else}}false{{end}}{{else}}null{{end}}}"#,
);
