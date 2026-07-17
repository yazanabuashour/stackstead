use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::Path,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const EVENT_VERSION: &str = "1";
const MAX_EVENT_BYTES: usize = 1024 * 1024;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Create,
    Adopt,
    PointerGenerate,
    EnvironmentGenerate,
    ContextGenerate,
    DependenciesInstall,
    RuntimeStart,
    DatabaseWait,
    DatabaseSeed,
    HealthWait,
    RuntimeStop,
    Repair,
    Destroy,
    RuntimeRemove,
    SourceRemove,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Started,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub kind: String,
    pub version: String,
    pub timestamp: DateTime<Utc>,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub status: EventStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLog {
    pub events: Vec<Event>,
    pub truncated_tail: bool,
}

pub fn append(
    path: &Path,
    event_type: EventType,
    status: EventStatus,
    message: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let event = Event {
        kind: "StacksteadEvent".into(),
        version: EVENT_VERSION.into(),
        timestamp: Utc::now(),
        event_type,
        status,
        message: message.map(bounded_message),
    };
    let mut encoded = serde_json::to_vec(&event)?;
    encoded.push(b'\n');
    if encoded.len() > MAX_EVENT_BYTES {
        anyhow::bail!("event record exceeds 1 MiB after message truncation");
    }
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    truncate_torn_tail(&mut file)?;
    file.write_all(&encoded)?;
    file.sync_data()?;
    Ok(())
}

pub fn read(path: &Path) -> anyhow::Result<EventLog> {
    let file = File::open(path)
        .map_err(|error| anyhow::anyhow!("cannot read event log {}: {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut events = Vec::new();
    let mut truncated_tail = false;
    let mut record = 0usize;
    loop {
        let mut line = Vec::new();
        let read = reader
            .by_ref()
            .take((MAX_EVENT_BYTES + 2) as u64)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        record += 1;
        let terminated = line.last() == Some(&b'\n');
        if terminated {
            line.pop();
        }
        if line.len() > MAX_EVENT_BYTES {
            anyhow::bail!(
                "event log {} record {} exceeds 1 MiB",
                path.display(),
                record
            );
        }
        if !terminated {
            truncated_tail = true;
            break;
        }
        if line.is_empty() {
            anyhow::bail!(
                "event log {} contains a blank line at {}",
                path.display(),
                record
            );
        }
        let event: Event = serde_json::from_slice(&line).map_err(|error| {
            anyhow::anyhow!(
                "invalid event log {} record {}: {error}",
                path.display(),
                record
            )
        })?;
        if event.kind != "StacksteadEvent" || event.version != EVENT_VERSION {
            anyhow::bail!(
                "unsupported event contract in {} record {}: kind={} version={}",
                path.display(),
                record,
                event.kind,
                event.version
            );
        }
        events.push(event);
    }
    Ok(EventLog {
        events,
        truncated_tail,
    })
}

fn truncate_torn_tail(file: &mut File) -> anyhow::Result<()> {
    let mut end = file.metadata()?.len();
    if end == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    if last[0] == b'\n' {
        return Ok(());
    }

    let mut buffer = [0u8; 8192];
    while end > 0 {
        let start = end.saturating_sub(buffer.len() as u64);
        let length = usize::try_from(end - start)?;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..length])?;
        if let Some(index) = buffer[..length].iter().rposition(|byte| *byte == b'\n') {
            file.set_len(start + index as u64 + 1)?;
            return Ok(());
        }
        end = start;
    }
    file.set_len(0)?;
    Ok(())
}

fn bounded_message(message: &str) -> String {
    let mut message = redact_message(message);
    if message.len() <= MAX_MESSAGE_BYTES {
        return message;
    }
    let mut end = MAX_MESSAGE_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message.truncate(end);
    message.push_str(" [truncated]");
    message
}

fn redact_message(message: &str) -> String {
    crate::command::redact(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_typed_synced_lines_and_redacts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        append(
            &path,
            EventType::Create,
            EventStatus::Succeeded,
            Some("TOKEN=private done"),
        )
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        let log = read(&path).unwrap();
        assert_eq!(log.events.len(), 1);
        assert_eq!(log.events[0].event_type, EventType::Create);
        assert!(!String::from_utf8(bytes).unwrap().contains("private"));
    }

    #[test]
    fn event_messages_use_the_shared_redaction_policy() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        append(
            &path,
            EventType::DependenciesInstall,
            EventStatus::Failed,
            Some(
                "Authorization: Bearer header-secret\nAUTH_TOKEN=\"quoted secret\"\nfatal: https://user:password@example.invalid/repo\nordinary  detail",
            ),
        )
        .unwrap();

        let message = read(&path).unwrap().events[0].message.clone().unwrap();
        assert_eq!(
            message,
            "Authorization: [REDACTED]\nAUTH_TOKEN=[REDACTED]\nfatal: https://[REDACTED]@example.invalid/repo\nordinary  detail"
        );
        for secret in ["header-secret", "quoted secret", "user:password"] {
            assert!(!message.contains(secret));
        }
    }

    #[test]
    fn ignores_only_an_unterminated_tail() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        append(&path, EventType::Destroy, EventStatus::Started, None).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"kind\":\"StacksteadEvent\"")
            .unwrap();
        let log = read(&path).unwrap();
        assert!(log.truncated_tail);
        assert_eq!(log.events.len(), 1);
    }

    #[test]
    fn appending_discards_a_torn_tail() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        append(&path, EventType::Destroy, EventStatus::Started, None).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"kind\":\"StacksteadEvent\"")
            .unwrap();
        append(
            &path,
            EventType::RuntimeRemove,
            EventStatus::Succeeded,
            None,
        )
        .unwrap();
        let log = read(&path).unwrap();
        assert!(!log.truncated_tail);
        assert_eq!(log.events.len(), 2);
        assert_eq!(log.events[1].event_type, EventType::RuntimeRemove);
    }

    #[test]
    fn oversized_messages_are_bounded_before_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        append(
            &path,
            EventType::RuntimeStart,
            EventStatus::Failed,
            Some(&"x".repeat(MAX_EVENT_BYTES * 2)),
        )
        .unwrap();
        let log = read(&path).unwrap();
        let message = log.events[0].message.as_deref().unwrap();
        assert!(message.ends_with(" [truncated]"));
        assert!(message.len() < MAX_EVENT_BYTES);
    }

    #[test]
    fn rejects_malformed_completed_records() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        std::fs::write(&path, b"not-json\n").unwrap();
        assert!(read(&path).is_err());
        std::fs::write(&path, b"\n").unwrap();
        assert!(read(&path).is_err());
    }
}
