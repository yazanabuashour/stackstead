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
fn worktree_parsers_preserve_newlines_in_paths() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test()?;
    let repository = directory.path().join("repository\npath");
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
    assert_eq!(
        primary_worktree(&worktree).test()?,
        std::fs::canonicalize(&repository).test()?
    );
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
