use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;

use super::empty_env;
use crate::{command, lock::LockGuard};

const GENERATED_MARKER: &str = "# Stackstead generated output";

pub fn ensure_generated_excluded<'a>(
    worktree: &Path,
    outputs: impl IntoIterator<Item = &'a PathBuf>,
) -> anyhow::Result<PathBuf> {
    let exclude = ensure_excluded(worktree, ".stackstead/")?;
    for output in outputs {
        let normalized = output
            .components()
            .filter(|part| !matches!(part, std::path::Component::CurDir))
            .collect::<PathBuf>();
        let relative = normalized
            .to_str()
            .context("generated output path is not UTF-8")?;
        // Git patterns are physical lines; lifecycle clean checks validate these exact owned paths.
        if relative.contains(['\r', '\n']) {
            continue;
        }
        let mut pattern = String::from("/");
        for character in relative.chars() {
            if matches!(character, '\\' | '*' | '?' | '[' | ']' | '!' | '#' | ' ') {
                pattern.push('\\');
            }
            pattern.push(character);
        }
        append_excluded(worktree, &pattern, Some(GENERATED_MARKER))?;
    }
    Ok(exclude)
}

pub(super) fn lock_excludes(repository: &Path) -> anyhow::Result<(PathBuf, LockGuard)> {
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
    if !path.is_absolute() {
        anyhow::bail!("Git returned a non-absolute exclusion path");
    }
    let guard = LockGuard::acquire(
        &path.with_file_name("exclude.stackstead.lock"),
        "Git generated exclusions",
    )?;
    Ok((path, guard))
}

pub fn ensure_excluded(repository: &Path, pattern: &str) -> anyhow::Result<PathBuf> {
    append_excluded(repository, pattern, None)
}

fn append_excluded(
    repository: &Path,
    pattern: &str,
    marker: Option<&str>,
) -> anyhow::Result<PathBuf> {
    let (path, _guard) = lock_excludes(repository)?;
    let existing = match std::fs::read_to_string(&path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("cannot read Git exclude file {}", path.display()));
        }
    };
    if !existing.lines().any(|line| line == pattern) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        if !existing.is_empty() && !existing.ends_with('\n') {
            writeln!(file)?;
        }
        if let Some(marker) = marker {
            writeln!(file, "{marker}")?;
        }
        writeln!(file, "{pattern}")?;
    }
    Ok(path)
}

pub(super) fn is_generated_exclusion(
    worktree: &Path,
    relative: &[u8],
    exclude: &Path,
    contents: &str,
) -> anyhow::Result<bool> {
    let mut input = tempfile::NamedTempFile::new()?;
    input.write_all(b"./")?;
    input.write_all(relative)?;
    input.write_all(&[0])?;
    let output = command::run_with_stdin_file(
        "git",
        &[
            "check-ignore".into(),
            "--verbose".into(),
            "-z".into(),
            "--stdin".into(),
        ],
        worktree,
        &empty_env(),
        input.reopen()?,
    )?;
    let fields = output.stdout.split(|byte| *byte == 0).collect::<Vec<_>>();
    let [source, line, _, _, ..] = fields.as_slice() else {
        anyhow::bail!("invalid Git ignore provenance");
    };
    let source = crate::paths::absolute_from(worktree, Path::new(std::str::from_utf8(source)?))?;
    if source != exclude {
        return Ok(false);
    }
    let line = std::str::from_utf8(line)?.parse::<usize>()?;
    Ok(line
        .checked_sub(2)
        .and_then(|index| contents.lines().nth(index))
        == Some(GENERATED_MARKER))
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
