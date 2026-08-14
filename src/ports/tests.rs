use super::*;
use crate::test_support::TestResultExt as _;

fn services() -> Vec<String> {
    ["web", "api", "postgres", "redis"]
        .map(str::to_owned)
        .to_vec()
}

fn available(_: u16) -> io::Result<bool> {
    Ok(true)
}

#[test]
fn allocates_first_and_second_slots_deterministically() -> anyhow::Result<()> {
    let first =
        allocate_ports_with_probe(39000, 50, &services(), &BTreeSet::new(), available).test()?;
    assert_eq!(first.slot, 0, "test contract values differ");
    assert_eq!(first.ports["web"], 39000, "test contract values differ");
    assert_eq!(first.ports["redis"], 39003, "test contract values differ");

    let used = first.ports.values().copied().collect();
    let second = allocate_ports_with_probe(39000, 50, &services(), &used, available).test()?;
    assert_eq!(second.slot, 1, "test contract values differ");
    assert_eq!(second.ports["web"], 39050, "test contract values differ");
    Ok(())
}

#[test]
fn reuses_a_hole_at_the_first_slot() -> anyhow::Result<()> {
    let used = BTreeSet::from([39050, 39051, 39052, 39053]);
    let allocation = allocate_ports_with_probe(39000, 50, &services(), &used, available).test()?;
    assert_eq!(allocation.slot, 0, "test contract values differ");
    Ok(())
}

#[test]
fn skips_a_slot_with_an_occupied_os_port() -> anyhow::Result<()> {
    let allocation = allocate_ports_with_probe(39000, 50, &services(), &BTreeSet::new(), |port| {
        Ok(port != 39002)
    })
    .test()?;
    assert_eq!(allocation.slot, 1, "test contract values differ");
    Ok(())
}

#[test]
fn rejects_invalid_base_stride_and_duplicate_services() -> anyhow::Result<()> {
    assert!(
        matches!(
            ports_for_slot(0, 50, &services(), 0),
            Err(PortAllocationError::InvalidBase)
        ),
        "test contract condition failed"
    );
    assert!(
        matches!(
            ports_for_slot(39000, 0, &services(), 0),
            Err(PortAllocationError::InvalidStride)
        ),
        "test contract condition failed"
    );
    assert!(
        matches!(
            ports_for_slot(39000, 2, &services(), 0),
            Err(PortAllocationError::StrideTooSmall { .. })
        ),
        "test contract condition failed"
    );
    assert!(
        matches!(
            ports_for_slot(39000, 50, &["web".to_owned(), "web".to_owned()], 0),
            Err(PortAllocationError::DuplicateService(service)) if service == "web"
        ),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn reports_port_range_overflow() -> anyhow::Result<()> {
    assert!(
        matches!(
            ports_for_slot(65535, 1, &["web".to_owned(), "api".to_owned()], 0),
            Err(PortAllocationError::StrideTooSmall { .. })
        ),
        "test contract condition failed"
    );
    assert!(
        matches!(
            ports_for_slot(65535, 2, &["web".to_owned(), "api".to_owned()], 0),
            Err(PortAllocationError::PortRangeOverflow { .. })
        ),
        "test contract condition failed"
    );
    assert!(
        matches!(
            ports_for_slot(65000, 50, &services(), 20),
            Err(PortAllocationError::PortRangeOverflow { .. })
        ),
        "test contract condition failed"
    );
    Ok(())
}
