use crate::{lock::LockGuard, manifest::StacksteadManifest};

#[derive(Debug)]
pub struct CreateOutcome {
    manifest: StacksteadManifest,
    mutation_lock: LockGuard,
}

impl CreateOutcome {
    pub(super) const fn new(manifest: StacksteadManifest, mutation_lock: LockGuard) -> Self {
        Self {
            manifest,
            mutation_lock,
        }
    }

    pub const fn manifest(&self) -> &StacksteadManifest {
        &self.manifest
    }

    pub fn into_manifest(self) -> StacksteadManifest {
        self.manifest
    }
}

#[derive(Debug)]
enum RunLease {
    Exclusive(LockGuard),
    Shared(LockGuard),
}

/// Holds mutation access and a run lease for one exact environment. Operation-specific
/// contract checks still belong to the caller and must use the reloaded manifest.
#[derive(Debug)]
pub struct HeldEnvironment {
    manifest: StacksteadManifest,
    mutation_lock: LockGuard,
    run_lease: RunLease,
}

impl HeldEnvironment {
    pub fn exclusive(manifest: StacksteadManifest) -> anyhow::Result<Self> {
        let mutation_lock =
            LockGuard::acquire_existing(&manifest.state_dir.join("lock"), "stackstead")?;
        Self::acquire_run(manifest, mutation_lock, None)
    }

    pub fn shared(manifest: StacksteadManifest, kind: &'static str) -> anyhow::Result<Self> {
        let mutation_lock =
            LockGuard::acquire_existing(&manifest.state_dir.join("lock"), "stackstead")?;
        Self::acquire_run(manifest, mutation_lock, Some(kind))
    }

    pub(super) fn after_create(
        created: CreateOutcome,
        resolved: &StacksteadManifest,
    ) -> anyhow::Result<Self> {
        ensure_same_environment(&created.manifest, resolved)?;
        Self::acquire_run(created.manifest, created.mutation_lock, None)
    }

    fn acquire_run(
        manifest: StacksteadManifest,
        mutation_lock: LockGuard,
        shared_kind: Option<&'static str>,
    ) -> anyhow::Result<Self> {
        let path = manifest.state_dir.join("run.lock");
        let run_lease = match shared_kind {
            Some(kind) => RunLease::Shared(LockGuard::acquire_existing_shared(&path, kind)?),
            None => RunLease::Exclusive(LockGuard::acquire_existing(
                &path,
                "active stackstead agent",
            )?),
        };
        let mut held = Self {
            manifest,
            mutation_lock,
            run_lease,
        };
        held.reload()?;
        Ok(held)
    }

    pub fn for_run(mut self, resolved: &StacksteadManifest) -> anyhow::Result<Self> {
        ensure_same_environment(&self.manifest, resolved)?;
        // Downgrading unlocks briefly. Keep mutation ownership through the conversion
        // so lifecycle mutations cannot enter the gap between exclusive and shared.
        self.run_lease = match self.run_lease {
            RunLease::Exclusive(lease) => RunLease::Shared(lease.downgrade_to_shared()?),
            RunLease::Shared(lease) => RunLease::Shared(lease),
        };
        self.reload()?;
        Ok(self)
    }

    fn reload(&mut self) -> anyhow::Result<()> {
        let latest = StacksteadManifest::read(&self.manifest.manifest_path())?;
        ensure_same_environment(&self.manifest, &latest)?;
        self.manifest = latest;
        Ok(())
    }

    pub const fn manifest(&self) -> &StacksteadManifest {
        &self.manifest
    }

    pub const fn manifest_mut(&mut self) -> &mut StacksteadManifest {
        &mut self.manifest
    }

    pub fn into_manifest(self) -> StacksteadManifest {
        self.manifest
    }

    /// Release mutation access only after the caller's checks, retaining the shared
    /// lease with the manifest for the host supervisor or foreground Compose child.
    pub fn into_run(self) -> anyhow::Result<(StacksteadManifest, LockGuard)> {
        let RunLease::Shared(run_lease) = self.run_lease else {
            anyhow::bail!("cannot hand an exclusive environment lease to a child");
        };
        drop(self.mutation_lock);
        Ok((self.manifest, run_lease))
    }
}

fn ensure_same_environment(
    held: &StacksteadManifest,
    resolved: &StacksteadManifest,
) -> anyhow::Result<()> {
    if held.stackstead_id != resolved.stackstead_id
        || held.runtime_token != resolved.runtime_token
        || held.state_dir != resolved.state_dir
    {
        anyhow::bail!(
            "environment identity changed while holding access to {}; refusing to use a different environment under its locks",
            held.stackstead_id
        );
    }
    Ok(())
}
