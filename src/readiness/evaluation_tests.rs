use super::*;
use crate::test_support::TestResultExt;

fn contract(role: Role, replicas: u64) -> Contract {
    Contract::Declared {
        required: BTreeMap::from([("worker".into(), role)]),
        resolved: Some(Requirements {
            profiles: None,
            model_hash: "b".repeat(64),
            services: BTreeMap::from([(
                "worker".into(),
                ServiceRequirement {
                    replicas,
                    config_hash: "a".repeat(64),
                },
            )]),
        }),
    }
}

fn container(number: u64, state: &str) -> ServiceObservation {
    ServiceObservation {
        service: "worker".into(),
        container: format!("worker-{number}"),
        id: format!("owned-container-{number}"),
        state: state.into(),
        exit_code: Some(0),
        health: None,
        healthcheck_enabled: Some(false),
        oneoff: Some(false),
        container_number: Some(number),
        config_hash: Some("a".repeat(64)),
    }
}

#[test]
fn explicit_roles_determine_whether_exit_is_success() {
    for (role, state, exit_code, expected) in [
        (
            Role::LongRunning,
            "running",
            Some(0),
            ReadinessStatus::Ready,
        ),
        (
            Role::LongRunning,
            "exited",
            Some(0),
            ReadinessStatus::NotReady,
        ),
        (Role::Job, "exited", Some(0), ReadinessStatus::Ready),
        (Role::Job, "exited", Some(7), ReadinessStatus::NotReady),
        (Role::Job, "exited", None, ReadinessStatus::Unknown),
        (Role::Job, "running", Some(0), ReadinessStatus::NotReady),
        (
            Role::LongRunning,
            "unexpected",
            Some(0),
            ReadinessStatus::Unknown,
        ),
    ] {
        let contract = contract(role, 1);
        let mut row = container(1, state);
        row.exit_code = exit_code;
        let report = evaluate(&contract, contract.resolved(), Some(&[row]));
        assert_eq!(
            report.status, expected,
            "{role:?} {state} exit={exit_code:?}"
        );
    }
}

#[test]
fn actual_healthcheck_evidence_is_required_when_enabled() {
    for (enabled, health, expected) in [
        (Some(true), Some("healthy"), ReadinessStatus::Ready),
        (Some(true), Some("starting"), ReadinessStatus::NotReady),
        (Some(true), Some("unhealthy"), ReadinessStatus::NotReady),
        (Some(true), None, ReadinessStatus::Unknown),
        (None, Some("healthy"), ReadinessStatus::Unknown),
        (Some(false), Some("healthy"), ReadinessStatus::Unknown),
    ] {
        let contract = contract(Role::LongRunning, 1);
        let mut row = container(1, "running");
        row.healthcheck_enabled = enabled;
        row.health = health.map(str::to_owned);
        let report = evaluate(&contract, contract.resolved(), Some(&[row]));
        assert_eq!(
            report.status, expected,
            "health={health:?} enabled={enabled:?}"
        );
    }
}

#[test]
fn replica_counts_allow_gaps_but_not_missing_or_ambiguous_instances() -> anyhow::Result<()> {
    let two = contract(Role::LongRunning, 2);
    let mut rows = vec![container(2, "running"), container(3, "running")];
    let ready = evaluate(&two, two.resolved(), Some(&rows));
    assert_eq!(ready.status, ReadinessStatus::Ready);
    assert_eq!(ready.required.first().test()?.satisfied_instances, 2);

    rows.pop().test()?;
    rows.first_mut().test()?.healthcheck_enabled = Some(true);
    assert_eq!(
        evaluate(&two, two.resolved(), Some(&rows)).status,
        ReadinessStatus::NotReady
    );

    let single = contract(Role::LongRunning, 1);
    let mut duplicate = container(3, "running");
    duplicate.container_number = Some(2);
    let duplicates = [container(2, "running"), duplicate];
    assert_eq!(
        evaluate(&single, single.resolved(), Some(&duplicates)).status,
        ReadinessStatus::Unknown
    );
    assert_eq!(
        evaluate(
            &single,
            single.resolved(),
            Some(&[container(2, "running"), container(3, "running")])
        )
        .status,
        ReadinessStatus::Unknown
    );
    Ok(())
}

#[test]
fn oneoffs_stale_and_unprovable_instances_cannot_stand_in() {
    let contract = contract(Role::LongRunning, 1);
    let base = container(1, "running");
    let mut oneoff = base.clone();
    oneoff.oneoff = Some(true);
    let mut stale = base.clone();
    stale.config_hash = Some("c".repeat(64));
    let mut missing_hash = base.clone();
    missing_hash.config_hash = None;
    let mut missing_number = base.clone();
    missing_number.container_number = None;
    let mut missing_oneoff = base;
    missing_oneoff.oneoff = None;
    for (row, expected) in [
        (oneoff, ReadinessStatus::NotReady),
        (stale, ReadinessStatus::NotReady),
        (missing_hash, ReadinessStatus::Unknown),
        (missing_number, ReadinessStatus::Unknown),
        (missing_oneoff, ReadinessStatus::Unknown),
    ] {
        assert_eq!(
            evaluate(&contract, contract.resolved(), Some(&[row])).status,
            expected
        );
    }
}

#[test]
fn absent_and_stale_evidence_never_becomes_ready() -> anyhow::Result<()> {
    let mut contract = contract(Role::LongRunning, 1);
    let rows = [container(1, "running")];
    let mut current = contract.resolved().test()?.clone();
    current.model_hash = "c".repeat(64);
    assert_eq!(
        evaluate(&contract, Some(&current), Some(&rows)).status,
        ReadinessStatus::Unknown
    );
    assert_eq!(
        evaluate(&contract, contract.resolved(), None).status,
        ReadinessStatus::Unknown
    );
    assert_eq!(
        evaluate(
            &contract,
            contract.resolved(),
            Some(&[rows.first().test()?.clone(), rows.first().test()?.clone()])
        )
        .status,
        ReadinessStatus::Unknown
    );
    contract.invalidate();
    assert_eq!(
        evaluate(&contract, Some(&current), Some(&rows)).status,
        ReadinessStatus::Unknown
    );
    assert_eq!(
        evaluate(&Contract::Unconfigured {}, None, Some(&rows)).status,
        ReadinessStatus::Unconfigured
    );
    Ok(())
}

#[test]
fn optional_failures_do_not_override_required_success() {
    let contract = contract(Role::LongRunning, 1);
    let mut optional = container(2, "exited");
    optional.service = "optional".into();
    optional.exit_code = Some(7);
    let mut rows = [container(1, "running"), optional];
    assert_eq!(
        evaluate(&contract, contract.resolved(), Some(&rows)).status,
        ReadinessStatus::Ready
    );
    rows[1].service.clear();
    assert_eq!(
        evaluate(&contract, contract.resolved(), Some(&rows)).status,
        ReadinessStatus::Unknown
    );
}

#[test]
fn large_replica_expectations_do_not_allocate_missing_instances() {
    let contract = contract(Role::LongRunning, u64::MAX);
    assert_eq!(
        evaluate(&contract, contract.resolved(), Some(&[])).status,
        ReadinessStatus::NotReady
    );
}
