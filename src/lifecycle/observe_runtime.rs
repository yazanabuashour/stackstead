use crate::{
    compose,
    manifest::{ComponentStatus, StacksteadManifest},
    readiness::{self, ReadinessReport},
};

#[derive(Debug, Clone)]
pub struct RuntimeObservation {
    pub services: Option<Vec<compose::ServiceObservation>>,
    pub readiness: ReadinessReport,
    pub issues: Vec<String>,
}

impl RuntimeObservation {
    pub fn activity(&self) -> &'static str {
        let Some(services) = &self.services else {
            return "unknown";
        };
        if services
            .iter()
            .any(|service| matches!(service.state.as_str(), "running" | "paused" | "restarting"))
        {
            "active"
        } else if services
            .iter()
            .all(|service| matches!(service.state.as_str(), "created" | "exited" | "dead"))
        {
            "inactive"
        } else {
            "unknown"
        }
    }

    pub fn running(&self) -> Option<bool> {
        let services = self.services.as_ref()?;
        if services.iter().any(|service| service.state == "running") {
            Some(true)
        } else if services.iter().all(|service| {
            matches!(
                service.state.as_str(),
                "created" | "exited" | "dead" | "paused" | "restarting"
            )
        }) {
            Some(false)
        } else {
            None
        }
    }

    pub fn service_status(&self, name: &str) -> ComponentStatus {
        let Some(services) = &self.services else {
            return ComponentStatus::Unknown;
        };
        let mut status = ComponentStatus::Stopped;
        for service in services {
            if service.oneoff == Some(true) {
                continue;
            }
            if service.service.is_empty() || (service.service == name && service.oneoff.is_none()) {
                status = ComponentStatus::Unknown;
                continue;
            }
            if service.service != name {
                continue;
            }
            match service.state.as_str() {
                "running" => return ComponentStatus::Running,
                "created" | "exited" | "dead" | "paused" | "restarting" => {}
                _ => status = ComponentStatus::Unknown,
            }
        }
        status
    }

    pub fn status(&self) -> ComponentStatus {
        match self.running() {
            Some(true) => ComponentStatus::Running,
            Some(false) => ComponentStatus::Stopped,
            None => ComponentStatus::Unknown,
        }
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
    let services = match compose::service_observations(manifest, None) {
        Ok(services) => Some(services),
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
    let readiness = readiness::evaluate(&manifest.readiness, current.as_ref(), services.as_deref());
    RuntimeObservation {
        services,
        readiness,
        issues,
    }
}

#[cfg(test)]
#[path = "observe_runtime_tests.rs"]
mod tests;
