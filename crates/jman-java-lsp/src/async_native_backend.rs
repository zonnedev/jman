use std::{
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

use javac_frontend::{EditorQueryResult, Frontend, SemanticResult};
use serde_json::Value;

use crate::{AnalysisBackend, CacheStatus, NativeBackend, SourceMetadata};

type Reply<T> = mpsc::Sender<Result<T, String>>;
type StructuralDocuments = Vec<(String, String, SemanticResult)>;

enum Command {
    Initialize(Option<String>, Option<Value>),
    Analyze(String, String, String, Reply<SemanticResult>),
    EditorQuery(String, String, String, u32, Reply<EditorQueryResult>),
    Sources(Reply<Vec<PathBuf>>),
    Close(String),
    Invalidate(Vec<String>),
    Reload(Vec<String>, Reply<bool>),
    SourceSaved(String, Reply<bool>),
    CacheStatus(Reply<CacheStatus>),
    BuildSystem(Reply<Option<String>>),
    RebuildIndex(Reply<()>),
    ClearProjectCache(Reply<()>),
    UpdateConfiguration(Value, Reply<()>),
    SourceMetadata(String, Reply<SourceMetadata>),
}

/// Non-blocking LSP facade. The worker creates and owns the Graal isolate, so
/// native contexts are never moved between threads.
pub struct AsyncNativeBackend {
    commands: mpsc::Sender<Command>,
    structural_documents: Arc<RwLock<StructuralDocuments>>,
    structural_revision: Arc<AtomicU64>,
    cancellation_generation: Arc<AtomicU64>,
}

impl AsyncNativeBackend {
    pub fn new() -> Self {
        let (commands, receiver) = mpsc::channel();
        let structural_documents = Arc::new(RwLock::new(Vec::new()));
        let structural_revision = Arc::new(AtomicU64::new(0));
        let worker_documents = Arc::clone(&structural_documents);
        let worker_revision = Arc::clone(&structural_revision);
        let cancellation_generation = Arc::new(AtomicU64::new(0));
        let worker_cancellation = Arc::clone(&cancellation_generation);
        std::thread::Builder::new()
            .name("jman-java-native".to_owned())
            .spawn(move || {
                let frontend = match Frontend::new() {
                    Ok(frontend) => frontend,
                    Err(error) => {
                        eprintln!("jman-java-lsp cannot create native frontend: {error:?}");
                        return;
                    }
                };
                let mut backend = NativeBackend::with_cancellation(frontend, worker_cancellation);
                let mut initialization_error = None;
                let mut structural_retry_pending = false;
                let mut refine_pending = false;
                loop {
                    let command = if structural_retry_pending || refine_pending {
                        let idle_delay = if structural_retry_pending {
                            std::time::Duration::from_millis(100)
                        } else {
                            std::time::Duration::from_secs(30)
                        };
                        match receiver.recv_timeout(idle_delay) {
                            Ok(command) => command,
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                if structural_retry_pending {
                                    match backend.resume_structural_index() {
                                        Ok(()) => {
                                            structural_retry_pending = false;
                                            initialization_error = None;
                                            publish_structural(
                                                &backend,
                                                &worker_documents,
                                                &worker_revision,
                                            );
                                            if backend.defer_semantic_refinement() {
                                                refine_pending = true;
                                            } else if let Err(error) =
                                                backend.refine_semantic_index()
                                            {
                                                if !is_project_state_cancellation(&error) {
                                                    eprintln!(
                                                        "jman-java-lsp semantic index refinement failed: {error}"
                                                    );
                                                }
                                            } else {
                                                publish_structural(
                                                    &backend,
                                                    &worker_documents,
                                                    &worker_revision,
                                                );
                                            }
                                        }
                                        Err(error) if is_project_state_cancellation(&error) => {}
                                        Err(error) => {
                                            structural_retry_pending = false;
                                            eprintln!(
                                                "jman-java-lsp workspace indexing retry failed: {error}"
                                            );
                                            initialization_error = Some(error);
                                        }
                                    }
                                    continue;
                                }
                                refine_pending = false;
                                if let Err(error) = backend.refine_semantic_index() {
                                    if !is_project_state_cancellation(&error) {
                                        eprintln!(
                                            "jman-java-lsp semantic index refinement failed: {error}"
                                        );
                                    }
                                } else {
                                    publish_structural(
                                        &backend,
                                        &worker_documents,
                                        &worker_revision,
                                    );
                                }
                                continue;
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    } else {
                        let Ok(command) = receiver.recv() else {
                            break;
                        };
                        command
                    };
                    match command {
                        Command::Initialize(root, options) => {
                            match backend.initialize(root.as_deref(), options.as_ref()) {
                                Err(error) if is_project_state_cancellation(&error) => {
                                    // An editor can open or query a document while the initial
                                    // workspace parse is running. Serve that queued work first,
                                    // then rebuild the structural index once the input stream is
                                    // briefly idle. A superseded parse is expected control flow,
                                    // not a permanent project initialization failure.
                                    initialization_error = None;
                                    structural_retry_pending = true;
                                }
                                Err(error) => {
                                    eprintln!(
                                        "jman-java-lsp background initialization failed: {error}"
                                    );
                                    initialization_error = Some(error);
                                }
                                Ok(()) => {
                                    initialization_error = None;
                                    publish_structural(
                                        &backend,
                                        &worker_documents,
                                        &worker_revision,
                                    );
                                    if backend.defer_semantic_refinement() {
                                        refine_pending = true;
                                    } else if let Err(error) = backend.refine_semantic_index() {
                                        eprintln!(
                                            "jman-java-lsp semantic index refinement failed: {error}"
                                        );
                                    } else {
                                        publish_structural(
                                            &backend,
                                            &worker_documents,
                                            &worker_revision,
                                        );
                                    }
                                }
                            }
                        }
                        Command::Analyze(uri, file_name, source, reply) => {
                            let result = initialization_error.clone().map_or_else(
                                || backend.analyze(&uri, &file_name, &source),
                                |error| Err(format!("project initialization failed: {error}")),
                            );
                            let _ = reply.send(result);
                        }
                        Command::EditorQuery(uri, file_name, source, cursor, reply) => {
                            let result = initialization_error.clone().map_or_else(
                                || backend.editor_query(&uri, &file_name, &source, cursor),
                                |error| Err(format!("project initialization failed: {error}")),
                            );
                            let _ = reply.send(result);
                        }
                        Command::Sources(reply) => {
                            let _ = reply.send(Ok(backend.workspace_source_files()));
                        }
                        Command::Close(uri) => backend.close(&uri),
                        Command::Invalidate(documents) => backend.invalidate(&documents),
                        Command::Reload(changed, reply) => {
                            let result = backend.reload(&changed);
                            if result.as_ref() == Ok(&true) {
                                initialization_error = None;
                                publish_structural(&backend, &worker_documents, &worker_revision);
                                if backend.defer_semantic_refinement() {
                                    refine_pending = true;
                                } else if backend.refine_semantic_index().is_ok() {
                                    publish_structural(
                                        &backend,
                                        &worker_documents,
                                        &worker_revision,
                                    );
                                }
                            }
                            let _ = reply.send(result);
                        }
                        Command::SourceSaved(uri, reply) => {
                            let result = backend.source_saved(&uri);
                            if result.as_ref() == Ok(&true) {
                                publish_structural(&backend, &worker_documents, &worker_revision);
                                if backend.defer_semantic_refinement() {
                                    refine_pending = true;
                                } else if backend.refine_semantic_index().is_ok() {
                                    publish_structural(
                                        &backend,
                                        &worker_documents,
                                        &worker_revision,
                                    );
                                }
                            }
                            let _ = reply.send(result);
                        }
                        Command::CacheStatus(reply) => {
                            let _ = reply.send(Ok(backend.cache_status()));
                        }
                        Command::BuildSystem(reply) => {
                            let _ = reply.send(Ok(backend.build_system()));
                        }
                        Command::RebuildIndex(reply) => {
                            let result = backend.rebuild_index();
                            if result.is_ok() {
                                publish_structural(&backend, &worker_documents, &worker_revision);
                            }
                            let _ = reply.send(result);
                        }
                        Command::ClearProjectCache(reply) => {
                            let result = backend.clear_project_cache();
                            let _ = reply.send(result);
                        }
                        Command::UpdateConfiguration(settings, reply) => {
                            let result = backend.update_configuration(&settings);
                            if result.is_ok() {
                                publish_structural(&backend, &worker_documents, &worker_revision);
                            }
                            let _ = reply.send(result);
                        }
                        Command::SourceMetadata(uri, reply) => {
                            let _ = reply.send(Ok(backend.source_metadata(&uri)));
                        }
                    }
                }
            })
            .expect("spawn native Java frontend worker");
        Self {
            commands,
            structural_documents,
            structural_revision,
            cancellation_generation,
        }
    }

    fn request<T>(&self, build: impl FnOnce(Reply<T>) -> Command) -> Result<T, String> {
        let (reply, response) = mpsc::channel();
        self.commands
            .send(build(reply))
            .map_err(|_| "native Java frontend worker stopped".to_owned())?;
        response
            .recv()
            .map_err(|_| "native Java frontend worker stopped".to_owned())?
    }
}

impl Default for AsyncNativeBackend {
    fn default() -> Self {
        Self::new()
    }
}

fn is_project_state_cancellation(error: &str) -> bool {
    error.contains("cancelled by a newer project state")
}

fn publish_structural(
    backend: &NativeBackend,
    documents: &RwLock<StructuralDocuments>,
    revision: &AtomicU64,
) {
    if let Ok(mut target) = documents.write() {
        *target = backend.workspace_structural_documents();
        revision.fetch_add(1, Ordering::Release);
    }
}

impl AnalysisBackend for AsyncNativeBackend {
    fn initialize(
        &mut self,
        root_uri: Option<&str>,
        options: Option<&Value>,
    ) -> Result<(), String> {
        self.commands
            .send(Command::Initialize(
                root_uri.map(str::to_owned),
                options.cloned(),
            ))
            .map_err(|_| "native Java frontend worker stopped".to_owned())
    }

    fn analyze(
        &mut self,
        uri: &str,
        file_name: &str,
        source: &str,
    ) -> Result<SemanticResult, String> {
        self.cancellation_generation.fetch_add(1, Ordering::AcqRel);
        self.request(|reply| {
            Command::Analyze(
                uri.to_owned(),
                file_name.to_owned(),
                source.to_owned(),
                reply,
            )
        })
    }

    fn workspace_source_files(&self) -> Vec<PathBuf> {
        self.request(Command::Sources).unwrap_or_default()
    }

    fn editor_query(
        &mut self,
        uri: &str,
        file_name: &str,
        source: &str,
        cursor: u32,
    ) -> Result<EditorQueryResult, String> {
        self.cancellation_generation.fetch_add(1, Ordering::AcqRel);
        self.request(|reply| {
            Command::EditorQuery(
                uri.to_owned(),
                file_name.to_owned(),
                source.to_owned(),
                cursor,
                reply,
            )
        })
    }

    fn workspace_structural_documents(&self) -> StructuralDocuments {
        self.structural_documents
            .read()
            .map(|documents| documents.clone())
            .unwrap_or_default()
    }

    fn workspace_structural_revision(&self) -> u64 {
        self.structural_revision.load(Ordering::Acquire)
    }

    fn workspace_index_pending(&self) -> bool {
        self.workspace_structural_revision() == 0
    }

    fn build_system(&self) -> Option<String> {
        self.request(Command::BuildSystem).unwrap_or_default()
    }

    fn close(&mut self, uri: &str) {
        let _ = self.commands.send(Command::Close(uri.to_owned()));
    }

    fn invalidate(&mut self, documents: &[String]) {
        let _ = self
            .commands
            .send(Command::Invalidate(documents.to_owned()));
    }

    fn reload(&mut self, changed_uris: &[String]) -> Result<bool, String> {
        if changed_uris.is_empty() || changed_uris.iter().any(|uri| is_build_file(uri)) {
            self.cancellation_generation.fetch_add(1, Ordering::AcqRel);
        }
        self.request(|reply| Command::Reload(changed_uris.to_owned(), reply))
    }

    fn source_saved(&mut self, uri: &str) -> Result<bool, String> {
        self.cancellation_generation.fetch_add(1, Ordering::AcqRel);
        self.request(|reply| Command::SourceSaved(uri.to_owned(), reply))
    }

    fn cache_status(&self) -> CacheStatus {
        self.request(Command::CacheStatus).unwrap_or_default()
    }

    fn rebuild_index(&mut self) -> Result<(), String> {
        self.request(Command::RebuildIndex)
    }

    fn clear_project_cache(&mut self) -> Result<(), String> {
        self.request(Command::ClearProjectCache)
    }

    fn update_configuration(&mut self, settings: &Value) -> Result<(), String> {
        self.request(|reply| Command::UpdateConfiguration(settings.clone(), reply))
    }

    fn cancel_request(&mut self, _id: &Value) {
        self.cancellation_generation.fetch_add(1, Ordering::AcqRel);
    }

    fn source_metadata(&self, uri: &str) -> SourceMetadata {
        self.request(|reply| Command::SourceMetadata(uri.to_owned(), reply))
            .unwrap_or_default()
    }
}

fn is_build_file(uri: &str) -> bool {
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "pom.xml"
            | "settings.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "gradle.properties"
            | "gradle-wrapper.properties"
            | "libs.versions.toml"
    ) || name.ends_with(".gradle")
        || name.ends_with(".gradle.kts")
        || path.contains("/.mvn/")
        || path.contains("/buildSrc/")
}

#[cfg(test)]
mod tests {
    use super::is_project_state_cancellation;

    #[test]
    fn superseded_workspace_work_is_classified_as_non_fatal() {
        assert!(is_project_state_cancellation(
            "workspace parse cancelled by a newer project state"
        ));
        assert!(is_project_state_cancellation(
            "semantic indexing cancelled by a newer project state"
        ));
        assert!(!is_project_state_cancellation(
            "cannot resolve the selected project model"
        ));
    }
}
