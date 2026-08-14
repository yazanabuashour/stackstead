use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::BufReader,
    path::Path,
};

use serde::{Deserialize, Serialize};

use super::{LeaseIdentity, REGISTRY_KIND, REGISTRY_VERSION};
use crate::manifest::write_json_atomic;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Registry {
    kind: String,
    version: String,
    leases: Vec<Lease>,
}

impl Registry {
    pub(super) fn leases(&self) -> &[Lease] {
        &self.leases
    }

    pub(super) fn add(&mut self, owner: &str, identity: &LeaseIdentity, ports: &BTreeSet<u16>) {
        self.leases.extend(ports.iter().map(|port| Lease {
            port: *port,
            owner: owner.to_owned(),
            stackstead_id: identity.stackstead_id.clone(),
            project: identity.project.clone(),
        }));
        self.leases.sort_by_key(|lease| lease.port);
    }

    pub(super) fn remove(&mut self, owner: &str) {
        self.leases.retain(|lease| lease.owner != owner);
    }

    pub(super) fn empty() -> Self {
        Self {
            kind: REGISTRY_KIND.into(),
            version: REGISTRY_VERSION.into(),
            leases: Vec::new(),
        }
    }

    pub(super) fn read(path: &Path) -> anyhow::Result<Self> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!("port lease registry {} is a symlink", path.display())
            }
            Ok(metadata) if !metadata.is_file() => {
                anyhow::bail!(
                    "port lease registry {} is not a regular file",
                    path.display()
                )
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::empty()),
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "cannot inspect port lease registry {}: {error}",
                    path.display()
                ));
            }
        }

        let registry: Self = serde_json::from_reader(BufReader::new(open_registry(path)?))
            .map_err(|error| {
                anyhow::anyhow!(
                    "cannot parse port lease registry {}: {error}",
                    path.display()
                )
            })?;
        registry.validate(path)?;
        Ok(registry)
    }

    pub(super) fn validate(&self, path: &Path) -> anyhow::Result<()> {
        if self.kind != REGISTRY_KIND || self.version != REGISTRY_VERSION {
            anyhow::bail!(
                "unsupported port lease registry contract in {}: kind={} version={}",
                path.display(),
                self.kind,
                self.version
            );
        }

        let mut ports = BTreeSet::new();
        let mut owners = BTreeMap::<&str, (&str, &str)>::new();
        for lease in &self.leases {
            if lease.port == 0
                || lease.owner.is_empty()
                || lease.stackstead_id.is_empty()
                || lease.project.is_empty()
            {
                anyhow::bail!("invalid port lease entry in {}", path.display());
            }
            if !ports.insert(lease.port) {
                anyhow::bail!(
                    "duplicate port {} in port lease registry {}",
                    lease.port,
                    path.display()
                );
            }
            let identity = (lease.stackstead_id.as_str(), lease.project.as_str());
            if let Some(existing) = owners.insert(lease.owner.as_str(), identity)
                && existing != identity
            {
                anyhow::bail!(
                    "ambiguous identity for lease owner `{}` in {}",
                    lease.owner,
                    path.display()
                );
            }
        }
        Ok(())
    }

    pub(super) fn save(&self, path: &Path) -> anyhow::Result<()> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!("port lease registry {} is a symlink", path.display())
            }
            Ok(metadata) if !metadata.is_file() => {
                anyhow::bail!(
                    "port lease registry {} is not a regular file",
                    path.display()
                )
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "cannot inspect port lease registry {}: {error}",
                    path.display()
                ));
            }
        }
        write_json_atomic(path, self)
    }
}

fn open_registry(path: &Path) -> anyhow::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|error| {
        anyhow::anyhow!(
            "cannot open port lease registry {}: {error}",
            path.display()
        )
    })?;
    if !file.metadata()?.is_file() {
        anyhow::bail!(
            "port lease registry {} is not a regular file",
            path.display()
        );
    }
    Ok(file)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Lease {
    port: u16,
    owner: String,
    stackstead_id: String,
    project: String,
}

impl Lease {
    pub(super) const fn port(&self) -> u16 {
        self.port
    }

    pub(super) fn owner(&self) -> &str {
        &self.owner
    }

    pub(super) fn stackstead_id(&self) -> &str {
        &self.stackstead_id
    }

    pub(super) fn project(&self) -> &str {
        &self.project
    }

    pub(super) fn belongs_to(&self, identity: &LeaseIdentity) -> bool {
        self.stackstead_id == identity.stackstead_id && self.project == identity.project
    }
}
