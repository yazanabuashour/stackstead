use crate::{
    lease::{LeaseIdentity, PortLeaseStore},
    manifest::StacksteadManifest,
};

pub fn verify_port_leases(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if manifest.ports.is_empty() {
        return Ok(());
    }
    manifest_port_lease_store(manifest)?.transaction()?.verify(
        &manifest.runtime_token,
        &LeaseIdentity::new(&manifest.stackstead_id, &manifest.project),
        &manifest.ports.values().copied().collect(),
    )
}

pub(super) fn release_port_leases(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    if manifest.ports.is_empty() {
        return Ok(());
    }
    manifest_port_lease_store(manifest)?.transaction()?.release(
        &manifest.runtime_token,
        &LeaseIdentity::new(&manifest.stackstead_id, &manifest.project),
        &manifest.ports.values().copied().collect(),
    )
}

pub(super) fn release_port_leases_after_destroy(
    manifest: &StacksteadManifest,
) -> anyhow::Result<()> {
    if manifest.ports.is_empty() {
        return Ok(());
    }
    manifest_port_lease_store(manifest)?
        .transaction()?
        .release_if_owned_or_absent(
            &manifest.runtime_token,
            &LeaseIdentity::new(&manifest.stackstead_id, &manifest.project),
            &manifest.ports.values().copied().collect(),
        )
}

fn manifest_port_lease_store(manifest: &StacksteadManifest) -> anyhow::Result<PortLeaseStore> {
    manifest
        .port_lease_state_dir
        .as_ref()
        .map(|path| PortLeaseStore::at(path.clone()))
        .ok_or_else(|| anyhow::anyhow!("manifest is missing its durable port lease registry path"))
}
