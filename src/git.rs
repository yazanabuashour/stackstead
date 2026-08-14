use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::command;

const fn empty_env() -> BTreeMap<String, String> {
    BTreeMap::new()
}

pub fn repo_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    let output = command::run(
        "git",
        &["rev-parse".into(), "--show-toplevel".into()],
        cwd,
        &empty_env(),
    )?;
    Ok(PathBuf::from(String::from_utf8(output.stdout)?.trim()))
}

pub fn primary_worktree(cwd: &Path) -> anyhow::Result<PathBuf> {
    let output = command::run(
        "git",
        &[
            "worktree".into(),
            "list".into(),
            "--porcelain".into(),
            "-z".into(),
        ],
        cwd,
        &empty_env(),
    )?;
    let record = output
        .stdout
        .split(|byte| *byte == 0)
        .next()
        .context("Git did not report a primary worktree")?;
    let path = record
        .strip_prefix(b"worktree ")
        .context("Git did not report a primary worktree")?;
    std::fs::canonicalize(String::from_utf8(path.to_vec())?).context("resolve primary Git worktree")
}

mod worktree;
pub use worktree::registered_worktree_branch;

pub fn ensure_repository_ready(repo_root: &Path, base: &str) -> anyhow::Result<String> {
    command::run(
        "git",
        &["rev-parse".into(), "--verify".into(), "HEAD".into()],
        repo_root,
        &empty_env(),
    )?;
    let output = command::run(
        "git",
        &[
            "rev-parse".into(),
            "--verify".into(),
            format!("{base}^{{commit}}"),
        ],
        repo_root,
        &empty_env(),
    )?;
    Ok(String::from_utf8(output.stdout)?.trim().into())
}

pub fn ensure_revision_ancestor(checkout: &Path, revision: &str) -> anyhow::Result<()> {
    command::run(
        "git",
        &[
            "merge-base".into(),
            "--is-ancestor".into(),
            revision.into(),
            "HEAD".into(),
        ],
        checkout,
        &empty_env(),
    )
    .map(|_| ())
    .map_err(|error| {
        anyhow::anyhow!(
            "source checkout {} is not based on pinned commit {revision}; update or recreate the manager worktree before adoption: {error}",
            checkout.display()
        )
    })
}

pub fn ensure_contract_on_revision(
    checkout: &Path,
    revision: &str,
    compose_files: &[PathBuf],
) -> anyhow::Result<()> {
    let files = std::iter::once(Path::new(crate::config::CONFIG_FILE))
        .chain(compose_files.iter().map(PathBuf::as_path));
    for file in files {
        let object = format!("{revision}:{}", file.display());
        command::run(
            "git",
            &["cat-file".into(), "-e".into(), object],
            checkout,
            &empty_env(),
        )
        .map_err(|error| {
            anyhow::anyhow!(
                "runtime contract file `{}` is not present on source.base commit `{revision}`; commit or merge stackstead.yaml and its Compose files before provisioning: {error}",
                file.display()
            )
        })?;
        if command::run(
            "git",
            &[
                "diff".into(),
                "--quiet".into(),
                "--no-ext-diff".into(),
                revision.into(),
                "--".into(),
                file.display().to_string(),
            ],
            checkout,
            &empty_env(),
        )
        .is_err()
        {
            anyhow::bail!(
                "runtime contract file `{}` differs from source.base commit `{revision}`; commit or merge stackstead.yaml and its Compose files before provisioning",
                file.display()
            );
        }
    }
    Ok(())
}

pub fn create_worktree(
    repo_root: &Path,
    worktree: &Path,
    branch: &str,
    base: &str,
) -> anyhow::Result<()> {
    command::run(
        "git",
        &["check-ref-format".into(), "--branch".into(), branch.into()],
        repo_root,
        &empty_env(),
    )?;
    let branch_exists = command::run(
        "git",
        &[
            "show-ref".into(),
            "--verify".into(),
            "--quiet".into(),
            format!("refs/heads/{branch}"),
        ],
        repo_root,
        &empty_env(),
    )
    .is_ok();
    let args = if branch_exists {
        command::run(
            "git",
            &[
                "merge-base".into(),
                "--is-ancestor".into(),
                base.into(),
                branch.into(),
            ],
            repo_root,
            &empty_env(),
        )
        .map_err(|error| {
            anyhow::anyhow!(
                "existing branch `{branch}` does not contain pinned source.base commit `{base}`; merge or rebase it before recreating the stackstead: {error}"
            )
        })?;
        vec![
            "worktree".into(),
            "add".into(),
            worktree.display().to_string(),
            branch.into(),
        ]
    } else {
        vec![
            "worktree".into(),
            "add".into(),
            "-b".into(),
            branch.into(),
            worktree.display().to_string(),
            base.into(),
        ]
    };
    command::run("git", &args, repo_root, &empty_env())?;
    Ok(())
}

pub fn remove_worktree(repo_root: &Path, worktree: &Path) -> anyhow::Result<()> {
    command::run(
        "git",
        &[
            "worktree".into(),
            "remove".into(),
            worktree.display().to_string(),
        ],
        repo_root,
        &empty_env(),
    )?;
    Ok(())
}

pub fn is_registered_worktree(repo_root: &Path, worktree: &Path) -> anyhow::Result<bool> {
    let listed = command::run(
        "git",
        &[
            "worktree".into(),
            "list".into(),
            "--porcelain".into(),
            "-z".into(),
        ],
        repo_root,
        &empty_env(),
    )?;
    let expected = canonicalize_if_exists(worktree)?;
    for path in String::from_utf8(listed.stdout)?
        .split('\0')
        .filter_map(|field| field.strip_prefix("worktree "))
        .map(Path::new)
    {
        if canonicalize_if_exists(path)? == expected {
            return Ok(true);
        }
    }
    Ok(false)
}

fn canonicalize_if_exists(path: &Path) -> anyhow::Result<PathBuf> {
    crate::paths::resolve_existing_ancestor(path)
        .with_context(|| format!("cannot resolve worktree path {}", path.display()))
}

pub fn ensure_worktree_clean(worktree: &Path) -> anyhow::Result<()> {
    let output = command::run(
        "git",
        &[
            "status".into(),
            "--porcelain=v1".into(),
            "--untracked-files=all".into(),
        ],
        worktree,
        &empty_env(),
    )?;
    if !output.stdout.is_empty() {
        anyhow::bail!(
            "worktree {} has uncommitted or untracked changes; commit or remove them before destroy",
            worktree.display()
        );
    }
    Ok(())
}

mod exclude;
#[cfg(test)]
pub use exclude::ensure_excluded;
pub use exclude::{ensure_stackstead_excluded, is_stackstead_ignored};

#[cfg(all(test, unix))]
mod tests;
