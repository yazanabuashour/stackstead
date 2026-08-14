use std::collections::BTreeSet;

use crate::manifest::StacksteadManifest;

use super::{
    claim::runtime_claim_name,
    docker::run_docker_control,
    model::{COMPOSE_PROJECT_LABEL, RUNTIME_TOKEN_LABEL},
    runtime_names::expected_runtime_names,
};

pub(super) fn verify_runtime_resources(manifest: &StacksteadManifest) -> anyhow::Result<bool> {
    let expected = verify_expected_runtime_names(manifest)?;
    let labeled = verify_labeled_runtime_resources(manifest)?;
    Ok(expected || labeled)
}

pub(super) fn remove_labeled_runtime_resources(
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    for (kind, list_args, identifier_template, labels_template, remove_args) in [
        (
            "container",
            vec!["container", "ls", "--all"],
            "{{.ID}}",
            ".Config.Labels",
            vec!["container", "rm", "--force"],
        ),
        (
            "network",
            vec!["network", "ls"],
            "{{.ID}}",
            ".Labels",
            vec!["network", "rm"],
        ),
        (
            "volume",
            vec!["volume", "ls"],
            "{{.Name}}",
            ".Labels",
            vec!["volume", "rm"],
        ),
    ] {
        let mut args = list_args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        args.extend([
            "--filter".into(),
            format!("label={COMPOSE_PROJECT_LABEL}={}", manifest.compose_project),
            "--format".into(),
            identifier_template.into(),
        ]);
        let output = run_docker_control(manifest, &args)?;
        for identifier in String::from_utf8(output.stdout)?
            .lines()
            .map(str::trim)
            .filter(|identifier| !identifier.is_empty())
        {
            if kind == "volume" && identifier == runtime_claim_name(manifest) {
                continue;
            }
            verify_resource_label(manifest, kind, identifier, labels_template)?;
            let mut args = remove_args
                .iter()
                .map(|value| (*value).into())
                .collect::<Vec<_>>();
            args.push(identifier.into());
            run_docker_control(manifest, &args)?;
        }
    }
    Ok(())
}

pub(super) fn verify_labeled_runtime_resources(
    manifest: &StacksteadManifest,
) -> anyhow::Result<bool> {
    let mut present = false;
    for (kind, list_args, identifier_template, labels_template) in [
        (
            "container",
            vec!["container", "ls", "--all"],
            "{{.ID}}",
            ".Config.Labels",
        ),
        ("network", vec!["network", "ls"], "{{.ID}}", ".Labels"),
        ("volume", vec!["volume", "ls"], "{{.Name}}", ".Labels"),
    ] {
        let mut args = list_args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        args.extend([
            "--filter".into(),
            format!("label={COMPOSE_PROJECT_LABEL}={}", manifest.compose_project),
            "--format".into(),
            identifier_template.into(),
        ]);
        let output = run_docker_control(manifest, &args)?;
        let identifiers = String::from_utf8(output.stdout)?;
        for identifier in identifiers
            .lines()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            if kind == "volume" && identifier == runtime_claim_name(manifest) {
                continue;
            }
            present = true;
            verify_resource_label(manifest, kind, identifier, labels_template)?;
        }
    }
    Ok(present)
}

fn verify_expected_runtime_names(manifest: &StacksteadManifest) -> anyhow::Result<bool> {
    let mut present = false;
    for (kind, format, labels_template, expected) in expected_runtime_names(manifest)? {
        let mut args = vec![kind.clone(), "ls".into()];
        if kind == "container" {
            args.push("--all".into());
        }
        args.extend(["--format".into(), format]);
        let output = run_docker_control(manifest, &args)?;
        let names = String::from_utf8(output.stdout)?;
        let actual = names
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect::<BTreeSet<_>>();
        for name in expected
            .iter()
            .filter(|name| actual.contains(name.as_str()))
        {
            present = true;
            verify_resource_label(manifest, &kind, name, &labels_template)?;
        }
    }
    Ok(present)
}

pub(super) fn verify_resource_label(
    manifest: &StacksteadManifest,
    kind: &str,
    identifier: &str,
    labels_template: &str,
) -> anyhow::Result<()> {
    let args = vec![
        kind.into(),
        "inspect".into(),
        "--format".into(),
        format!("{{{{json {labels_template}}}}}"),
        identifier.into(),
    ];
    let output = run_docker_control(manifest, &args)?;
    let labels: serde_json::Value =
        serde_json::from_slice(output.stdout.trim_ascii()).map_err(|error| {
            anyhow::anyhow!("Docker returned invalid labels for {kind} `{identifier}`: {error}")
        })?;
    let token = labels
        .as_object()
        .and_then(|labels| labels.get(RUNTIME_TOKEN_LABEL))
        .and_then(serde_json::Value::as_str);
    if token != Some(manifest.runtime_token.as_str()) {
        anyhow::bail!(
            "refusing to target foreign {kind} `{identifier}` in Compose project `{}`: ownership label is missing or mismatched",
            manifest.compose_project
        );
    }
    Ok(())
}
