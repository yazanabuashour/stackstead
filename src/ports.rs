use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::io;
use std::net::TcpListener;
use std::num::TryFromIntError;

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
    PortRangeOverflow { source: Option<TryFromIntError> },
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
            Self::PortRangeOverflow { .. } => {
                write!(f, "port slot exceeds the valid TCP port range")
            }
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
            Self::PortRangeOverflow {
                source: Some(source),
            } => Some(source),
            Self::Probe { source, .. } => Some(source),
            Self::InvalidBase
            | Self::InvalidStride
            | Self::StrideTooSmall { .. }
            | Self::EmptyServiceName
            | Self::DuplicateService(_)
            | Self::PortRangeOverflow { source: None }
            | Self::NoAvailableSlot => None,
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

#[expect(
    clippy::arithmetic_side_effects,
    reason = "input validation and the preceding range check prove this arithmetic is safe"
)]
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

    let last_service_offset =
        u32::try_from(service_names.len().saturating_sub(1)).map_err(|source| {
            PortAllocationError::PortRangeOverflow {
                source: Some(source),
            }
        })?;
    if u32::from(base)
        .checked_add(last_service_offset)
        .is_none_or(|last| last > u32::from(u16::MAX))
    {
        return Err(PortAllocationError::PortRangeOverflow { source: None });
    }
    let available_span = u32::from(u16::MAX) - u32::from(base) - last_service_offset;
    let max_slot = available_span / u32::from(stride);

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
                .ok_or(PortAllocationError::PortRangeOverflow { source: None })?,
        )
        .ok_or(PortAllocationError::PortRangeOverflow { source: None })?;

    service_names
        .iter()
        .enumerate()
        .map(|(index, service)| {
            let port = slot_start
                .checked_add(u32::try_from(index).map_err(|source| {
                    PortAllocationError::PortRangeOverflow {
                        source: Some(source),
                    }
                })?)
                .ok_or(PortAllocationError::PortRangeOverflow { source: None })?;
            let port =
                u16::try_from(port).map_err(|source| PortAllocationError::PortRangeOverflow {
                    source: Some(source),
                })?;
            Ok((service.clone(), port))
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
mod tests;
