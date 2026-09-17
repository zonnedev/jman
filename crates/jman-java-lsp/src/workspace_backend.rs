use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

use javac_frontend::{EditorQueryResult, SemanticResult};
use serde_json::Value;

use crate::{
    AnalysisBackend, AsyncNativeBackend, CacheStatus, JpmsCatalog, SourceMetadata, file_uri_to_path,
};

struct Workspace {
    uri: String,
    path: PathBuf,
    backend: AsyncNativeBackend,
}

/// Routes LSP analysis to one thread-confined native backend per workspace.
///
/// Keeping the registry above `AsyncNativeBackend` preserves the existing
/// single-project cache, session, and Graal-isolate ownership boundaries.
#[derive(Default)]
pub struct WorkspaceBackend {
    workspaces: Vec<Workspace>,
    initialization_options: Option<Value>,
    configuration: Value,
}

impl WorkspaceBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn add(&mut self, uri: &str) -> Result<(), String> {
        if self.workspaces.iter().any(|workspace| workspace.uri == uri) {
            return Ok(());
        }
        let path = file_uri_to_path(uri)
            .ok_or_else(|| format!("workspace folder is not a file URI: {uri}"))?;
        let mut backend = AsyncNativeBackend::new();
        backend.initialize(Some(uri), self.initialization_options.as_ref())?;
        if !self.configuration.is_null() {
            backend.update_configuration(&self.configuration)?;
        }
        self.workspaces.push(Workspace {
            uri: uri.trim_end_matches('/').to_owned(),
            path,
            backend,
        });
        self.workspaces
            .sort_by(|left, right| left.uri.cmp(&right.uri));
        Ok(())
    }

    fn owning_index(&self, uri: &str) -> Option<usize> {
        let path = file_uri_to_path(uri)?;
        self.workspaces
            .iter()
            .enumerate()
            .filter(|(_, workspace)| path.starts_with(&workspace.path))
            .max_by_key(|(_, workspace)| workspace.path.components().count())
            .map(|(index, _)| index)
    }

    fn owning(&self, uri: &str) -> Option<&AsyncNativeBackend> {
        self.owning_index(uri)
            .map(|index| &self.workspaces[index].backend)
    }

    fn owning_mut(&mut self, uri: &str) -> Option<&mut AsyncNativeBackend> {
        self.owning_index(uri)
            .map(|index| &mut self.workspaces[index].backend)
    }

    fn roots_for_changes(&self, changes: &[String]) -> Vec<usize> {
        if changes.is_empty() {
            return (0..self.workspaces.len()).collect();
        }
        let mut indices: Vec<_> = changes
            .iter()
            .filter_map(|uri| self.owning_index(uri))
            .collect();
        indices.sort_unstable();
        indices.dedup();
        indices
    }
}

impl AnalysisBackend for WorkspaceBackend {
    fn initialize(
        &mut self,
        root_uri: Option<&str>,
        options: Option<&Value>,
    ) -> Result<(), String> {
        self.initialization_options = options.cloned();
        if let Some(root_uri) = root_uri {
            self.add(root_uri)?;
        }
        Ok(())
    }

    fn analyze(
        &mut self,
        uri: &str,
        file_name: &str,
        source: &str,
    ) -> Result<SemanticResult, String> {
        self.owning_mut(uri)
            .ok_or_else(|| format!("no workspace folder owns {uri}"))?
            .analyze(uri, file_name, source)
    }

    fn editor_query(
        &mut self,
        uri: &str,
        file_name: &str,
        source: &str,
        cursor: u32,
    ) -> Result<EditorQueryResult, String> {
        self.owning_mut(uri)
            .ok_or_else(|| format!("no workspace folder owns {uri}"))?
            .editor_query(uri, file_name, source, cursor)
    }

