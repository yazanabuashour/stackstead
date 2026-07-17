use std::path::{Component, Path, PathBuf};

use crate::{
    error::StacksteadError,
    manifest::{SourceOwnership, StacksteadManifest},
};

pub fn absolute_from(base: &Path, value: &Path) -> anyhow::Result<PathBuf> {
    let joined = if value.is_absolute() {
        value.to_path_buf()
    } else {
        base.join(value)
    };
    normalize_absolute(&joined)
}

pub fn safe_generated_path(worktree: &Path, relative: &Path) -> anyhow::Result<PathBuf> {
    if relative.is_absolute() {
        return Err(StacksteadError::UnsafePath(format!(
            "generated path must be relative: {}",
            relative.display()
        ))
        .into());
    }
    let path = normalize_absolute(&worktree.join(relative))?;
    let worktree = normalize_absolute(worktree)?;
    reject_symlink_base(&worktree, "worktree")?;
    if !path.starts_with(&worktree) || path == worktree {
        return Err(StacksteadError::UnsafePath(format!(
            "{} escapes worktree {}",
            relative.display(),
            worktree.display()
        ))
        .into());
    }
    reject_symlink_components(&worktree, relative)?;
    Ok(path)
}

fn reject_symlink_base(path: &Path, label: &str) -> anyhow::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(StacksteadError::UnsafePath(
            format!("{label} path is a symlink: {}", path.display()),
        )
        .into()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn reject_symlink_components(worktree: &Path, relative: &Path) -> anyhow::Result<()> {
    if !worktree.exists() {
        return Ok(());
    }
    let mut current = worktree.to_path_buf();
    for component in relative.components() {
        if matches!(component, Component::CurDir) {
            continue;
        }
        let Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(StacksteadError::UnsafePath(format!(
                    "generated path traverses symlink {}",
                    current.display()
                ))
                .into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub fn validate_destroy_target(
    manifest: &StacksteadManifest,
    project_state_root: &Path,
) -> anyhow::Result<()> {
    let project_state_root = normalize_absolute(project_state_root)?;
    let stackstead_root = normalize_absolute(&manifest.stackstead_root)?;
    let expected_parent = project_state_root.join(&manifest.project);
    let worktree = normalize_absolute(&manifest.worktree)?;
    reject_symlink_base(&project_state_root, "project state root")?;
    reject_symlink_base(&expected_parent, "project state directory")?;
    reject_symlink_base(&stackstead_root, "stackstead root")?;
    reject_symlink_base(&worktree, "worktree")?;
    let source_layout_valid = match manifest.source_ownership {
        SourceOwnership::Stackstead => worktree == stackstead_root.join("source"),
        SourceOwnership::External => {
            worktree != stackstead_root && !worktree.starts_with(&stackstead_root)
        }
    };
    if stackstead_root.parent() != Some(expected_parent.as_path())
        || stackstead_root.file_name().and_then(|name| name.to_str())
            != Some(manifest.stackstead_id.as_str())
        || !source_layout_valid
        || normalize_absolute(&manifest.state_dir)? != stackstead_root.join("state")
        || manifest.manifest_path() != stackstead_root.join("state/manifest.json")
    {
        return Err(StacksteadError::UnsafePath(format!(
            "manifest paths do not identify a direct Stackstead child of {}",
            expected_parent.display()
        ))
        .into());
    }
    if stackstead_root.exists() {
        let canonical_project_root = std::fs::canonicalize(&project_state_root)?;
        let canonical_root = std::fs::canonicalize(&stackstead_root)?;
        let canonical_parent = std::fs::canonicalize(&expected_parent)?;
        if canonical_parent.parent() != Some(canonical_project_root.as_path())
            || canonical_root.parent() != Some(canonical_parent.as_path())
        {
            return Err(StacksteadError::UnsafePath(format!(
                "{} or its project directory resolves outside {}",
                stackstead_root.display(),
                project_state_root.display()
            ))
            .into());
        }
    }
    Ok(())
}

pub fn remove_generated_dir(worktree: &Path, relative: &Path) -> anyhow::Result<()> {
    let path = safe_generated_path(worktree, relative)?;
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            std::fs::remove_dir_all(path)?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub fn remove_stackstead_root(
    manifest: &StacksteadManifest,
    project_state_root: &Path,
) -> anyhow::Result<()> {
    validate_destroy_target(manifest, project_state_root)?;
    std::fs::remove_dir_all(&manifest.stackstead_root)?;
    Ok(())
}

pub fn normalize_absolute(path: &Path) -> anyhow::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(StacksteadError::UnsafePath(path.display().to_string()).into());
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Ok(normalized)
}

pub fn resolve_existing_ancestor(path: &Path) -> anyhow::Result<PathBuf> {
    let normalized = normalize_absolute(path)?;
    let mut existing = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = existing.file_name().ok_or_else(|| {
                    anyhow::anyhow!("{} has no existing ancestor", normalized.display())
                })?;
                missing.push(name.to_os_string());
                existing = existing.parent().ok_or_else(|| {
                    anyhow::anyhow!("{} has no existing ancestor", normalized.display())
                })?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let mut resolved = std::fs::canonicalize(existing)?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    normalize_absolute(&resolved)
}

#[cfg(test)]
mod tests {
    use crate::test_support::TestResultExt as _;
    use std::collections::BTreeMap;

    use chrono::Utc;

    use super::*;
    use crate::manifest::{ManifestStatus, SourceOwnership, StacksteadManifest};

    #[test]
    fn generated_paths_cannot_escape() -> anyhow::Result<()> {
        assert!(safe_generated_path(Path::new("/tmp/cell/source"), Path::new("../other")).is_err());
        assert_eq!(
            safe_generated_path(Path::new("/tmp/cell/source"), Path::new(".stackstead/.env"))
                .test()?,
            Path::new("/tmp/cell/source/.stackstead/.env")
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn generated_paths_cannot_traverse_symlinks() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let worktree = directory.path().join("source");
        let outside = directory.path().join("outside");
        std::fs::create_dir_all(&worktree).test()?;
        std::fs::create_dir_all(&outside).test()?;
        std::os::unix::fs::symlink(&outside, worktree.join(".stackstead")).test()?;
        assert!(safe_generated_path(&worktree, Path::new(".stackstead/.env")).is_err());

        std::fs::remove_dir_all(&worktree).test()?;
        std::os::unix::fs::symlink(&outside, &worktree).test()?;
        assert!(safe_generated_path(&worktree, Path::new(".stackstead/.env")).is_err());
        Ok(())
    }

    #[test]
    fn destroy_requires_exact_layout() -> anyhow::Result<()> {
        let root = PathBuf::from("/tmp/state/demo/cell-a");
        let manifest = StacksteadManifest {
            kind: "StacksteadManifest".into(),
            version: crate::manifest::MANIFEST_VERSION.into(),
            stackstead_id: "cell-a".into(),
            slug: "cell".into(),
            short_id: "a".into(),
            runtime_token: "0123456789abcdef0123456789abcdef".into(),
            project: "demo".into(),
            branch: "cell".into(),
            base: "main".into(),
            source_ownership: SourceOwnership::Stackstead,
            repo_root: "/tmp/repo".into(),
            project_state_root: "/tmp/state".into(),
            stackstead_root: root.clone(),
            worktree: root.join("source"),
            state_dir: root.join("state"),
            port_lease_state_dir: None,
            compose_project: "demo-cell-a".into(),
            compose_files: vec![],
            ports: BTreeMap::new(),
            container_ports: BTreeMap::new(),
            urls: BTreeMap::new(),
            env_file: root.join("source/.stackstead/.env"),
            agent_context: root.join("source/.stackstead/AGENT_CONTEXT.md"),
            pointer_file: root.join("source/.stackstead/stackstead.json"),
            event_log: root.join("state/events.jsonl"),
            env_keys: vec![],
            status: ManifestStatus::default(),
            database: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        validate_destroy_target(&manifest, Path::new("/tmp/state")).test()?;
        let mut unsafe_manifest = manifest;
        unsafe_manifest.stackstead_root = PathBuf::from("/tmp");
        assert!(validate_destroy_target(&unsafe_manifest, Path::new("/tmp/state")).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn destroy_rejects_a_symlinked_project_state_directory() -> anyhow::Result<()> {
        let directory = tempfile::tempdir().test()?;
        let state_root = directory.path().join("state");
        let outside_parent = directory.path().join("outside/demo");
        let root = state_root.join("demo/cell-a");
        std::fs::create_dir_all(root.join("source")).test()?;
        std::fs::create_dir_all(root.join("state")).test()?;
        std::fs::create_dir_all(&state_root).test()?;
        std::fs::create_dir_all(&outside_parent).test()?;
        std::fs::remove_dir_all(state_root.join("demo")).test()?;
        std::os::unix::fs::symlink(&outside_parent, state_root.join("demo")).test()?;

        let manifest = StacksteadManifest {
            kind: "StacksteadManifest".into(),
            version: crate::manifest::MANIFEST_VERSION.into(),
            stackstead_id: "cell-a".into(),
            slug: "cell".into(),
            short_id: "a".into(),
            runtime_token: "0123456789abcdef0123456789abcdef".into(),
            project: "demo".into(),
            branch: "cell".into(),
            base: "main".into(),
            source_ownership: SourceOwnership::Stackstead,
            repo_root: directory.path().join("repo"),
            project_state_root: state_root.clone(),
            stackstead_root: root.clone(),
            worktree: root.join("source"),
            state_dir: root.join("state"),
            port_lease_state_dir: None,
            compose_project: "demo-cell-a".into(),
            compose_files: vec![],
            ports: BTreeMap::new(),
            container_ports: BTreeMap::new(),
            urls: BTreeMap::new(),
            env_file: root.join("source/.stackstead/.env"),
            agent_context: root.join("source/.stackstead/AGENT_CONTEXT.md"),
            pointer_file: root.join("source/.stackstead/stackstead.json"),
            event_log: root.join("state/events.jsonl"),
            env_keys: vec![],
            status: ManifestStatus::default(),
            database: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(validate_destroy_target(&manifest, &state_root).is_err());
        Ok(())
    }
}
