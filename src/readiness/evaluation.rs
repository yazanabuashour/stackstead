use std::collections::{BTreeMap, BTreeSet};

use crate::compose::ServiceObservation;

use super::{Contract, Requirements, Role, ServiceRequirement, is_sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessStatus {
    Unconfigured,
    Ready,
    NotReady,
    Unknown,
}

impl ReadinessStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::Ready => "ready",
            Self::NotReady => "not_ready",
            Self::Unknown => "unknown",
        }
    }

    fn combine(self, other: Self) -> Self {
        if self == Self::NotReady || other == Self::NotReady {
            Self::NotReady
        } else if self != Self::Ready || other != Self::Ready {
            Self::Unknown
        } else {
            Self::Ready
        }
    }
}

impl std::fmt::Display for ReadinessStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct ReadinessReport {
    pub status: ReadinessStatus,
    pub required: Vec<RequiredService>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RequiredService {
    pub service: String,
    pub role: Role,
    pub expected_instances: Option<u64>,
    pub observed_containers: usize,
    pub satisfied_instances: usize,
    pub status: ReadinessStatus,
    pub issues: Vec<String>,
}

impl RequiredService {
    fn note(&mut self, status: ReadinessStatus, issue: String) {
        self.status = self.status.combine(status);
        self.issues.push(issue);
    }
}

// The caller supplies a complete ownership-verified snapshot, or None on failure.
pub fn evaluate(
    contract: &Contract,
    current: Option<&Requirements>,
    observations: Option<&[ServiceObservation]>,
) -> ReadinessReport {
    let mut report = ReadinessReport {
        status: ReadinessStatus::Ready,
        required: Vec::new(),
        issues: Vec::new(),
    };
    if let Err(error) = contract.validate() {
        report.status = ReadinessStatus::Unknown;
        report.issues.push(error.to_string());
        return report;
    }
    let Some(required) = contract.required() else {
        report.status = ReadinessStatus::Unconfigured;
        return report;
    };
    let resolved = contract.resolved();
    if resolved.is_none() {
        report
            .issues
            .push("runtime requirements have not been resolved by startup".into());
    } else if current != resolved {
        report
            .issues
            .push("effective Compose inputs differ or could not be verified".into());
    }
    let observations = observations.filter(|rows| {
        let mut ids = BTreeSet::new();
        rows.iter()
            .all(|row| !row.id.is_empty() && ids.insert(&row.id))
    });
    if observations.is_none() {
        report
            .issues
            .push("a complete, distinct owned-container snapshot is unavailable".into());
    }
    if observations.is_some_and(|rows| {
        rows.iter()
            .any(|row| row.service.is_empty() && row.oneoff != Some(true))
    }) {
        report
            .issues
            .push("owned containers lack Compose service attribution".into());
    }
    if !report.issues.is_empty() {
        report.status = ReadinessStatus::Unknown;
    }
    for (service, role) in required {
        let expectation = resolved
            .filter(|resolved| Some(*resolved) == current)
            .and_then(|model| model.services.get(service));
        let result = evaluate_service(service, *role, expectation, observations);
        report.status = report.status.combine(result.status);
        report.issues.extend(
            result
                .issues
                .iter()
                .map(|issue| format!("{service}: {issue}")),
        );
        report.required.push(result);
    }
    report
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum InstanceKey<'a> {
    Number(u64),
    Unclassified(&'a str),
}

fn evaluate_service(
    service: &str,
    role: Role,
    expectation: Option<&ServiceRequirement>,
    observations: Option<&[ServiceObservation]>,
) -> RequiredService {
    let mut report = RequiredService {
        service: service.into(),
        role,
        expected_instances: expectation.map(|value| value.replicas),
        observed_containers: 0,
        satisfied_instances: 0,
        status: ReadinessStatus::Unknown,
        issues: Vec::new(),
    };
    let Some(rows) = observations else {
        return report;
    };
    let candidates = rows
        .iter()
        .filter(|row| row.service == service && row.oneoff != Some(true))
        .collect::<Vec<_>>();
    report.observed_containers = candidates.len();
    let Some(expectation) = expectation else {
        return report;
    };
    report.status = ReadinessStatus::Ready;
    let mut possible = BTreeSet::new();
    let mut current = BTreeMap::<u64, Vec<&ServiceObservation>>::new();
    for row in candidates {
        let hash = row.config_hash.as_deref().filter(|hash| is_sha256(hash));
        if hash.is_some_and(|hash| hash != expectation.config_hash) {
            report.note(
                ReadinessStatus::Unknown,
                format!("{} has a different Compose configuration", row.container),
            );
            continue;
        }
        let number = row.container_number.filter(|number| *number > 0);
        possible.insert(number.map_or(InstanceKey::Unclassified(&row.id), InstanceKey::Number));
        if row.oneoff != Some(false) || number.is_none() || hash.is_none() {
            report.note(
                ReadinessStatus::Unknown,
                format!("{} lacks provable regular-instance metadata", row.container),
            );
        } else if let Some(number) = number {
            current.entry(number).or_default().push(row);
        }
    }
    let expected = usize::try_from(expectation.replicas).ok();
    if expected.is_none_or(|expected| possible.len() < expected) {
        report.note(
            ReadinessStatus::NotReady,
            format!(
                "only {} possible current instances; expected {}",
                possible.len(),
                expectation.replicas
            ),
        );
    } else if expected.is_some_and(|expected| possible.len() > expected) {
        report.note(
            ReadinessStatus::Unknown,
            format!(
                "{} possible current instances exceed the expected {}",
                possible.len(),
                expectation.replicas
            ),
        );
    }
    for (number, rows) in current {
        if rows.len() != 1 {
            report.note(
                ReadinessStatus::Unknown,
                format!("instance number {number} belongs to multiple containers"),
            );
            continue;
        }
        if let Some(row) = rows.first() {
            let (status, reason) = satisfaction(role, row);
            if status == ReadinessStatus::Ready {
                report.satisfied_instances = report.satisfied_instances.saturating_add(1);
            } else {
                report.note(
                    status,
                    format!("instance {number} ({}): {reason}", row.container),
                );
            }
        }
    }
    report
}

fn satisfaction(role: Role, row: &ServiceObservation) -> (ReadinessStatus, String) {
    let state = row.state.as_str();
    if role == Role::Job && state == "exited" {
        return match row.exit_code {
            Some(0) => (ReadinessStatus::Ready, String::new()),
            Some(code) => (
                ReadinessStatus::NotReady,
                format!("job exited with code {code}"),
            ),
            None => (
                ReadinessStatus::Unknown,
                "job exit code is unavailable".into(),
            ),
        };
    }
    if !matches!(
        state,
        "running" | "created" | "exited" | "restarting" | "paused" | "dead" | "removing"
    ) {
        return (
            ReadinessStatus::Unknown,
            format!("unrecognized container state {state:?}"),
        );
    }
    if role == Role::Job {
        return (
            ReadinessStatus::NotReady,
            format!("job is {state}, not successfully exited"),
        );
    }
    if state != "running" {
        return (
            ReadinessStatus::NotReady,
            format!("long-running service is {state}"),
        );
    }
    match (row.healthcheck_enabled, row.health.as_deref()) {
        (Some(false), None) | (Some(true), Some("healthy")) => {
            (ReadinessStatus::Ready, String::new())
        }
        (Some(true), Some("unhealthy")) => (
            ReadinessStatus::NotReady,
            "container health is unhealthy".into(),
        ),
        (Some(true), Some("starting")) => (
            ReadinessStatus::NotReady,
            "container health is starting".into(),
        ),
        _ => (
            ReadinessStatus::Unknown,
            "container healthcheck evidence is missing or inconsistent".into(),
        ),
    }
}

#[cfg(test)]
#[path = "evaluation_tests.rs"]
mod tests;
