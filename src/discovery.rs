use std::path::{Path, PathBuf};

use crate::{
    error::StacksteadError,
    manifest::{StacksteadManifest, StacksteadPointer},
};

#[derive(Debug, Clone)]
pub enum Discovery {
    Stackstead {
        pointer_path: PathBuf,
        pointer: StacksteadPointer,
        manifest: Box<StacksteadManifest>,
    },
    Project {
        repo_root: PathBuf,
        config_path: PathBuf,
    },
}

pub fn discover(start: &Path) -> anyhow::Result<Discovery> {
    let start = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };
    for directory in start.ancestors() {
        let pointer_path =
            crate::paths::normalize_absolute(&directory.join(".stackstead/stackstead.json"))?;
        if pointer_path.is_file() {
            let pointer = StacksteadPointer::read(&pointer_path)?;
            let manifest = StacksteadManifest::read(&pointer.manifest)?;
            if manifest.stackstead_id != pointer.stackstead_id
                || manifest.repo_root != pointer.repo_root
                || manifest.stackstead_root != pointer.stackstead_root
                || manifest.project != pointer.project
                || manifest.project_state_root != pointer.project_state_root
                || crate::paths::normalize_absolute(&pointer.manifest)?
                    != crate::paths::normalize_absolute(&manifest.manifest_path())?
                || pointer_path != crate::paths::normalize_absolute(&manifest.pointer_file)?
            {
                anyhow::bail!(
                    "pointer {} does not match its manifest",
                    pointer_path.display()
                );
            }
            return Ok(Discovery::Stackstead {
                pointer_path,
                pointer,
                manifest: Box::new(manifest),
            });
        }
        let config_path = directory.join("stackstead.yaml");
        if config_path.is_file() {
            return Ok(Discovery::Project {
                repo_root: directory.to_path_buf(),
                config_path,
            });
        }
    }
    Err(StacksteadError::ProjectNotFound(start.to_path_buf()).into())
}

pub fn project_root(discovery: &Discovery) -> &Path {
    match discovery {
        Discovery::Stackstead { pointer, .. } => &pointer.repo_root,
        Discovery::Project { repo_root, .. } => repo_root,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;

    use super::*;
    use crate::manifest::{ManifestStatus, SourceOwnership, write_json_atomic, write_pointer};

    fn write_stackstead(root: &Path) -> StacksteadManifest {
        let repo_root = root.join("repo");
        let project_state_root = root.join("state-root");
        let stackstead_root = project_state_root.join("demo/cell-a");
        let worktree = stackstead_root.join("source");
        let state_dir = stackstead_root.join("state");
        let pointer_file = worktree.join(".stackstead/stackstead.json");
        std::fs::create_dir_all(pointer_file.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
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
            repo_root,
            project_state_root,
            stackstead_root,
            worktree,
            state_dir,
            port_lease_state_dir: None,
            compose_project: "demo-cell-a".into(),
            compose_files: vec![],
            ports: BTreeMap::new(),
            container_ports: BTreeMap::new(),
            urls: BTreeMap::new(),
            env_file: root.join("env"),
            agent_context: root.join("context"),
            pointer_file,
            event_log: root.join("events.jsonl"),
            env_keys: vec![],
            status: ManifestStatus::default(),
            database: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        write_json_atomic(&manifest.manifest_path(), &manifest).unwrap();
        write_pointer(
            &manifest.pointer_file,
            &StacksteadPointer {
                kind: "StacksteadPointer".into(),
                version: crate::manifest::POINTER_VERSION.into(),
                stackstead_id: manifest.stackstead_id.clone(),
                manifest: manifest.manifest_path(),
                project: manifest.project.clone(),
                repo_root: manifest.repo_root.clone(),
                project_state_root: manifest.project_state_root.clone(),
                stackstead_root: manifest.stackstead_root.clone(),
            },
        )
        .unwrap();
        manifest
    }

    #[test]
    fn climbs_to_project_config() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("stackstead.yaml"), "version: '1'").unwrap();
        let nested = directory.path().join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        assert!(matches!(
            discover(&nested).unwrap(),
            Discovery::Project { .. }
        ));
    }

    #[test]
    fn rejects_pointer_copied_outside_its_manifest_location() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = write_stackstead(&directory.path().join("canonical"));
        let copied_root = directory.path().join("copied");
        let copied_pointer = copied_root.join(".stackstead/stackstead.json");
        std::fs::create_dir_all(copied_pointer.parent().unwrap()).unwrap();
        std::fs::copy(&manifest.pointer_file, copied_pointer).unwrap();

        assert!(
            discover(&copied_root)
                .unwrap_err()
                .to_string()
                .contains("does not match its manifest")
        );
    }

    #[test]
    fn discovers_canonical_pointer_from_nested_worktree_path() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = write_stackstead(directory.path());
        let nested = manifest.worktree.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();

        match discover(&nested).unwrap() {
            Discovery::Stackstead {
                pointer_path,
                manifest: discovered,
                ..
            } => {
                assert_eq!(pointer_path, manifest.pointer_file);
                assert_eq!(discovered.stackstead_id, manifest.stackstead_id);
            }
            Discovery::Project { .. } => panic!("expected stackstead discovery"),
        }
    }
}
