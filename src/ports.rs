use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::io;
use std::net::TcpListener;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortAllocation {
    pub slot: u32,
    pub ports: BTreeMap<String, u16>,
}

#[derive(Debug)]
pub enum PortAllocationError {
    InvalidBase,
    InvalidStride,
    StrideTooSmall { stride: u16, service_count: usize },
    EmptyServiceName,
    DuplicateService(String),
    PortRangeOverflow,
    NoAvailableSlot,
    Probe { port: u16, source: io::Error },
}

impl fmt::Display for PortAllocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBase => write!(f, "port base must be greater than zero"),
            Self::InvalidStride => write!(f, "port stride must be greater than zero"),
            Self::StrideTooSmall {
                stride,
                service_count,
            } => write!(
                f,
                "port stride {stride} is smaller than exposed service count {service_count}"
            ),
            Self::EmptyServiceName => write!(f, "exposed service name cannot be empty"),
            Self::DuplicateService(service) => {
                write!(f, "duplicate exposed service name `{service}`")
            }
            Self::PortRangeOverflow => write!(f, "port slot exceeds the valid TCP port range"),
            Self::NoAvailableSlot => write!(f, "no deterministic port slot is available"),
            Self::Probe { port, source } => {
                write!(f, "failed to probe 127.0.0.1:{port}: {source}")
            }
        }
    }
}

impl Error for PortAllocationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Probe { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub fn allocate_ports(
    base: u16,
    stride: u16,
    service_names: &[String],
    used_ports: &BTreeSet<u16>,
) -> Result<PortAllocation, PortAllocationError> {
    allocate_ports_with_probe(base, stride, service_names, used_ports, port_is_available)
}

pub fn allocate_ports_with_probe<F>(
    base: u16,
    stride: u16,
    service_names: &[String],
    used_ports: &BTreeSet<u16>,
    mut is_available: F,
) -> Result<PortAllocation, PortAllocationError>
where
    F: FnMut(u16) -> io::Result<bool>,
{
    validate_inputs(base, stride, service_names)?;

    if service_names.is_empty() {
        return Ok(PortAllocation {
            slot: 0,
            ports: BTreeMap::new(),
        });
    }

    let last_service_offset = service_names.len() as u32 - 1;
    if u32::from(base) + last_service_offset > u32::from(u16::MAX) {
        return Err(PortAllocationError::PortRangeOverflow);
    }
    let max_slot =
        (u32::from(u16::MAX) - u32::from(base) - last_service_offset) / u32::from(stride);

    for slot in 0..=max_slot {
        let ports = ports_for_slot(base, stride, service_names, slot)?;
        if ports.values().any(|port| used_ports.contains(port)) {
            continue;
        }

        let mut slot_available = true;
        for port in ports.values().copied() {
            match is_available(port) {
                Ok(true) => {}
                Ok(false) => {
                    slot_available = false;
                    break;
                }
                Err(source) => return Err(PortAllocationError::Probe { port, source }),
            }
        }

        if slot_available {
            return Ok(PortAllocation { slot, ports });
        }
    }

    Err(PortAllocationError::NoAvailableSlot)
}

pub fn ports_for_slot(
    base: u16,
    stride: u16,
    service_names: &[String],
    slot: u32,
) -> Result<BTreeMap<String, u16>, PortAllocationError> {
    validate_inputs(base, stride, service_names)?;

    let slot_start = u32::from(base)
        .checked_add(
            slot.checked_mul(u32::from(stride))
                .ok_or(PortAllocationError::PortRangeOverflow)?,
        )
        .ok_or(PortAllocationError::PortRangeOverflow)?;

    service_names
        .iter()
        .enumerate()
        .map(|(index, service)| {
            let port = slot_start
                .checked_add(index as u32)
                .filter(|port| *port <= u32::from(u16::MAX))
                .ok_or(PortAllocationError::PortRangeOverflow)?;
            Ok((service.clone(), port as u16))
        })
        .collect()
}

pub fn port_is_available(port: u16) -> io::Result<bool> {
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => Ok(false),
        Err(error) => Err(error),
    }
}

fn validate_inputs(
    base: u16,
    stride: u16,
    service_names: &[String],
) -> Result<(), PortAllocationError> {
    if base == 0 {
        return Err(PortAllocationError::InvalidBase);
    }
    if stride == 0 {
        return Err(PortAllocationError::InvalidStride);
    }
    if usize::from(stride) < service_names.len() {
        return Err(PortAllocationError::StrideTooSmall {
            stride,
            service_count: service_names.len(),
        });
    }

    let mut unique = BTreeSet::new();
    for service in service_names {
        if service.is_empty() {
            return Err(PortAllocationError::EmptyServiceName);
        }
        if !unique.insert(service) {
            return Err(PortAllocationError::DuplicateService(service.clone()));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn services() -> Vec<String> {
        ["web", "api", "postgres", "redis"]
            .map(str::to_owned)
            .to_vec()
    }

    fn available(_: u16) -> io::Result<bool> {
        Ok(true)
    }

    #[test]
    fn allocates_first_and_second_slots_deterministically() {
        let first =
            allocate_ports_with_probe(39000, 50, &services(), &BTreeSet::new(), available).unwrap();
        assert_eq!(first.slot, 0);
        assert_eq!(first.ports["web"], 39000);
        assert_eq!(first.ports["redis"], 39003);

        let used = first.ports.values().copied().collect();
        let second = allocate_ports_with_probe(39000, 50, &services(), &used, available).unwrap();
        assert_eq!(second.slot, 1);
        assert_eq!(second.ports["web"], 39050);
    }

    #[test]
    fn reuses_a_hole_at_the_first_slot() {
        let used = BTreeSet::from([39050, 39051, 39052, 39053]);
        let allocation =
            allocate_ports_with_probe(39000, 50, &services(), &used, available).unwrap();
        assert_eq!(allocation.slot, 0);
    }

    #[test]
    fn skips_a_slot_with_an_occupied_os_port() {
        let allocation =
            allocate_ports_with_probe(39000, 50, &services(), &BTreeSet::new(), |port| {
                Ok(port != 39002)
            })
            .unwrap();
        assert_eq!(allocation.slot, 1);
    }

    #[test]
    fn rejects_invalid_base_stride_and_duplicate_services() {
        assert!(matches!(
            ports_for_slot(0, 50, &services(), 0),
            Err(PortAllocationError::InvalidBase)
        ));
        assert!(matches!(
            ports_for_slot(39000, 0, &services(), 0),
            Err(PortAllocationError::InvalidStride)
        ));
        assert!(matches!(
            ports_for_slot(39000, 2, &services(), 0),
            Err(PortAllocationError::StrideTooSmall { .. })
        ));
        assert!(matches!(
            ports_for_slot(39000, 50, &["web".to_owned(), "web".to_owned()], 0),
            Err(PortAllocationError::DuplicateService(service)) if service == "web"
        ));
    }

    #[test]
    fn reports_port_range_overflow() {
        assert!(matches!(
            ports_for_slot(65535, 1, &["web".to_owned(), "api".to_owned()], 0),
            Err(PortAllocationError::StrideTooSmall { .. })
        ));
        assert!(matches!(
            ports_for_slot(65535, 2, &["web".to_owned(), "api".to_owned()], 0),
            Err(PortAllocationError::PortRangeOverflow)
        ));
        assert!(matches!(
            ports_for_slot(65000, 50, &services(), 20),
            Err(PortAllocationError::PortRangeOverflow)
        ));
    }
}
