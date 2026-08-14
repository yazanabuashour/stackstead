use std::collections::BTreeSet;

use super::LeaseIdentity;

pub(super) fn validate_request(
    owner: &str,
    identity: &LeaseIdentity,
    ports: &BTreeSet<u16>,
) -> anyhow::Result<()> {
    validate_owner_and_ports(owner, ports)?;
    if identity.stackstead_id.is_empty() || identity.project.is_empty() {
        anyhow::bail!("port lease identity must include a stackstead id and project");
    }
    Ok(())
}

fn validate_owner_and_ports(owner: &str, ports: &BTreeSet<u16>) -> anyhow::Result<()> {
    if owner.is_empty() {
        anyhow::bail!("port lease owner must not be empty");
    }
    if ports.is_empty() || ports.contains(&0) {
        anyhow::bail!("port lease set must contain at least one nonzero port");
    }
    Ok(())
}

pub(super) fn display_ports(ports: &BTreeSet<u16>) -> String {
    ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