    fn workspace_source_files(&self) -> Vec<PathBuf> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.backend.workspace_source_files())
            .collect()
    }

    fn workspace_structural_documents(&self) -> Vec<(String, String, SemanticResult)> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.backend.workspace_structural_documents())
            .collect()
    }

    fn workspace_structural_revision(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        for workspace in &self.workspaces {
            workspace.uri.hash(&mut hasher);
            workspace
                .backend
                .workspace_structural_revision()
                .hash(&mut hasher);
        }
        hasher.finish()
    }

    fn workspace_index_pending(&self) -> bool {
        self.workspaces
            .iter()
            .any(|workspace| workspace.backend.workspace_index_pending())
    }

    fn build_system(&self) -> Option<String> {
        let mut systems: Vec<_> = self
            .workspaces
            .iter()
            .filter_map(|workspace| workspace.backend.build_system())
            .collect();
        systems.sort();
        systems.dedup();
        match systems.as_slice() {
            [] => None,
            [system] => Some(system.clone()),
            _ => Some("multi".to_owned()),
        }
    }

    fn build_system_for(&self, uri: &str) -> Option<String> {
        self.owning(uri).and_then(AnalysisBackend::build_system)
    }

    fn jpms_catalog(&self, uri: &str) -> JpmsCatalog {
        self.owning(uri)
            .map(|backend| backend.jpms_catalog(uri))
            .unwrap_or_default()
    }

    fn close(&mut self, uri: &str) {
        if let Some(backend) = self.owning_mut(uri) {
            backend.close(uri);
        }
    }

    fn invalidate(&mut self, documents: &[String]) {
        for index in self.roots_for_changes(documents) {
            let owned: Vec<_> = documents
                .iter()
                .filter(|uri| self.owning_index(uri) == Some(index))
                .cloned()
                .collect();
            self.workspaces[index].backend.invalidate(&owned);
        }
    }

    fn reload(&mut self, changed_uris: &[String]) -> Result<bool, String> {
        let mut changed = false;
        for index in self.roots_for_changes(changed_uris) {
            let owned: Vec<_> = changed_uris
                .iter()
                .filter(|uri| self.owning_index(uri) == Some(index))
                .cloned()
                .collect();
            changed |= self.workspaces[index].backend.reload(&owned)?;
        }
        Ok(changed)
    }

    fn source_saved(&mut self, uri: &str) -> Result<bool, String> {
        self.owning_mut(uri)
            .map_or(Ok(false), |backend| backend.source_saved(uri))
    }

    fn cache_status(&self) -> CacheStatus {
        let statuses: Vec<_> = self
            .workspaces
            .iter()
            .map(|workspace| workspace.backend.cache_status())
            .collect();
        let single = (statuses.len() == 1).then(|| &statuses[0]);
        CacheStatus {
            project_id: statuses
                .iter()
                .map(|status| status.project_id.as_str())
                .collect::<Vec<_>>()
                .join(","),
            directory: statuses
                .iter()
                .map(|status| status.directory.as_str())
                .collect::<Vec<_>>()
                .join(";"),
            structural_entries: statuses
                .iter()
                .map(|status| status.structural_entries)
                .sum(),
            structural_hits: statuses.iter().map(|status| status.structural_hits).sum(),
            structural_misses: statuses.iter().map(|status| status.structural_misses).sum(),
            semantic_entries: statuses.iter().map(|status| status.semantic_entries).sum(),
            semantic_hits: statuses.iter().map(|status| status.semantic_hits).sum(),
            semantic_misses: statuses.iter().map(|status| status.semantic_misses).sum(),
            external_entries: statuses.iter().map(|status| status.external_entries).sum(),
            bytes: statuses.iter().map(|status| status.bytes).sum(),
            indexing_milliseconds: statuses
                .iter()
                .map(|status| status.indexing_milliseconds)
                .sum(),
            build_tool_version: single.and_then(|status| status.build_tool_version.clone()),
            build_java_home: single.and_then(|status| status.build_java_home.clone()),
            build_java_version: single.and_then(|status| status.build_java_version.clone()),
            build_java_major: single.and_then(|status| status.build_java_major),
            build_java_source: single.and_then(|status| status.build_java_source.clone()),
        }
    }

    fn rebuild_index(&mut self) -> Result<(), String> {
        for workspace in &mut self.workspaces {
            workspace.backend.rebuild_index()?;
        }
        Ok(())
    }

    fn clear_project_cache(&mut self) -> Result<(), String> {
        for workspace in &mut self.workspaces {
            workspace.backend.clear_project_cache()?;
        }
        Ok(())
    }

    fn workspace_folders_changed(
        &mut self,
        added: &[String],
        removed: &[String],
    ) -> Result<(), String> {
        self.workspaces
            .retain(|workspace| !removed.iter().any(|uri| same_root(uri, &workspace.uri)));
        for uri in added {
            self.add(uri)?;
        }
        Ok(())
    }

    fn update_configuration(&mut self, settings: &Value) -> Result<(), String> {
        self.configuration = settings.clone();
        for workspace in &mut self.workspaces {
            workspace.backend.update_configuration(settings)?;
        }
        Ok(())
    }

    fn cancel_request(&mut self, id: &Value) {
        for workspace in &mut self.workspaces {
            workspace.backend.cancel_request(id);
        }
    }

    fn source_metadata(&self, uri: &str) -> SourceMetadata {
        self.owning(uri)
            .map(|backend| backend.source_metadata(uri))
            .unwrap_or_default()
    }
}

fn same_root(left: &str, right: &str) -> bool {
    left.trim_end_matches('/') == right.trim_end_matches('/')
}
