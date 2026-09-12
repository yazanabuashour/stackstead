use crate::manifest::ComponentStatus;

use super::model::ServiceObservation;

/// Evidence collected after Compose ownership checks, not an atomic view or ongoing authorization.
#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    services: Vec<ServiceObservation>,
}

impl RuntimeSnapshot {
    pub(super) const fn new(services: Vec<ServiceObservation>) -> Self {
        Self { services }
    }

    pub fn evidence(&self) -> &[ServiceObservation] {
        &self.services
    }

    pub fn activity(&self) -> &'static str {
        if self
            .services
            .iter()
            .any(|service| matches!(service.state.as_str(), "running" | "paused" | "restarting"))
        {
            "active"
        } else if self
            .services
            .iter()
            .all(|service| matches!(service.state.as_str(), "created" | "exited" | "dead"))
        {
            "inactive"
        } else {
            "unknown"
        }
    }

    pub fn running(&self) -> Option<bool> {
        if self
            .services
            .iter()
            .any(|service| service.state == "running")
        {
            Some(true)
        } else if self.services.iter().all(|service| {
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
        let mut status = ComponentStatus::Stopped;
        for service in &self.services {
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

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
