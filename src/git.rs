use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::command;

fn empty_env() -> BTreeMap<String, String> {
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

pub fn registered_worktree_branch(repo_root: &Path, worktree: &Path) -> anyhow::Result<String> {
    let worktree = std::fs::canonicalize(worktree).map_err(|error| {
        anyhow::anyhow!("cannot access worktree {}: {error}", worktree.display())
    })?;
    let top_level = self::repo_root(&worktree)?;
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
    .map_err(|_| anyhow::anyhow!("worktree must have a checked-out branch"))?;
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
    .map_err(|_| {
        anyhow::anyhow!(
            "source checkout {} is not based on pinned commit {revision}; update or recreate the manager worktree before adoption",
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
        .map_err(|_| {
            anyhow::anyhow!(
                "runtime contract file `{}` is not present on source.base commit `{revision}`; commit or merge stackstead.yaml and its Compose files before provisioning",
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
        .map_err(|_| {
            anyhow::anyhow!(
                "existing branch `{branch}` does not contain pinned source.base commit `{base}`; merge or rebase it before recreating the stackstead"
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

#[cfg(all(test, unix))]
mod tests {
    use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};
    use std::os::unix::fs::symlink;
    use std::process::Command;

    use super::*;

    fn git(repository: &Path, arguments: &[&str]) -> anyhow::Result<()> {
        assert!(
            Command::new("git")
                .args(arguments)
                .current_dir(repository)
                .status()
                .test()?
                .success()
        );
        Ok(())
    }

    #[test]
    fn registered_worktree_parser_preserves_newlines_in_paths() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let repository = directory.path().join("repository");
        let worktrees = directory.path().join("worktrees");
        let worktree_alias = directory.path().join("worktree-alias");
        std::fs::create_dir(&repository).test()?;
        std::fs::create_dir(&worktrees).test()?;
        symlink(&worktrees, &worktree_alias).test()?;
        git(&repository, &["init", "-q"])?;
        git(
            &repository,
            &["config", "user.email", "stackstead-tests@example.invalid"],
        )?;
        git(&repository, &["config", "user.name", "Stackstead Tests"])?;
        std::fs::write(repository.join("README.md"), "test\n").test()?;
        git(&repository, &["add", "README.md"])?;
        git(&repository, &["commit", "-qm", "initial"])?;

        let worktree = worktree_alias.join("line\nbreak");
        let output = Command::new("git")
            .args(["worktree", "add", "-q", "-b", "newline-path"])
            .arg(&worktree)
            .current_dir(&repository)
            .status()
            .test()?;
        assert!(output.success());
        assert!(is_registered_worktree(&repository, &worktree).test()?);
        std::fs::remove_dir_all(&worktree).test()?;
        assert!(is_registered_worktree(&repository, &worktree).test()?);
        Ok(())
    }

    #[test]
    fn registered_worktree_check_propagates_canonicalization_errors() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let repository = directory.path().join("repository");
        std::fs::create_dir(&repository).test()?;
        git(&repository, &["init", "-q"])?;
        let loop_path = directory.path().join("loop");
        symlink(&loop_path, &loop_path).test()?;

        let error = is_registered_worktree(&repository, &loop_path)
            .test_err()?
            .to_string();

        assert!(error.contains("cannot resolve worktree path"));
        Ok(())
    }

    #[test]
    fn ensure_excluded_preserves_invalid_utf8_on_error() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let repository = directory.path().join("repository");
        std::fs::create_dir(&repository).test()?;
        git(&repository, &["init", "-q"])?;
        let exclude = repository.join(".git/info/exclude");
        let original = b"existing\n\xffinvalid\n";
        std::fs::write(&exclude, original).test()?;

        let error = ensure_excluded(&repository, ".stackstead/")
            .test_err()?
            .to_string();

        assert!(error.contains("cannot read Git exclude file"));
        assert_eq!(std::fs::read(exclude).test()?, original);
        Ok(())
    }
}
