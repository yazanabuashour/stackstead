use serde::Deserialize;

use crate::manifest::StacksteadManifest;

use super::{
    docker::run_docker_control,
    model::{ServiceObservation, is_sha256},
};

// Project-filtered inventory is not ownership proof. Capture identity and runtime fields
// together, then check both ownership labels and the exact immutable inventory ID.
// Never request container environment, health logs, or arbitrary labels.
const INSPECT_TEMPLATE: &str = concat!(
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

pub(super) fn inspect_owned_container(
    manifest: &StacksteadManifest,
    identifier: &str,
    deadline: Option<std::time::Instant>,
) -> anyhow::Result<ServiceObservation> {
    if !is_sha256(identifier) {
        anyhow::bail!("Docker container inventory did not provide a full immutable ID");
    }
    let args = vec![
        "container".into(),
        "inspect".into(),
        "--format".into(),
        INSPECT_TEMPLATE.into(),
        identifier.into(),
    ];
    let output = run_docker_control(manifest, &args, deadline)?;
    parse_owned_container(
        &output.stdout,
        identifier,
        &manifest.compose_project,
        &manifest.runtime_token,
    )
}

fn parse_owned_container(
    output: &[u8],
    identifier: &str,
    project: &str,
    runtime_token: &str,
) -> anyhow::Result<ServiceObservation> {
    let metadata: ContainerMetadata = serde_json::from_slice(output).map_err(|_error| {
        anyhow::anyhow!("Docker returned invalid container metadata; content withheld")
    })?;
    if !is_sha256(&metadata.id) || metadata.id != identifier {
        anyhow::bail!("Docker container inspection did not match the immutable inventory ID");
    }
    if metadata.project.as_deref() != Some(project)
        || metadata.runtime_token.as_deref() != Some(runtime_token)
    {
        anyhow::bail!(
            "refusing foreign container evidence: project or ownership label is missing or mismatched"
        );
    }
    let container = metadata.container.trim_start_matches('/').to_owned();
    if container.is_empty() {
        anyhow::bail!("Docker container inspection did not provide a container name");
    }
    Ok(ServiceObservation {
        // Keep unattributed owned containers visible. The evaluator cannot assume they
        // are optional work or silently remove them from the instance inventory.
        service: metadata.service.unwrap_or_default(),
        container,
        id: metadata.id,
        state: metadata.state.unwrap_or_default(),
        exit_code: metadata.exit_code,
        health: metadata.health.filter(|value| !value.is_empty()),
        healthcheck_enabled: metadata.healthcheck_enabled,
        oneoff: metadata.oneoff.as_deref().and_then(parse_oneoff),
        container_number: metadata.container_number.as_deref().and_then(parse_ordinal),
        config_hash: metadata.config_hash.filter(|value| is_sha256(value)),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContainerMetadata {
    id: String,
    container: String,
    project: Option<String>,
    runtime_token: Option<String>,
    service: Option<String>,
    state: Option<String>,
    exit_code: Option<i64>,
    health: Option<String>,
    healthcheck_enabled: Option<bool>,
    oneoff: Option<String>,
    container_number: Option<String>,
    config_hash: Option<String>,
}

const fn parse_oneoff(value: &str) -> Option<bool> {
    if value.eq_ignore_ascii_case("true") {
        Some(true)
    } else if value.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

fn parse_ordinal(value: &str) -> Option<u64> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().filter(|ordinal| *ordinal > 0)
}

#[cfg(test)]
#[path = "observations_tests.rs"]
mod tests;
