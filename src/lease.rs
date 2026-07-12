use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{BufReader, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{lock::LockGuard, manifest::write_json_atomic};

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

pub struct PortLeaseTransaction {
    _lock: LockGuard,
    registry_path: PathBuf,
    registry: Registry,
}

impl PortLeaseTransaction {
    pub fn used_ports(&self) -> BTreeSet<u16> {
        self.registry
            .leases
            .iter()
            .map(|lease| lease.port)
            .collect()
    }

    pub fn reserve(
        &mut self,
        owner: &str,
        identity: &LeaseIdentity,
        ports: &BTreeSet<u16>,
    ) -> anyhow::Result<()> {
        validate_request(owner, identity, ports)?;
        for lease in &self.registry.leases {
            if ports.contains(&lease.port) {
                anyhow::bail!(
                    "port {} is already leased to stackstead `{}` in project `{}`",
                    lease.port,
                    lease.stackstead_id,
                    lease.project
                );
            }
        }

        let mut updated = self.registry.clone();
        updated.leases.extend(ports.iter().map(|port| Lease {
            port: *port,
            owner: owner.to_owned(),
            stackstead_id: identity.stackstead_id.clone(),
            project: identity.project.clone(),
        }));
        updated.leases.sort_by_key(|lease| lease.port);
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
        if self.registry.leases.iter().any(|lease| {
            lease.owner == owner
                && (lease.stackstead_id != identity.stackstead_id
                    || lease.project != identity.project)
        }) {
            anyhow::bail!(
                "port leases for owner `{owner}` do not belong to stackstead `{}` in project `{}`",
                identity.stackstead_id,
                identity.project
            );
        }
        let actual = self
            .registry
            .leases
            .iter()
            .filter(|lease| lease.owner == owner)
            .map(|lease| lease.port)
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
            .leases
            .iter()
            .filter(|lease| lease.owner == owner)
            .map(|lease| lease.port)
            .collect::<BTreeSet<_>>();
        if actual.is_empty() {
            return Ok(());
        }
        if self.registry.leases.iter().any(|lease| {
            lease.owner == owner
                && (lease.stackstead_id != identity.stackstead_id
                    || lease.project != identity.project)
        }) {
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
        updated.leases.retain(|lease| lease.owner != owner);
        updated.save(&self.registry_path)?;
        self.registry = updated;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    kind: String,
    version: String,
    leases: Vec<Lease>,
}

impl Registry {
    fn empty() -> Self {
        Self {
            kind: REGISTRY_KIND.into(),
            version: REGISTRY_VERSION.into(),
            leases: Vec::new(),
        }
    }

    fn read(path: &Path) -> anyhow::Result<Self> {
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

    fn validate(&self, path: &Path) -> anyhow::Result<()> {
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

    fn save(&self, path: &Path) -> anyhow::Result<()> {
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
struct Lease {
    port: u16,
    owner: String,
    stackstead_id: String,
    project: String,
}

fn validate_request(
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

fn display_ports(ports: &BTreeSet<u16>) -> String {
    ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(directory: &tempfile::TempDir) -> PortLeaseStore {
        PortLeaseStore::at(directory.path().join("state"))
    }

    fn identity(name: &str) -> LeaseIdentity {
        LeaseIdentity::new(name, "demo")
    }

    fn ports(values: &[u16]) -> BTreeSet<u16> {
        values.iter().copied().collect()
    }

    #[test]
    fn resolves_per_user_state_paths_without_mutating_the_environment() {
        let xdg = PortLeaseStore::from_environment(Some("/state".into()), Some("/home/me".into()))
            .unwrap();
        assert_eq!(xdg.state_dir, Path::new("/state/stackstead"));

        let home =
            PortLeaseStore::from_environment(Some("relative".into()), Some("/home/me".into()))
                .unwrap();
        assert_eq!(
            home.state_dir,
            Path::new("/home/me/.local/state/stackstead")
        );
        assert!(PortLeaseStore::from_environment(None, Some("relative".into())).is_err());
        assert!(PortLeaseStore::from_environment(None, None).is_err());
    }

    #[test]
    fn independent_owners_conflict_but_disjoint_ports_succeed() {
        let directory = tempfile::tempdir().unwrap();
        let store = store(&directory);
        let mut transaction = store.transaction().unwrap();
        transaction
            .reserve("owner-a", &identity("alpha"), &ports(&[39000, 39001]))
            .unwrap();

        let error = transaction
            .reserve("owner-b", &identity("beta"), &ports(&[39001]))
            .unwrap_err();
        assert!(error.to_string().contains("alpha"));
        transaction
            .reserve("owner-b", &identity("beta"), &ports(&[39002]))
            .unwrap();
        assert_eq!(transaction.used_ports(), ports(&[39000, 39001, 39002]));
    }

    #[test]
    fn destroy_release_is_idempotent_only_after_the_exact_owner_is_gone() {
        let directory = tempfile::tempdir().unwrap();
        let store = store(&directory);
        let leased = ports(&[39000, 39001]);
        let mut transaction = store.transaction().unwrap();
        transaction
            .reserve("owner-a", &identity("alpha"), &leased)
            .unwrap();
        assert!(
            transaction
                .release_if_owned_or_absent("owner-a", &identity("alpha"), &ports(&[39000]))
                .is_err()
        );
        transaction
            .release_if_owned_or_absent("owner-a", &identity("alpha"), &leased)
            .unwrap();
        transaction
            .release_if_owned_or_absent("owner-a", &identity("alpha"), &leased)
            .unwrap();
    }

    #[test]
    fn transaction_holds_the_global_lock_for_its_lifetime() {
        let directory = tempfile::tempdir().unwrap();
        let store = store(&directory);
        let transaction = store.transaction().unwrap();
        assert!(store.transaction().is_err());
        drop(transaction);
        assert!(store.transaction().is_ok());
    }

    #[test]
    fn leases_persist_across_reopen_until_exact_release() {
        let directory = tempfile::tempdir().unwrap();
        let store = store(&directory);
        {
            let mut transaction = store.transaction().unwrap();
            transaction
                .reserve("owner-a", &identity("alpha"), &ports(&[39000, 39001]))
                .unwrap();
        }

        let mut transaction = store.transaction().unwrap();
        assert_eq!(transaction.used_ports(), ports(&[39000, 39001]));
        transaction
            .verify("owner-a", &identity("alpha"), &ports(&[39000, 39001]))
            .unwrap();
        assert!(
            transaction
                .release("owner-a", &identity("alpha"), &ports(&[39000]))
                .is_err()
        );
        assert_eq!(transaction.used_ports(), ports(&[39000, 39001]));
        transaction
            .release("owner-a", &identity("alpha"), &ports(&[39000, 39001]))
            .unwrap();
        assert!(transaction.used_ports().is_empty());
    }

    #[test]
    fn verify_and_release_reject_wrong_owner_or_mismatched_sets() {
        let directory = tempfile::tempdir().unwrap();
        let store = store(&directory);
        let mut transaction = store.transaction().unwrap();
        transaction
            .reserve("owner-a", &identity("alpha"), &ports(&[39000, 39001]))
            .unwrap();

        assert!(
            transaction
                .verify("owner-b", &identity("alpha"), &ports(&[39000, 39001]))
                .is_err()
        );
        assert!(
            transaction
                .verify("owner-a", &identity("other"), &ports(&[39000, 39001]))
                .is_err()
        );
        assert!(
            transaction
                .verify("owner-a", &identity("alpha"), &ports(&[39000]))
                .is_err()
        );
        assert!(
            transaction
                .release("owner-b", &identity("alpha"), &ports(&[39000, 39001]),)
                .is_err()
        );
        assert!(
            transaction
                .release("owner-a", &identity("alpha"), &ports(&[39001]))
                .is_err()
        );
        assert_eq!(transaction.used_ports(), ports(&[39000, 39001]));
    }

    #[test]
    fn malformed_duplicate_and_ambiguous_registries_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let store = store(&directory);
        std::fs::create_dir_all(&store.state_dir).unwrap();
        let path = store.state_dir.join(REGISTRY_FILE);

        std::fs::write(&path, b"not json").unwrap();
        assert!(store.transaction().is_err());

        std::fs::write(
            &path,
            br#"{"kind":"StacksteadPortLeaseRegistry","version":"1","leases":[{"port":39000,"owner":"a","stackstead_id":"alpha","project":"demo"},{"port":39000,"owner":"b","stackstead_id":"beta","project":"demo"}]}"#,
        )
        .unwrap();
        let error = match store.transaction() {
            Ok(_) => panic!("duplicate registry was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("duplicate"));

        std::fs::write(
            &path,
            br#"{"kind":"StacksteadPortLeaseRegistry","version":"1","leases":[{"port":39000,"owner":"a","stackstead_id":"alpha","project":"demo"},{"port":39001,"owner":"a","stackstead_id":"other","project":"demo"}]}"#,
        )
        .unwrap();
        let error = match store.transaction() {
            Ok(_) => panic!("ambiguous registry was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("ambiguous"));

        std::fs::write(
            &path,
            br#"{"kind":"StacksteadPortLeaseRegistry","version":"2","leases":[]}"#,
        )
        .unwrap();
        assert!(store.transaction().is_err());

        std::fs::write(
            &path,
            br#"{"kind":"StacksteadPortLeaseRegistry","version":"1","leases":[],"extra":true}"#,
        )
        .unwrap();
        assert!(store.transaction().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_registry_fails_closed_without_reading_its_target() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let store = store(&directory);
        std::fs::create_dir_all(&store.state_dir).unwrap();
        let target = directory.path().join("target.json");
        std::fs::write(&target, b"not json").unwrap();
        symlink(&target, store.state_dir.join(REGISTRY_FILE)).unwrap();

        let error = match store.transaction() {
            Ok(_) => panic!("symlinked registry was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("symlink"));
        assert_eq!(std::fs::read(&target).unwrap(), b"not json");
    }

    #[test]
    fn first_transaction_initializes_a_durable_empty_registry() {
        let directory = tempfile::tempdir().unwrap();
        let store = store(&directory);
        let path = store.state_dir.join(REGISTRY_FILE);
        let mut transaction = store.transaction().unwrap();
        assert!(transaction.used_ports().is_empty());
        assert!(path.is_file());

        assert!(
            transaction
                .reserve("", &identity("alpha"), &ports(&[39000]))
                .is_err()
        );
        assert!(path.is_file());

        transaction
            .reserve("owner-a", &identity("alpha"), &ports(&[39000]))
            .unwrap();
        assert!(path.is_file());
        assert!(store.state_dir.join(INITIALIZED_FILE).is_file());
        transaction
            .verify("owner-a", &identity("alpha"), &ports(&[39000]))
            .unwrap();
    }

    #[test]
    fn interrupted_lock_creation_does_not_wedge_first_initialization() {
        let directory = tempfile::tempdir().unwrap();
        let store = store(&directory);
        std::fs::create_dir_all(&store.state_dir).unwrap();
        std::fs::write(store.state_dir.join(LOCK_FILE), b"").unwrap();

        let transaction = store.transaction().unwrap();
        assert!(transaction.registry_path.is_file());
        assert!(store.state_dir.join(INITIALIZED_FILE).is_file());
    }

    #[test]
    fn initialized_registry_cannot_silently_reinitialize_after_deletion() {
        let directory = tempfile::tempdir().unwrap();
        let store = store(&directory);
        drop(store.transaction().unwrap());
        std::fs::remove_file(store.state_dir.join(REGISTRY_FILE)).unwrap();
        let error = match store.transaction() {
            Ok(_) => panic!("missing initialized registry was recreated"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("initialized port lease registry")
        );
    }
}
