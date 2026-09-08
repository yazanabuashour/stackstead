use super::super::*;
use super::fixture::{OwnedDocker, row};

pub(super) struct ReadinessFixture {
    pub project: Project,
    pub manifest: StacksteadManifest,
    pub docker: OwnedDocker,
    pub model: Value,
    pub rows: Vec<Value>,
}

impl ReadinessFixture {
    pub fn new(required: Value) -> anyhow::Result<Self> {
        let project = Project::initialized()?;
        let compose = project.repo.join("docker-compose.yml");
        let mut source = fs::read_to_string(&compose).test()?;
        source.push_str("  worker:\n    image: busybox:1.36\n    scale: 2\n  migrate:\n    image: busybox:1.36\n  optional:\n    image: busybox:1.36\n");
        fs::write(compose, source).test()?;
        git(&project.repo, &["add", "docker-compose.yml"])?;
        git(&project.repo, &["commit", "-m", "add readiness services"])?;
        let mut config = load_config(&project.repo.join("stackstead.yaml"))?;
        config["database"]["postgres"] = serde_yaml::Value::Null;
        config["health"]["checks"] = serde_yaml::Value::Sequence(vec![]);
        config["health"]["timeout_seconds"] = 1.into();
        config["health"]["interval_millis"] = 10.into();
        if !required.is_null() {
            config["runtime"]["readiness"] =
                serde_yaml::to_value(std::collections::BTreeMap::from([("required", required)]))
                    .test()?;
        }
        config["hooks"]["pre_up"] = serde_yaml::to_value([serde_json::json!({
            "command": "if grep -q '\"resolved\"' \"$FAKE_MANIFEST\"; then exit 83; fi; test ! -f \"$FAKE_STATE/fail-pre\"",
            "shell": true,
        })]).test()?;
        config["hooks"]["post_up"] = serde_yaml::to_value([serde_json::json!({
            "command": "if test -f \"$FAKE_STATE/after-model.json\"; then cp \"$FAKE_STATE/after-model.json\" \"$FAKE_STATE/model.json\"; fi",
            "shell": true,
        })]).test()?;
        project.write_config(&config, "declare readiness contract")?;
        let manifest = project.create("feature-a")?;
        let docker = OwnedDocker::new(&manifest)?;
        let mut model = serde_json::json!({"services": {
            "web": {"image": "nginx:alpine"},
            "postgres": {"image": "postgres:16-alpine"},
            "worker": {"image": "busybox:1.36", "scale": 2},
            "migrate": {"image": "busybox:1.36"},
            "optional": {"image": "busybox:1.36"},
        }});
        for service in model["services"].as_object_mut().test()?.values_mut() {
            service["labels"] =
                serde_json::json!({"io.stackstead.runtime-token": manifest.runtime_token});
        }
        docker.model(&model)?;
        let rows = vec![
            row(&manifest, "worker", 2, 2, "running", 0),
            row(&manifest, "worker", 3, 3, "running", 0),
            row(&manifest, "migrate", 1, 4, "exited", 0),
            row(&manifest, "optional", 1, 5, "exited", 7),
        ];
        docker.rows(&rows)?;
        Ok(Self {
            project,
            manifest,
            docker,
            model,
            rows,
        })
    }

    pub fn up(&self) -> anyhow::Result<StacksteadManifest> {
        let output = self
            .docker
            .command(&self.manifest)
            .args(["--json", "up", &self.manifest.stackstead_id])
            .assert()
            .success();
        self.docker.assert_supported()?;
        changed_manifest(&output.get_output().stdout, "started")
    }

    pub fn readings(&self) -> anyhow::Result<Value> {
        let inspect = self
            .docker
            .command(&self.manifest)
            .env(
                "COMPOSE_PROFILES",
                "caller-must-not-replace-startup-profiles",
            )
            .args(["--json", "inspect", &self.manifest.stackstead_id])
            .assert()
            .success();
        let inspect: Value = serde_json::from_slice(&inspect.get_output().stdout).test()?;
        let list = self
            .docker
            .command(&self.manifest)
            .env(
                "COMPOSE_PROFILES",
                "caller-must-not-replace-startup-profiles",
            )
            .args(["--json", "ps"])
            .assert()
            .success();
        let list: Value = serde_json::from_slice(&list.get_output().stdout).test()?;
        assert_eq!(inspect["kind"], "StacksteadInspection");
        assert_eq!(inspect["version"], "4");
        assert_eq!(list["kind"], "StacksteadList");
        assert_eq!(list["version"], "2");
        let item = &list["stacksteads"][0];
        assert_eq!(item["stackstead_id"], self.manifest.stackstead_id);
        assert_eq!(item["runtime"], inspect["live"]["runtime"]["activity"]);
        assert_eq!(item["readiness"], inspect["live"]["readiness"]);
        assert_eq!(item["services"], inspect["live"]["services"]);
        assert!(item["issues"].is_array());
        self.docker.assert_supported()?;
        Ok(inspect)
    }
}
