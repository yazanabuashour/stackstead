use serde::Serialize;

use crate::{compose::ServiceObservation, readiness::ReadinessReport};

#[derive(Debug, Serialize)]
pub(super) struct ReadinessOutput {
    status: &'static str,
    required: Vec<RequiredServiceOutput>,
    issues: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RequiredServiceOutput {
    service: String,
    role: &'static str,
    expected_instances: Option<u64>,
    observed_containers: usize,
    satisfied_instances: usize,
    status: &'static str,
    issues: Vec<String>,
}

impl From<&ReadinessReport> for ReadinessOutput {
    fn from(report: &ReadinessReport) -> Self {
        Self {
            status: report.status.as_str(),
            required: report
                .required
                .iter()
                .map(|service| RequiredServiceOutput {
                    service: service.service.clone(),
                    role: service.role.as_str(),
                    expected_instances: service.expected_instances,
                    observed_containers: service.observed_containers,
                    satisfied_instances: service.satisfied_instances,
                    status: service.status.as_str(),
                    issues: service.issues.clone(),
                })
                .collect(),
            issues: report.issues.clone(),
        }
    }
}

impl ReadinessOutput {
    pub(super) const fn status(&self) -> &'static str {
        self.status
    }

    pub(super) fn issues(&self) -> impl Iterator<Item = &String> {
        self.issues.iter()
    }
}

#[derive(Debug, Serialize)]
pub(super) struct LiveServiceOutput {
    service: String,
    container: String,
    id: String,
    state: String,
    status: String,
    exit_code: Option<i64>,
    health: Option<String>,
    healthcheck_enabled: Option<bool>,
    oneoff: Option<bool>,
    container_number: Option<u64>,
    config_hash: Option<String>,
}

impl LiveServiceOutput {
    pub(super) fn summary(&self) -> String {
        let health = self.health.as_deref().unwrap_or_else(|| {
            if self.healthcheck_enabled == Some(false) {
                "not configured"
            } else {
                "unknown"
            }
        });
        let service = if self.service.is_empty() {
            "unattributed"
        } else {
            &self.service
        };
        format!(
            "{service}/{}: {} health={health}",
            self.container, self.status
        )
    }
}

impl From<&ServiceObservation> for LiveServiceOutput {
    fn from(service: &ServiceObservation) -> Self {
        Self {
            service: service.service.clone(),
            container: service.container.clone(),
            id: service.id.clone(),
            state: service.state.clone(),
            status: service.status(),
            exit_code: service.exit_code,
            health: service.health.clone(),
            healthcheck_enabled: service.healthcheck_enabled,
            oneoff: service.oneoff,
            container_number: service.container_number,
            config_hash: service.config_hash.clone(),
        }
    }
}
