use super::*;

#[test]
fn malformed_duplicate_and_ambiguous_registries_fail_closed() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let store = store(&directory);
    std::fs::create_dir_all(&store.state_dir).test()?;
    let path = store.state_dir.join(REGISTRY_FILE);

    std::fs::write(&path, b"not json").test()?;
    (store.transaction()).test_err()?;

    std::fs::write(
            &path,
            br#"{"kind":"StacksteadPortLeaseRegistry","version":"1","leases":[{"port":39000,"owner":"a","stackstead_id":"alpha","project":"demo"},{"port":39000,"owner":"b","stackstead_id":"beta","project":"demo"}]}"#,
        )
        .test()?;
    let Err(error) = store.transaction() else {
        anyhow::bail!("duplicate registry was accepted");
    };
    assert!(error.to_string().contains("duplicate"));

    std::fs::write(
            &path,
            br#"{"kind":"StacksteadPortLeaseRegistry","version":"1","leases":[{"port":39000,"owner":"a","stackstead_id":"alpha","project":"demo"},{"port":39001,"owner":"a","stackstead_id":"other","project":"demo"}]}"#,
        )
        .test()?;
    let Err(error) = store.transaction() else {
        anyhow::bail!("ambiguous registry was accepted");
    };
    assert!(error.to_string().contains("ambiguous"));

    std::fs::write(
        &path,
        br#"{"kind":"StacksteadPortLeaseRegistry","version":"2","leases":[]}"#,
    )
    .test()?;
    (store.transaction()).test_err()?;

    std::fs::write(
        &path,
        br#"{"kind":"StacksteadPortLeaseRegistry","version":"1","leases":[],"extra":true}"#,
    )
    .test()?;
    (store.transaction()).test_err()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlinked_registry_fails_closed_without_reading_its_target() -> anyhow::Result<()> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().test()?;
    let store = store(&directory);
    std::fs::create_dir_all(&store.state_dir).test()?;
    let target = directory.path().join("target.json");
    std::fs::write(&target, b"not json").test()?;
    symlink(&target, store.state_dir.join(REGISTRY_FILE)).test()?;

    let Err(error) = store.transaction() else {
        anyhow::bail!("symlinked registry was accepted");
    };
    assert!(error.to_string().contains("symlink"));
    assert_eq!(std::fs::read(&target).test()?, b"not json");
    Ok(())
}

#[test]
fn first_transaction_initializes_a_durable_empty_registry() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let store = store(&directory);
    let path = store.state_dir.join(REGISTRY_FILE);
    let mut transaction = store.transaction().test()?;
    assert!(transaction.used_ports().is_empty());
    assert!(path.is_file());

    (transaction.reserve("", &identity("alpha"), &ports(&[39000]))).test_err()?;
    assert!(path.is_file());

    transaction
        .reserve("owner-a", &identity("alpha"), &ports(&[39000]))
        .test()?;
    assert!(path.is_file());
    assert!(store.state_dir.join(INITIALIZED_FILE).is_file());
    transaction
        .verify("owner-a", &identity("alpha"), &ports(&[39000]))
        .test()?;
    Ok(())
}

#[test]
fn interrupted_lock_creation_does_not_wedge_first_initialization() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let store = store(&directory);
    std::fs::create_dir_all(&store.state_dir).test()?;
    std::fs::write(store.state_dir.join(LOCK_FILE), b"").test()?;

    let transaction = store.transaction().test()?;
    assert!(transaction.registry_path.is_file());
    assert!(store.state_dir.join(INITIALIZED_FILE).is_file());
    Ok(())
}

#[test]
fn initialized_registry_cannot_silently_reinitialize_after_deletion() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let store = store(&directory);
    drop(store.transaction().test()?);
    std::fs::remove_file(store.state_dir.join(REGISTRY_FILE)).test()?;
    let Err(error) = store.transaction() else {
        anyhow::bail!("missing initialized registry was recreated");
    };
    assert!(
        error
            .to_string()
            .contains("initialized port lease registry")
    );
    Ok(())
}
