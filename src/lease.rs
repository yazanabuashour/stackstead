use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use crate::lock::LockGuard;

const REGISTRY_KIND: &str = "StacksteadPortLeaseRegistry";
const REGISTRY_VERSION: &str = "1";
const REGISTRY_FILE: &str = "port-leases.json";
const LOCK_FILE: &str = "port-leases.lock";
const INITIALIZED_FILE: &str = "port-leases.initialized";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseIdentity {
    pub stackstead_id: String,
    pub project: String,
}

impl LeaseIdentity {
    pub fn new(stackstead_id: impl Into<String>, project: impl Into<String>) -> Self {
        Self {
            stackstead_id: stackstead_id.into(),
            project: project.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PortLeaseStore {
    state_dir: PathBuf,
}

impl PortLeaseStore {
    pub fn for_current_user() -> anyhow::Result<Self> {
        Self::from_environment(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
    }

    pub fn at(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
        }
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn transaction(&self) -> anyhow::Result<PortLeaseTransaction> {
        let lock_path = self.state_dir.join(LOCK_FILE);
        let registry_path = self.state_dir.join(REGISTRY_FILE);
        let initialized_path = self.state_dir.join(INITIALIZED_FILE);
        let lock = LockGuard::acquire(&lock_path, "port lease registry")?;
        let initialized = initialization_complete(&initialized_path)?;
        if initialized && !registry_path.exists() {
            anyhow::bail!(
                "initialized port lease registry {} is missing; restore it before allocating or operating stacksteads",
                registry_path.display()
            );
        }
        if !registry_path.exists() {
            Registry::empty().save(&registry_path)?;
        }
        let registry = Registry::read(&registry_path)?;
        if !initialized {
            mark_initialized(&initialized_path)?;
        }
        Ok(PortLeaseTransaction {
            _lock: lock,
            registry_path,
            registry,
        })
    }

    fn from_environment(
        xdg_state_home: Option<OsString>,
        home: Option<OsString>,
    ) -> anyhow::Result<Self> {
        if let Some(path) = xdg_state_home
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            && !path.as_os_str().is_empty()
        {
            return Ok(Self::at(path.join("stackstead")));
        }

        let home = home
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && !path.as_os_str().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "cannot locate the per-user Stackstead state directory: set XDG_STATE_HOME or HOME to an absolute path"
                )
            })?;
        Ok(Self::at(home.join(".local/state/stackstead")))
    }
}

fn initialization_complete(path: &Path) -> anyhow::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            anyhow::bail!(
                "port lease initialization marker {} is a symlink",
                path.display()
            )
        }
        Ok(metadata) if !metadata.is_file() => anyhow::bail!(
            "port lease initialization marker {} is not a regular file",
            path.display()
        ),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn mark_initialized(path: &Path) -> anyhow::Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(b"StacksteadPortLeaseRegistry initialized\n")?;
    file.sync_all()?;
    Ok(())
}

#[derive(Debug)]
pub struct PortLeaseTransaction {
    _lock: LockGuard,
    registry_path: PathBuf,
    registry: Registry,
}

impl PortLeaseTransaction {
    pub fn used_ports(&self) -> BTreeSet<u16> {
        self.registry
            .leases()
            .iter()
            .map(registry::Lease::port)
            .collect()
    }

    pub fn reserve(
        &mut self,
        owner: &str,
        identity: &LeaseIdentity,
        ports: &BTreeSet<u16>,
    ) -> anyhow::Result<()> {
        validate_request(owner, identity, ports)?;
        for lease in self.registry.leases() {
            if ports.contains(&lease.port()) {
                anyhow::bail!(
                    "port {} is already leased to stackstead `{}` in project `{}`",
                    lease.port(),
                    lease.stackstead_id(),
                    lease.project()
                );
            }
        }

        let mut updated = self.registry.clone();
        updated.add(owner, identity, ports);
        updated.validate(&self.registry_path)?;
        updated.save(&self.registry_path)?;
        self.registry = updated;
        Ok(())
    }

    pub fn verify(
        &self,
        owner: &str,
        identity: &LeaseIdentity,
        ports: &BTreeSet<u16>,
    ) -> anyhow::Result<()> {
        validate_request(owner, identity, ports)?;
        if self
            .registry
            .leases()
            .iter()
            .any(|lease| lease.owner() == owner && !lease.belongs_to(identity))
        {
            anyhow::bail!(
                "port leases for owner `{owner}` do not belong to stackstead `{}` in project `{}`",
                identity.stackstead_id,
                identity.project
            );
        }
        let actual = self
            .registry
            .leases()
            .iter()
            .filter(|lease| lease.owner() == owner)
            .map(registry::Lease::port)
            .collect::<BTreeSet<_>>();
        if actual != *ports {
            anyhow::bail!(
                "port leases for owner `{owner}` do not match: expected {}, found {}",
                display_ports(ports),
                display_ports(&actual)
            );
        }
        Ok(())
    }

    pub fn release(
        &mut self,
        owner: &str,
        identity: &LeaseIdentity,
        ports: &BTreeSet<u16>,
    ) -> anyhow::Result<()> {
        self.verify(owner, identity, ports)?;
        self.remove_owner(owner)
    }

    pub fn release_if_owned_or_absent(
        &mut self,
        owner: &str,
        identity: &LeaseIdentity,
        ports: &BTreeSet<u16>,
    ) -> anyhow::Result<()> {
        validate_request(owner, identity, ports)?;
        let actual = self
            .registry
            .leases()
            .iter()
            .filter(|lease| lease.owner() == owner)
            .map(registry::Lease::port)
            .collect::<BTreeSet<_>>();
        if actual.is_empty() {
            return Ok(());
        }
        if self
            .registry
            .leases()
            .iter()
            .any(|lease| lease.owner() == owner && !lease.belongs_to(identity))
        {
            anyhow::bail!(
                "port leases for owner `{owner}` do not belong to stackstead `{}` in project `{}` during destroy recovery",
                identity.stackstead_id,
                identity.project
            );
        }
        if actual != *ports {
            anyhow::bail!(
                "port leases for owner `{owner}` do not match during destroy recovery: expected {}, found {}",
                display_ports(ports),
                display_ports(&actual)
            );
        }
        self.remove_owner(owner)
    }

    fn remove_owner(&mut self, owner: &str) -> anyhow::Result<()> {
        let mut updated = self.registry.clone();
        updated.remove(owner);
        updated.save(&self.registry_path)?;
        self.registry = updated;
        Ok(())
    }
}

mod registry;
use registry::Registry;

mod request;
use request::{display_ports, validate_request};

#[cfg(test)]
mod tests;
