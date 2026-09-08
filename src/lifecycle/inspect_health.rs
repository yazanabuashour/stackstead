use std::collections::BTreeMap;

use crate::{
    compose, config::StacksteadConfig, health, manifest::StacksteadManifest,
    template::render_template,
};

use super::contract::template_context;

pub(super) fn observed_passive_health(
    config: &StacksteadConfig,
    manifest: &StacksteadManifest,
    services: &[compose::ServiceObservation],
) -> anyhow::Result<Option<bool>> {
    if config.health.checks.is_empty()
        || config.health.checks.iter().any(|check| check.url.is_none())
    {
        return Ok(None);
    }
    for check in &config.health.checks {
        let Some(template) = check.url.as_deref() else {
            return Ok(None);
        };
        let Some((endpoint, target)) = correlate_health_target(config, manifest, template)? else {
            continue;
        };
        if !services
            .iter()
            .any(|service| service.service == target.service && service.state == "running")
            || !compose::endpoint_is_published(
                manifest,
                &target.service,
                target.container_port,
                &endpoint.endpoint.host,
                endpoint.endpoint.port,
            )?
        {
            return Ok(Some(false));
        }
    }
    Ok(health::healthy_passive(
        &config.health,
        manifest,
        &BTreeMap::new(),
    ))
}

fn correlate_health_target(
    config: &StacksteadConfig,
    manifest: &StacksteadManifest,
    template: &str,
) -> anyhow::Result<Option<(crate::open::LaunchEndpoint, compose::ComposePortTarget)>> {
    let url = render_template(template, &template_context(manifest))?;
    let Ok(endpoint) = crate::open::manifest_endpoint(&url, manifest) else {
        return Ok(None);
    };
    // Once a URL maps to a manifest port, a broken Compose mapping must not
    // downgrade it to an uncorrelated application-health target.
    let target = compose::resolve_port_target(
        &manifest.compose_files,
        &manifest.container_ports,
        &config.env.generate,
        &endpoint.contract_key,
    )?;
    Ok(Some((endpoint, target)))
}
