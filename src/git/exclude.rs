use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;

use super::empty_env;
use crate::command;

pub fn ensure_stackstead_excluded(worktree: &Path) -> anyhow::Result<PathBuf> {
    ensure_excluded(worktree, ".stackstead/")
}

pub fn ensure_excluded(repository: &Path, pattern: &str) -> anyhow::Result<PathBuf> {
    let output = command::run(
        "git",
        &[
            "rev-parse".into(),
            "--path-format=absolute".into(),
            "--git-path".into(),
            "info/exclude".into(),
        ],
        repository,
        &empty_env(),
    )?;
    let path = PathBuf::from(String::from_utf8(output.stdout)?.trim());
    let existing = match std::fs::read_to_string(&path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("cannot read Git exclude file {}", path.display()));
        }
    };
    if !existing.lines().any(|line| line.trim() == pattern) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        if !existing.is_empty() && !existing.ends_with('\n') {
            writeln!(file)?;
        }
        writeln!(file, "{pattern}")?;
    }
    Ok(path)
}

pub fn is_stackstead_ignored(worktree: &Path) -> bool {
    command::run(
        "git",
        &[
            "check-ignore".into(),
            "--quiet".into(),
            ".stackstead/stackstead.json".into(),
        ],
        worktree,
        &empty_env(),
    )
    .is_ok()
}
