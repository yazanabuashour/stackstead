use std::io::Write;

use crate::manifest::StacksteadManifest;

pub fn write_agent_context(manifest: &StacksteadManifest, rules: &[String]) -> anyhow::Result<()> {
    let content = render_agent_context(manifest, rules);
    let parent = manifest
        .agent_context
        .parent()
        .ok_or_else(|| anyhow::anyhow!("agent context path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(content.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&manifest.agent_context)
        .map_err(|error| error.error)?;
    Ok(())
}

pub fn render_agent_context(manifest: &StacksteadManifest, rules: &[String]) -> String {
    let mut output = format!(
        "# Stackstead: {}\n\nProject: {}  \nBranch: {}  \nWorktree: {}  \nCompose project: {}\n\n\
         ## Runtime Contract\n\nThis stackstead owns this source checkout, Compose project, env file, ports, database state, and logs. Its runtime identity and state are isolated from peer stacksteads.\n\n\
         Do not use shared development ports or shared development databases while working in this stackstead.\n\n\
         ## URLs and Ports\n\n",
        manifest.stackstead_id,
        manifest.project,
        manifest.branch,
        manifest.worktree.display(),
        manifest.compose_project
    );
    for (service, port) in &manifest.ports {
        if let Some(url) = manifest.urls.get(service) {
            output.push_str(&format!("- {service}: {url}\n"));
        } else {
            output.push_str(&format!("- {service}: 127.0.0.1:{port}\n"));
        }
    }
    output.push_str("\n## Database\n\n");
    if let Some(database) = &manifest.database {
        output.push_str(&format!(
            "- Service: {}\n- Strategy: {}\n- Endpoint: {}:{}\n- Database: {}\n- Credentials: use the generated environment; secrets are not copied into this context.\n",
            database.service,
            database.strategy,
            database.host,
            database.port,
            database.database
        ));
    } else {
        output.push_str("Not configured.\n");
    }
    output.push_str(&format!(
        "\n## Environment\n\nGenerated env file:\n\n{}\n\n\
         ## Files\n\n- Manifest: {}\n- Event log: {}\n- Pointer: {}\n\n\
         ## Rules\n\n",
        manifest.env_file.display(),
        manifest.manifest_path().display(),
        manifest.event_log.display(),
        manifest.pointer_file.display()
    ));
    for rule in rules {
        if manifest.database.is_none() && rule.contains("db status") {
            continue;
        }
        output.push_str(&format!("- {rule}\n"));
    }
    output.push_str("\n## Exact Commands\n\n### Inspect\n\n```sh\n");
    output.push_str(&format!("stackstead inspect {}\n", manifest.stackstead_id));
    output.push_str(&format!(
        "stackstead context {} --print\n",
        manifest.stackstead_id
    ));
    output.push_str("```\n\n### Logs\n\n```sh\n");
    output.push_str(&format!(
        "stackstead logs {} --tail 200\n",
        manifest.stackstead_id
    ));
    if manifest.database.is_some() {
        output.push_str(&format!(
            "stackstead db status {}\n",
            manifest.stackstead_id
        ));
    }
    output.push_str("```\n\n### URLs\n\n```sh\n");
    for service in manifest.urls.keys() {
        output.push_str(&format!(
            "stackstead open {} {service} --print\n",
            manifest.stackstead_id
        ));
    }
    output.push_str("```\n\n### Recovery\n\n`up` may rerun configured dependency installation, database seeding, and lifecycle hooks.\n\n```sh\n");
    output.push_str(&format!("stackstead up {}\n", manifest.stackstead_id));
    output.push_str(&format!("stackstead repair {}\n", manifest.stackstead_id));
    output.push_str("```\n\n### Exact teardown\n\n```sh\n");
    output.push_str(&format!("stackstead stop {}\n", manifest.stackstead_id));
    output.push_str(&format!(
        "stackstead destroy {} --yes\n```\n",
        manifest.stackstead_id
    ));
    output
}

#[cfg(test)]
mod tests {
    use crate::manifest::StacksteadManifest;
    use crate::test_support::TestResultExt as _;

    use super::*;

    fn manifest(database: bool, urls: serde_json::Value) -> anyhow::Result<StacksteadManifest> {
        let mut value = serde_json::json!({
            "kind":"StacksteadManifest","version":"2","stackstead_id":"a-b123","slug":"a","short_id":"b123",
            "runtime_token":"0123456789abcdef0123456789abcdef",
            "project":"demo","branch":"a","base":"main","repo_root":"/repo","project_state_root":"/state",
            "source_ownership":"stackstead",
            "stackstead_root":"/state/demo/a-b123","worktree":"/state/demo/a-b123/source","state_dir":"/state/demo/a-b123/state",
            "compose_project":"demo-a-b123","compose_files":["/state/demo/a-b123/source/compose.yml"],
            "ports":{"dashboard":39000},"container_ports":{"dashboard":3000},"urls":urls,
            "env_file":"/state/demo/a-b123/source/.stackstead/.env","agent_context":"/x","pointer_file":"/y","event_log":"/z","env_keys":[],
            "status":{"source":"created","dependencies":"unknown","runtime":"stopped","database":"unknown","health":"unknown"},
            "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"
        });
        if database {
            value["database"] = serde_json::json!({
                "strategy":"compose-volume","service":"postgres","host":"127.0.0.1","port":39001,
                "database":"app","seed_status":"unknown"
            });
        }
        serde_json::from_value(value).test_context("parse manifest fixture")
    }

    #[test]
    fn useful_commands_follow_the_manifest_services() -> anyhow::Result<()> {
        let without_database = render_agent_context(
            &manifest(false, serde_json::json!({}))?,
            &["Run stackstead db status before migrations.".into()],
        );
        assert!(!without_database.contains("db status"));
        assert!(!without_database.contains("open a web"));

        let configured = render_agent_context(
            &manifest(
                true,
                serde_json::json!({
                    "api":"http://127.0.0.1:39001",
                    "dashboard":"http://127.0.0.1:39000"
                }),
            )?,
            &[],
        );
        assert!(configured.contains("stackstead db status a-b123\n```"));
        assert!(configured.contains("Service: postgres"));
        assert!(configured.contains("Endpoint: 127.0.0.1:39001"));
        assert!(configured.contains("Database: app"));
        assert!(configured.contains("stackstead open a-b123 api --print"));
        assert!(configured.contains("stackstead open a-b123 dashboard --print"));
        assert!(configured.contains("stackstead context a-b123 --print"));
        assert!(configured.contains("stackstead up a-b123"));
        assert!(configured.contains("may rerun configured dependency installation"));
        assert!(configured.contains("stackstead repair a-b123"));
        assert!(configured.contains("stackstead stop a-b123"));
        assert!(configured.contains("stackstead destroy a-b123 --yes"));
        assert!(!configured.contains("stackstead inspect a\n"));
        assert!(!configured.contains("postgres://"));
        Ok(())
    }
}
