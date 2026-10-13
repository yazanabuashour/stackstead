use std::path::Path;

use super::{empty_env, exclude};
use crate::command;

pub fn is_tracked(worktree: &Path, relative: &Path) -> anyhow::Result<bool> {
    let output = command::run(
        "git",
        &[
            "--literal-pathspecs".into(),
            "ls-files".into(),
            "-z".into(),
            "--".into(),
            relative.display().to_string(),
        ],
        worktree,
        &empty_env(),
    )?;
    Ok(!output.stdout.is_empty())
}

fn same_generated_entry(
    worktree: &Path,
    reported: &[u8],
    generated: &Path,
) -> anyhow::Result<bool> {
    if generated
        .to_str()
        .is_some_and(|path| path.as_bytes() == reported)
    {
        return Ok(true);
    }
    let Ok(reported) = std::str::from_utf8(reported) else {
        return Ok(false);
    };
    let reported = worktree.join(reported);
    let generated = worktree.join(generated);
    if !std::fs::symlink_metadata(&reported)?.is_file()
        || !same_file::is_same_file(&reported, &generated)?
    {
        return Ok(false);
    }
    let (Some(reported_parent), Some(generated_parent)) = (reported.parent(), generated.parent())
    else {
        return Ok(false);
    };
    if !same_file::is_same_file(reported_parent, generated_parent)? {
        return Ok(false);
    }
    let (Some(reported_name), Some(generated_name)) = (reported.file_name(), generated.file_name())
    else {
        return Ok(false);
    };
    if reported_name == generated_name {
        return Ok(true);
    }
    let names = std::fs::read_dir(generated_parent)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    Ok(!names.iter().any(|name| name == reported_name)
        || !names.iter().any(|name| name == generated_name))
}

pub fn ensure_worktree_clean(worktree: &Path, generated: &[&Path]) -> anyhow::Result<()> {
    let (exclude_path, _guard) = exclude::lock_excludes(worktree)?;
    let exclusions = match std::fs::read_to_string(&exclude_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let output = command::run(
        "git",
        &[
            "status".into(),
            "--porcelain=v1".into(),
            "-z".into(),
            "--untracked-files=all".into(),
            "--ignored=matching".into(),
        ],
        worktree,
        &empty_env(),
    )?;
    for row in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
    {
        let ignored = row.starts_with(b"!! ");
        if let Some(path) = row
            .strip_prefix(b"!! ")
            .or_else(|| row.strip_prefix(b"?? "))
        {
            if ignored
                && !exclude::is_generated_exclusion(worktree, path, &exclude_path, &exclusions)?
            {
                continue;
            }
            let mut owned = false;
            for generated in generated {
                if same_generated_entry(worktree, path, generated)? {
                    owned = true;
                    break;
                }
            }
            if owned {
                continue;
            }
        }
        anyhow::bail!(
            "worktree {} has uncommitted or untracked changes; commit or remove them before destroy",
            worktree.display()
        );
    }
    Ok(())
}
