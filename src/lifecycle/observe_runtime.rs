use crate::{
    compose,
    manifest::{ComponentStatus, StacksteadManifest},
    readiness::{self, ReadinessReport},
};

#[derive(Debug, Clone)]
pub struct RuntimeObservation {
    pub snapshot: Option<compose::RuntimeSnapshot>,
    pub readiness: ReadinessReport,
    pub issues: Vec<String>,
}

impl RuntimeObservation {
    pub fn evidence(&self) -> Option<&[compose::ServiceObservation]> {
        self.snapshot
            .as_ref()
            .map(compose::RuntimeSnapshot::evidence)
    }

    pub fn activity(&self) -> &'static str {
        self.snapshot
            .as_ref()
            .map_or("unknown", compose::RuntimeSnapshot::activity)
    }

    pub fn running(&self) -> Option<bool> {
        self.snapshot
            .as_ref()
            .and_then(compose::RuntimeSnapshot::running)
    }

    pub fn service_status(&self, name: &str) -> ComponentStatus {
        self.snapshot
            .as_ref()
            .map_or(ComponentStatus::Unknown, |snapshot| {
                snapshot.service_status(name)
            })
    }

    pub fn status(&self) -> ComponentStatus {
        self.snapshot
            .as_ref()
            .map_or(ComponentStatus::Unknown, compose::RuntimeSnapshot::status)
    }
}

// Callers validate the durable manifest binding before observing its namespace.
pub fn observe_runtime(manifest: &StacksteadManifest) -> RuntimeObservation {
    let mut issues = Vec::new();
    let mut current = match (manifest.readiness.required(), manifest.readiness.resolved()) {
        (Some(required), Some(resolved)) => {
            match compose::resolve_requirements(
                manifest,
                required,
                resolved.profiles.as_deref(),
                None,
            ) {
                Ok(requirements) => Some(requirements),
                Err(error) => {
                    issues.push(format!(
                        "could not verify effective Compose requirements: {error}"
                    ));
                    None
                }
            }
        }
        _ => None,
    };
    let snapshot = match compose::service_observations(manifest, None) {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            issues.push(format!("could not inspect Docker runtime: {error}"));
            None
        }
    };
    if current.is_some() {
        match StacksteadManifest::read(&manifest.manifest_path()) {
            Ok(latest)
                if latest.updated_at == manifest.updated_at
                    && latest.readiness == manifest.readiness => {}
            Ok(_) => {
                issues.push("manifest changed during runtime observation; retry inspection".into());
                current = None;
            }
            Err(error) => {
                issues.push(format!(
                    "could not revalidate the observed manifest: {error}"
                ));
                current = None;
            }
        }
    }
    let readiness = readiness::evaluate(
        &manifest.readiness,
        current.as_ref(),
        snapshot.as_ref().map(compose::RuntimeSnapshot::evidence),
    );
    RuntimeObservation {
        snapshot,
        readiness,
        issues,
    }
}

#[cfg(test)]
#[path = "observe_runtime_tests.rs"]
mod tests;
