use std::collections::BTreeMap;

use anyhow::Context as _;

use crate::{
    compose,
    config::HealthConfig,
    health,
    manifest::{ComponentStatus, StacksteadManifest},
    readiness::{self, Contract, ReadinessStatus, Requirements},
};

pub(super) fn capture_profiles() -> anyhow::Result<Option<String>> {
    match std::env::var("COMPOSE_PROFILES") {
        Ok(profiles) => Ok(Some(profiles)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("COMPOSE_PROFILES is not UTF-8; cannot capture startup profile selection")
        }
    }
}

pub(super) fn resolve(
    manifest: &StacksteadManifest,
    profiles: Option<&str>,
) -> anyhow::Result<Option<Requirements>> {
    manifest
        .readiness
        .required()
        .map(|required| compose::resolve_requirements(manifest, required, profiles, None))
        .transpose()
}

pub(super) fn wait(
    config: &HealthConfig,
    manifest: &mut StacksteadManifest,
    environment: &BTreeMap<String, String>,
    expected: Option<&Requirements>,
) -> anyhow::Result<()> {
    if manifest.readiness.required().is_none() {
        let result = health::wait(config, manifest, environment);
        manifest.status.health = application_status(!config.checks.is_empty(), result.is_ok());
        return result;
    }
    let expected = expected.ok_or_else(|| {
        anyhow::anyhow!("runtime readiness was not resolved before Compose startup")
    })?;
    // One final deadline starts after hooks. Model verification and observations
    // consume the same budget as application probes; polling never resets it.
    let deadline = health::deadline(config)?;
    verify_current(manifest, expected, deadline)?;
    if let Contract::Declared { resolved, .. } = &mut manifest.readiness {
        *resolved = Some(expected.clone());
    }
    manifest.save_atomic()?;
    let mut report = readiness::evaluate(&manifest.readiness, Some(expected), None);
    let mut failed = Vec::new();
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(timeout_error(config, &report, &failed));
        }
        failed = health::failed_checks(config, manifest, environment, deadline);
        manifest.status.health = application_status(!config.checks.is_empty(), failed.is_empty());
        match observe_readiness(manifest, expected, deadline) {
            Ok(current) => report = current,
            Err(error) if std::time::Instant::now() >= deadline => {
                anyhow::bail!(
                    "{}; latest observation failed: {error:#}",
                    timeout_error(config, &report, &failed)
                );
            }
            Err(error) => return Err(error),
        }
        let expired = std::time::Instant::now() >= deadline;
        if report.status == ReadinessStatus::Ready && failed.is_empty() && !expired {
            return Ok(());
        }
        if expired {
            return Err(timeout_error(config, &report, &failed));
        }
        health::pause(config, deadline);
    }
}

fn observe_readiness(
    manifest: &mut StacksteadManifest,
    expected: &Requirements,
    deadline: std::time::Instant,
) -> anyhow::Result<readiness::ReadinessReport> {
    let snapshot =
        compose::service_observations(manifest, Some(deadline)).inspect_err(|_error| {
            manifest.status.runtime = ComponentStatus::Unknown;
        })?;
    let readiness = readiness::evaluate(
        &manifest.readiness,
        Some(expected),
        Some(snapshot.evidence()),
    );
    manifest.status.runtime = snapshot.status();
    // Application probes can execute commands. Reject model drift after those probes
    // and observations before accepting a pass.
    verify_current(manifest, expected, deadline)?;
    Ok(readiness)
}

fn verify_current(
    manifest: &StacksteadManifest,
    expected: &Requirements,
    deadline: std::time::Instant,
) -> anyhow::Result<()> {
    let current = manifest
        .readiness
        .required()
        .map(|required| {
            compose::resolve_requirements(
                manifest,
                required,
                expected.profiles.as_deref(),
                Some(deadline),
            )
        })
        .transpose()
        .context("cannot verify effective Compose inputs after startup")?;
    if current.as_ref() != Some(expected) {
        anyhow::bail!(
            "effective Compose inputs changed after startup; readiness remains unresolved; rerun stackstead up once inputs are stable"
        );
    }
    Ok(())
}

const fn application_status(configured: bool, passed: bool) -> ComponentStatus {
    if !configured {
        ComponentStatus::Unknown
    } else if passed {
        ComponentStatus::Ready
    } else {
        ComponentStatus::Failed
    }
}

fn timeout_error(
    config: &HealthConfig,
    report: &readiness::ReadinessReport,
    failed: &[String],
) -> anyhow::Error {
    let mut diagnostics = Vec::new();
    if report.status != ReadinessStatus::Ready {
        diagnostics.push(format!(
            "last runtime readiness {}: {}",
            report.status,
            report.issues.join("; ")
        ));
    }
    if !failed.is_empty() {
        diagnostics.push(format!(
            "application health checks failed: {}",
            failed.join(", ")
        ));
    }
    if diagnostics.is_empty() {
        diagnostics.push("shared verification deadline expired".into());
    }
    anyhow::anyhow!(
        "startup verification did not pass within {}s: {}",
        config.timeout_seconds,
        diagnostics.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_status_requires_checks_and_only_depends_on_their_result() {
        assert_eq!(application_status(false, true), ComponentStatus::Unknown);
        assert_eq!(application_status(false, false), ComponentStatus::Unknown);
        assert_eq!(application_status(true, true), ComponentStatus::Ready);
        assert_eq!(application_status(true, false), ComponentStatus::Failed);
    }
}
