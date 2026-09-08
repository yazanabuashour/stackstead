use super::*;
use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};

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
fn resolves_per_user_state_paths_without_mutating_the_environment() -> anyhow::Result<()> {
    let xdg =
        PortLeaseStore::from_environment(Some("/state".into()), Some("/home/me".into())).test()?;
    assert_eq!(xdg.state_dir, Path::new("/state/stackstead"));

    let home = PortLeaseStore::from_environment(Some("relative".into()), Some("/home/me".into()))
        .test()?;
    assert_eq!(
        home.state_dir,
        Path::new("/home/me/.local/state/stackstead")
    );
    (PortLeaseStore::from_environment(None, Some("relative".into()))).test_err()?;
    (PortLeaseStore::from_environment(None, None)).test_err()?;
    Ok(())
}

#[test]
fn independent_owners_conflict_but_disjoint_ports_succeed() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let store = store(&directory);
    let mut transaction = store.transaction().test()?;
    transaction
        .reserve("owner-a", &identity("alpha"), &ports(&[39000, 39001]))
        .test()?;

    let error = transaction
        .reserve("owner-b", &identity("beta"), &ports(&[39001]))
        .test_err()?;
    assert!(error.to_string().contains("alpha"));
    transaction
        .reserve("owner-b", &identity("beta"), &ports(&[39002]))
        .test()?;
    assert_eq!(transaction.used_ports(), ports(&[39000, 39001, 39002]));
    Ok(())
}

#[test]
fn destroy_release_is_idempotent_only_after_the_exact_owner_is_gone() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let store = store(&directory);
    let leased = ports(&[39000, 39001]);
    let mut transaction = store.transaction().test()?;
    transaction
        .reserve("owner-a", &identity("alpha"), &leased)
        .test()?;
    (transaction.release_if_owned_or_absent("owner-a", &identity("alpha"), &ports(&[39000])))
        .test_err()?;
    transaction
        .release_if_owned_or_absent("owner-a", &identity("alpha"), &leased)
        .test()?;
    transaction
        .release_if_owned_or_absent("owner-a", &identity("alpha"), &leased)
        .test()?;
    Ok(())
}

#[test]
fn transaction_holds_the_global_lock_for_its_lifetime() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let store = store(&directory);
    let lock_path = store.state_dir().join(LOCK_FILE);
    let transaction = store.transaction().test()?;
    assert!(
        !LockGuard::can_acquire(&lock_path),
        "the registry lock must stay held while its transaction lives"
    );
    drop(transaction);
    assert!(
        LockGuard::can_acquire(&lock_path),
        "dropping the transaction must release the registry lock"
    );
    (store.transaction()).test()?;
    Ok(())
}

#[test]
fn leases_persist_across_reopen_until_exact_release() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let store = store(&directory);
    {
        let mut transaction = store.transaction().test()?;
        transaction
            .reserve("owner-a", &identity("alpha"), &ports(&[39000, 39001]))
            .test()?;
    }

    let mut transaction = store.transaction().test()?;
    assert_eq!(transaction.used_ports(), ports(&[39000, 39001]));
    transaction
        .verify("owner-a", &identity("alpha"), &ports(&[39000, 39001]))
        .test()?;
    (transaction.release("owner-a", &identity("alpha"), &ports(&[39000]))).test_err()?;
    assert_eq!(transaction.used_ports(), ports(&[39000, 39001]));
    transaction
        .release("owner-a", &identity("alpha"), &ports(&[39000, 39001]))
        .test()?;
    assert!(transaction.used_ports().is_empty());
    Ok(())
}

#[test]
fn verify_and_release_reject_wrong_owner_or_mismatched_sets() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let store = store(&directory);
    let mut transaction = store.transaction().test()?;
    transaction
        .reserve("owner-a", &identity("alpha"), &ports(&[39000, 39001]))
        .test()?;

    (transaction.verify("owner-b", &identity("alpha"), &ports(&[39000, 39001]))).test_err()?;
    (transaction.verify("owner-a", &identity("other"), &ports(&[39000, 39001]))).test_err()?;
    (transaction.verify("owner-a", &identity("alpha"), &ports(&[39000]))).test_err()?;
    (transaction.release("owner-b", &identity("alpha"), &ports(&[39000, 39001]))).test_err()?;
    (transaction.release("owner-a", &identity("alpha"), &ports(&[39001]))).test_err()?;
    assert_eq!(transaction.used_ports(), ports(&[39000, 39001]));
    Ok(())
}

mod registry_cases;
