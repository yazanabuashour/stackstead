use super::*;

#[test]
fn unavailable_evidence_is_unknown() {
    let observation = RuntimeObservation {
        snapshot: None,
        readiness: readiness::evaluate(&readiness::Contract::Unconfigured {}, None, None),
        issues: Vec::new(),
    };
    assert!(observation.evidence().is_none());
    assert_eq!(observation.activity(), "unknown");
    assert_eq!(observation.running(), None);
    assert_eq!(observation.status(), ComponentStatus::Unknown);
    assert_eq!(
        observation.service_status("postgres"),
        ComponentStatus::Unknown
    );
}
