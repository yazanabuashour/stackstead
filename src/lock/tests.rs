use super::*;
use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};

#[cfg(unix)]
#[test]
fn lock_acquisition_rejects_symlinks_without_modifying_the_target() -> anyhow::Result<()> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().test()?;
    let target = directory.path().join("target");
    let lock = directory.path().join("lock");
    std::fs::write(&target, b"unchanged").test()?;
    symlink(&target, &lock).test()?;

    (LockGuard::acquire(&lock, "stackstead")).test_err()?;
    (LockGuard::acquire_existing(&lock, "stackstead")).test_err()?;
    (LockGuard::acquire_existing_shared(&lock, "stackstead")).test_err()?;
    assert!(!LockGuard::can_acquire(&lock));
    assert_eq!(std::fs::read(&target).test()?, b"unchanged");
    Ok(())
}

#[test]
fn lock_acquisition_preserves_regular_file_behavior() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("lock");

    drop(LockGuard::acquire(&path, "stackstead").test()?);
    assert!(LockGuard::can_acquire(&path));
    drop(LockGuard::acquire_existing(&path, "stackstead").test()?);
    drop(LockGuard::acquire_existing_shared(&path, "stackstead").test()?);
    Ok(())
}

#[test]
fn exclusive_lock_can_be_downgraded_for_shared_run_leases() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("lock");
    let lock = LockGuard::acquire(&path, "stackstead").test()?;

    let lock = lock.downgrade_to_shared().test()?;
    drop(LockGuard::acquire_existing_shared(&path, "stackstead").test()?);
    let contender = open_lock(&path, false).test()?;
    (wait_for_lock(
        &contender,
        &path,
        "stackstead",
        false,
        Duration::from_millis(100),
    ))
    .test_err()?;
    drop(lock);
    drop(LockGuard::acquire_existing(&path, "stackstead").test()?);
    Ok(())
}

#[cfg(unix)]
#[test]
fn handoff_closes_without_unlocking_the_inherited_file_description() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("lock");
    let lock = LockGuard::acquire(&path, "stackstead").test()?;
    let inherited = lock.file.try_clone().test()?;

    lock.close_after_handoff();
    (open_lock(&path, false).test()?.try_lock_exclusive()).test_err()?;

    drop(inherited);
    drop(LockGuard::acquire_existing(&path, "stackstead").test()?);
    Ok(())
}

#[test]
fn bounded_wait_acquires_after_the_contender_releases() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("lock");
    let lock = LockGuard::acquire(&path, "stackstead").test()?;
    let contender = open_lock(&path, false).test()?;
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        drop(lock);
    });

    wait_for_lock(
        &contender,
        &path,
        "stackstead",
        false,
        Duration::from_secs(1),
    )
    .test()?;
    release.join().test()?;
    Ok(())
}

#[test]
fn existing_lock_acquisition_never_recreates_destroyed_state() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("destroyed/state/lock");
    (LockGuard::acquire_existing(&path, "stackstead")).test_err()?;
    assert!(!directory.path().join("destroyed").exists());
    (LockGuard::acquire_existing_shared(&path, "stackstead")).test_err()?;
    assert!(!directory.path().join("destroyed").exists());
    Ok(())
}
