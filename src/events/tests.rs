use super::*;
use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};

#[test]
fn appends_typed_synced_lines_and_redacts() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("events.jsonl");
    append(
        &path,
        EventType::Create,
        EventStatus::Succeeded,
        Some("TOKEN=private done"),
    )
    .test()?;
    let bytes = std::fs::read(&path).test()?;
    assert_eq!(bytes.last(), Some(&b'\n'), "test contract values differ");
    let log = read(&path).test()?;
    assert_eq!(log.events.len(), 1, "test contract values differ");
    assert_eq!(
        log.events[0].event_type,
        EventType::Create,
        "test contract values differ"
    );
    assert!(
        !String::from_utf8(bytes).test()?.contains("private"),
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn event_messages_use_the_shared_redaction_policy() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("events.jsonl");
    append(
            &path,
            EventType::DependenciesInstall,
            EventStatus::Failed,
            Some(
                "Authorization: Bearer header-secret\nAUTH_TOKEN=\"quoted secret\"\nfatal: https://user:password@example.invalid/repo\nordinary  detail",
            ),
        )
        .test()?;

    let message = read(&path).test()?.events[0].message.clone().test()?;
    assert_eq!(
        message,
        "Authorization: [REDACTED]\nAUTH_TOKEN=[REDACTED]\nfatal: https://[REDACTED]@example.invalid/repo\nordinary  detail",
        "test contract values differ"
    );
    for secret in ["header-secret", "quoted secret", "user:password"] {
        assert!(!message.contains(secret), "test contract condition failed");
    }
    Ok(())
}

#[test]
fn ignores_only_an_unterminated_tail() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("events.jsonl");
    append(&path, EventType::Destroy, EventStatus::Started, None).test()?;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .test()?
        .write_all(b"{\"kind\":\"StacksteadEvent\"")
        .test()?;
    let log = read(&path).test()?;
    assert!(log.truncated_tail, "test contract condition failed");
    assert_eq!(log.events.len(), 1, "test contract values differ");
    Ok(())
}

#[test]
fn appending_discards_a_torn_tail() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("events.jsonl");
    append(&path, EventType::Destroy, EventStatus::Started, None).test()?;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .test()?
        .write_all(b"{\"kind\":\"StacksteadEvent\"")
        .test()?;
    append(
        &path,
        EventType::RuntimeRemove,
        EventStatus::Succeeded,
        None,
    )
    .test()?;
    let log = read(&path).test()?;
    assert!(!log.truncated_tail, "test contract condition failed");
    assert_eq!(log.events.len(), 2, "test contract values differ");
    assert_eq!(
        log.events[1].event_type,
        EventType::RuntimeRemove,
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn oversized_messages_are_bounded_before_writing() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("events.jsonl");
    append(
        &path,
        EventType::RuntimeStart,
        EventStatus::Failed,
        Some(&"x".repeat(MAX_EVENT_BYTES * 2)),
    )
    .test()?;
    let log = read(&path).test()?;
    let message = log.events[0].message.as_deref().test()?;
    assert!(
        message.ends_with(" [truncated]"),
        "test contract condition failed"
    );
    assert!(
        message.len() < MAX_EVENT_BYTES,
        "test contract condition failed"
    );
    Ok(())
}

#[test]
fn rejects_malformed_completed_records() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let path = directory.path().join("events.jsonl");
    std::fs::write(&path, b"not-json\n").test()?;
    (read(&path)).test_err()?;
    std::fs::write(&path, b"\n").test()?;
    (read(&path)).test_err()?;
    Ok(())
}
