use std::path::{Path, PathBuf};

use crate::{error::StacksteadError, manifest::StacksteadManifest};

#[derive(Debug, Clone)]
pub struct ProjectPaths {
    pub repo_root: PathBuf,
    pub state_root: PathBuf,
    pub project_state_dir: PathBuf,
}

impl ProjectPaths {
    pub fn new(repo_root: PathBuf, state_root: PathBuf, project: &str) -> Self {
        let project_state_dir = state_root.join(project);
        Self {
            repo_root,
            state_root,
            project_state_dir,
        }
    }

    pub fn manifests(&self) -> anyhow::Result<Vec<StacksteadManifest>> {
        load_manifests(&self.project_state_dir)
    }

    pub fn resolve(&self, name: &str) -> anyhow::Result<StacksteadManifest> {
        resolve_manifest(&self.manifests()?, name)
    }
}

pub fn load_manifests(project_state_dir: &Path) -> anyhow::Result<Vec<StacksteadManifest>> {
    if !project_state_dir.exists() {
        return Ok(vec![]);
    }
    let mut manifests = vec![];
    for entry in std::fs::read_dir(project_state_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path().join("state/manifest.json");
        if path.is_file() {
            manifests.push(StacksteadManifest::read(&path)?);
        }
    }
    manifests.sort_by(|left, right| left.stackstead_id.cmp(&right.stackstead_id));
    Ok(manifests)
}

pub fn resolve_manifest(
    manifests: &[StacksteadManifest],
    name: &str,
) -> anyhow::Result<StacksteadManifest> {
    let matching = manifests
        .iter()
        .filter(|manifest| manifest.stackstead_id == name || manifest.slug == name)
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [manifest] => Ok((*manifest).clone()),
        [] => Err(StacksteadError::StacksteadNotFound(name.to_string()).into()),
        candidates => Err(StacksteadError::AmbiguousStackstead {
            name: name.to_string(),
            candidates: candidates
                .iter()
                .map(|manifest| manifest.stackstead_id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        }
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;

    use super::*;
    use crate::manifest::{ManifestStatus, SourceOwnership};

    fn manifest(id: &str, slug: &str) -> StacksteadManifest {
        let root = PathBuf::from(format!("/tmp/{id}"));
        StacksteadManifest {
            kind: "StacksteadManifest".into(),
            version: crate::manifest::MANIFEST_VERSION.into(),
            stackstead_id: id.into(),
            slug: slug.into(),
            short_id: id.rsplit('-').next().unwrap().into(),
            runtime_token: "0123456789abcdef0123456789abcdef".into(),
            project: "demo".into(),
            branch: slug.into(),
            base: "main".into(),
            source_ownership: SourceOwnership::Stackstead,
            repo_root: "/tmp/repo".into(),
            project_state_root: "/tmp".into(),
            stackstead_root: root.clone(),
            worktree: root.join("source"),
            state_dir: root.join("state"),
            port_lease_state_dir: None,
            compose_project: format!("demo-{id}"),
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
        }
    }

    #[test]
    fn exact_id_and_unique_slug_resolve() {
        let manifests = vec![manifest("feature-a-a17c", "feature-a")];
        assert_eq!(
            resolve_manifest(&manifests, "feature-a-a17c")
                .unwrap()
                .short_id,
            "a17c"
        );
        assert_eq!(
            resolve_manifest(&manifests, "feature-a").unwrap().short_id,
            "a17c"
        );
    }

    #[test]
    fn ambiguous_slug_lists_candidates() {
        let manifests = vec![
            manifest("feature-a-a17c", "feature-a"),
            manifest("feature-a-b92d", "feature-a"),
        ];
        let error = resolve_manifest(&manifests, "feature-a")
            .unwrap_err()
            .to_string();
        assert!(error.contains("feature-a-a17c"));
        assert!(error.contains("feature-a-b92d"));
    }

    #[test]
    fn exact_id_that_is_another_slug_is_ambiguous() {
        let manifests = vec![
            manifest("feature-a-a17c", "feature-a"),
            manifest("other-b92d", "feature-a-a17c"),
        ];
        let error = resolve_manifest(&manifests, "feature-a-a17c")
            .unwrap_err()
            .to_string();
        assert!(error.contains("ambiguous"));
        assert!(error.contains("feature-a-a17c"));
        assert!(error.contains("other-b92d"));
    }
}
