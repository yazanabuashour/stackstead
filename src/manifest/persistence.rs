use std::{
    fs::File,
    io::{BufReader, BufWriter, Write},
    path::Path,
};

use serde::Serialize;

use super::{POINTER_VERSION, StacksteadPointer};

impl StacksteadPointer {
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let file = File::open(path)
            .map_err(|error| anyhow::anyhow!("cannot open pointer {}: {error}", path.display()))?;
        let value: serde_json::Value = serde_json::from_reader(BufReader::new(file))
            .map_err(|error| anyhow::anyhow!("cannot parse pointer {}: {error}", path.display()))?;
        let kind = value.get("kind").and_then(serde_json::Value::as_str);
        let version = value.get("version").and_then(serde_json::Value::as_str);
        if kind != Some("StacksteadPointer") || version != Some(POINTER_VERSION) {
            anyhow::bail!(
                "unsupported pointer contract in {}: kind={} version={}",
                path.display(),
                kind.unwrap_or("<missing>"),
                version.unwrap_or("<missing>")
            );
        }
        serde_json::from_value(value)
            .map_err(|error| anyhow::anyhow!("cannot parse pointer {}: {error}", path.display()))
    }
}

pub fn write_pointer(path: &Path, pointer: &StacksteadPointer) -> anyhow::Result<()> {
    write_json_atomic(path, pointer)
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut writer = BufWriter::new(temp.as_file_mut());
        serde_json::to_writer_pretty(&mut writer, value)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| anyhow::anyhow!("cannot replace {}: {}", path.display(), error.error))?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}
