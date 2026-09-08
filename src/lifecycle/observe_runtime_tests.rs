use super::*;

fn row(service: &str, state: &str) -> compose::ServiceObservation {
    compose::ServiceObservation {
        service: service.into(),
        container: service.into(),
        id: format!("owned-{service}"),
        state: state.into(),
        exit_code: Some(0),
        health: None,
        healthcheck_enabled: Some(false),
        oneoff: Some(false),
        container_number: Some(1),
        config_hash: Some("a".repeat(64)),
    }
}

fn observation(services: Option<Vec<compose::ServiceObservation>>) -> RuntimeObservation {
    RuntimeObservation {
        services,
        readiness: readiness::evaluate(&readiness::Contract::Unconfigured {}, None, None),
        issues: Vec::new(),
    }
}

#[test]
fn activity_running_and_unknown_observations_are_distinct() {
    for (state, activity, running) in [
        ("running", "active", Some(true)),
        ("paused", "active", Some(false)),
        ("restarting", "active", Some(false)),
        ("exited", "inactive", Some(false)),
        ("created", "inactive", Some(false)),
        ("dead", "inactive", Some(false)),
        ("removing", "unknown", None),
        ("", "unknown", None),
    ] {
        let runtime = observation(Some(vec![row("web", state)]));
        assert_eq!(runtime.activity(), activity, "state={state}");
        assert_eq!(runtime.running(), running, "state={state}");
    }
    assert_eq!(observation(None).activity(), "unknown");
    assert_eq!(observation(Some(vec![])).activity(), "inactive");
}

#[test]
fn other_services_and_oneoffs_do_not_prove_database_activity() {
    let mut database = row("postgres", "running");
    for (oneoff, expected) in [
        (Some(false), ComponentStatus::Running),
        (Some(true), ComponentStatus::Stopped),
        (None, ComponentStatus::Unknown),
    ] {
        database.oneoff = oneoff;
        let runtime = observation(Some(vec![row("web", "running"), database.clone()]));
        assert_eq!(runtime.service_status("postgres"), expected);
    }
    let runtime = observation(Some(vec![row("web", "running"), row("postgres", "exited")]));
    assert_eq!(runtime.activity(), "active");
    assert_eq!(runtime.service_status("postgres"), ComponentStatus::Stopped);
    assert_eq!(
        observation(None).service_status("postgres"),
        ComponentStatus::Unknown
    );
}
