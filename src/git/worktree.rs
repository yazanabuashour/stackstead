use std::path::{Path, PathBuf};

use super::empty_env;
use crate::command;

pub fn registered_worktree_branch(repo_root: &Path, worktree: &Path) -> anyhow::Result<String> {
    let worktree = std::fs::canonicalize(worktree).map_err(|error| {
        anyhow::anyhow!("cannot access worktree {}: {error}", worktree.display())
    })?;
    let top_level = super::repo_root(&worktree)?;
    if std::fs::canonicalize(&top_level)? != worktree {
        anyhow::bail!(
            "worktree path must be its Git checkout root: {}",
            worktree.display()
        );
    }
    if git_common_dir(repo_root)? != git_common_dir(&worktree)? {
        anyhow::bail!(
            "{} is not a registered worktree of {}",
            worktree.display(),
            repo_root.display()
        );
    }
    let output = command::run(
        "git",
        &[
            "symbolic-ref".into(),
            "--quiet".into(),
            "--short".into(),
            "HEAD".into(),
        ],
        &worktree,
        &empty_env(),
    )
    .map_err(|error| anyhow::anyhow!("worktree must have a checked-out branch: {error}"))?;
    Ok(String::from_utf8(output.stdout)?.trim().into())
}

fn git_common_dir(cwd: &Path) -> anyhow::Result<PathBuf> {
    let output = command::run(
        "git",
        &[
            "rev-parse".into(),
            "--path-format=absolute".into(),
            "--git-common-dir".into(),
        ],
        cwd,
        &empty_env(),
    )?;
    std::fs::canonicalize(PathBuf::from(String::from_utf8(output.stdout)?.trim()))
        .map_err(Into::into)
}
