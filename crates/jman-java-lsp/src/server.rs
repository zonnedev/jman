use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use javac_frontend::{EditorQueryResult, FormatResult, ProjectState, SemanticResult};
use serde_json::{Value, json};

use crate::{Documents, file_uri_to_path, offset_to_position};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JpmsCatalog {
    pub modules: Vec<String>,
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheStatus {
    pub project_id: String,
    pub directory: String,
    pub structural_entries: usize,
    pub structural_hits: usize,
    pub structural_misses: usize,
    pub semantic_entries: usize,
    pub semantic_hits: usize,
    pub semantic_misses: usize,
    pub external_entries: usize,
    pub bytes: u64,
    pub indexing_milliseconds: u128,
    pub build_tool_version: Option<String>,
    pub build_java_home: Option<String>,
    pub build_java_version: Option<String>,
    pub build_java_major: Option<u16>,
    pub build_java_source: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceMetadata {
    pub generated: bool,
    pub read_only: bool,
    pub origin: Option<String>,
}

pub trait AnalysisBackend {
    fn initialize(
        &mut self,
        _root_uri: Option<&str>,
        _options: Option<&Value>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn analyze(
        &mut self,
        uri: &str,
        file_name: &str,
        source: &str,
    ) -> Result<SemanticResult, String>;

    fn editor_query(
        &mut self,
        _uri: &str,
        _file_name: &str,
        _source: &str,
        _cursor: u32,
    ) -> Result<EditorQueryResult, String> {
        Ok(EditorQueryResult {
            completions: Vec::new(),
            signatures: Vec::new(),
            hover: None,
            definition: None,
            type_definition: None,
        })
    }

    fn format(
        &mut self,
        _uri: &str,
        _file_name: &str,
        _source: &str,
    ) -> Result<FormatResult, String> {
        Err("Java formatting is unavailable".to_owned())
    }

    fn workspace_source_files(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn workspace_structural_documents(&self) -> Vec<(String, String, SemanticResult)> {
        Vec::new()
    }

    fn workspace_structural_revision(&self) -> u64 {
        0
    }

    fn workspace_index_pending(&self) -> bool {
        false
    }

    fn build_system(&self) -> Option<String> {
        None
    }

    fn build_system_for(&self, _uri: &str) -> Option<String> {
        self.build_system()
    }

    fn build_java_home_for(&self, _uri: Option<&str>) -> Option<String> {
        self.cache_status().build_java_home
    }

    fn jpms_catalog(&self, _uri: &str) -> JpmsCatalog {
        JpmsCatalog::default()
    }

    fn close(&mut self, _uri: &str) {}

    fn invalidate(&mut self, _documents: &[String]) {}

    fn reload(&mut self, _changed_uris: &[String]) -> Result<bool, String> {
        Ok(false)
    }

    fn source_saved(&mut self, _uri: &str) -> Result<bool, String> {
        Ok(false)
    }

    fn cache_status(&self) -> CacheStatus {
        CacheStatus::default()
    }

    fn rebuild_index(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn clear_project_cache(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn workspace_folders_changed(
        &mut self,
        _added: &[String],
        _removed: &[String],
    ) -> Result<(), String> {
        Ok(())
    }

    fn update_configuration(&mut self, _settings: &Value) -> Result<(), String> {
        Ok(())
    }

    fn cancel_request(&mut self, _id: &Value) {}

    fn source_metadata(&self, _uri: &str) -> SourceMetadata {
        SourceMetadata::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Uninitialized,
    Running,
    Shutdown,
}

#[derive(Debug, PartialEq)]
pub enum Dispatch {
    Reply(Value),
    Batch(Vec<Value>),
    Notification,
    Exit(i32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BuildSyncMode {
    Prompt,
    Manual,
    #[default]
    Automatic,
}

impl BuildSyncMode {
    fn from_options(options: Option<&Value>) -> Self {
        match options.and_then(|value| value["buildSync"].as_str()) {
            Some("manual") => Self::Manual,
            Some("prompt") => Self::Prompt,
            Some("automatic") => Self::Automatic,
            _ => Self::Automatic,
        }
    }

    fn from_settings(settings: &Value, fallback: Self) -> Self {
        match settings["buildSync"].as_str() {
            Some("manual") => Self::Manual,
            Some("prompt") => Self::Prompt,
            Some("automatic") => Self::Automatic,
            _ => fallback,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum BuildSyncState {
    #[default]
    Ready,
    Required,
    Syncing,
    Failed(String),
}

const RESOLVE_CACHE_CAPACITY: usize = 1_024;
const SEMANTIC_TOKEN_CACHE_CAPACITY: usize = 32;

#[derive(Clone, Debug)]
struct ResolveEntry {
    uri: String,
    version: Option<i64>,
    properties: Value,
}

#[derive(Clone, Debug)]
struct SemanticTokenSnapshot {
    uri: String,
    data: Vec<u64>,
}

pub struct Server<'backend> {
    lifecycle: Lifecycle,
    backend: Option<Box<dyn AnalysisBackend + 'backend>>,
    documents: Documents,
    indexed_sources: HashMap<String, String>,
    external_origins: HashMap<String, String>,
    analyses: HashMap<String, SemanticResult>,
    project: ProjectState,
    structural_project: ProjectState,
    structural_revision: u64,
    workspace_roots: Vec<PathBuf>,
    build_sync_mode: BuildSyncMode,
    build_sync_state: BuildSyncState,
    pending_build_changes: HashSet<String>,
    completion_resolve_properties: HashSet<String>,
    code_action_resolve_properties: HashSet<String>,
    resolve_sequence: u64,
    completion_resolutions: HashMap<u64, ResolveEntry>,
    code_action_resolutions: HashMap<u64, ResolveEntry>,
    semantic_token_sequence: u64,
    semantic_token_snapshots: HashMap<String, SemanticTokenSnapshot>,
}

impl Default for Server<'_> {
    fn default() -> Self {
        Self {
            lifecycle: Lifecycle::Uninitialized,
            backend: None,
            documents: Documents::default(),
            indexed_sources: HashMap::new(),
            external_origins: HashMap::new(),
            analyses: HashMap::new(),
            project: ProjectState::default(),
            structural_project: ProjectState::default(),
            structural_revision: 0,
            workspace_roots: Vec::new(),
            build_sync_mode: BuildSyncMode::default(),
            build_sync_state: BuildSyncState::default(),
            pending_build_changes: HashSet::new(),
            completion_resolve_properties: HashSet::new(),
            code_action_resolve_properties: HashSet::new(),
            resolve_sequence: 0,
            completion_resolutions: HashMap::new(),
            code_action_resolutions: HashMap::new(),
            semantic_token_sequence: 0,
            semantic_token_snapshots: HashMap::new(),
        }
    }
}

impl<'backend> Server<'backend> {
    pub fn with_backend(backend: impl AnalysisBackend + 'backend) -> Self {
        Self {
            backend: Some(Box::new(backend)),
            ..Self::default()
        }
    }

    pub fn dispatch(&mut self, message: Value) -> Dispatch {
        let id = message.get("id").cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return id.map_or(Dispatch::Notification, |id| {
                Dispatch::Reply(error(id, -32600, "Invalid Request"))
            });
        };
        if method == "exit" {
            return Dispatch::Exit(if self.lifecycle == Lifecycle::Shutdown {
                0
            } else {
                1
            });
        }
        if self.lifecycle == Lifecycle::Running {
            self.refresh_structural_workspace();
        }
        if id.is_none() {
            return self.notification(method, message.get("params"));
        }
        let id = id.unwrap();
        if self.lifecycle == Lifecycle::Uninitialized && method != "initialize" {
            return Dispatch::Reply(error(id, -32002, "Server not initialized"));
        }
        match method {
            "initialize" if self.lifecycle == Lifecycle::Uninitialized => {
                let params = message.get("params");
                self.completion_resolve_properties = resolve_support_properties(
                    params,
                    "/capabilities/textDocument/completion/completionItem/resolveSupport/properties",
                );
                self.code_action_resolve_properties = resolve_support_properties(
                    params,
                    "/capabilities/textDocument/codeAction/resolveSupport/properties",
                );
                self.build_sync_mode = BuildSyncMode::from_options(
                    params.and_then(|value| value.get("initializationOptions")),
                );
                let folders = initialization_workspace_folders(params);
                let primary = params
                    .and_then(|value| value["rootUri"].as_str())
                    .map(str::to_owned)
                    .or_else(|| folders.first().cloned());
                self.workspace_roots = folders
                    .iter()
                    .chain(primary.iter())
                    .filter_map(|uri| file_uri_to_path(uri))
                    .collect();
                self.workspace_roots.sort();
                self.workspace_roots.dedup();
                if let Some(backend) = self.backend.as_mut() {
                    let options = params.and_then(|value| value.get("initializationOptions"));
                    if let Err(message) = backend.initialize(primary.as_deref(), options) {
                        return Dispatch::Reply(error(id, -32603, &message));
                    }
                    let additional: Vec<_> = folders
                        .into_iter()
                        .filter(|folder| Some(folder.as_str()) != primary.as_deref())
                        .collect();
                    if let Err(message) = backend.workspace_folders_changed(&additional, &[]) {
                        return Dispatch::Reply(error(id, -32603, &message));
                    }
                }
                self.index_workspace();
                self.lifecycle = Lifecycle::Running;
                Dispatch::Reply(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "capabilities": {
                            "positionEncoding": "utf-16",
                            "workspace": {
                                "workspaceFolders": {
                                    "supported": true,
                                    "changeNotifications": true
                                },
                                "fileOperations": {
                                    "willCreate": {"filters": [java_file_operation_filter()]},
                                    "didCreate": {"filters": [java_file_operation_filter()]},
                                    "willRename": {"filters": [java_file_operation_filter()]},
                                    "didRename": {"filters": [java_file_operation_filter()]},
                                    "willDelete": {"filters": [java_file_operation_filter()]},
                                    "didDelete": {"filters": [java_file_operation_filter()]}
                                }
                            },
                            "textDocumentSync": {
                                "openClose": true,
                                "change": 2,
                                "save": {"includeText": false}
                            },
                            "definitionProvider": true,
                            "declarationProvider": true,
                            "implementationProvider": true,
                            "typeDefinitionProvider": true,
                            "callHierarchyProvider": true,
                            "typeHierarchyProvider": true,
                            "documentSymbolProvider": true,
                            "documentHighlightProvider": true,
                            "referencesProvider": true,
                            "workspaceSymbolProvider": true,
                            "renameProvider": {"prepareProvider": true},
                            "completionProvider": {
                                "triggerCharacters": [".", "@"],
                                "resolveProvider": true
                            },
                            "hoverProvider": true,
                            "signatureHelpProvider": {
                                "triggerCharacters": ["(", ","],
                                "retriggerCharacters": [","]
                            },
                            "semanticTokensProvider": {
                                "legend": {
                                    "tokenTypes": [
                                        "namespace", "class", "interface", "enum",
                                        "typeParameter", "method", "property", "variable",
                                        "struct", "parameter", "enumMember", "decorator"
                                    ],
                                    "tokenModifiers": [
                                        "declaration", "static", "readonly", "abstract",
                                        "deprecated", "defaultLibrary", "modification"
                                    ]
                                },
                                "range": true,
                                "full": {"delta": true}
                            },
                            "codeActionProvider": {
                                "resolveProvider": true,
                                "codeActionKinds": ["quickfix", "refactor", "source.organizeImports"]
                            },
                            "codeLensProvider": {"resolveProvider": false},
                            "documentFormattingProvider": true,
                            "inlayHintProvider": true,
                            "selectionRangeProvider": true,
                            "foldingRangeProvider": true,
                            "documentLinkProvider": {"resolveProvider": false},
                            "diagnosticProvider": {
                                "identifier": "jman-java",
                                "interFileDependencies": true,
                                "workspaceDiagnostics": true
                            },
                            "executeCommandProvider": {
                                "commands": [
                                    "jman.java.status",
                                    "jman.java.syncWorkspace",
                                    "jman.java.rebuildIndex",
                                    "jman.java.clearWorkspaceCache"
                                ]
                            },
                            "experimental": {
                                "jmanJavaTesting": {
                                    "protocolVersion": 2,
                                    "discovery": true,
                                    "run": true,
                                    "streamingEvents": true,
                                    "parameterizedTests": true,
                                    "rerun": true,
                                    "debugDescriptors": true,
                                    "coverage": true
                                },
                                "jmanJavaSourceMetadata": {
                                    "protocolVersion": 1,
                                    "generatedSources": true
                                }
                            }
                        },
                        "serverInfo": {
                            "name": "io.github.zonnedev.jman.lsp",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                }))
            }
            "shutdown" if self.lifecycle == Lifecycle::Running => {
                self.lifecycle = Lifecycle::Shutdown;
                Dispatch::Reply(json!({"jsonrpc":"2.0","id":id,"result":null}))
            }
            "textDocument/definition" if self.lifecycle == Lifecycle::Running => {
                self.definition(id, message.get("params"))
            }
            "textDocument/declaration" if self.lifecycle == Lifecycle::Running => {
                self.declaration(id, message.get("params"))
            }
            "textDocument/implementation" if self.lifecycle == Lifecycle::Running => {
                self.implementation(id, message.get("params"))
            }
            "textDocument/typeDefinition" if self.lifecycle == Lifecycle::Running => {
                self.type_definition(id, message.get("params"))
            }
            "textDocument/prepareCallHierarchy" if self.lifecycle == Lifecycle::Running => {
                self.prepare_call_hierarchy(id, message.get("params"))
            }
            "callHierarchy/incomingCalls" if self.lifecycle == Lifecycle::Running => {
                self.incoming_calls(id, message.get("params"))
            }
            "callHierarchy/outgoingCalls" if self.lifecycle == Lifecycle::Running => {
                self.outgoing_calls(id, message.get("params"))
            }
            "textDocument/prepareTypeHierarchy" if self.lifecycle == Lifecycle::Running => {
                self.prepare_type_hierarchy(id, message.get("params"))
            }
            "typeHierarchy/supertypes" if self.lifecycle == Lifecycle::Running => {
                self.supertypes(id, message.get("params"))
            }
            "typeHierarchy/subtypes" if self.lifecycle == Lifecycle::Running => {
                self.subtypes(id, message.get("params"))
            }
            "textDocument/documentSymbol" if self.lifecycle == Lifecycle::Running => {
                self.document_symbols(id, message.get("params"))
            }
            "textDocument/documentHighlight" if self.lifecycle == Lifecycle::Running => {
                self.document_highlights(id, message.get("params"))
            }
            "textDocument/references" if self.lifecycle == Lifecycle::Running => {
                self.references(id, message.get("params"))
            }
            "textDocument/completion" if self.lifecycle == Lifecycle::Running => {
                self.completion(id, message.get("params"))
            }
            "completionItem/resolve" if self.lifecycle == Lifecycle::Running => {
                self.resolve_completion(id, message.get("params"))
            }
            "textDocument/hover" if self.lifecycle == Lifecycle::Running => {
                self.hover(id, message.get("params"))
            }
            "textDocument/signatureHelp" if self.lifecycle == Lifecycle::Running => {
                self.signature_help(id, message.get("params"))
            }
            "textDocument/semanticTokens/full" if self.lifecycle == Lifecycle::Running => {
                self.semantic_tokens_full(id, message.get("params"))
            }
            "textDocument/semanticTokens/full/delta" if self.lifecycle == Lifecycle::Running => {
                self.semantic_tokens_delta(id, message.get("params"))
            }
            "textDocument/semanticTokens/range" if self.lifecycle == Lifecycle::Running => {
                self.semantic_tokens_range(id, message.get("params"))
            }
            "textDocument/codeAction" if self.lifecycle == Lifecycle::Running => {
                self.code_actions(id, message.get("params"))
            }
            "codeAction/resolve" if self.lifecycle == Lifecycle::Running => {
                self.resolve_code_action(id, message.get("params"))
            }
            "textDocument/codeLens" if self.lifecycle == Lifecycle::Running => {
                self.test_code_lenses(id, message.get("params"))
            }
            "textDocument/formatting" if self.lifecycle == Lifecycle::Running => {
                self.format_document(id, message.get("params"))
            }
            "textDocument/inlayHint" if self.lifecycle == Lifecycle::Running => {
                self.inlay_hints(id, message.get("params"))
            }
            "textDocument/selectionRange" if self.lifecycle == Lifecycle::Running => {
                self.selection_ranges(id, message.get("params"))
            }
            "textDocument/foldingRange" if self.lifecycle == Lifecycle::Running => {
                self.folding_ranges(id, message.get("params"))
            }
            "textDocument/documentLink" if self.lifecycle == Lifecycle::Running => {
                self.document_links(id, message.get("params"))
            }
            "textDocument/diagnostic" if self.lifecycle == Lifecycle::Running => {
                self.document_diagnostic(id, message.get("params"))
            }
            "workspace/diagnostic" if self.lifecycle == Lifecycle::Running => {
                self.workspace_diagnostic(id, message.get("params"))
            }
            "workspace/willCreateFiles"
            | "workspace/willRenameFiles"
            | "workspace/willDeleteFiles"
                if self.lifecycle == Lifecycle::Running =>
            {
                Dispatch::Reply(success(id, Value::Null))
            }
            "workspace/symbol" if self.lifecycle == Lifecycle::Running => {
                self.workspace_symbols(id, message.get("params"))
            }
            "workspace/executeCommand" if self.lifecycle == Lifecycle::Running => {
                self.execute_command(id, message.get("params"))
            }
            "textDocument/prepareRename" if self.lifecycle == Lifecycle::Running => {
                self.prepare_rename(id, message.get("params"))
            }
            "textDocument/rename" if self.lifecycle == Lifecycle::Running => {
                self.rename(id, message.get("params"))
            }
            "jman.java/changeSignature" if self.lifecycle == Lifecycle::Running => {
                self.change_signature(id, message.get("params"))
            }
            "jman.java/tests/discover" if self.lifecycle == Lifecycle::Running => {
                self.discover_tests(id, message.get("params"))
            }
            "jman.java/tests/run" if self.lifecycle == Lifecycle::Running => {
                self.prepare_test_run(id, message.get("params"))
            }
            "jman.java/sourceMetadata" if self.lifecycle == Lifecycle::Running => {
                self.source_metadata(id, message.get("params"))
            }
            _ if self.lifecycle == Lifecycle::Shutdown => {
                Dispatch::Reply(error(id, -32600, "Server has shut down"))
            }
            _ => Dispatch::Reply(error(id, -32601, "Method not found")),
        }
    }

    fn refresh_structural_workspace(&mut self) {
        let revision = self
            .backend
            .as_ref()
            .map(|backend| backend.workspace_structural_revision())
            .unwrap_or(0);
        if revision != self.structural_revision {
            self.index_workspace();
            self.structural_revision = revision;
        }
    }

    fn notification(&mut self, method: &str, params: Option<&Value>) -> Dispatch {
        if self.lifecycle != Lifecycle::Running {
            return Dispatch::Notification;
        }
        let Some(params) = params else {
            return Dispatch::Notification;
        };
        match method {
            "textDocument/didOpen" => {
                let document = &params["textDocument"];
                let Some(uri) = document["uri"].as_str() else {
                    return Dispatch::Notification;
                };
                let version = document["version"].as_i64().unwrap_or(0);
                let text = document["text"].as_str().unwrap_or_default().to_owned();
                self.documents.open(uri.to_owned(), version, text);
                if is_external_source_uri(uri) {
                    self.restore_external_origin(uri);
                    return self.analyze_external_document(uri, version);
                }
                self.analyze_document(uri)
            }
            "textDocument/didChange" => {
                let document = &params["textDocument"];
                let Some(uri) = document["uri"].as_str() else {
                    return Dispatch::Notification;
                };
                let version = document["version"].as_i64().unwrap_or(0);
                let Some(changes) = params["contentChanges"].as_array() else {
                    return Dispatch::Notification;
                };
                if self.documents.change(uri, version, changes).is_err() {
                    return Dispatch::Notification;
                }
                if is_external_source_uri(uri) {
                    return self.analyze_external_document(uri, version);
                }
                self.analyze_document(uri)
            }
            "textDocument/didClose" => {
                let Some(uri) = params["textDocument"]["uri"].as_str() else {
                    return Dispatch::Notification;
                };
                self.documents.close(uri);
                if let Some(backend) = self.backend.as_mut() {
                    backend.close(uri);
                }
                if let Some(source) = self.indexed_sources.get(uri).cloned() {
                    self.update_index(uri, &source);
                } else {
                    self.analyses.remove(uri);
                    let change = self.project.remove_document(uri);
                    if let Some(backend) = self.backend.as_mut() {
                        backend.invalidate(&change.invalidated_documents);
                    }
                }
                Dispatch::Reply(publish_diagnostics(uri, None, &[], ""))
            }
            "workspace/didChangeWatchedFiles" => {
                let changed: Vec<_> = params["changes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|change| change["uri"].as_str().map(str::to_owned))
                    .collect();
                let build_changes: Vec<_> = changed
                    .into_iter()
                    .filter(|uri| is_build_model_file(uri))
                    .collect();
                if build_changes.is_empty() {
                    return Dispatch::Notification;
                }
                self.pending_build_changes.extend(build_changes);
                self.build_sync_state = BuildSyncState::Required;
                if self.build_sync_mode == BuildSyncMode::Automatic {
                    self.synchronize_project()
                } else {
                    Dispatch::Reply(self.build_sync_notification())
                }
            }
            "workspace/didChangeWorkspaceFolders" => {
                let added = workspace_folder_uris(&params["event"]["added"]);
                let removed = workspace_folder_uris(&params["event"]["removed"]);
                let result = self.backend.as_mut().map_or(Ok(()), |backend| {
                    backend.workspace_folders_changed(&added, &removed)
                });
                match result {
                    Ok(()) => {
                        let removed_paths: HashSet<_> = removed
                            .iter()
                            .filter_map(|uri| file_uri_to_path(uri))
                            .collect();
                        self.workspace_roots
                            .retain(|root| !removed_paths.contains(root));
                        self.workspace_roots
                            .extend(added.iter().filter_map(|uri| file_uri_to_path(uri)));
                        self.workspace_roots.sort();
                        self.workspace_roots.dedup();
                        self.reindex_open_documents()
                    }
                    Err(message) => Dispatch::Reply(json!({
                        "jsonrpc":"2.0","method":"window/logMessage",
                        "params":{"type":1,"message":message}
                    })),
                }
            }
            "workspace/didChangeConfiguration" => {
                self.build_sync_mode =
                    BuildSyncMode::from_settings(&params["settings"], self.build_sync_mode);
                let result = self.backend.as_mut().map_or(Ok(()), |backend| {
                    backend.update_configuration(&params["settings"])
                });
                match result {
                    Ok(()) => self.reindex_open_documents(),
                    Err(message) => Dispatch::Reply(json!({
                        "jsonrpc":"2.0","method":"window/logMessage",
                        "params":{"type":1,"message":message}
                    })),
                }
            }
            "workspace/didCreateFiles"
            | "workspace/didRenameFiles"
            | "workspace/didDeleteFiles" => {
                let changed = file_operation_uris(params);
                let result = self
                    .backend
                    .as_mut()
                    .map_or(Ok(()), |backend| backend.rebuild_index());
                match result {
                    Ok(()) => self.reindex_open_documents(),
                    Err(message) => Dispatch::Reply(json!({
                        "jsonrpc":"2.0","method":"window/logMessage",
                        "params":{"type":1,"message":format!(
                            "cannot refresh Java workspace after file changes ({}): {message}",
                            changed.join(", ")
                        )}
                    })),
                }
            }
            "$/cancelRequest" => {
                if let Some(backend) = self.backend.as_mut() {
                    backend.cancel_request(&params["id"]);
                }
                Dispatch::Notification
            }
            "textDocument/didSave" => {
                let Some(uri) = params["textDocument"]["uri"].as_str() else {
                    return Dispatch::Notification;
                };
                let regenerated = match self.backend.as_mut() {
                    Some(backend) => backend.source_saved(uri),
                    None => Ok(false),
                };
                match regenerated {
                    Ok(false) => Dispatch::Notification,
                    Err(message) => Dispatch::Reply(json!({
                        "jsonrpc":"2.0","method":"window/logMessage",
                        "params":{"type":1,"message":message}
                    })),
                    Ok(true) => self.reindex_open_documents(),
                }
            }
            _ => Dispatch::Notification,
        }
    }

    fn reindex_open_documents(&mut self) -> Dispatch {
        self.project = ProjectState::default();
        self.structural_project = ProjectState::default();
        self.analyses.clear();
        self.indexed_sources.clear();
        self.index_workspace();
        let uris: Vec<_> = self.documents.uris().map(str::to_owned).collect();
        Dispatch::Batch(
            uris.iter()
                .filter_map(|uri| match self.analyze_document(uri) {
                    Dispatch::Reply(message) => Some(message),
                    _ => None,
                })
                .collect(),
        )
    }

    fn analyze_document(&mut self, uri: &str) -> Dispatch {
        let Some(document) = self.documents.get(uri) else {
            return Dispatch::Notification;
        };
        let Some(backend) = self.backend.as_mut() else {
            return Dispatch::Notification;
        };
        let file_name = uri.rsplit('/').next().unwrap_or("Input.java");
        match backend.analyze(uri, file_name, &document.text) {
            Ok(result) => {
                let notification = publish_diagnostics(
                    uri,
                    Some(document.version),
                    &result.diagnostics,
                    &document.text,
                );
                let change = self.project.update_document(uri, &result);
                let dependents: Vec<_> = change
                    .invalidated_documents
                    .into_iter()
                    .filter(|document| document != uri)
                    .collect();
                backend.invalidate(&dependents);
                self.analyses.insert(uri.to_owned(), result);
                Dispatch::Reply(notification)
            }
            Err(message) => Dispatch::Reply(json!({
                "jsonrpc":"2.0",
                "method":"window/logMessage",
                "params":{"type":1,"message":message}
            })),
        }
    }

    fn clear_diagnostics(&self, uri: &str, version: i64) -> Dispatch {
        let source = self.source_text(uri).unwrap_or_default();
        Dispatch::Reply(publish_diagnostics(uri, Some(version), &[], source))
    }

    fn analyze_external_document(&mut self, uri: &str, version: i64) -> Dispatch {
        let Some(origin) = self.external_origins.get(uri).cloned() else {
            return self.clear_diagnostics(uri, version);
        };
        let Some(source) = self.source_text(uri).map(str::to_owned) else {
            return self.clear_diagnostics(uri, version);
        };
        let file_name = uri.rsplit('/').next().unwrap_or("External.java");
        if let Some(backend) = self.backend.as_mut()
            && let Ok(result) = backend.analyze(&origin, file_name, &source)
        {
            self.analyses.insert(uri.to_owned(), result);
        }
        self.clear_diagnostics(uri, version)
    }

    fn restore_external_origin(&mut self, uri: &str) {
        if self.external_origins.contains_key(uri) {
            return;
        }
        let Some(path) = uri.strip_prefix("file://").map(PathBuf::from) else {
            return;
        };
        if let Ok(origin) = std::fs::read_to_string(external_origin_path(&path)) {
            let origin = origin.trim();
            if !origin.is_empty() {
                self.external_origins
                    .insert(uri.to_owned(), origin.to_owned());
            }
        }
    }

    fn index_workspace(&mut self) {
        let started = std::time::Instant::now();
        let structural = self
            .backend
            .as_ref()
            .map(|backend| backend.workspace_structural_documents())
            .unwrap_or_default();
        if !structural.is_empty() {
            self.structural_project = ProjectState::default();
            self.indexed_sources.clear();
            let source_count = structural.len();
            for (uri, source, result) in structural {
                self.indexed_sources.insert(uri.clone(), source);
                if self.documents.get(&uri).is_none() {
                    self.analyses.insert(uri.clone(), result.clone());
                }
                self.structural_project.update_document(uri, &result);
            }
            self.analyses.retain(|uri, _| {
                self.documents.get(uri).is_some() || self.indexed_sources.contains_key(uri)
            });
            crate::metrics::emit(
                "workspace-index",
                json!({
                    "milliseconds": started.elapsed().as_millis(),
                    "sourceFiles": source_count,
                    "indexedFiles": source_count,
                    "semanticResults": 0,
                    "mode": "structural"
                }),
            );
            return;
        }
        if self
            .backend
            .as_ref()
            .is_some_and(|backend| backend.workspace_index_pending())
        {
            return;
        }
        let sources = self
            .backend
            .as_ref()
            .map(|backend| backend.workspace_source_files())
            .unwrap_or_default();
        let source_count = sources.len();
        let mut indexed = 0_usize;
        for path in sources {
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            let uri = format!("file://{}", path.display());
            self.indexed_sources.insert(uri.clone(), source.clone());
            self.update_index(&uri, &source);
            indexed += 1;
        }
        crate::metrics::emit(
            "workspace-index",
            json!({
                "milliseconds": started.elapsed().as_millis(),
                "sourceFiles": source_count,
                "indexedFiles": indexed,
                "semanticResults": self.analyses.len()
            }),
        );
    }

    fn update_index(&mut self, uri: &str, source: &str) {
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        let file_name = uri.rsplit('/').next().unwrap_or("Input.java");
        if let Ok(result) = backend.analyze(uri, file_name, source) {
            let change = self.project.update_document(uri, &result);
            backend.invalidate(&change.invalidated_documents);
            self.analyses.insert(uri.to_owned(), result);
        }
    }

    fn definition(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid definition params"));
        };
        if let Some(origin) = self.external_origins.get(uri).cloned() {
            let source = self.source_text(uri).unwrap_or_default().to_owned();
            let definition = self.backend.as_mut().and_then(|backend| {
                backend
                    .editor_query(
                        &origin,
                        uri.rsplit('/').next().unwrap_or("External.java"),
                        &source,
                        u32::try_from(offset).unwrap_or(u32::MAX),
                    )
                    .ok()
                    .and_then(|result| result.definition)
            });
            return Dispatch::Reply(success(
                id,
                definition
                    .and_then(|definition| {
                        self.workspace_editor_definition_locations(&definition)
                            .into_iter()
                            .next()
                            .or_else(|| self.external_location(&origin, &definition).ok())
                    })
                    .map(|location| json!([location]))
                    .unwrap_or(Value::Null),
            ));
        }
        if uri.ends_with("/module-info.java") {
            let source = self.source_text(uri).unwrap_or_default();
            let byte = utf16_offset_to_byte(source, offset);
            if let Some(module) = jpms_module_token_at(source, byte) {
                let locations: Vec<_> = self
                    .indexed_sources
                    .iter()
                    .filter_map(|(candidate_uri, candidate_source)| {
                        module_declaration_range(candidate_source, module).map(|(start, end)| {
                            json!({
                                "uri": candidate_uri,
                                "range": {
                                    "start": offset_to_position(candidate_source, start as u64),
                                    "end": offset_to_position(candidate_source, end as u64)
                                }
                            })
                        })
                    })
                    .collect();
                if !locations.is_empty() {
                    return Dispatch::Reply(success(id, json!(locations)));
                }
            }
        }
        let Some(symbol) = self.project.symbol_at(uri, offset).cloned() else {
            let source = self.source_text(uri).unwrap_or_default().to_owned();
            let external = self.backend.as_mut().and_then(|backend| {
                backend
                    .editor_query(
                        uri,
                        uri.rsplit('/').next().unwrap_or("Input.java"),
                        &source,
                        u32::try_from(offset).unwrap_or(u32::MAX),
                    )
                    .ok()
                    .and_then(|result| result.definition)
            });
            if let Some(definition) = external {
                let workspace = self.workspace_editor_definition_locations(&definition);
                if !workspace.is_empty() {
                    return Dispatch::Reply(success(id, json!(workspace)));
                }
                if let Ok(location) = self.external_location(uri, &definition) {
                    return Dispatch::Reply(success(id, json!([location])));
                }
            }
            return Dispatch::Reply(success(id, Value::Null));
        };
        let symbol_id = location_symbol_id(&symbol);
        let project_definitions = self.project.definitions(symbol_id);
        let structural_definitions = if project_definitions.is_empty() {
            let exact = self.structural_project.definitions(symbol_id);
            if exact.is_empty() {
                self.structural_project.definitions(&symbol.qualified_name)
            } else {
                exact
            }
        } else {
            &[]
        };
        let locations: Vec<_> = project_definitions
            .iter()
            .chain(structural_definitions)
            .filter_map(|location| self.location(location))
            .collect();
        if !locations.is_empty() {
            return Dispatch::Reply(success(id, json!(locations)));
        }
        let source = self.source_text(uri).unwrap_or_default().to_owned();
        let external = self.backend.as_mut().and_then(|backend| {
            backend
                .editor_query(
                    uri,
                    uri.rsplit('/').next().unwrap_or("Input.java"),
                    &source,
                    u32::try_from(offset).unwrap_or(u32::MAX),
                )
                .ok()
                .and_then(|result| result.definition)
        });
        if let Some(definition) = external {
            let workspace = self.workspace_editor_definition_locations(&definition);
            if !workspace.is_empty() {
                return Dispatch::Reply(success(id, json!(workspace)));
            }
            if let Ok(location) = self.external_location(uri, &definition) {
                return Dispatch::Reply(success(id, json!([location])));
            }
        }
        Dispatch::Reply(success(id, json!(locations)))
    }

    fn declaration(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid declaration params"));
        };
        let symbol = self
            .project
            .symbol_at(uri, offset)
            .or_else(|| self.structural_project.symbol_at(uri, offset))
            .cloned();
        if let Some(symbol) = symbol {
            let selected_id = location_symbol_id(&symbol);
            let override_family = self
                .project
                .override_family(selected_id)
                .or_else(|| self.structural_project.override_family(selected_id));
            let declaration_id = override_family.unwrap_or(selected_id);
            let mut declarations = self.all_definitions(declaration_id);
            if declarations.is_empty()
                && let Some(structural_id) = structural_method_identity(declaration_id)
            {
                declarations = self.all_definitions(&structural_id);
            }
            if declarations.is_empty() && override_family.is_none() && symbol.role == "declaration"
            {
                declarations.push(symbol);
            }
            let locations: Vec<_> = declarations
                .iter()
                .filter_map(|location| self.location(location))
                .collect();
            if !locations.is_empty() {
                return Dispatch::Reply(success(id, json!(locations)));
            }
        }
        let source = self.source_text(uri).unwrap_or_default().to_owned();
        let origin = self
            .external_origins
            .get(uri)
            .map(String::as_str)
            .unwrap_or(uri)
            .to_owned();
        let declaration = self.backend.as_mut().and_then(|backend| {
            backend
                .editor_query(
                    &origin,
                    uri.rsplit('/').next().unwrap_or("Input.java"),
                    &source,
                    u32::try_from(offset).unwrap_or(u32::MAX),
                )
                .ok()
                .and_then(|result| result.definition)
        });
        let Some(declaration) = declaration else {
            return Dispatch::Reply(success(id, Value::Null));
        };
        let workspace = self.workspace_editor_definition_locations(&declaration);
        if !workspace.is_empty() {
            return Dispatch::Reply(success(id, json!(workspace)));
        }
        let location = self.external_location(&origin, &declaration).ok();
        Dispatch::Reply(success(
            id,
            location
                .map(|location| json!([location]))
                .unwrap_or(Value::Null),
        ))
    }

    fn implementation(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid implementation params"));
        };
        let Some(symbol) = self.project.symbol_at(uri, offset) else {
            return Dispatch::Reply(success(id, Value::Null));
        };
        let selected_id = location_symbol_id(symbol);
        let related = self.related_symbol_ids(selected_id);
        if related.len() <= 1 {
            return Dispatch::Reply(success(id, json!([])));
        }
        let mut seen = HashSet::new();
        let locations: Vec<_> = related
            .iter()
            .filter(|candidate| symbol.role != "declaration" || candidate.as_str() != selected_id)
            .flat_map(|candidate| self.all_definitions(candidate))
            .filter(|location| {
                seen.insert((location.document.clone(), location.start, location.end))
            })
            .filter_map(|location| self.location(&location))
            .collect();
        Dispatch::Reply(success(id, json!(locations)))
    }

    fn type_definition(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid type-definition params"));
        };
        let source = self.source_text(uri).unwrap_or_default().to_owned();
        let definition = self.backend.as_mut().and_then(|backend| {
            backend
                .editor_query(
                    uri,
                    uri.rsplit('/').next().unwrap_or("Input.java"),
                    &source,
                    u32::try_from(offset).unwrap_or(u32::MAX),
                )
                .ok()
                .and_then(|result| result.type_definition)
        });
        let Some(definition) = definition else {
            return Dispatch::Reply(success(id, Value::Null));
        };
        let identity = definition_identity(&definition);
        let mut workspace_locations: Vec<_> = self
            .all_definitions(&definition.symbol_id)
            .into_iter()
            .chain(self.all_definitions(&identity))
            .filter_map(|location| self.location(&location))
            .collect();
        if workspace_locations.is_empty() {
            workspace_locations = self.workspace_editor_definition_locations(&definition);
        }
        if !workspace_locations.is_empty() {
            return Dispatch::Reply(success(id, json!(workspace_locations)));
        }
        let location = self.external_location(uri, &definition).ok();
        Dispatch::Reply(success(
            id,
            location
                .map(|location| json!([location]))
                .unwrap_or(Value::Null),
        ))
    }

    fn call_hierarchy_item(&self, location: &javac_frontend::SymbolLocation) -> Option<Value> {
        let target = self.location(location)?;
        Some(json!({
            "name": location.name,
            "kind": lsp_symbol_kind(&location.kind),
            "detail": location.qualified_name,
            "uri": target["uri"],
            "range": target["range"],
            "selectionRange": target["range"],
            "data": {"symbolId": location_symbol_id(location)}
        }))
    }

    fn prepare_call_hierarchy(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid call-hierarchy params"));
        };
        let Some(symbol) = self.project.symbol_at(uri, offset) else {
            return Dispatch::Reply(success(id, Value::Null));
        };
        let symbol_id = location_symbol_id(symbol);
        let definition = self
            .all_definitions(symbol_id)
            .into_iter()
            .next()
            .or_else(|| (symbol.role == "declaration").then(|| symbol.clone()));
        Dispatch::Reply(success(
            id,
            definition
                .as_ref()
                .and_then(|location| self.call_hierarchy_item(location))
                .map(|item| json!([item]))
                .unwrap_or(Value::Null),
        ))
    }

    fn incoming_calls(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(symbol_id) = params.and_then(|value| value["item"]["data"]["symbolId"].as_str())
        else {
            return Dispatch::Reply(error(id, -32602, "Invalid incoming-call params"));
        };
        let mut edges = self.project.incoming_calls(symbol_id);
        edges.extend(self.structural_project.incoming_calls(symbol_id));
        let mut grouped: HashMap<String, Vec<javac_frontend::SymbolLocation>> = HashMap::new();
        for edge in edges {
            grouped
                .entry(edge.qualified_name.clone())
                .or_default()
                .push(edge);
        }
        let mut calls: Vec<_> = grouped
            .into_iter()
            .filter_map(|(caller, edges)| {
                let definition = self.all_definitions(&caller).into_iter().next()?;
                let item = self.call_hierarchy_item(&definition)?;
                let ranges: Vec<_> = edges
                    .iter()
                    .filter_map(|edge| {
                        self.location(edge)
                            .map(|location| location["range"].clone())
                    })
                    .collect();
                Some(json!({"from": item, "fromRanges": ranges}))
            })
            .collect();
        calls.sort_by(|left, right| {
            left["from"]["name"]
                .as_str()
                .cmp(&right["from"]["name"].as_str())
        });
        Dispatch::Reply(success(id, json!(calls)))
    }

    fn outgoing_calls(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(symbol_id) = params.and_then(|value| value["item"]["data"]["symbolId"].as_str())
        else {
            return Dispatch::Reply(error(id, -32602, "Invalid outgoing-call params"));
        };
        let mut edges = self.project.outgoing_calls(symbol_id);
        edges.extend(self.structural_project.outgoing_calls(symbol_id));
        let mut grouped: HashMap<String, Vec<javac_frontend::SymbolLocation>> = HashMap::new();
        for edge in edges {
            grouped
                .entry(edge.symbol_id.clone())
                .or_default()
                .push(edge);
        }
        let mut calls: Vec<_> = grouped
            .into_iter()
            .filter_map(|(callee, edges)| {
                let definition = self.all_definitions(&callee).into_iter().next()?;
                let item = self.call_hierarchy_item(&definition)?;
                let ranges: Vec<_> = edges
                    .iter()
                    .filter_map(|edge| {
                        self.location(edge)
                            .map(|location| location["range"].clone())
                    })
                    .collect();
                Some(json!({"to": item, "fromRanges": ranges}))
            })
            .collect();
        calls.sort_by(|left, right| {
            left["to"]["name"]
                .as_str()
                .cmp(&right["to"]["name"].as_str())
        });
        Dispatch::Reply(success(id, json!(calls)))
    }

    fn type_hierarchy_item(&self, location: &javac_frontend::SymbolLocation) -> Option<Value> {
        let target = self.location(location)?;
        Some(json!({
            "name": location.name,
            "kind": lsp_symbol_kind(&location.kind),
            "detail": location.qualified_name,
            "uri": target["uri"],
            "range": target["range"],
            "selectionRange": target["range"],
            "data": {"symbolId": location_symbol_id(location)}
        }))
    }

    fn prepare_type_hierarchy(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid type-hierarchy params"));
        };
        let Some(symbol) = self.project.symbol_at(uri, offset) else {
            return Dispatch::Reply(success(id, Value::Null));
        };
        let definition = self
            .all_definitions(location_symbol_id(symbol))
            .into_iter()
            .find(|candidate| {
                matches!(
                    candidate.kind.as_str(),
                    "class" | "interface" | "enum" | "record"
                )
            });
        Dispatch::Reply(success(
            id,
            definition
                .as_ref()
                .and_then(|location| self.type_hierarchy_item(location))
                .map(|item| json!([item]))
                .unwrap_or(Value::Null),
        ))
    }

    fn supertypes(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(symbol_id) = params.and_then(|value| value["item"]["data"]["symbolId"].as_str())
        else {
            return Dispatch::Reply(error(id, -32602, "Invalid supertype params"));
        };
        let mut edges = self.project.direct_supertypes(symbol_id);
        edges.extend(self.structural_project.direct_supertypes(symbol_id));
        let mut seen = HashSet::new();
        let items: Vec<_> = edges
            .into_iter()
            .filter(|edge| seen.insert(edge.symbol_id.clone()))
            .filter_map(|edge| self.all_definitions(&edge.symbol_id).into_iter().next())
            .filter_map(|location| self.type_hierarchy_item(&location))
            .collect();
        Dispatch::Reply(success(id, json!(items)))
    }

    fn subtypes(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(symbol_id) = params.and_then(|value| value["item"]["data"]["symbolId"].as_str())
        else {
            return Dispatch::Reply(error(id, -32602, "Invalid subtype params"));
        };
        let mut edges = self.project.direct_subtypes(symbol_id);
        edges.extend(self.structural_project.direct_subtypes(symbol_id));
        let mut seen = HashSet::new();
        let items: Vec<_> = edges
            .into_iter()
            .filter(|edge| seen.insert(edge.qualified_name.clone()))
            .filter_map(|edge| {
                self.all_definitions(&edge.qualified_name)
                    .into_iter()
                    .next()
            })
            .filter_map(|location| self.type_hierarchy_item(&location))
            .collect();
        Dispatch::Reply(success(id, json!(items)))
    }

    fn external_location(
        &mut self,
        origin: &str,
        definition: &javac_frontend::EditorDefinition,
    ) -> Result<Value, String> {
        if definition.source.is_empty() {
            return Err(format!(
                "source for {}.{} is unavailable",
                definition.owner, definition.name
            ));
        }
        let path = persist_external_source(definition)?;
        let target_uri = format!("file://{}", path.display());
        persist_external_origin(&path, origin)?;
        self.external_origins
            .insert(target_uri.clone(), origin.to_owned());
        Ok(json!({
            "uri": target_uri,
            "range": {
                "start": offset_to_position(&definition.source, definition.start),
                "end": offset_to_position(&definition.source, definition.end)
            }
        }))
    }

    fn references(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid references params"));
        };
        if let Some(origin) = self.external_origins.get(uri).cloned() {
            let source = self.source_text(uri).unwrap_or_default().to_owned();
            let identity = self.backend.as_mut().and_then(|backend| {
                backend
                    .editor_query(
                        &origin,
                        uri.rsplit('/').next().unwrap_or("External.java"),
                        &source,
                        u32::try_from(offset).unwrap_or(u32::MAX),
                    )
                    .ok()
                    .and_then(|result| result.definition)
            });
            let locations = identity
                .map(|definition| definition_identity(&definition))
                .map(|identity| {
                    self.related_symbol_ids(&identity)
                        .iter()
                        .flat_map(|symbol_id| self.all_references(symbol_id))
                        .filter_map(|location| self.location(&location))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            return Dispatch::Reply(success(id, json!(locations)));
        }
        let Some(symbol_name) = self
            .project
            .symbol_at(uri, offset)
            .or_else(|| self.structural_project.symbol_at(uri, offset))
            .map(|symbol| symbol.name.clone())
        else {
            return Dispatch::Reply(success(id, json!([])));
        };
        self.ensure_workspace_symbol_semantics(&symbol_name);
        let Some(symbol) = self
            .project
            .symbol_at(uri, offset)
            .or_else(|| self.structural_project.symbol_at(uri, offset))
        else {
            return Dispatch::Reply(success(id, json!([])));
        };
        let symbol_id = location_symbol_id(symbol);
        let related = self.related_symbol_ids(symbol_id);
        let include_declaration = params
            .and_then(|value| value["context"]["includeDeclaration"].as_bool())
            .unwrap_or(false);
        let mut matches = Vec::new();
        for related_id in related {
            if include_declaration {
                matches.extend(self.all_definitions(&related_id));
            }
            matches.extend(self.all_references(&related_id));
        }
        matches.sort_by(|left, right| {
            left.document
                .cmp(&right.document)
                .then_with(|| left.start.cmp(&right.start))
                .then_with(|| left.end.cmp(&right.end))
        });
        matches.dedup_by(|left, right| {
            left.document == right.document && left.start == right.start && left.end == right.end
        });
        let locations: Vec<_> = matches
            .iter()
            .filter_map(|location| self.location(location))
            .collect();
        Dispatch::Reply(success(id, json!(locations)))
    }

    fn completion(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid completion params"));
        };
        let source = self.source_text(uri).unwrap_or_default().to_owned();
        let context = self
            .external_origins
            .get(uri)
            .map(String::as_str)
            .unwrap_or(uri)
            .to_owned();
        let byte = utf16_offset_to_byte(&source, offset);
        let prefix = java_identifier_prefix(&source[..byte]).to_owned();
        if uri.ends_with("/module-info.java")
            && let Some(context) = jpms_completion_context(&source, byte)
        {
            let catalog = self
                .backend
                .as_ref()
                .map(|backend| backend.jpms_catalog(uri))
                .unwrap_or_default();
            let mut items = Vec::new();
            match context {
                JpmsCompletionContext::Module(prefix) => {
                    for module in catalog
                        .modules
                        .into_iter()
                        .filter(|module| module.starts_with(&prefix))
                    {
                        items.push(json!({
                            "label": module,
                            "kind": 9,
                            "detail": "JPMS module",
                            "sortText": format!("0:{module}"),
                            "insertText": module
                        }));
                    }
                }
                JpmsCompletionContext::Package(prefix) => {
                    for package in catalog
                        .packages
                        .into_iter()
                        .filter(|package| package.starts_with(&prefix))
                    {
                        items.push(json!({
                            "label": package,
                            "kind": 9,
                            "detail": "Package in current module",
                            "sortText": format!("0:{package}"),
                            "insertText": package
                        }));
                    }
                }
                JpmsCompletionContext::Service(prefix) => {
                    let query = prefix.rsplit('.').next().unwrap_or(&prefix);
                    for symbol in self
                        .project
                        .completion_symbols(query, 100)
                        .into_iter()
                        .chain(self.structural_project.completion_symbols(query, 200))
                        .filter(|symbol| {
                            symbol.qualified_name.starts_with(&prefix) && symbol.kind == "interface"
                        })
                    {
                        items.push(json!({
                            "label": symbol.qualified_name,
                            "kind": completion_item_kind(&symbol.kind),
                            "detail": "Service interface",
                            "sortText": format!("0:{}", symbol.qualified_name),
                            "insertText": symbol.qualified_name
                        }));
                    }
                }
                JpmsCompletionContext::Provider(prefix) => {
                    let query = prefix.rsplit('.').next().unwrap_or(&prefix);
                    for symbol in self
                        .project
                        .completion_symbols(query, 100)
                        .into_iter()
                        .chain(self.structural_project.completion_symbols(query, 200))
                        .filter(|symbol| {
                            symbol.qualified_name.starts_with(&prefix)
                                && is_type_symbol(&symbol.kind)
                        })
                    {
                        items.push(json!({
                            "label": symbol.qualified_name,
                            "kind": completion_item_kind(&symbol.kind),
                            "detail": "Service provider",
                            "sortText": format!("0:{}", symbol.qualified_name),
                            "insertText": symbol.qualified_name
                        }));
                    }
                }
            }
            let items = self.defer_completion_items(uri, items);
            return Dispatch::Reply(success(id, json!({"isIncomplete":false,"items":items})));
        }
        let typed = self
            .backend
            .as_mut()
            .and_then(|backend| {
                backend
                    .editor_query(
                        &context,
                        uri.rsplit('/').next().unwrap_or("Input.java"),
                        &source,
                        u32::try_from(offset).unwrap_or(u32::MAX),
                    )
                    .ok()
            })
            .unwrap_or(EditorQueryResult {
                completions: Vec::new(),
                signatures: Vec::new(),
                hover: None,
                definition: None,
                type_definition: None,
            });
        let receiver_aware = !typed.completions.is_empty();
        let mut seen = std::collections::HashSet::new();
        let mut items = Vec::new();
        for completion in typed.completions {
            if seen.insert(format!("{}:{}", completion.kind, completion.detail)) {
                items.push(json!({
                    "label": completion.label,
                    "kind": completion_item_kind(&completion.kind),
                    "detail": completion.detail,
                    "documentation": {
                        "kind": "markdown",
                        "value": completion.documentation
                    },
                    "sortText": format!("0:{}", completion.label),
                    "insertText": completion.insert_text
                }));
            }
        }
        if !receiver_aware {
            for symbol in self
                .project
                .completion_symbols(&prefix, 100)
                .into_iter()
                .chain(self.structural_project.completion_symbols(&prefix, 200))
                .filter(|symbol| seen.insert(symbol.qualified_name.clone()))
                .take(200)
            {
                let mut item = json!({
                    "label": symbol.name,
                    "kind": completion_item_kind(&symbol.kind),
                    "detail": symbol.qualified_name,
                    "filterText": symbol.name,
                    "sortText": completion_sort_text(&symbol.name, &prefix),
                    "insertText": symbol.name
                });
                if is_type_symbol(&symbol.kind)
                    && let Some(edit) = import_edit(&source, &symbol.qualified_name)
                {
                    item["additionalTextEdits"] = json!([edit]);
                    item["detail"] = json!(format!("{} — add import", symbol.qualified_name));
                }
                items.push(item);
            }
            items.extend(
                JAVA_COMPLETION_KEYWORDS
                    .iter()
                    .filter(|keyword| keyword.starts_with(&prefix))
                    .map(|keyword| {
                        json!({
                            "label": keyword,
                            "kind": 14,
                            "sortText": format!("3:{keyword}"),
                            "insertText": keyword
                        })
                    }),
            );
        }
        let items = self.defer_completion_items(uri, items);
        Dispatch::Reply(success(id, json!({"isIncomplete": false, "items": items})))
    }

    fn defer_completion_items(&mut self, uri: &str, mut items: Vec<Value>) -> Vec<Value> {
        if self.completion_resolve_properties.is_empty() {
            return items;
        }
        let version = self.documents.get(uri).map(|document| document.version);
        for item in &mut items {
            let Some(object) = item.as_object_mut() else {
                continue;
            };
            let mut properties = serde_json::Map::new();
            for property in ["detail", "documentation", "additionalTextEdits"] {
                if self.completion_resolve_properties.contains(property)
                    && let Some(value) = object.remove(property)
                {
                    properties.insert(property.to_owned(), value);
                }
            }
            if properties.is_empty() {
                continue;
            }
            let resolve_id = self.next_resolve_id();
            if self.completion_resolutions.len() >= RESOLVE_CACHE_CAPACITY {
                self.completion_resolutions.clear();
            }
            self.completion_resolutions.insert(
                resolve_id,
                ResolveEntry {
                    uri: uri.to_owned(),
                    version,
                    properties: Value::Object(properties),
                },
            );
            object.insert(
                "data".to_owned(),
                json!({"jmanResolve": {"kind": "completion", "id": resolve_id}}),
            );
        }
        items
    }

    fn resolve_completion(&self, id: Value, params: Option<&Value>) -> Dispatch {
        self.resolve_item(id, params, "completion", &self.completion_resolutions)
    }

    fn hover(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid hover params"));
        };
        if let Some(origin) = self.external_origins.get(uri).cloned() {
            let source = self.source_text(uri).unwrap_or_default().to_owned();
            let cursor = utf16_offset_to_byte(&source, offset);
            let hover = self.backend.as_mut().and_then(|backend| {
                backend
                    .editor_query(
                        &origin,
                        uri.rsplit('/').next().unwrap_or("External.java"),
                        &source,
                        u32::try_from(offset).unwrap_or(u32::MAX),
                    )
                    .ok()
                    .and_then(|result| result.hover)
            });
            return Dispatch::Reply(success(
                id,
                hover
                    .map(|hover| {
                        json!({
                            "contents": {
                                "kind": "markdown",
                                "value": format!(
                                    "```java\n{}\n```\n\n{}",
                                    hover.detail, hover.documentation
                                )
                            },
                            "range": identifier_range(&source, cursor)
                        })
                    })
                    .unwrap_or(Value::Null),
            ));
        }
        let symbol = self
            .project
            .symbol_at(uri, offset)
            .or_else(|| self.structural_project.symbol_at(uri, offset))
            .cloned();
        let Some(symbol) = symbol else {
            return Dispatch::Reply(success(id, Value::Null));
        };
        let source = self.source_text(uri).unwrap_or_default().to_owned();
        let typed = self.backend.as_mut().and_then(|backend| {
            backend
                .editor_query(
                    uri,
                    uri.rsplit('/').next().unwrap_or("Input.java"),
                    &source,
                    u32::try_from(symbol.end).unwrap_or(u32::MAX),
                )
                .ok()
                .and_then(|result| result.hover)
        });
        let range = self
            .location(&symbol)
            .map(|location| location["range"].clone());
        let detail = typed
            .as_ref()
            .map(|hover| hover.detail.as_str())
            .unwrap_or(&symbol.qualified_name);
        let documentation = typed
            .as_ref()
            .map(|hover| hover.documentation.as_str())
            .unwrap_or_default();
        Dispatch::Reply(success(
            id,
            json!({
                "contents": {
                    "kind": "markdown",
                    "value": format!(
                        "```java\n{} {}\n```\n\n{}",
                        symbol.kind, detail, documentation
                    )
                },
                "range": range
            }),
        ))
    }

    fn signature_help(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid signature-help params"));
        };
        let source = self.source_text(uri).unwrap_or_default().to_owned();
        let context = self
            .external_origins
            .get(uri)
            .map(String::as_str)
            .unwrap_or(uri)
            .to_owned();
        let byte = utf16_offset_to_byte(&source, offset);
        let before = &source[..byte];
        let Some(open) = before.rfind('(') else {
            return Dispatch::Reply(success(id, Value::Null));
        };
        let name = java_identifier_prefix(before[..open].trim_end());
        if name.is_empty() {
            return Dispatch::Reply(success(id, Value::Null));
        }
        let active_parameter = before[open + 1..]
            .chars()
            .filter(|character| *character == ',')
            .count();
        let typed = self
            .backend
            .as_mut()
            .and_then(|backend| {
                backend
                    .editor_query(
                        &context,
                        uri.rsplit('/').next().unwrap_or("Input.java"),
                        &source,
                        u32::try_from(offset).unwrap_or(u32::MAX),
                    )
                    .ok()
            })
            .unwrap_or(EditorQueryResult {
                completions: Vec::new(),
                signatures: Vec::new(),
                hover: None,
                definition: None,
                type_definition: None,
            });
        let mut seen = std::collections::HashSet::new();
        let mut signatures = Vec::new();
        for signature in typed.signatures {
            if seen.insert(signature.label.clone()) {
                signatures.push(json!({
                    "label": signature.label,
                    "parameters": signature.parameters.into_iter()
                        .map(|label| json!({"label": label})).collect::<Vec<_>>(),
                    "documentation": {
                        "kind": "markdown",
                        "value": if signature.return_type.is_empty() {
                            signature.documentation
                        } else if signature.documentation.is_empty() {
                            format!("Returns `{}`", signature.return_type)
                        } else {
                            signature.documentation
                        }
                    }
                }));
            }
        }
        for symbol in self
            .project
            .workspace_symbols(name, 100)
            .into_iter()
            .chain(self.structural_project.workspace_symbols(name, 100))
            .filter(|symbol| {
                matches!(symbol.kind.as_str(), "method" | "constructor")
                    && symbol.name == name
                    && seen.insert(symbol.qualified_name.clone())
            })
        {
            signatures.push(json!({
                "label": format!("{}(...)", symbol.qualified_name),
                "documentation": {
                    "kind": "markdown",
                    "value": format!("`{}`", symbol.qualified_name)
                }
            }));
        }
        if signatures.is_empty() {
            return Dispatch::Reply(success(id, Value::Null));
        }
        Dispatch::Reply(success(
            id,
            json!({
                "signatures": signatures,
                "activeSignature": 0,
                "activeParameter": active_parameter
            }),
        ))
    }

    fn semantic_tokens_full(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(uri) = params.and_then(|value| value["textDocument"]["uri"].as_str()) else {
            return Dispatch::Reply(error(id, -32602, "Invalid semantic-token params"));
        };
        let data = self.semantic_token_data(uri, None);
        let result_id = self.store_semantic_tokens(uri, data.clone());
        Dispatch::Reply(success(id, json!({"resultId": result_id, "data": data})))
    }

    fn semantic_tokens_delta(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(params) = params else {
            return Dispatch::Reply(error(id, -32602, "Invalid semantic-token delta params"));
        };
        let Some(uri) = params["textDocument"]["uri"].as_str() else {
            return Dispatch::Reply(error(id, -32602, "Semantic-token delta has no URI"));
        };
        let Some(previous_id) = params["previousResultId"].as_str() else {
            return Dispatch::Reply(error(
                id,
                -32602,
                "Semantic-token delta has no previous result ID",
            ));
        };
        let data = self.semantic_token_data(uri, None);
        let previous = self
            .semantic_token_snapshots
            .get(previous_id)
            .filter(|snapshot| snapshot.uri == uri)
            .map(|snapshot| snapshot.data.clone());
        let result_id = self.store_semantic_tokens(uri, data.clone());
        let Some(previous) = previous else {
            return Dispatch::Reply(success(id, json!({"resultId": result_id, "data": data})));
        };
        let edits = semantic_token_delta(&previous, &data);
        Dispatch::Reply(success(id, json!({"resultId": result_id, "edits": edits})))
    }

    fn semantic_tokens_range(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(params) = params else {
            return Dispatch::Reply(error(id, -32602, "Invalid semantic-token range params"));
        };
        let Some(uri) = params["textDocument"]["uri"].as_str() else {
            return Dispatch::Reply(error(id, -32602, "Semantic-token range has no URI"));
        };
        if !params["range"].is_object() {
            return Dispatch::Reply(error(id, -32602, "Semantic-token request has no range"));
        }
        Dispatch::Reply(success(
            id,
            json!({"data": self.semantic_token_data(uri, Some(&params["range"]))}),
        ))
    }

    fn semantic_token_data(&self, uri: &str, range: Option<&Value>) -> Vec<u64> {
        let Some(source) = self.source_text(uri) else {
            return Vec::new();
        };
        let Some(analysis) = self.analyses.get(uri) else {
            return Vec::new();
        };
        let mut symbols = analysis.symbols.clone();
        symbols.sort_by_key(|symbol| (symbol.start, symbol.end));
        let mut data = Vec::new();
        let mut previous_line = 0_u64;
        let mut previous_character = 0_u64;
        for symbol in symbols {
            let Some(token_type) = semantic_token_type(&symbol.kind, &symbol.role) else {
                continue;
            };
            let start = offset_to_position(source, symbol.start);
            let end = offset_to_position(source, symbol.end);
            if range.is_some_and(|range| !ranges_intersect(&start, &end, range)) {
                continue;
            }
            let line = start["line"].as_u64().unwrap_or(0);
            let character = start["character"].as_u64().unwrap_or(0);
            if end["line"].as_u64() != Some(line) {
                continue;
            }
            let length = end["character"]
                .as_u64()
                .unwrap_or(character)
                .saturating_sub(character);
            if length == 0 {
                continue;
            }
            let delta_line = line.saturating_sub(previous_line);
            let delta_start = if delta_line == 0 {
                character.saturating_sub(previous_character)
            } else {
                character
            };
            data.extend([
                delta_line,
                delta_start,
                length,
                token_type,
                semantic_token_modifiers(&symbol),
            ]);
            previous_line = line;
            previous_character = character;
        }
        data
    }

    fn store_semantic_tokens(&mut self, uri: &str, data: Vec<u64>) -> String {
        if self.semantic_token_snapshots.len() >= SEMANTIC_TOKEN_CACHE_CAPACITY {
            self.semantic_token_snapshots.clear();
        }
        self.semantic_token_sequence = self.semantic_token_sequence.wrapping_add(1);
        let result_id = self.semantic_token_sequence.to_string();
        self.semantic_token_snapshots.insert(
            result_id.clone(),
            SemanticTokenSnapshot {
                uri: uri.to_owned(),
                data,
            },
        );
        result_id
    }

    fn code_actions(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(params) = params else {
            return Dispatch::Reply(error(id, -32602, "Invalid code-action params"));
        };
        let Some(uri) = params["textDocument"]["uri"].as_str() else {
            return Dispatch::Reply(error(id, -32602, "Code action has no URI"));
        };
        let mut actions: Vec<_> = params["context"]["diagnostics"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|diagnostic| {
                diagnostic["code"]
                    .as_str()
                    .is_some_and(|code| code == "compiler.err.expected")
                    && diagnostic["message"]
                        .as_str()
                        .is_some_and(|message| message.contains(';'))
            })
            .map(|diagnostic| {
                json!({
                    "title": "Insert missing ';'",
                    "kind": "quickfix",
                    "diagnostics": [diagnostic],
                    "isPreferred": true,
                    "edit": {
                        "changes": {
                            (uri): [{
                                "range": {
                                    "start": diagnostic["range"]["end"],
                                    "end": diagnostic["range"]["end"]
                                },
                                "newText": ";"
                            }]
                        }
                    }
                })
            })
            .collect();
        let source = self.source_text(uri).unwrap_or_default();
        if let Some(edit) = organize_imports_edit(source) {
            actions.push(json!({
                "title": "Organize imports",
                "kind": "source.organizeImports",
                "edit": {"changes": {(uri): [edit]}}
            }));
        }
        actions.extend(generation_actions(source, uri));
        let mut imports = std::collections::HashSet::new();
        for diagnostic in params["context"]["diagnostics"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if !diagnostic["code"]
                .as_str()
                .is_some_and(|code| code.contains("cant.resolve"))
            {
                continue;
            }
            let Some(name) = diagnostic["message"].as_str().and_then(missing_type_name) else {
                continue;
            };
            for symbol in self
                .project
                .completion_symbols(name, 100)
                .into_iter()
                .chain(self.structural_project.completion_symbols(name, 200))
                .filter(|symbol| {
                    symbol.name == name
                        && is_type_symbol(&symbol.kind)
                        && imports.insert(symbol.qualified_name.clone())
                })
            {
                let Some(edit) = import_edit(source, &symbol.qualified_name) else {
                    continue;
                };
                actions.push(json!({
                    "title": format!("Import {}", symbol.qualified_name),
                    "kind": "quickfix",
                    "diagnostics": [diagnostic],
                    "isPreferred": actions.iter().all(|action| {
                        !action["title"].as_str().is_some_and(|title| title.starts_with("Import "))
                    }),
                    "edit": {"changes": {(uri): [edit]}}
                }));
            }
        }
        for diagnostic in params["context"]["diagnostics"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let Some(message) = diagnostic["message"].as_str() else {
                continue;
            };
            if let Some(method) = missing_abstract_method(message)
                && let Some(action) = self.implement_method_action(uri, source, diagnostic, method)
            {
                actions.push(action);
            }
            if let Some(exception) = unreported_exception(message) {
                if let Some(edit) = add_throws_edit(source, diagnostic, exception) {
                    actions.push(json!({
                        "title": format!("Add throws {exception}"),
                        "kind": "quickfix",
                        "diagnostics": [diagnostic],
                        "isPreferred": true,
                        "edit": {"changes": {(uri): [edit]}}
                    }));
                }
                if let Some(edit) = surround_with_try_catch_edit(source, diagnostic, exception) {
                    actions.push(json!({
                        "title": format!("Surround with try/catch ({exception})"),
                        "kind": "quickfix",
                        "diagnostics": [diagnostic],
                        "edit": {"changes": {(uri): [edit]}}
                    }));
                }
            }
            if diagnostic["code"]
                .as_str()
                .is_some_and(|code| code.contains("method.does.not.override"))
                && let Some(edit) = remove_override_edit(source, diagnostic)
            {
                actions.push(json!({
                    "title": "Remove invalid @Override annotation",
                    "kind": "quickfix",
                    "diagnostics": [diagnostic],
                    "isPreferred": true,
                    "edit": {"changes": {(uri): [edit]}}
                }));
            }
            if let Some(required) = jpms_unread_module(message)
                && let Some((descriptor_uri, descriptor_source)) =
                    self.owning_module_descriptor(uri)
            {
                for (title, directive, preferred) in [
                    (
                        format!("Add requires {required}"),
                        format!("requires {required};"),
                        true,
                    ),
                    (
                        format!("Add requires transitive {required}"),
                        format!("requires transitive {required};"),
                        false,
                    ),
                ] {
                    if let Some(edit) = insert_module_directive(&descriptor_source, &directive) {
                        actions.push(json!({
                            "title": title,
                            "kind": "quickfix",
                            "diagnostics": [diagnostic],
                            "isPreferred": preferred,
                            "edit": {"changes": {(descriptor_uri.clone()): [edit]}}
                        }));
                    }
                }
            }
            if let Some((module, package)) = jpms_unexported_package(message)
                && let Some((descriptor_uri, descriptor_source)) = self.module_descriptor(module)
            {
                for directive in [format!("exports {package};"), format!("opens {package};")] {
                    if let Some(edit) = insert_module_directive(&descriptor_source, &directive) {
                        actions.push(json!({
                            "title": format!("Add {directive} to {module}"),
                            "kind": "quickfix",
                            "diagnostics": [diagnostic],
                            "edit": {"changes": {(descriptor_uri.clone()): [edit]}}
                        }));
                    }
                }
            }
            if message.contains("unnamed module") && message.contains("does not read") {
                actions.push(json!({
                    "title": "Move the dependency from the classpath to the module path",
                    "kind": "quickfix",
                    "diagnostics": [diagnostic],
                    "disabled": {
                        "reason": "The owning Maven or Gradle dependency declaration must be changed explicitly"
                    }
                }));
            }
        }
        let actions = self.defer_code_actions(uri, actions);
        Dispatch::Reply(success(id, json!(actions)))
    }

    fn defer_code_actions(&mut self, uri: &str, mut actions: Vec<Value>) -> Vec<Value> {
        if !self.code_action_resolve_properties.contains("edit") {
            return actions;
        }
        let version = self.documents.get(uri).map(|document| document.version);
        for action in &mut actions {
            let Some(object) = action.as_object_mut() else {
                continue;
            };
            let Some(edit) = object.remove("edit") else {
                continue;
            };
            let resolve_id = self.next_resolve_id();
            if self.code_action_resolutions.len() >= RESOLVE_CACHE_CAPACITY {
                self.code_action_resolutions.clear();
            }
            self.code_action_resolutions.insert(
                resolve_id,
                ResolveEntry {
                    uri: uri.to_owned(),
                    version,
                    properties: json!({"edit": edit}),
                },
            );
            object.insert(
                "data".to_owned(),
                json!({"jmanResolve": {"kind": "codeAction", "id": resolve_id}}),
            );
        }
        actions
    }

    fn resolve_code_action(&self, id: Value, params: Option<&Value>) -> Dispatch {
        self.resolve_item(id, params, "codeAction", &self.code_action_resolutions)
    }

    fn resolve_item(
        &self,
        id: Value,
        params: Option<&Value>,
        expected_kind: &str,
        resolutions: &HashMap<u64, ResolveEntry>,
    ) -> Dispatch {
        let Some(mut item) = params.cloned().filter(Value::is_object) else {
            return Dispatch::Reply(error(id, -32602, "Invalid resolve params"));
        };
        let Some(resolve) = item.pointer("/data/jmanResolve") else {
            return Dispatch::Reply(success(id, item));
        };
        if resolve["kind"].as_str() != Some(expected_kind) {
            return Dispatch::Reply(success(id, item));
        }
        let Some(resolve_id) = resolve["id"].as_u64() else {
            return Dispatch::Reply(success(id, item));
        };
        let Some(entry) = resolutions.get(&resolve_id) else {
            return Dispatch::Reply(success(id, item));
        };
        let stale = entry.version.is_some_and(|version| {
            self.documents
                .get(&entry.uri)
                .is_none_or(|document| document.version != version)
        });
        if let (Some(item), Some(properties)) = (item.as_object_mut(), entry.properties.as_object())
        {
            for (name, value) in properties {
                if stale && matches!(name.as_str(), "edit" | "additionalTextEdits") {
                    continue;
                }
                item.insert(name.clone(), value.clone());
            }
        }
        Dispatch::Reply(success(id, item))
    }

    fn next_resolve_id(&mut self) -> u64 {
        self.resolve_sequence = self.resolve_sequence.wrapping_add(1);
        self.resolve_sequence
    }

    fn owning_module_descriptor(&self, uri: &str) -> Option<(String, String)> {
        if uri.ends_with("/module-info.java") {
            return self
                .source_text(uri)
                .map(|source| (uri.to_owned(), source.to_owned()));
        }
        self.indexed_sources
            .iter()
            .filter(|(candidate, _)| candidate.ends_with("/module-info.java"))
            .filter_map(|(candidate, source)| {
                let root = candidate.strip_suffix("module-info.java")?;
                uri.starts_with(root)
                    .then_some((candidate.to_owned(), source.to_owned()))
            })
            .max_by_key(|(candidate, _)| candidate.len())
    }

    fn module_descriptor(&self, module: &str) -> Option<(String, String)> {
        self.indexed_sources.iter().find_map(|(uri, source)| {
            module_declaration_range(source, module).map(|_| (uri.to_owned(), source.to_owned()))
        })
    }

    fn workspace_symbols(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let query = params
            .and_then(|value| value["query"].as_str())
            .unwrap_or_default();
        let symbols: Vec<_> = self
            .project
            .workspace_symbols(query, 100)
            .into_iter()
            .chain(self.structural_project.workspace_symbols(query, 100))
            .filter_map(|location| {
                Some(json!({
                    "name": location.name,
                    "kind": lsp_symbol_kind(&location.kind),
                    "location": self.location(&location)?,
                    "containerName": location.qualified_name.rsplit_once('.').map(|(owner, _)| owner)
                }))
            })
            .collect();
        Dispatch::Reply(success(id, json!(symbols)))
    }

    fn document_symbols(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(uri) = params.and_then(|value| value["textDocument"]["uri"].as_str()) else {
            return Dispatch::Reply(error(id, -32602, "Invalid document-symbol params"));
        };
        let Some(source) = self.source_text(uri) else {
            return Dispatch::Reply(success(id, json!([])));
        };
        let mut seen = std::collections::HashSet::new();
        let mut symbols: Vec<_> = self
            .project
            .document_symbols(uri)
            .into_iter()
            .chain(self.structural_project.document_symbols(uri))
            .filter(|symbol| seen.insert((symbol.symbol_id.clone(), symbol.start, symbol.end)))
            .filter(|symbol| !matches!(symbol.kind.as_str(), "package" | "module"))
            .collect();
        symbols.sort_by_key(|symbol| (symbol.start, symbol.end));
        let owners: HashMap<_, _> = symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| {
                matches!(
                    symbol.kind.as_str(),
                    "class" | "interface" | "enum" | "record"
                )
            })
            .map(|(index, symbol)| (symbol.qualified_name.clone(), index))
            .collect();
        let mut children = vec![Vec::new(); symbols.len()];
        let mut roots = Vec::new();
        for (index, symbol) in symbols.iter().enumerate() {
            let owner = if matches!(
                symbol.kind.as_str(),
                "class" | "interface" | "enum" | "record"
            ) {
                symbol
                    .qualified_name
                    .rsplit_once('.')
                    .map(|(owner, _)| owner)
            } else {
                symbol
                    .qualified_name
                    .rsplit_once('#')
                    .or_else(|| symbol.qualified_name.rsplit_once('.'))
                    .map(|(owner, _)| owner)
            };
            if let Some(parent) = owner.and_then(|owner| owners.get(owner)).copied()
                && parent != index
            {
                children[parent].push(index);
            } else {
                roots.push(index);
            }
        }
        fn item(
            index: usize,
            symbols: &[javac_frontend::SymbolLocation],
            children: &[Vec<usize>],
            source: &str,
        ) -> Value {
            let symbol = &symbols[index];
            let range = json!({
                "start": offset_to_position(source, symbol.start),
                "end": offset_to_position(source, symbol.end)
            });
            let nested: Vec<_> = children[index]
                .iter()
                .map(|child| item(*child, symbols, children, source))
                .collect();
            json!({
                "name": symbol.name,
                "detail": symbol.qualified_name,
                "kind": lsp_symbol_kind(&symbol.kind),
                "range": range,
                "selectionRange": range,
                "children": nested
            })
        }
        let outline: Vec<_> = roots
            .into_iter()
            .map(|root| item(root, &symbols, &children, source))
            .collect();
        Dispatch::Reply(success(id, json!(outline)))
    }

    fn document_highlights(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid document-highlight params"));
        };
        let Some(symbol) = self.project.symbol_at(uri, offset) else {
            return Dispatch::Reply(success(id, json!([])));
        };
        let symbol_id = location_symbol_id(symbol);
        let mut locations = self.all_definitions(symbol_id);
        locations.extend(self.all_references(symbol_id));
        locations.retain(|location| {
            location.document == uri
                && matches!(
                    location.role.as_str(),
                    "declaration" | "reference" | "write"
                )
        });
        locations.sort_by_key(|location| (location.start, location.end));
        locations.dedup_by(|left, right| left.start == right.start && left.end == right.end);
        let highlights: Vec<_> = locations
            .into_iter()
            .filter_map(|location| {
                let target = self.location(&location)?;
                Some(json!({
                    "range": target["range"],
                    "kind": if matches!(location.role.as_str(), "declaration" | "write") {
                        3
                    } else {
                        2
                    }
                }))
            })
            .collect();
        Dispatch::Reply(success(id, json!(highlights)))
    }

    fn format_document(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(params) = params else {
            return Dispatch::Reply(error(id, -32602, "Invalid formatting params"));
        };
        let Some(uri) = params["textDocument"]["uri"].as_str() else {
            return Dispatch::Reply(error(id, -32602, "Formatting request has no document"));
        };
        let Some(source) = self.source_text(uri).map(str::to_owned) else {
            return Dispatch::Reply(success(id, json!([])));
        };
        let file_name = file_uri_to_path(uri)
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "Input.java".to_owned());
        let Some(backend) = self.backend.as_mut() else {
            return Dispatch::Reply(error(id, -32603, "Java formatting backend is unavailable"));
        };
        match backend.format(uri, &file_name, &source) {
            Err(message) => Dispatch::Reply(error(id, -32603, &message)),
            Ok(result) if !result.diagnostics.is_empty() => Dispatch::Reply(error(
                id,
                -32602,
                &format!(
                    "Cannot format invalid Java source: {}",
                    result.diagnostics[0].message
                ),
            )),
            Ok(result) if result.source == source => Dispatch::Reply(success(id, json!([]))),
            Ok(result) => Dispatch::Reply(success(
                id,
                json!([{
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": offset_to_position(&source, source.len() as u64)
                    },
                    "newText": result.source
                }]),
            )),
        }
    }

    fn inlay_hints(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(params) = params else {
            return Dispatch::Reply(error(id, -32602, "Invalid inlay-hint params"));
        };
        let Some(uri) = params["textDocument"]["uri"].as_str() else {
            return Dispatch::Reply(error(id, -32602, "Inlay-hint request has no document"));
        };
        let Some(source) = self.source_text(uri) else {
            return Dispatch::Reply(success(id, json!([])));
        };
        let start_line = params["range"]["start"]["line"].as_u64().unwrap_or(0);
        let end_line = params["range"]["end"]["line"].as_u64().unwrap_or(u64::MAX);
        let mut edges = self.project.document_call_edges(uri);
        edges.extend(self.structural_project.document_call_edges(uri));
        edges.sort_by_key(|edge| (edge.start, edge.end));
        edges.dedup_by(|left, right| {
            left.symbol_id == right.symbol_id && left.start == right.start && left.end == right.end
        });
        let mut hints = Vec::new();
        for edge in edges {
            let call_position = offset_to_position(source, edge.start);
            let line = call_position["line"].as_u64().unwrap_or(0);
            if line < start_line || line > end_line {
                continue;
            }
            let Some(definition) = self.all_definitions(&edge.symbol_id).into_iter().next() else {
                continue;
            };
            let Some(definition_source) = self.source_text(&definition.document) else {
                continue;
            };
            let definition_byte = utf16_offset_to_byte(definition_source, definition.end);
            let Some((_, _, parameters)) =
                parenthesized_parts_after(definition_source, definition_byte)
            else {
                continue;
            };
            let call_byte = utf16_offset_to_byte(source, edge.end);
            let Some((_, _, arguments)) = parenthesized_parts_after(source, call_byte) else {
                continue;
            };
            for ((_, parameter), (argument_start, argument)) in parameters.iter().zip(arguments) {
                let Some(name) = parameter_name(parameter) else {
                    continue;
                };
                if argument.trim() == name {
                    continue;
                }
                let argument_start =
                    argument_start + argument.len().saturating_sub(argument.trim_start().len());
                let offset = source[..argument_start].encode_utf16().count() as u64;
                hints.push(json!({
                    "position": offset_to_position(source, offset),
                    "label": format!("{name}:"),
                    "kind": 2,
                    "paddingRight": true
                }));
            }
        }
        Dispatch::Reply(success(id, json!(hints)))
    }

    fn selection_ranges(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(params) = params else {
            return Dispatch::Reply(error(id, -32602, "Invalid selection-range params"));
        };
        let Some(uri) = params["textDocument"]["uri"].as_str() else {
            return Dispatch::Reply(error(id, -32602, "Selection-range request has no document"));
        };
        let Some(source) = self.source_text(uri) else {
            return Dispatch::Reply(success(id, json!([])));
        };
        let Some(positions) = params["positions"].as_array() else {
            return Dispatch::Reply(error(
                id,
                -32602,
                "Selection-range request has no positions",
            ));
        };
        let ranges: Vec<_> = positions
            .iter()
            .filter_map(|position| crate::position_to_byte(source, position).ok())
            .map(|byte| semantic_selection_range(source, byte))
            .collect();
        Dispatch::Reply(success(id, json!(ranges)))
    }

    fn folding_ranges(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(uri) = params.and_then(|value| value["textDocument"]["uri"].as_str()) else {
            return Dispatch::Reply(error(id, -32602, "Invalid folding-range params"));
        };
        let ranges = self
            .source_text(uri)
            .map(java_folding_ranges)
            .unwrap_or_default();
        Dispatch::Reply(success(id, json!(ranges)))
    }

    fn document_links(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(uri) = params.and_then(|value| value["textDocument"]["uri"].as_str()) else {
            return Dispatch::Reply(error(id, -32602, "Invalid document-link params"));
        };
        let Some(source) = self.source_text(uri) else {
            return Dispatch::Reply(success(id, json!([])));
        };
        let links: Vec<_> = javadoc_references(source)
            .into_iter()
            .filter_map(|reference| {
                let simple = reference
                    .target
                    .split('#')
                    .next()
                    .unwrap_or(&reference.target)
                    .rsplit('.')
                    .next()
                    .unwrap_or(&reference.target);
                let symbol = self
                    .project
                    .completion_symbols(simple, 100)
                    .into_iter()
                    .chain(self.structural_project.completion_symbols(simple, 200))
                    .find(|symbol| {
                        symbol.name == simple
                            && (reference.target.starts_with(&symbol.qualified_name)
                                || symbol.qualified_name.ends_with(&format!(".{simple}")))
                    })?;
                let target = self.location(&symbol)?;
                Some(json!({
                    "range": {
                        "start": offset_to_position(source, reference.start),
                        "end": offset_to_position(source, reference.end)
                    },
                    "target": target["uri"],
                    "tooltip": symbol.qualified_name
                }))
            })
            .collect();
        Dispatch::Reply(success(id, json!(links)))
    }

    fn document_diagnostic(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(params) = params else {
            return Dispatch::Reply(error(id, -32602, "Invalid document-diagnostic params"));
        };
        let Some(uri) = params["textDocument"]["uri"].as_str() else {
            return Dispatch::Reply(error(id, -32602, "Diagnostic request has no document"));
        };
        let result_id = self.diagnostic_result_id(uri);
        if params["previousResultId"].as_str() == Some(&result_id) {
            return work_done_response(
                id,
                params.get("workDoneToken"),
                "Java diagnostics",
                json!({"kind":"unchanged", "resultId":result_id}),
            );
        }
        let items = self.diagnostics_for(uri);
        work_done_response(
            id,
            params.get("workDoneToken"),
            "Java diagnostics",
            json!({"kind":"full", "resultId":result_id, "items":items}),
        )
    }

    fn workspace_diagnostic(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let previous: HashMap<_, _> = params
            .and_then(|value| value["previousResultIds"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|item| Some((item["uri"].as_str()?, item["value"].as_str()?)))
            .collect();
        let mut uris: Vec<_> = self
            .indexed_sources
            .keys()
            .chain(self.analyses.keys())
            .cloned()
            .collect();
        uris.sort();
        uris.dedup();
        let items: Vec<_> = uris
            .into_iter()
            .map(|uri| {
                let result_id = self.diagnostic_result_id(&uri);
                let version = self.documents.get(&uri).map(|document| document.version);
                if previous.get(uri.as_str()).copied() == Some(result_id.as_str()) {
                    json!({
                        "uri":uri, "version":version, "kind":"unchanged",
                        "resultId":result_id
                    })
                } else {
                    json!({
                        "uri":uri, "version":version, "kind":"full",
                        "resultId":result_id, "items":self.diagnostics_for(&uri)
                    })
                }
            })
            .collect();
        if let Some(token) = params.and_then(|value| value.get("partialResultToken")) {
            let mut messages = vec![progress_notification(token, json!({"items":items}))];
            messages.push(success(id, json!({"items":[]})));
            return Dispatch::Batch(messages);
        }
        work_done_response(
            id,
            params.and_then(|value| value.get("workDoneToken")),
            "Workspace Java diagnostics",
            json!({"items":items}),
        )
    }

    fn diagnostic_result_id(&self, uri: &str) -> String {
        let version = self
            .documents
            .get(uri)
            .map(|document| document.version.to_string())
            .unwrap_or_else(|| "closed".to_owned());
        format!("{}:{version}", self.structural_revision)
    }

    fn diagnostics_for(&self, uri: &str) -> Vec<Value> {
        let source = self.source_text(uri).unwrap_or_default();
        self.analyses
            .get(uri)
            .map(|analysis| lsp_diagnostics(&analysis.diagnostics, source))
            .unwrap_or_default()
    }

    fn test_items(&self, uri_filter: Option<&str>) -> Vec<Value> {
        let mut items = Vec::new();
        let mut parents = HashSet::new();
        let mut documents: Vec<_> = self
            .indexed_sources
            .keys()
            .chain(self.analyses.keys())
            .cloned()
            .collect();
        documents.sort();
        documents.dedup();
        for uri in documents {
            if uri_filter.is_some_and(|filter| filter != uri) {
                continue;
            }
            let Some(source) = self.source_text(&uri) else {
                continue;
            };
            let mut symbols = self.project.document_symbols(&uri);
            symbols.extend(self.structural_project.document_symbols(&uri));
            symbols.sort_by_key(|symbol| (symbol.start, symbol.end));
            symbols.dedup_by(|left, right| {
                left.symbol_id == right.symbol_id
                    && left.start == right.start
                    && left.end == right.end
            });
            let type_symbols: Vec<_> = symbols
                .iter()
                .filter(|symbol| {
                    matches!(
                        symbol.kind.as_str(),
                        "class" | "interface" | "enum" | "record"
                    )
                })
                .cloned()
                .collect();
            for symbol in symbols.into_iter().filter(|symbol| {
                matches!(symbol.kind.as_str(), "method" | "constructor")
                    && has_junit_annotation(source, symbol.start)
            }) {
                let declared_owner = symbol
                    .qualified_name
                    .rsplit_once('#')
                    .or_else(|| symbol.qualified_name.rsplit_once('.'))
                    .map(|(owner, _)| owner);
                let owner = declared_owner
                    .and_then(|owner| {
                        type_symbols
                            .iter()
                            .find(|candidate| candidate.qualified_name == owner)
                    })
                    .or_else(|| {
                        type_symbols
                            .iter()
                            .filter(|candidate| candidate.start <= symbol.start)
                            .max_by_key(|candidate| candidate.start)
                    });
                let Some(owner) = owner else {
                    continue;
                };
                let owner_name = owner.qualified_name.as_str();
                let parent_id = format!("class:{uri}:{owner_name}");
                if parents.insert(parent_id.clone()) {
                    items.push(json!({
                        "id": parent_id,
                        "label": owner_name.rsplit(['.', '$']).next().unwrap_or(owner_name),
                        "kind": "class",
                        "uri": uri,
                        "range": {
                            "start": offset_to_position(source, owner.start),
                            "end": offset_to_position(source, owner.end)
                        },
                        "selector": owner_name
                    }));
                }
                let selector = format!("{owner_name}#{}", symbol.name);
                items.push(json!({
                    "id": format!("test:{uri}:{selector}"),
                    "parentId": parent_id,
                    "label": symbol.name,
                    "kind": "test",
                    "uri": uri,
                    "range": {
                        "start": offset_to_position(source, symbol.start),
                        "end": offset_to_position(source, symbol.end)
                    },
                    "selector": selector,
                    "tags": junit_tags(source, symbol.start)
                }));
            }
        }
        items
    }

    fn source_metadata(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(uri) = params.and_then(|value| value["textDocument"]["uri"].as_str()) else {
            return Dispatch::Reply(error(id, -32602, "Source metadata has no document"));
        };
        let metadata = self
            .backend
            .as_ref()
            .map(|backend| backend.source_metadata(uri))
            .unwrap_or_default();
        Dispatch::Reply(success(
            id,
            json!({
                "protocolVersion":1,
                "uri":uri,
                "generated":metadata.generated,
                "readOnly":metadata.read_only,
                "origin":metadata.origin
            }),
        ))
    }

    fn discover_tests(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let uri = params.and_then(|value| value["textDocument"]["uri"].as_str());
        Dispatch::Reply(success(
            id,
            json!({"protocolVersion": 2, "items": self.test_items(uri)}),
        ))
    }

    fn prepare_test_run(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let uri = params.and_then(|value| value["uri"].as_str());
        let coverage_requested = params
            .and_then(|value| value["coverage"].as_bool())
            .unwrap_or(false);
        let selectors: Vec<_> = params
            .and_then(|value| value["selectors"].as_array())
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        if selectors.is_empty() {
            return Dispatch::Reply(error(id, -32602, "At least one test selector is required"));
        }
        let build_system = self
            .backend
            .as_ref()
            .and_then(|backend| {
                uri.and_then(|uri| backend.build_system_for(uri))
                    .or_else(|| backend.build_system())
            })
            .unwrap_or_else(|| "jman".to_owned());
        let build_java_home = self
            .backend
            .as_ref()
            .and_then(|backend| backend.build_java_home_for(uri));
        let (program, mut args, report) = match build_system.as_str() {
            "gradle" => {
                let mut args = vec!["test".to_owned()];
                for selector in &selectors {
                    args.push("--tests".to_owned());
                    args.push(selector.replace('#', "."));
                }
                ("gradle", args, "junit-xml")
            }
            "maven" => (
                "maven",
                vec![
                    "test".to_owned(),
                    format!("-Dtest={}", selectors.join(",")),
                    "-Dsurefire.failIfNoSpecifiedTests=false".to_owned(),
                ],
                "junit-xml",
            ),
            _ => {
                let mut args = vec![
                    "--no-progress".to_owned(),
                    "test".to_owned(),
                    "--report".to_owned(),
                    "json".to_owned(),
                ];
                for selector in &selectors {
                    args.push("--tests".to_owned());
                    args.push(selector.clone());
                }
                ("jman", args, "json-lines")
            }
        };
        if coverage_requested && build_system == "jman" {
            args.push("--coverage".to_owned());
        }
        let mut debug_args = args.clone();
        if build_system == "jman" {
            debug_args.insert(2, "--debug".to_owned());
        }
        Dispatch::Reply(success(
            id,
            json!({
                "protocolVersion": 2,
                "runId": format!("jman-test-{}", std::process::id()),
                "buildSystem": build_system,
                "buildJavaHome": build_java_home,
                "program": program,
                "arguments": args,
                "report": report,
                "selectors": selectors,
                "rerun": {
                    "program": program,
                    "arguments": args
                },
                "debug": {
                    "supported": build_system == "jman",
                    "program": program,
                    "arguments": debug_args,
                    "transport": "jdwp",
                    "port": 0,
                    "suspend": true,
                    "reason": if build_system == "jman" {
                        Value::Null
                    } else {
                        json!("Debug descriptors are currently available for native JMAN tests")
                    }
                },
                "coverage": {
                    "supported": build_system == "jman",
                    "engine": if build_system == "jman" { json!("jacoco") } else { Value::Null },
                    "protocolVersion": if build_system == "jman" { json!(1) } else { Value::Null },
                    "reason": if build_system == "jman" {
                        Value::Null
                    } else {
                        json!("Coverage descriptors are currently available for native JMAN tests")
                    }
                }
            }),
        ))
    }

    fn test_code_lenses(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(uri) = params.and_then(|value| value["textDocument"]["uri"].as_str()) else {
            return Dispatch::Reply(error(id, -32602, "Invalid code-lens params"));
        };
        let lenses: Vec<_> = self
            .test_items(Some(uri))
            .into_iter()
            .filter_map(|item| {
                let title = match item["kind"].as_str()? {
                    "class" => "Run Test Class",
                    "test" => "Run Test",
                    _ => return None,
                };
                Some(json!({
                    "range": item.get("range")?,
                    "command": {
                        "title": title,
                        "command": "jman.java.test",
                        "arguments": [item.get("selector")?]
                    }
                }))
            })
            .collect();
        Dispatch::Reply(success(id, json!(lenses)))
    }

    fn execute_command(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some(command) = params.and_then(|params| params["command"].as_str()) else {
            return Dispatch::Reply(error(id, -32602, "Missing command"));
        };
        if command == "jman.java.syncWorkspace" {
            return match self.synchronize_project() {
                Dispatch::Batch(mut messages) => {
                    let result = self.status_result();
                    messages.push(success(id, result));
                    Dispatch::Batch(messages)
                }
                Dispatch::Reply(notification) => {
                    if notification.get("id").is_some() {
                        Dispatch::Reply(notification)
                    } else {
                        Dispatch::Batch(vec![notification, success(id, self.status_result())])
                    }
                }
                _ => Dispatch::Reply(success(id, self.status_result())),
            };
        }
        if matches!(
            command,
            "jman.java.rebuildIndex" | "jman.java.clearWorkspaceCache"
        ) {
            let result = match self.backend.as_mut() {
                Some(backend) if command == "jman.java.clearWorkspaceCache" => backend
                    .clear_project_cache()
                    .and_then(|()| backend.rebuild_index()),
                Some(backend) => backend.rebuild_index(),
                None => Ok(()),
            };
            if let Err(message) = result {
                return Dispatch::Reply(error(id, -32603, &message));
            }
            self.project = ProjectState::default();
            self.structural_project = ProjectState::default();
            self.analyses.clear();
            self.indexed_sources.clear();
            self.index_workspace();
        } else if command != "jman.java.status" {
            return Dispatch::Reply(error(id, -32602, "Unknown command"));
        }
        let uri = params
            .and_then(|params| params["arguments"].as_array())
            .and_then(|arguments| arguments.first())
            .and_then(Value::as_str);
        Dispatch::Reply(success(id, self.status_result_for(uri)))
    }

    fn status_result(&self) -> Value {
        self.status_result_for(None)
    }

    fn status_result_for(&self, uri: Option<&str>) -> Value {
        let cache = self
            .backend
            .as_ref()
            .map(|backend| backend.cache_status())
            .unwrap_or_default();
        let (sync_state, sync_error) = match &self.build_sync_state {
            BuildSyncState::Ready => ("ready", None),
            BuildSyncState::Required => ("required", None),
            BuildSyncState::Syncing => ("syncing", None),
            BuildSyncState::Failed(message) => ("failed", Some(message.as_str())),
        };
        let build_system = self.backend.as_ref().and_then(|backend| {
            uri.map_or_else(
                || backend.build_system(),
                |uri| backend.build_system_for(uri),
            )
        });
        let native_operations = build_system.as_deref() == Some("jman");
        let build_runtime = cache.build_java_major.map(|major| {
            json!({
                "buildToolVersion": cache.build_tool_version.as_deref(),
                "javaHome": cache.build_java_home.as_deref(),
                "javaVersion": cache.build_java_version.as_deref(),
                "javaMajor": major,
                "source": cache.build_java_source.as_deref()
            })
        });
        json!({
            "protocolVersion": 1,
            "state": "ready",
            "workspace": {
                "buildSystem": build_system,
                "nativeOperations": native_operations,
                "buildRuntime": build_runtime
            },
            "buildSync": {
                "state": sync_state,
                "pendingChanges": self.pending_build_changes.len(),
                "error": sync_error
            },
            "openDocuments": self.documents.uris().count(),
            "indexedDocuments": self.indexed_sources.len(),
            "semanticDocuments": self.analyses.len(),
            "structuralRevision": self.structural_revision,
            "externalSources": self.external_origins.len(),
            "workspaceIndexPending": self.backend
                .as_ref()
                .is_some_and(|backend| backend.workspace_index_pending()),
            "cache": {
                "projectId": cache.project_id,
                "directory": cache.directory,
                "bytes": cache.bytes,
                "indexingMilliseconds": cache.indexing_milliseconds,
                "structural": {
                    "entries": cache.structural_entries,
                    "hits": cache.structural_hits,
                    "misses": cache.structural_misses
                },
                "semantic": {
                    "entries": cache.semantic_entries,
                    "hits": cache.semantic_hits,
                    "misses": cache.semantic_misses
                },
                "externalEntries": cache.external_entries
            }
        })
    }

    fn build_sync_notification(&self) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "jman.java/buildSyncStatus",
            "params": self.status_result()["buildSync"]
        })
    }

    fn synchronize_project(&mut self) -> Dispatch {
        self.build_sync_state = BuildSyncState::Syncing;
        let changed: Vec<_> = self.pending_build_changes.iter().cloned().collect();
        let reloaded = match self.backend.as_mut() {
            Some(backend) => backend.reload(&changed),
            None => Ok(false),
        };
        match reloaded {
            Err(message) => {
                self.build_sync_state = BuildSyncState::Failed(message.clone());
                Dispatch::Batch(vec![
                    self.build_sync_notification(),
                    json!({
                        "jsonrpc":"2.0","method":"window/logMessage",
                        "params":{"type":1,"message":message}
                    }),
                ])
            }
            Ok(reloaded) => {
                self.pending_build_changes.clear();
                self.build_sync_state = BuildSyncState::Ready;
                let mut messages = vec![self.build_sync_notification()];
                if reloaded {
                    self.project = ProjectState::default();
                    self.structural_project = ProjectState::default();
                    self.analyses.clear();
                    self.indexed_sources.clear();
                    self.index_workspace();
                    let uris: Vec<_> = self.documents.uris().map(str::to_owned).collect();
                    messages.extend(uris.iter().filter_map(
                        |uri| match self.analyze_document(uri) {
                            Dispatch::Reply(message) => Some(message),
                            _ => None,
                        },
                    ));
                }
                Dispatch::Batch(messages)
            }
        }
    }

    fn prepare_rename(&self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid prepareRename params"));
        };
        let Some(symbol) = self.project.symbol_at(uri, offset) else {
            return Dispatch::Reply(success(id, Value::Null));
        };
        if self
            .related_symbol_ids(location_symbol_id(symbol))
            .iter()
            .flat_map(|symbol_id| self.all_definitions(symbol_id))
            .next()
            .is_none()
        {
            return Dispatch::Reply(success(id, Value::Null));
        }
        let Some(location) = self.location(symbol) else {
            return Dispatch::Reply(success(id, Value::Null));
        };
        Dispatch::Reply(success(
            id,
            json!({
                "range": location["range"],
                "placeholder": symbol.name
            }),
        ))
    }

    fn rename(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid rename params"));
        };
        let Some(new_name) = params.and_then(|value| value["newName"].as_str()) else {
            return Dispatch::Reply(error(id, -32602, "Rename has no newName"));
        };
        let symbol_name = self
            .project
            .symbol_at(uri, offset)
            .map(|symbol| symbol.name.clone());
        let Some(symbol_name) = symbol_name else {
            return Dispatch::Reply(error(id, -32803, "No symbol at rename position"));
        };
        self.ensure_workspace_symbol_semantics(&symbol_name);
        let Some(symbol) = self.project.symbol_at(uri, offset) else {
            return Dispatch::Reply(error(id, -32803, "No symbol at rename position"));
        };
        if !javac_frontend::is_java_identifier(new_name) {
            return Dispatch::Reply(error(id, -32602, "Invalid Java identifier"));
        }
        let symbol_id = location_symbol_id(symbol);
        let related = self.related_symbol_ids(symbol_id);
        let definitions: Vec<_> = related
            .iter()
            .flat_map(|related_id| self.all_definitions(related_id))
            .collect();
        if definitions.is_empty() {
            return Dispatch::Reply(error(id, -32803, "Symbol definition is not indexed"));
        }
        if self
            .project
            .declarations_named(new_name)
            .into_iter()
            .chain(self.structural_project.declarations_named(new_name))
            .any(|candidate| {
                candidate.document == definitions[0].document
                    && candidate.symbol_id != symbol_id
                    && candidate.kind == symbol.kind
            })
        {
            return Dispatch::Reply(error(
                id,
                -32803,
                "Rename would conflict with an existing declaration",
            ));
        }
        let mut edits = definitions;
        edits.extend(
            related
                .iter()
                .flat_map(|related_id| self.all_references(related_id)),
        );
        edits.sort_by(|left, right| {
            left.document
                .cmp(&right.document)
                .then_with(|| left.start.cmp(&right.start))
                .then_with(|| left.end.cmp(&right.end))
        });
        edits.dedup_by(|left, right| {
            left.document == right.document && left.start == right.start && left.end == right.end
        });
        let mut changes = serde_json::Map::new();
        for edit in edits {
            let Some(source) = self.source_text(&edit.document) else {
                continue;
            };
            changes
                .entry(edit.document)
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .unwrap()
                .push(json!({
                    "range": {
                        "start": offset_to_position(source, edit.start),
                        "end": offset_to_position(source, edit.end)
                    },
                    "newText": new_name
                }));
        }
        Dispatch::Reply(success(id, json!({"changes": changes})))
    }

    fn ensure_workspace_symbol_semantics(&mut self, name: &str) {
        let mut documents: Vec<_> = self
            .indexed_sources
            .iter()
            .filter(|(_, source)| contains_java_identifier(source, name))
            .map(|(document, _)| document.clone())
            .collect();
        documents.sort();
        documents.dedup();
        for document in documents {
            if self.project.contains_document(&document) {
                continue;
            }
            let Some(source) = self.indexed_sources.get(&document).cloned() else {
                continue;
            };
            self.update_index(&document, &source);
        }
    }

    fn change_signature(&mut self, id: Value, params: Option<&Value>) -> Dispatch {
        let Some((uri, offset)) = self.request_position(params) else {
            return Dispatch::Reply(error(id, -32602, "Invalid change-signature position"));
        };
        let symbol_name = self
            .project
            .symbol_at(uri, offset)
            .map(|symbol| symbol.name.clone());
        let Some(symbol_name) = symbol_name else {
            return Dispatch::Reply(error(id, -32803, "No method at change-signature position"));
        };
        self.ensure_workspace_symbol_semantics(&symbol_name);
        let Some(symbol) = self.project.symbol_at(uri, offset) else {
            return Dispatch::Reply(error(id, -32803, "No method at change-signature position"));
        };
        if symbol.kind != "method" {
            return Dispatch::Reply(error(id, -32803, "Change signature requires a method"));
        }
        let Some(new_parameters) = params
            .and_then(|value| value["newParameters"].as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
        else {
            return Dispatch::Reply(error(id, -32602, "newParameters must be a string array"));
        };
        let Some(order) = params
            .and_then(|value| value["argumentOrder"].as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_u64)
                    .map(|index| index as usize)
                    .collect::<Vec<_>>()
            })
        else {
            return Dispatch::Reply(error(id, -32602, "argumentOrder must be an index array"));
        };
        let related = self.related_symbol_ids(location_symbol_id(symbol));
        let mut locations: Vec<_> = related
            .iter()
            .flat_map(|symbol_id| {
                self.all_definitions(symbol_id)
                    .into_iter()
                    .chain(self.all_references(symbol_id))
            })
            .collect();
        locations.sort_by(|left, right| {
            left.document
                .cmp(&right.document)
                .then_with(|| left.start.cmp(&right.start))
        });
        locations
            .dedup_by(|left, right| left.document == right.document && left.start == right.start);
        let mut changes = serde_json::Map::new();
        for location in locations {
            let Some(source) = self.source_text(&location.document) else {
                continue;
            };
            let name = utf16_offset_to_byte(source, location.start);
            let Some((open, close, arguments)) =
                parenthesized_after(source, name + location.name.len())
            else {
                return Dispatch::Reply(error(
                    id,
                    -32803,
                    "Method references prevent this change-signature rewrite",
                ));
            };
            let replacement = if location.role == "declaration" {
                if order.len() != arguments.len()
                    || new_parameters.len() != order.len()
                    || order.iter().any(|index| *index >= arguments.len())
                    || new_parameters
                        .iter()
                        .zip(&order)
                        .any(|(new, old)| parameter_type(new) != parameter_type(arguments[*old]))
                {
                    return Dispatch::Reply(error(
                        id,
                        -32803,
                        "Change signature currently permits only safe parameter renames and reordering",
                    ));
                }
                new_parameters.join(", ")
            } else {
                if order.len() != arguments.len()
                    || order.iter().any(|index| *index >= arguments.len())
                {
                    return Dispatch::Reply(error(
                        id,
                        -32602,
                        "argumentOrder does not match an existing call",
                    ));
                }
                order
                    .iter()
                    .map(|index| arguments[*index].trim())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            changes
                .entry(location.document)
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .unwrap()
                .push(json!({
                    "range": {
                        "start": offset_to_position(
                            source,
                            source[..open + 1].encode_utf16().count() as u64
                        ),
                        "end": offset_to_position(
                            source,
                            source[..close].encode_utf16().count() as u64
                        )
                    },
                    "newText": replacement
                }));
        }
        Dispatch::Reply(success(id, json!({"changes": changes})))
    }

    fn request_position<'a>(&self, params: Option<&'a Value>) -> Option<(&'a str, u64)> {
        let params = params?;
        let uri = params["textDocument"]["uri"].as_str()?;
        let document = self.documents.get(uri)?;
        let byte = crate::position_to_byte(&document.text, &params["position"]).ok()?;
        let offset = document.text[..byte].encode_utf16().count() as u64;
        Some((uri, offset))
    }

    fn location(&self, location: &javac_frontend::SymbolLocation) -> Option<Value> {
        let source = self.source_text(&location.document)?;
        Some(json!({
            "uri": location.document,
            "range": {
                "start": offset_to_position(source, location.start),
                "end": offset_to_position(source, location.end)
            }
        }))
    }

    fn source_text(&self, uri: &str) -> Option<&str> {
        self.documents
            .get(uri)
            .map(|document| document.text.as_str())
            .or_else(|| self.indexed_sources.get(uri).map(String::as_str))
    }

    fn all_definitions(&self, symbol_id: &str) -> Vec<javac_frontend::SymbolLocation> {
        merge_locations(
            self.project.definitions(symbol_id),
            self.structural_project.definitions(symbol_id),
        )
    }

    fn workspace_editor_definition_locations(
        &self,
        definition: &javac_frontend::EditorDefinition,
    ) -> Vec<Value> {
        let mut candidates = vec![definition.symbol_id.clone()];
        if definition.name != "<init>" {
            candidates.push(format!("{}#{}", definition.owner, definition.name));
            candidates.push(format!("{}.{}", definition.owner, definition.name));
        }
        if definition.decompiled
            || definition.name == "<init>"
            || definition.owner.rsplit('.').next() == Some(definition.name.as_str())
        {
            candidates.push(definition.owner.clone());
        }
        for candidate in candidates {
            let locations: Vec<_> = self
                .all_definitions(&candidate)
                .iter()
                .filter_map(|location| self.location(location))
                .collect();
            if !locations.is_empty() {
                return locations;
            }
        }
        self.pending_local_source_location(definition)
            .into_iter()
            .collect()
    }

    fn pending_local_source_location(
        &self,
        definition: &javac_frontend::EditorDefinition,
    ) -> Option<Value> {
        if !self
            .backend
            .as_ref()
            .is_some_and(|backend| backend.workspace_index_pending())
        {
            return None;
        }
        let file_name = PathBuf::from(&definition.source_name)
            .file_name()?
            .to_owned();
        let owner = definition.owner.replace('$', ".");
        for root in &self.workspace_roots {
            let mut pending = vec![root.clone()];
            while let Some(directory) = pending.pop() {
                let Ok(entries) = std::fs::read_dir(&directory) else {
                    continue;
                };
                let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
                entries.sort_by_key(std::fs::DirEntry::file_name);
                for entry in entries.into_iter().rev() {
                    let path = entry.path();
                    let Ok(file_type) = entry.file_type() else {
                        continue;
                    };
                    if file_type.is_dir() {
                        if !excluded_local_source_directory(&entry.file_name()) {
                            pending.push(path);
                        }
                        continue;
                    }
                    if !file_type.is_file() || entry.file_name() != file_name {
                        continue;
                    }
                    let Ok(source) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    let package = java_import_context(&source).0.unwrap_or_default();
                    let Some(type_name) = path
                        .file_stem()
                        .map(|name| name.to_string_lossy().into_owned())
                    else {
                        continue;
                    };
                    let qualified = if package.is_empty() {
                        type_name.clone()
                    } else {
                        format!("{package}.{type_name}")
                    };
                    if owner != qualified && !owner.starts_with(&format!("{qualified}.")) {
                        continue;
                    }
                    let Some((start, end)) = java_type_declaration_range(&source, &type_name)
                    else {
                        continue;
                    };
                    return Some(json!({
                        "uri": format!("file://{}", path.display()),
                        "range": {
                            "start": offset_to_position(&source, start),
                            "end": offset_to_position(&source, end)
                        }
                    }));
                }
            }
        }
        None
    }

    fn related_symbol_ids(&self, symbol_id: &str) -> Vec<String> {
        let mut related = self.project.related_symbol_ids(symbol_id);
        related.extend(self.structural_project.related_symbol_ids(symbol_id));
        if symbol_id.contains('#') && !symbol_id.contains('|') {
            let method_name = symbol_id
                .rsplit_once('#')
                .map(|(_, name)| name)
                .unwrap_or("");
            for candidate in self.project.declarations_named(method_name) {
                let candidate_id = location_symbol_id(&candidate);
                let Some(family) = self.project.override_family(candidate_id) else {
                    continue;
                };
                if structural_method_identity(family).as_deref() != Some(symbol_id) {
                    continue;
                }
                related.extend(self.project.related_symbol_ids(candidate_id));
                related.push(family.to_owned());
            }
        }
        related.sort();
        related.dedup();
        if related.is_empty() {
            related.push(symbol_id.to_owned());
        }
        related
    }

    fn implement_method_action(
        &self,
        uri: &str,
        source: &str,
        diagnostic: &Value,
        method: &str,
    ) -> Option<Value> {
        let declaration = self
            .project
            .completion_symbols(method, 200)
            .into_iter()
            .chain(self.structural_project.completion_symbols(method, 200))
            .find(|symbol| {
                symbol.role == "declaration"
                    && symbol.kind == "method"
                    && symbol.name == method
                    && symbol.document != uri
            })?;
        let declaration_source = self.source_text(&declaration.document)?;
        let name_byte = utf16_offset_to_byte(declaration_source, declaration.start);
        let line_start = declaration_source[..name_byte]
            .rfind(['\n', '{', '}'])
            .map_or(0, |index| index + 1);
        let semicolon = declaration_source[name_byte..].find(';')? + name_byte;
        let mut signature = declaration_source[line_start..semicolon].trim().to_owned();
        signature = signature
            .trim_start_matches("abstract ")
            .trim_start_matches("default ")
            .to_owned();
        if !["public ", "protected "]
            .iter()
            .any(|modifier| signature.starts_with(modifier))
        {
            signature = format!("public {signature}");
        }
        let body_end = source.rfind('}')?;
        let insertion =
            offset_to_position(source, source[..body_end].encode_utf16().count() as u64);
        Some(json!({
            "title": format!("Implement method {method}"),
            "kind": "quickfix",
            "diagnostics": [diagnostic],
            "isPreferred": true,
            "edit": {"changes": {(uri): [{
                "range": {"start": insertion, "end": insertion},
                "newText": format!(
                    "\n    @Override\n    {signature} {{\n        throw new UnsupportedOperationException(\"TODO: implement\");\n    }}\n"
                )
            }]}}
        }))
    }

    fn all_references(&self, symbol_id: &str) -> Vec<javac_frontend::SymbolLocation> {
        merge_locations(
            self.project.references(symbol_id),
            self.structural_project.references(symbol_id),
        )
    }
}

fn merge_locations(
    primary: &[javac_frontend::SymbolLocation],
    secondary: &[javac_frontend::SymbolLocation],
) -> Vec<javac_frontend::SymbolLocation> {
    let mut locations: Vec<_> = primary.iter().chain(secondary).cloned().collect();
    locations.sort_by(|left, right| {
        left.document
            .cmp(&right.document)
            .then_with(|| left.start.cmp(&right.start))
            .then_with(|| left.end.cmp(&right.end))
    });
    locations.dedup_by(|left, right| {
        left.document == right.document && left.start == right.start && left.end == right.end
    });
    locations
}

fn persist_external_source(
    definition: &javac_frontend::EditorDefinition,
) -> Result<PathBuf, String> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    definition.owner.hash(&mut hasher);
    definition.source.hash(&mut hasher);
    let hash = hasher.finish();
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("io.github.zonnedev.jman.lsp/external-sources")
        .join(format!("{hash:016x}"));
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("cannot create external-source cache: {error}"))?;
    let _lock = crate::cache::FileLock::acquire(&root.join("external-source-cache"))?;
    let file_name = PathBuf::from(&definition.source_name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("External.java")
        .to_owned();
    let path = root.join(file_name);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(&definition.source) {
        let temporary = path.with_extension("java.tmp");
        std::fs::write(&temporary, &definition.source)
            .and_then(|_| std::fs::rename(&temporary, &path))
            .map_err(|error| format!("cannot publish external source: {error}"))?;
    }
    Ok(path)
}

fn external_origin_path(source: &std::path::Path) -> PathBuf {
    let mut name = source.as_os_str().to_owned();
    name.push(".origin");
    PathBuf::from(name)
}

fn persist_external_origin(source: &std::path::Path, origin: &str) -> Result<(), String> {
    let path = external_origin_path(source);
    if std::fs::read_to_string(&path).ok().as_deref() == Some(origin) {
        return Ok(());
    }
    let temporary = path.with_extension("origin.tmp");
    std::fs::write(&temporary, origin)
        .and_then(|_| std::fs::rename(&temporary, &path))
        .map_err(|error| format!("cannot persist external-source origin: {error}"))
}

fn identifier_range(source: &str, cursor: usize) -> Value {
    let cursor = cursor.min(source.len());
    let mut start = cursor;
    while let Some((offset, character)) = source[..start].char_indices().next_back() {
        if !(character.is_alphanumeric() || matches!(character, '_' | '$')) {
            break;
        }
        start = offset;
    }
    let mut end = cursor;
    while let Some(character) = source[end..].chars().next() {
        if !(character.is_alphanumeric() || matches!(character, '_' | '$')) {
            break;
        }
        end += character.len_utf8();
    }
    json!({
        "start": offset_to_position(source, source[..start].encode_utf16().count() as u64),
        "end": offset_to_position(source, source[..end].encode_utf16().count() as u64)
    })
}

fn semantic_selection_range(source: &str, cursor: usize) -> Value {
    let cursor = cursor.min(source.len());
    let mut ranges = Vec::new();
    let mut start = cursor;
    while let Some((offset, character)) = source[..start].char_indices().next_back() {
        if !(character.is_alphanumeric() || matches!(character, '_' | '$')) {
            break;
        }
        start = offset;
    }
    let mut end = cursor;
    while let Some(character) = source[end..].chars().next() {
        if !(character.is_alphanumeric() || matches!(character, '_' | '$')) {
            break;
        }
        end += character.len_utf8();
    }
    if start < end {
        ranges.push((start, end));
    }
    let mut stack: Vec<(u8, usize)> = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let value = bytes[index];
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if value == b'\\' {
                escaped = true;
            } else if value == delimiter {
                quote = None;
            }
            index += 1;
            continue;
        }
        if value == b'"' || value == b'\'' {
            quote = Some(value);
        } else if matches!(value, b'(' | b'[' | b'{') {
            stack.push((value, index));
        } else if matches!(value, b')' | b']' | b'}')
            && let Some(position) = stack.iter().rposition(|(open, _)| {
                matches!((*open, value), (b'(', b')') | (b'[', b']') | (b'{', b'}'))
            })
        {
            let (_, open) = stack.remove(position);
            if open <= cursor && cursor <= index + 1 {
                ranges.push((open, index + 1));
            }
        }
        index += 1;
    }
    let line_start = source[..cursor].rfind('\n').map_or(0, |value| value + 1);
    let line_end = source[cursor..]
        .find('\n')
        .map_or(source.len(), |value| cursor + value);
    ranges.push((line_start, line_end));
    ranges.push((0, source.len()));
    ranges.sort_by_key(|(start, end)| end.saturating_sub(*start));
    ranges.dedup();
    let mut parent = None;
    for (start, end) in ranges.into_iter().rev() {
        let range = json!({
            "start": offset_to_position(source, source[..start].encode_utf16().count() as u64),
            "end": offset_to_position(source, source[..end].encode_utf16().count() as u64)
        });
        parent = Some(match parent {
            Some(parent) => json!({"range": range, "parent": parent}),
            None => json!({"range": range}),
        });
    }
    parent.unwrap_or_else(|| {
        json!({
            "range": {"start":{"line":0,"character":0},"end":{"line":0,"character":0}}
        })
    })
}

fn java_folding_ranges(source: &str) -> Vec<Value> {
    let lines: Vec<_> = source.split('\n').collect();
    let mut ranges = Vec::new();
    let imports: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(line, text)| text.trim_start().starts_with("import ").then_some(line))
        .collect();
    if let (Some(start), Some(end)) = (imports.first(), imports.last())
        && end > start
    {
        ranges.push(json!({"startLine": start, "endLine": end, "kind": "imports"}));
    }
    let mut braces = Vec::new();
    let mut block_comment = None;
    let mut string = false;
    let mut character = false;
    let mut escaped = false;
    for (line_number, line) in lines.iter().enumerate() {
        let bytes = line.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let current = bytes[index];
            let next = bytes.get(index + 1).copied();
            if let Some(start) = block_comment {
                if current == b'*' && next == Some(b'/') {
                    if line_number > start {
                        ranges.push(json!({
                            "startLine": start,
                            "endLine": line_number,
                            "kind": "comment"
                        }));
                    }
                    block_comment = None;
                    index += 2;
                } else {
                    index += 1;
                }
                continue;
            }
            if string || character {
                if escaped {
                    escaped = false;
                } else if current == b'\\' {
                    escaped = true;
                } else if (string && current == b'"') || (character && current == b'\'') {
                    string = false;
                    character = false;
                }
                index += 1;
                continue;
            }
            if current == b'/' && next == Some(b'/') {
                break;
            }
            if current == b'/' && next == Some(b'*') {
                block_comment = Some(line_number);
                index += 2;
                continue;
            }
            match current {
                b'"' => string = true,
                b'\'' => character = true,
                b'{' => braces.push(line_number),
                b'}' => {
                    if let Some(start) = braces.pop()
                        && line_number > start
                    {
                        ranges.push(json!({
                            "startLine": start,
                            "endLine": line_number.saturating_sub(1),
                            "kind": "region"
                        }));
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }
    ranges.sort_by_key(|range| {
        (
            range["startLine"].as_u64().unwrap_or(0),
            range["endLine"].as_u64().unwrap_or(0),
        )
    });
    ranges
}

fn definition_identity(definition: &javac_frontend::EditorDefinition) -> String {
    if !definition.symbol_id.is_empty() {
        return definition.symbol_id.clone();
    }
    let owner_name = definition
        .owner
        .rsplit(['.', '$'])
        .next()
        .unwrap_or(&definition.owner);
    if owner_name == definition.name {
        definition.owner.clone()
    } else {
        format!("{}.{}", definition.owner, definition.name)
    }
}

fn location_symbol_id(location: &javac_frontend::SymbolLocation) -> &str {
    if location.symbol_id.is_empty() {
        &location.qualified_name
    } else {
        &location.symbol_id
    }
}

fn structural_method_identity(symbol_id: &str) -> Option<String> {
    let (_, member) = symbol_id.split_once('|')?;
    let descriptor = member.find('(')?;
    let method = &member[..descriptor];
    method
        .contains('#')
        .then(|| method.replace(['/', '$'], "."))
}

fn contains_java_identifier(source: &str, identifier: &str) -> bool {
    source.match_indices(identifier).any(|(start, _)| {
        let end = start + identifier.len();
        let bounded_start = source[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !CharacterExt::java_identifier_part(character));
        let bounded_end = source[end..]
            .chars()
            .next()
            .is_none_or(|character| !CharacterExt::java_identifier_part(character));
        bounded_start && bounded_end
    })
}

const JUNIT_ANNOTATIONS: [&str; 5] = [
    "Test",
    "ParameterizedTest",
    "RepeatedTest",
    "TestFactory",
    "TestTemplate",
];

fn junit_tags(source: &str, offset: u64) -> Vec<&'static str> {
    let byte = utf16_offset_to_byte(source, offset);
    let prefix = &source[..byte.min(source.len())];
    JUNIT_ANNOTATIONS
        .into_iter()
        .filter(|annotation| {
            prefix.lines().rev().take(8).any(|line| {
                let Some(value) = line.trim().strip_prefix('@') else {
                    return false;
                };
                value
                    .split(['(', ' ', '\t'])
                    .next()
                    .and_then(|name| name.rsplit('.').next())
                    == Some(*annotation)
            })
        })
        .collect()
}

fn has_junit_annotation(source: &str, offset: u64) -> bool {
    !junit_tags(source, offset).is_empty()
}

fn is_external_source_uri(uri: &str) -> bool {
    uri.split(['/', '\\'])
        .collect::<Vec<_>>()
        .windows(2)
        .any(|parts| parts[0] == "io.github.zonnedev.jman.lsp" && parts[1] == "external-sources")
}

fn publish_diagnostics(
    uri: &str,
    version: Option<i64>,
    diagnostics: &[javac_frontend::SemanticDiagnostic],
    source: &str,
) -> Value {
    let diagnostics = lsp_diagnostics(diagnostics, source);
    json!({
        "jsonrpc":"2.0",
        "method":"textDocument/publishDiagnostics",
        "params":{"uri":uri,"version":version,"diagnostics":diagnostics}
    })
}

fn lsp_diagnostics(diagnostics: &[javac_frontend::SemanticDiagnostic], source: &str) -> Vec<Value> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            json!({
                "range": {
                    "start": offset_to_position(source, diagnostic.start),
                    "end": offset_to_position(source, diagnostic.end)
                },
                "severity": match diagnostic.kind.as_str() {
                    "error" => 1,
                    "warning" | "mandatory_warning" => 2,
                    "note" => 3,
                    _ => 4
                },
                "code": diagnostic.code,
                "source": "javac",
                "message": diagnostic.message
            })
        })
        .collect()
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message}
    })
}

fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

fn progress_notification(token: &Value, value: Value) -> Value {
    json!({
        "jsonrpc":"2.0",
        "method":"$/progress",
        "params":{"token":token,"value":value}
    })
}

fn work_done_response(id: Value, token: Option<&Value>, title: &str, result: Value) -> Dispatch {
    let Some(token) = token else {
        return Dispatch::Reply(success(id, result));
    };
    Dispatch::Batch(vec![
        progress_notification(token, json!({"kind":"begin","title":title})),
        success(id, result),
        progress_notification(token, json!({"kind":"end"})),
    ])
}

fn lsp_symbol_kind(kind: &str) -> u8 {
    match kind {
        "class" => 5,
        "interface" => 11,
        "enum" => 10,
        "method" | "constructor" => 6,
        "field" | "enum_constant" => 8,
        "package" => 4,
        _ => 13,
    }
}

fn completion_item_kind(kind: &str) -> u8 {
    match kind {
        "method" | "constructor" => 2,
        "field" | "enum_constant" => 5,
        "class" => 7,
        "interface" => 8,
        "enum" => 13,
        "package" => 9,
        _ => 6,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JpmsCompletionContext {
    Module(String),
    Package(String),
    Service(String),
    Provider(String),
}

fn jpms_completion_context(source: &str, cursor: usize) -> Option<JpmsCompletionContext> {
    let before = &source[..cursor.min(source.len())];
    let statement = before
        .rsplit([';', '{'])
        .next()
        .unwrap_or(before)
        .trim_start();
    if let Some(rest) = statement.strip_prefix("requires ") {
        let mut prefix = rest.trim_start();
        loop {
            let stripped = prefix
                .strip_prefix("static ")
                .or_else(|| prefix.strip_prefix("transitive "));
            let Some(stripped) = stripped else {
                break;
            };
            prefix = stripped.trim_start();
        }
        return Some(JpmsCompletionContext::Module(prefix.to_owned()));
    }
    for directive in ["exports ", "opens "] {
        if let Some(rest) = statement.strip_prefix(directive) {
            if let Some((_, target)) = rest.rsplit_once(" to ") {
                let prefix = target.rsplit(',').next().unwrap_or(target).trim();
                return Some(JpmsCompletionContext::Module(prefix.to_owned()));
            }
            return Some(JpmsCompletionContext::Package(rest.trim().to_owned()));
        }
    }
    if let Some(rest) = statement.strip_prefix("uses ") {
        return Some(JpmsCompletionContext::Service(rest.trim().to_owned()));
    }
    if let Some(rest) = statement.strip_prefix("provides ") {
        if let Some((_, provider)) = rest.rsplit_once(" with ") {
            let prefix = provider.rsplit(',').next().unwrap_or(provider).trim();
            return Some(JpmsCompletionContext::Provider(prefix.to_owned()));
        }
        return Some(JpmsCompletionContext::Service(rest.trim().to_owned()));
    }
    None
}

fn jpms_module_token_at(source: &str, cursor: usize) -> Option<&str> {
    let (start, end) = java_token_range(source, cursor)?;
    let token = &source[start..end];
    let statement_start = source[..start]
        .rfind([';', '{'])
        .map_or(0, |index| index + 1);
    let statement = source[statement_start..end].trim_start();
    let module_position = statement.starts_with("requires ")
        || ((statement.starts_with("exports ") || statement.starts_with("opens "))
            && statement.contains(" to "));
    module_position.then_some(token)
}

fn java_token_range(source: &str, cursor: usize) -> Option<(usize, usize)> {
    let mut start = cursor.min(source.len());
    while start > 0 {
        let character = source[..start].chars().next_back()?;
        if !(CharacterExt::java_identifier_part(character) || character == '.') {
            break;
        }
        start -= character.len_utf8();
    }
    let mut end = cursor.min(source.len());
    while end < source.len() {
        let character = source[end..].chars().next()?;
        if !(CharacterExt::java_identifier_part(character) || character == '.') {
            break;
        }
        end += character.len_utf8();
    }
    (start < end).then_some((start, end))
}

trait CharacterExt {
    fn java_identifier_part(self) -> bool;
}

impl CharacterExt for char {
    fn java_identifier_part(self) -> bool {
        self == '_' || self == '$' || self.is_alphanumeric()
    }
}

fn module_declaration_range(source: &str, module: &str) -> Option<(usize, usize)> {
    for keyword in ["module ", "open module "] {
        let mut offset = 0;
        while let Some(found) = source[offset..].find(keyword) {
            let name_start = offset + found + keyword.len();
            let remaining = &source[name_start..];
            let name_end = remaining
                .find(|character: char| {
                    !(character.is_alphanumeric() || matches!(character, '.' | '_' | '$'))
                })
                .unwrap_or(remaining.len());
            if &remaining[..name_end] == module {
                return Some((name_start, name_start + name_end));
            }
            offset = name_start + name_end;
        }
    }
    None
}

fn jpms_unread_module(message: &str) -> Option<&str> {
    if !message.contains("does not read") {
        return None;
    }
    let remaining = message.split_once("declared in module ")?.1;
    let end = remaining
        .find(|character: char| character == ',' || character == ')' || character.is_whitespace())
        .unwrap_or(remaining.len());
    let module = &remaining[..end];
    (!module.is_empty() && module != "unnamed").then_some(module)
}

fn jpms_unexported_package(message: &str) -> Option<(&str, &str)> {
    if !(message.contains("does not export") || message.contains("is not visible")) {
        return None;
    }
    let package = message
        .strip_prefix("package ")?
        .split_whitespace()
        .next()?;
    let remaining = message.split_once("declared in module ")?.1;
    let end = remaining
        .find(|character: char| character == ',' || character == ')' || character.is_whitespace())
        .unwrap_or(remaining.len());
    let module = &remaining[..end];
    (message.contains("does not export") && !module.is_empty()).then_some((module, package))
}

fn insert_module_directive(source: &str, directive: &str) -> Option<Value> {
    if source.contains(directive) {
        return None;
    }
    let brace = source.find('{')? + 1;
    Some(json!({
        "range": {
            "start": offset_to_position(source, brace as u64),
            "end": offset_to_position(source, brace as u64)
        },
        "newText": format!("\n  {directive}")
    }))
}

fn resolve_support_properties(params: Option<&Value>, pointer: &str) -> HashSet<String> {
    params
        .and_then(|value| value.pointer(pointer))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn initialization_workspace_folders(params: Option<&Value>) -> Vec<String> {
    params
        .and_then(|value| value["workspaceFolders"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|folder| folder["uri"].as_str())
        .map(str::to_owned)
        .collect()
}

fn workspace_folder_uris(folders: &Value) -> Vec<String> {
    folders
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|folder| folder["uri"].as_str())
        .map(str::to_owned)
        .collect()
}

fn java_file_operation_filter() -> Value {
    json!({
        "scheme":"file",
        "pattern":{"glob":"**/*.{java,jman.toml,jman.lock,xml,gradle,gradle.kts}"}
    })
}

fn file_operation_uris(params: &Value) -> Vec<String> {
    let mut uris: Vec<_> = params["files"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|file| {
            [
                file["uri"].as_str(),
                file["oldUri"].as_str(),
                file["newUri"].as_str(),
            ]
        })
        .flatten()
        .map(str::to_owned)
        .collect();
    uris.sort();
    uris.dedup();
    uris
}

#[derive(Debug, PartialEq, Eq)]
struct JavadocReference {
    target: String,
    start: u64,
    end: u64,
}

fn javadoc_references(source: &str) -> Vec<JavadocReference> {
    let mut references = Vec::new();
    for marker in ["{@link ", "{@linkplain "] {
        let mut offset = 0;
        while let Some(relative) = source[offset..].find(marker) {
            let start = offset + relative + marker.len();
            let end = source[start..]
                .find(|character: char| character.is_whitespace() || character == '}')
                .map_or(source.len(), |relative| start + relative);
            push_javadoc_reference(source, start, end, &mut references);
            offset = end.max(start + 1);
        }
    }
    let mut offset = 0;
    while let Some(relative) = source[offset..].find("@see ") {
        let start = offset + relative + "@see ".len();
        let end = source[start..]
            .find(|character: char| character.is_whitespace() || character == '*')
            .map_or(source.len(), |relative| start + relative);
        push_javadoc_reference(source, start, end, &mut references);
        offset = end.max(start + 1);
    }
    references.sort_by_key(|reference| (reference.start, reference.end));
    references.dedup_by(|left, right| left.start == right.start && left.end == right.end);
    references
}

fn push_javadoc_reference(
    source: &str,
    start: usize,
    end: usize,
    references: &mut Vec<JavadocReference>,
) {
    let target = source[start..end].trim_matches(['#', '.']);
    if target.is_empty() {
        return;
    }
    let trimmed_start = source[start..end].find(target).unwrap_or(0) + start;
    references.push(JavadocReference {
        target: target.to_owned(),
        start: source[..trimmed_start].encode_utf16().count() as u64,
        end: source[..trimmed_start + target.len()]
            .encode_utf16()
            .count() as u64,
    });
}

fn semantic_token_type(kind: &str, role: &str) -> Option<u64> {
    Some(match kind {
        "package" | "module" => 0,
        "class" => 1,
        "interface" => 2,
        "annotation" | "annotation_type" if role == "declaration" => 2,
        "enum" => 3,
        "type_parameter" => 4,
        "method" | "constructor" => 5,
        "field" => 6,
        "local_variable" | "resource_variable" | "binding_variable" | "variable" => 7,
        "record" => 8,
        "parameter" | "exception_parameter" => 9,
        "enum_constant" => 10,
        "annotation" | "annotation_type" => 11,
        _ => return None,
    })
}

fn semantic_token_modifiers(symbol: &javac_frontend::SemanticSymbol) -> u64 {
    let mut modifiers = 0;
    if symbol.role == "declaration" {
        modifiers |= 1 << 0;
    }
    if symbol.kind == "enum_constant" {
        modifiers |= (1 << 1) | (1 << 2);
    }
    if symbol.symbol_id.starts_with("java.") && symbol.symbol_id.contains('|') {
        modifiers |= 1 << 5;
    }
    if symbol.role == "write" {
        modifiers |= 1 << 6;
    }
    modifiers
}

fn lsp_position(value: &Value) -> Option<(u64, u64)> {
    Some((value["line"].as_u64()?, value["character"].as_u64()?))
}

fn ranges_intersect(start: &Value, end: &Value, range: &Value) -> bool {
    let Some(token_start) = lsp_position(start) else {
        return false;
    };
    let Some(token_end) = lsp_position(end) else {
        return false;
    };
    let Some(range_start) = lsp_position(&range["start"]) else {
        return false;
    };
    let Some(range_end) = lsp_position(&range["end"]) else {
        return false;
    };
    token_end > range_start && token_start < range_end
}

fn semantic_token_delta(previous: &[u64], current: &[u64]) -> Vec<Value> {
    let mut prefix = previous
        .iter()
        .zip(current)
        .take_while(|(left, right)| left == right)
        .count();
    prefix -= prefix % 5;

    let maximum_suffix = previous.len().min(current.len()).saturating_sub(prefix);
    let mut suffix = previous
        .iter()
        .rev()
        .zip(current.iter().rev())
        .take(maximum_suffix)
        .take_while(|(left, right)| left == right)
        .count();
    suffix -= suffix % 5;

    let delete_count = previous.len().saturating_sub(prefix + suffix);
    let replacement_end = current.len().saturating_sub(suffix);
    let replacement = &current[prefix..replacement_end];
    if delete_count == 0 && replacement.is_empty() {
        return Vec::new();
    }
    if replacement.is_empty() {
        vec![json!({"start": prefix, "deleteCount": delete_count})]
    } else {
        vec![json!({
            "start": prefix,
            "deleteCount": delete_count,
            "data": replacement
        })]
    }
}

fn completion_sort_text(name: &str, prefix: &str) -> String {
    let rank = if name == prefix {
        0
    } else if name.starts_with(prefix) {
        1
    } else {
        2
    };
    format!("{rank}:{name}")
}

fn java_identifier_prefix(source: &str) -> &str {
    let start = source
        .char_indices()
        .rev()
        .find(|(_, character)| {
            !character.is_alphanumeric() && *character != '_' && *character != '$'
        })
        .map_or(0, |(index, character)| index + character.len_utf8());
    &source[start..]
}

fn utf16_offset_to_byte(source: &str, offset: u64) -> usize {
    let mut units = 0_u64;
    for (byte, character) in source.char_indices() {
        if units >= offset {
            return byte;
        }
        units += character.len_utf16() as u64;
    }
    source.len()
}

fn is_type_symbol(kind: &str) -> bool {
    matches!(
        kind,
        "class" | "interface" | "enum" | "annotation" | "record"
    )
}

fn import_edit(source: &str, qualified_name: &str) -> Option<Value> {
    let (package, imports, insertion) = java_import_context(source);
    if !qualified_name.contains('.')
        || qualified_name.starts_with("java.lang.")
        || package
            .as_deref()
            .is_some_and(|name| qualified_name.starts_with(&format!("{name}.")))
        || imports.iter().any(|imported| {
            imported == qualified_name
                || imported
                    .strip_suffix(".*")
                    .is_some_and(|prefix| qualified_name.starts_with(&format!("{prefix}.")))
        })
    {
        return None;
    }
    let position = offset_to_position(source, insertion as u64);
    Some(json!({
        "range": {"start": position, "end": position},
        "newText": format!("import {qualified_name};\n")
    }))
}

fn java_import_context(source: &str) -> (Option<String>, Vec<String>, usize) {
    let mut package = None;
    let mut imports = Vec::new();
    let mut insertion = 0usize;
    let mut offset = 0usize;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if let Some(value) = trimmed
            .strip_prefix("package ")
            .and_then(|value| value.strip_suffix(';'))
        {
            package = Some(value.trim().to_owned());
            insertion = offset + line.len();
        } else if let Some(value) = trimmed
            .strip_prefix("import ")
            .and_then(|value| value.strip_suffix(';'))
        {
            imports.push(value.trim().trim_start_matches("static ").to_owned());
            insertion = offset + line.len();
        }
        offset += line.len();
    }
    (package, imports, insertion)
}

fn excluded_local_source_directory(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(".git" | ".gradle" | ".idea" | ".jman" | "build" | "node_modules" | "out" | "target")
    )
}

fn java_type_declaration_range(source: &str, name: &str) -> Option<(u64, u64)> {
    for keyword in ["class", "interface", "enum", "record", "@interface"] {
        let mut offset = 0;
        while let Some(found) = source[offset..].find(keyword) {
            let keyword_start = offset + found;
            let keyword_end = keyword_start + keyword.len();
            let bounded_start = keyword_start == 0
                || !source[..keyword_start]
                    .chars()
                    .next_back()
                    .is_some_and(CharacterExt::java_identifier_part);
            let bounded_end = source[keyword_end..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace);
            if bounded_start && bounded_end {
                let whitespace = source[keyword_end..]
                    .chars()
                    .take_while(|character| character.is_whitespace())
                    .map(char::len_utf8)
                    .sum::<usize>();
                let name_start = keyword_end + whitespace;
                let name_end = name_start + name.len();
                if source.get(name_start..name_end) == Some(name)
                    && source[name_end..]
                        .chars()
                        .next()
                        .is_none_or(|character| !CharacterExt::java_identifier_part(character))
                {
                    return Some((
                        source[..name_start].encode_utf16().count() as u64,
                        source[..name_end].encode_utf16().count() as u64,
                    ));
                }
            }
            offset = keyword_end;
        }
    }
    None
}

fn missing_type_name(message: &str) -> Option<&str> {
    for marker in ["class ", "interface ", "enum ", "record ", "@interface "] {
        for suffix in message
            .match_indices(marker)
            .map(|(index, _)| &message[index + marker.len()..])
        {
            let candidate = suffix
                .trim_start()
                .split(|character: char| !character.is_alphanumeric() && character != '_')
                .next()
                .unwrap_or_default();
            if !candidate.is_empty() && candidate.chars().next().is_some_and(char::is_uppercase) {
                return Some(candidate);
            }
        }
    }
    None
}

fn remove_override_edit(source: &str, diagnostic: &Value) -> Option<Value> {
    let start = crate::position_to_byte(source, &diagnostic["range"]["start"]).ok()?;
    let annotation = source[..start].rfind("@Override")?;
    let between = &source[annotation + "@Override".len()..start];
    if between.contains(['}', ';']) {
        return None;
    }
    let mut end = annotation + "@Override".len();
    while source[end..].starts_with([' ', '\t']) {
        end += 1;
    }
    if source[end..].starts_with("\r\n") {
        end += 2;
    } else if source[end..].starts_with('\n') {
        end += 1;
    }
    Some(json!({
        "range": {
            "start": offset_to_position(source, source[..annotation].encode_utf16().count() as u64),
            "end": offset_to_position(source, source[..end].encode_utf16().count() as u64)
        },
        "newText": ""
    }))
}

fn unreported_exception(message: &str) -> Option<&str> {
    message
        .split_once("unreported exception ")
        .and_then(|(_, rest)| rest.split_once(';').map(|(name, _)| name.trim()))
        .filter(|name| !name.is_empty())
}

fn missing_abstract_method(message: &str) -> Option<&str> {
    let rest = message
        .split_once("does not override abstract method ")
        .map(|(_, rest)| rest)?;
    let signature = rest
        .split_once(" in ")
        .map_or(rest, |(signature, _)| signature);
    signature
        .split_once('(')
        .map(|(name, _)| name.trim())
        .filter(|name| !name.is_empty())
}

fn add_throws_edit(source: &str, diagnostic: &Value, exception: &str) -> Option<Value> {
    let cursor = crate::position_to_byte(source, &diagnostic["range"]["start"]).ok()?;
    let (body, close) = enclosing_braces(source, cursor)
        .into_iter()
        .rev()
        .find_map(|body| {
            let close = source[..body].rfind(')')?;
            let open = source[..close].rfind('(')?;
            let name = source[..open]
                .trim_end()
                .rsplit(|character: char| !CharacterExt::java_identifier_part(character))
                .next()?;
            (!matches!(
                name,
                "if" | "for" | "while" | "switch" | "catch" | "synchronized"
            ))
            .then_some((body, close))
        })?;
    let suffix = &source[close + 1..body];
    let new_text = if suffix.contains("throws") {
        format!(", {exception}")
    } else {
        format!(" throws {exception}")
    };
    let insertion = if suffix.contains("throws") {
        body - suffix
            .chars()
            .rev()
            .take_while(|character| character.is_whitespace())
            .count()
    } else {
        close + 1
    };
    Some(json!({
        "range": {
            "start": offset_to_position(source, source[..insertion].encode_utf16().count() as u64),
            "end": offset_to_position(source, source[..insertion].encode_utf16().count() as u64)
        },
        "newText": new_text
    }))
}

fn enclosing_braces(source: &str, cursor: usize) -> Vec<usize> {
    let mut stack = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in source[..cursor.min(source.len())].char_indices() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '{' => stack.push(index),
            '}' => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack
}

fn surround_with_try_catch_edit(
    source: &str,
    diagnostic: &Value,
    exception: &str,
) -> Option<Value> {
    let cursor = crate::position_to_byte(source, &diagnostic["range"]["start"]).ok()?;
    let line_start = source[..cursor].rfind('\n').map_or(0, |index| index + 1);
    let line_end = source[cursor..]
        .find('\n')
        .map_or(source.len(), |index| cursor + index);
    let line = &source[line_start..line_end];
    let indent: String = line
        .chars()
        .take_while(|character| character.is_whitespace())
        .collect();
    let statement = line.trim();
    if statement.is_empty() || !statement.ends_with(';') {
        return None;
    }
    Some(json!({
        "range": {
            "start": offset_to_position(source, source[..line_start].encode_utf16().count() as u64),
            "end": offset_to_position(source, source[..line_end].encode_utf16().count() as u64)
        },
        "newText": format!(
            "{indent}try {{\n{indent}    {statement}\n{indent}}} catch ({exception} exception) {{\n{indent}    throw new RuntimeException(exception);\n{indent}}}"
        )
    }))
}

fn organize_imports_edit(source: &str) -> Option<Value> {
    let mut imports = Vec::new();
    let mut start = None;
    let mut end = 0;
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("import ") && trimmed.ends_with(';') {
            start.get_or_insert(offset);
            end = offset + line.len();
            let imported = trimmed
                .trim_start_matches("import ")
                .trim_end_matches(';')
                .trim();
            let simple = imported
                .trim_start_matches("static ")
                .rsplit('.')
                .next()
                .unwrap_or(imported);
            let used = simple == "*"
                || source[end..]
                    .split(|character: char| !CharacterExt::java_identifier_part(character))
                    .any(|token| token == simple);
            if used {
                imports.push(trimmed.to_owned());
            }
        } else if start.is_some() && !trimmed.is_empty() {
            break;
        }
        offset += line.len();
    }
    let start = start?;
    imports.sort();
    imports.dedup();
    let replacement = if imports.is_empty() {
        String::new()
    } else {
        format!("{}\n", imports.join("\n"))
    };
    (source[start..end] != replacement).then(|| {
        json!({
            "range": {
                "start": offset_to_position(source, source[..start].encode_utf16().count() as u64),
                "end": offset_to_position(source, source[..end].encode_utf16().count() as u64)
            },
            "newText": replacement
        })
    })
}

#[derive(Debug)]
struct JavaField<'a> {
    ty: &'a str,
    name: &'a str,
    final_field: bool,
}

fn generation_actions(source: &str, uri: &str) -> Vec<Value> {
    let Some((class_name, body_end)) = simple_class(source) else {
        return Vec::new();
    };
    let fields = simple_fields(source);
    if fields.is_empty() {
        return Vec::new();
    }
    let indent = "    ";
    let insertion = json!({
        "start": offset_to_position(source, source[..body_end].encode_utf16().count() as u64),
        "end": offset_to_position(source, source[..body_end].encode_utf16().count() as u64)
    });
    let edit = |title: String, kind: &str, text: String| {
        json!({
            "title": title,
            "kind": kind,
            "edit": {"changes": {(uri): [{
                "range": insertion.clone(),
                "newText": format!("\n{text}\n")
            }]}}
        })
    };
    let parameters = fields
        .iter()
        .map(|field| format!("{} {}", field.ty, field.name))
        .collect::<Vec<_>>()
        .join(", ");
    let assignments = fields
        .iter()
        .map(|field| format!("{indent}{indent}this.{0} = {0};", field.name))
        .collect::<Vec<_>>()
        .join("\n");
    let constructor =
        format!("{indent}public {class_name}({parameters}) {{\n{assignments}\n{indent}}}");
    let mut accessors = Vec::new();
    for field in &fields {
        let capitalized = capitalize(field.name);
        let getter = if field.ty == "boolean" { "is" } else { "get" };
        accessors.push(format!(
            "{indent}public {} {getter}{capitalized}() {{\n{indent}{indent}return this.{};\n{indent}}}",
            field.ty, field.name
        ));
        if !field.final_field {
            accessors.push(format!(
                "{indent}public void set{capitalized}({ty} {name}) {{\n{indent}{indent}this.{name} = {name};\n{indent}}}",
                ty = field.ty,
                name = field.name
            ));
        }
    }
    let compared = fields
        .iter()
        .map(|field| {
            if matches!(
                field.ty,
                "boolean" | "byte" | "short" | "int" | "long" | "char" | "float" | "double"
            ) {
                format!("this.{0} == other.{0}", field.name)
            } else {
                format!("java.util.Objects.equals(this.{0}, other.{0})", field.name)
            }
        })
        .collect::<Vec<_>>()
        .join(&format!("\n{indent}{indent}{indent}&& "));
    let arguments = fields
        .iter()
        .map(|field| format!("this.{}", field.name))
        .collect::<Vec<_>>()
        .join(", ");
    let display = fields
        .iter()
        .map(|field| format!("{}=\" + this.{} + \"", field.name, field.name))
        .collect::<Vec<_>>()
        .join(", ");
    let object_methods = format!(
        "{indent}@Override\n{indent}public boolean equals(Object object) {{\n{indent}{indent}if (this == object) return true;\n{indent}{indent}if (!(object instanceof {class_name} other)) return false;\n{indent}{indent}return {compared};\n{indent}}}\n\n{indent}@Override\n{indent}public int hashCode() {{\n{indent}{indent}return java.util.Objects.hash({arguments});\n{indent}}}\n\n{indent}@Override\n{indent}public String toString() {{\n{indent}{indent}return \"{class_name}{{{display}}}\";\n{indent}}}"
    );
    vec![
        edit(
            format!("Generate constructor for {class_name}"),
            "refactor.rewrite",
            constructor,
        ),
        edit(
            "Generate getters and setters".to_owned(),
            "refactor.rewrite",
            accessors.join("\n\n"),
        ),
        edit(
            "Generate equals, hashCode and toString".to_owned(),
            "refactor.rewrite",
            object_methods,
        ),
    ]
}

fn simple_class(source: &str) -> Option<(&str, usize)> {
    let class = source.find("class ")? + "class ".len();
    let end = source[class..]
        .find(|character: char| !CharacterExt::java_identifier_part(character))
        .map_or(source.len(), |offset| class + offset);
    let name = &source[class..end];
    let body_end = source.rfind('}')?;
    (!name.is_empty()).then_some((name, body_end))
}

fn simple_fields(source: &str) -> Vec<JavaField<'_>> {
    source
        .lines()
        .filter_map(|line| {
            let declaration = line.trim().trim_end_matches(';');
            if !line.trim_end().ends_with(';')
                || declaration.contains('(')
                || declaration.starts_with("import ")
                || declaration.starts_with("package ")
                || declaration.contains('=')
            {
                return None;
            }
            let parts: Vec<_> = declaration.split_whitespace().collect();
            let name = *parts.last()?;
            let ty_index = parts.len().checked_sub(2)?;
            let ty = parts[ty_index];
            let modifiers = &parts[..ty_index];
            (!modifiers.contains(&"static") && !matches!(ty, "return" | "throw" | "case"))
                .then_some(JavaField {
                    ty,
                    name,
                    final_field: modifiers.contains(&"final"),
                })
        })
        .collect()
}

fn capitalize(value: &str) -> String {
    let mut characters = value.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

fn parenthesized_after(source: &str, from: usize) -> Option<(usize, usize, Vec<&str>)> {
    let (open, close, parts) = parenthesized_parts_after(source, from)?;
    Some((
        open,
        close,
        parts.into_iter().map(|(_, value)| value).collect(),
    ))
}

type ParenthesizedParts<'source> = (usize, usize, Vec<(usize, &'source str)>);

fn parenthesized_parts_after(source: &str, from: usize) -> Option<ParenthesizedParts<'_>> {
    let open = source[from..].find('(')? + from;
    if source[from..open].contains([';', '\n', ':']) {
        return None;
    }
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    let mut commas = Vec::new();
    let mut close = None;
    for (relative, character) in source[open..].char_indices() {
        let index = open + relative;
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '(' | '[' | '{' | '<' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(index);
                    break;
                }
            }
            ']' | '}' | '>' => depth = depth.saturating_sub(1),
            ',' if depth == 1 => commas.push(index),
            _ => {}
        }
    }
    let close = close?;
    let mut arguments = Vec::new();
    let mut start = open + 1;
    for comma in commas {
        arguments.push((start, &source[start..comma]));
        start = comma + 1;
    }
    if start < close || !source[open + 1..close].trim().is_empty() {
        arguments.push((start, &source[start..close]));
    }
    Some((open, close, arguments))
}

fn parameter_name(parameter: &str) -> Option<&str> {
    parameter
        .split_whitespace()
        .next_back()
        .map(|name| name.trim_end_matches("[]").trim_start_matches("..."))
        .filter(|name| !name.is_empty())
}

fn parameter_type(parameter: &str) -> &str {
    let parameter = parameter.trim();
    parameter
        .rsplit_once(char::is_whitespace)
        .map_or(parameter, |(ty, _)| ty.trim())
}

const JAVA_COMPLETION_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "sealed",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "var",
    "void",
    "volatile",
    "while",
    "yield",
];

fn is_build_model_file(uri: &str) -> bool {
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "jman.toml"
            | "jman.lock"
            | "pom.xml"
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
    use super::*;
    use javac_frontend::{SemanticDiagnostic, SemanticSymbol};
    use std::sync::{Arc, Mutex};

    #[test]
    fn enforces_initialize_shutdown_exit_lifecycle() {
        let mut server = Server::default();
        let before = server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"shutdown"}));
        assert_eq!(reply(&before)["error"]["code"], -32002);

        let initialize =
            server.dispatch(json!({"jsonrpc":"2.0","id":"init","method":"initialize"}));
        assert_eq!(
            reply(&initialize)["result"]["capabilities"]["positionEncoding"],
            "utf-16"
        );

        let shutdown = server.dispatch(json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}));
        assert!(reply(&shutdown)["result"].is_null());
        assert_eq!(
            server.dispatch(json!({"jsonrpc":"2.0","method":"exit"})),
            Dispatch::Exit(0)
        );
    }

    #[test]
    fn supports_workspace_lifecycle_pull_diagnostics_and_source_metadata() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut server = Server::with_backend(LifecycleBackend {
            calls: Arc::clone(&calls),
        });
        let initialized = server.dispatch(json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{
                "rootUri":"file:///one",
                "workspaceFolders":[
                    {"uri":"file:///one","name":"one"},
                    {"uri":"file:///two","name":"two"}
                ]
            }
        }));
        let capabilities = &reply(&initialized)["result"]["capabilities"];
        assert_eq!(
            capabilities["workspace"]["workspaceFolders"]["supported"],
            true
        );
        assert_eq!(
            capabilities["diagnosticProvider"]["workspaceDiagnostics"],
            true
        );
        assert!(capabilities["workspace"]["fileOperations"].is_object());
        assert!(capabilities["documentLinkProvider"].is_object());
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            ["initialize:file:///one", "folders:+file:///two:-"]
        );

        server.dispatch(json!({
            "jsonrpc":"2.0","method":"workspace/didChangeWorkspaceFolders",
            "params":{"event":{
                "added":[{"uri":"file:///three","name":"three"}],
                "removed":[{"uri":"file:///one","name":"one"}]
            }}
        }));
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"workspace/didChangeConfiguration",
            "params":{"settings":{"buildSync":"manual","buildSystem":"maven"}}
        }));
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":99}
        }));
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"workspace/didRenameFiles",
            "params":{"files":[{"oldUri":"file:///two/Old.java","newUri":"file:///two/New.java"}]}
        }));
        let recorded = calls.lock().unwrap().clone();
        assert!(recorded.contains(&"folders:+file:///three:-file:///one".to_owned()));
        assert!(recorded.contains(&"configuration:maven".to_owned()));
        assert!(recorded.contains(&"cancel:99".to_owned()));
        assert!(recorded.contains(&"rebuild".to_owned()));

        let uri = "file:///two/generated/Generated.java";
        let source = "class Generated {}";
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":uri,"version":1,"text":source}}
        }));
        let diagnostics = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/diagnostic",
            "params":{"textDocument":{"uri":uri}}
        }));
        assert_eq!(reply(&diagnostics)["result"]["kind"], "full");
        assert_eq!(reply(&diagnostics)["result"]["items"][0]["source"], "javac");
        let previous = reply(&diagnostics)["result"]["resultId"].clone();
        let unchanged = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"textDocument/diagnostic",
            "params":{"textDocument":{"uri":uri},"previousResultId":previous}
        }));
        assert_eq!(reply(&unchanged)["result"]["kind"], "unchanged");
        let workspace = server.dispatch(json!({
            "jsonrpc":"2.0","id":4,"method":"workspace/diagnostic","params":{}
        }));
        assert!(
            !reply(&workspace)["result"]["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let metadata = server.dispatch(json!({
            "jsonrpc":"2.0","id":5,"method":"jman.java/sourceMetadata",
            "params":{"textDocument":{"uri":uri}}
        }));
        assert_eq!(reply(&metadata)["result"]["generated"], true);
        assert_eq!(reply(&metadata)["result"]["readOnly"], true);
    }

    #[test]
    fn parses_inline_and_block_javadoc_links() {
        let source = "/** {@link java.util.List#add(Object) label}\n * @see java.util.Map\n */";
        let references = javadoc_references(source);
        assert_eq!(references.len(), 2);
        assert_eq!(references[0].target, "java.util.List#add(Object)");
        assert_eq!(references[1].target, "java.util.Map");
    }

    #[test]
    fn cache_commands_are_advertised_scoped_and_report_status() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut server = Server::with_backend(CacheBackend {
            calls: Arc::clone(&calls),
        });
        let initialized = server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        let commands =
            reply(&initialized)["result"]["capabilities"]["executeCommandProvider"]["commands"]
                .as_array()
                .unwrap();
        assert!(
            commands
                .iter()
                .any(|command| command == "jman.java.rebuildIndex")
        );
        assert!(
            commands
                .iter()
                .any(|command| command == "jman.java.clearWorkspaceCache")
        );

        let status = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"workspace/executeCommand",
            "params":{"command":"jman.java.status","arguments":[]}
        }));
        assert_eq!(reply(&status)["result"]["cache"]["projectId"], "project-1");
        assert_eq!(reply(&status)["result"]["cache"]["structural"]["hits"], 7);
        assert_eq!(reply(&status)["result"]["protocolVersion"], 1);
        assert_eq!(reply(&status)["result"]["workspace"]["buildSystem"], "jman");
        assert_eq!(
            reply(&status)["result"]["workspace"]["nativeOperations"],
            true
        );
        assert_eq!(
            reply(&status)["result"]["workspace"]["buildRuntime"]["javaMajor"],
            21
        );
        assert_eq!(
            reply(&status)["result"]["workspace"]["buildRuntime"]["buildToolVersion"],
            "8.7"
        );

        server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"workspace/executeCommand",
            "params":{"command":"jman.java.rebuildIndex","arguments":[]}
        }));
        server.dispatch(json!({
            "jsonrpc":"2.0","id":4,"method":"workspace/executeCommand",
            "params":{"command":"jman.java.clearWorkspaceCache","arguments":[]}
        }));
        assert_eq!(*calls.lock().unwrap(), ["rebuild", "clear", "rebuild"]);
    }

    #[test]
    fn manual_build_sync_keeps_changes_pending_until_the_command_runs() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut server = Server::with_backend(SyncBackend {
            calls: Arc::clone(&calls),
            fail: Arc::new(Mutex::new(false)),
        });
        let initialized = server.dispatch(json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"initializationOptions":{"buildSync":"manual"}}
        }));
        assert!(
            reply(&initialized)["result"]["capabilities"]["executeCommandProvider"]["commands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|command| command == "jman.java.syncWorkspace")
        );

        let changed = server.dispatch(json!({
            "jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles",
            "params":{"changes":[
                {"uri":"file:///workspace/pom.xml","type":2},
                {"uri":"file:///workspace/README.md","type":2}
            ]}
        }));
        assert_eq!(reply(&changed)["method"], "jman.java/buildSyncStatus");
        assert_eq!(reply(&changed)["params"]["state"], "required");
        assert_eq!(reply(&changed)["params"]["pendingChanges"], 1);
        assert!(calls.lock().unwrap().is_empty());

        let synchronized = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"workspace/executeCommand",
            "params":{"command":"jman.java.syncWorkspace","arguments":[]}
        }));
        let Dispatch::Batch(messages) = synchronized else {
            panic!("sync command should return status notifications and a response");
        };
        assert!(messages.iter().any(|message| message["id"] == 2));
        assert!(messages.iter().any(|message| {
            message["method"] == "jman.java/buildSyncStatus"
                && message["params"]["state"] == "ready"
        }));
        assert_eq!(
            *calls.lock().unwrap(),
            vec![vec!["file:///workspace/pom.xml".to_owned()]]
        );
    }

    #[test]
    fn automatic_build_sync_runs_immediately_and_failure_keeps_last_change_pending() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let fail = Arc::new(Mutex::new(false));
        let mut automatic = Server::with_backend(SyncBackend {
            calls: Arc::clone(&calls),
            fail: Arc::clone(&fail),
        });
        automatic.dispatch(json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"initializationOptions":{"buildSync":"automatic"}}
        }));
        let synchronized = automatic.dispatch(json!({
            "jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles",
            "params":{"changes":[{"uri":"file:///workspace/gradle/libs.versions.toml","type":2}]}
        }));
        let Dispatch::Batch(messages) = synchronized else {
            panic!("automatic sync should publish a completed state");
        };
        assert!(
            messages
                .iter()
                .any(|message| message["params"]["state"] == "ready")
        );

        *fail.lock().unwrap() = true;
        automatic.dispatch(json!({
            "jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles",
            "params":{"changes":[{"uri":"file:///workspace/buildSrc/build.gradle.kts","type":2}]}
        }));
        let status = automatic.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"workspace/executeCommand",
            "params":{"command":"jman.java.status","arguments":[]}
        }));
        assert_eq!(reply(&status)["result"]["buildSync"]["state"], "failed");
        assert_eq!(reply(&status)["result"]["buildSync"]["pendingChanges"], 1);
        assert_eq!(
            reply(&status)["result"]["buildSync"]["error"],
            "synthetic sync failure"
        );
    }

    #[test]
    fn open_and_change_publish_versioned_semantic_diagnostics() {
        let mut server = Server::with_backend(FakeBackend);
        server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        let opened = server.dispatch(json!({
            "jsonrpc":"2.0",
            "method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":"file:///Test.java","version":1,"text":"class Test { }"
            }}
        }));
        assert_eq!(reply(&opened)["params"]["version"], 1);
        assert_eq!(
            reply(&opened)["params"]["diagnostics"][0]["source"],
            "javac"
        );

        let changed = server.dispatch(json!({
            "jsonrpc":"2.0",
            "method":"textDocument/didChange",
            "params":{
                "textDocument":{"uri":"file:///Test.java","version":2},
                "contentChanges":[{"text":"class Test {}"}]
            }
        }));
        assert_eq!(reply(&changed)["params"]["version"], 2);

        let definition = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":"file:///Test.java"},
                "position":{"line":0,"character":7}
            }
        }));
        assert_eq!(reply(&definition)["result"][0]["uri"], "file:///Test.java");

        let declaration = server.dispatch(json!({
            "jsonrpc":"2.0","id":30,"method":"textDocument/declaration",
            "params":{
                "textDocument":{"uri":"file:///Test.java"},
                "position":{"line":0,"character":7}
            }
        }));
        assert_eq!(reply(&declaration)["result"], reply(&definition)["result"]);

        let references = server.dispatch(json!({
            "jsonrpc":"2.0","id":4,"method":"textDocument/references",
            "params":{
                "textDocument":{"uri":"file:///Test.java"},
                "position":{"line":0,"character":7},
                "context":{"includeDeclaration":true}
            }
        }));
        assert_eq!(reply(&references)["result"].as_array().unwrap().len(), 1);

        let symbols = server.dispatch(json!({
            "jsonrpc":"2.0","id":5,"method":"workspace/symbol",
            "params":{"query":"Te"}
        }));
        assert_eq!(reply(&symbols)["result"][0]["name"], "Test");

        let prepare = server.dispatch(json!({
            "jsonrpc":"2.0","id":6,"method":"textDocument/prepareRename",
            "params":{
                "textDocument":{"uri":"file:///Test.java"},
                "position":{"line":0,"character":7}
            }
        }));
        assert_eq!(reply(&prepare)["result"]["placeholder"], "Test");
        assert_eq!(reply(&prepare)["result"]["range"]["start"]["character"], 6);

        let rename = server.dispatch(json!({
            "jsonrpc":"2.0","id":7,"method":"textDocument/rename",
            "params":{
                "textDocument":{"uri":"file:///Test.java"},
                "position":{"line":0,"character":7},
                "newName":"Example"
            }
        }));
        assert_eq!(
            reply(&rename)["result"]["changes"]["file:///Test.java"][0]["newText"],
            "Example"
        );

        let invalid = server.dispatch(json!({
            "jsonrpc":"2.0","id":8,"method":"textDocument/rename",
            "params":{
                "textDocument":{"uri":"file:///Test.java"},
                "position":{"line":0,"character":7},
                "newName":"class"
            }
        }));
        assert_eq!(reply(&invalid)["error"]["code"], -32602);
    }

    #[test]
    fn external_source_cache_files_are_not_analyzed_as_project_sources() {
        let uri = "file:///tmp/io.github.zonnedev.jman.lsp/external-sources/hash/Object.java";
        let mut server = Server::with_backend(FakeBackend);
        server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));

        let opened = server.dispatch(json!({
            "jsonrpc":"2.0",
            "method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":uri,"version":1,"text":"package java.lang; public class Object {}"
            }}
        }));

        assert_eq!(reply(&opened)["method"], "textDocument/publishDiagnostics");
        assert_eq!(reply(&opened)["params"]["uri"], uri);
        assert!(
            reply(&opened)["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(!server.analyses.contains_key(uri));
    }

    #[test]
    fn definitions_can_chain_from_one_external_source_to_another() {
        let uri = "file:///tmp/io.github.zonnedev.jman.lsp/external-sources/hash/Object.java";
        let origin = "file:///workspace/Use.java";
        let source = "public class Object { protected native Object clone() throws CloneNotSupportedException; }";
        let mut server = Server::with_backend(ExternalDefinitionBackend);
        server
            .external_origins
            .insert(uri.to_owned(), origin.to_owned());
        server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":uri,"version":1,"text":source}}
        }));

        let definition = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":uri},
                "position":{"line":0,"character":70}
            }
        }));

        let target = reply(&definition)["result"][0]["uri"].as_str().unwrap();
        assert!(
            target.ends_with("/CloneNotSupportedException.java"),
            "{target}"
        );
        assert_eq!(
            server.external_origins.get(target).map(String::as_str),
            Some(origin)
        );

        let target = target.to_owned();
        let target_source = std::fs::read_to_string(target.trim_start_matches("file://")).unwrap();
        let mut restarted = Server::with_backend(ExternalDefinitionBackend);
        restarted.dispatch(json!({"jsonrpc":"2.0","id":3,"method":"initialize"}));
        restarted.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":target,"version":1,"text":target_source}}
        }));
        assert_eq!(
            restarted.external_origins.get(&target).map(String::as_str),
            Some(origin)
        );
    }

    #[test]
    fn workspace_source_definition_beats_decompiled_classpath_definition() {
        let root = std::env::temp_dir().join(format!(
            "jman-java-workspace-definition-precedence-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let response = root.join("HelloResponse.java");
        std::fs::write(
            &response,
            "package com.example; record HelloResponse(String name) {}",
        )
        .unwrap();
        let response_uri = format!("file://{}", response.display());
        let test_uri = format!("file://{}/HelloResponseTest.java", root.display());
        let test_source = "package com.example; class HelloResponseTest { HelloResponse value; }";
        let mut server = Server::with_backend(WorkspaceShadowBackend {
            source: response.clone(),
        });
        server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":test_uri,"version":1,"text":test_source}}
        }));

        let definition = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":test_uri},
                "position":{"line":0,"character":55}
            }
        }));

        assert_eq!(reply(&definition)["result"][0]["uri"], response_uri);
        assert!(server.external_origins.is_empty());

        let type_definition = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"textDocument/typeDefinition",
            "params":{
                "textDocument":{"uri":test_uri},
                "position":{"line":0,"character":55}
            }
        }));

        assert_eq!(reply(&type_definition)["result"][0]["uri"], response_uri);
        assert!(server.external_origins.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_workspace_index_still_prefers_local_source_over_decompilation() {
        let root = std::env::temp_dir().join(format!(
            "jman-java-pending-definition-precedence-{}",
            std::process::id()
        ));
        let package = root.join("module/src/main/java/com/example");
        std::fs::create_dir_all(&package).unwrap();
        let response = package.join("HelloResponse.java");
        std::fs::write(
            &response,
            "package com.example;\npublic record HelloResponse(String name) {}",
        )
        .unwrap();
        let response_uri = format!("file://{}", response.display());
        let test_uri = format!("file://{}/HelloResponseTest.java", root.display());
        let test_source = "package com.example; class HelloResponseTest { HelloResponse value; }";
        let mut server = Server::with_backend(PendingWorkspaceShadowBackend);
        server.dispatch(json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"rootUri":format!("file://{}", root.display())}
        }));
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":test_uri,"version":1,"text":test_source}}
        }));

        let definition = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":test_uri},
                "position":{"line":0,"character":55}
            }
        }));

        assert_eq!(reply(&definition)["result"][0]["uri"], response_uri);
        assert!(server.external_origins.is_empty());

        let type_definition = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"textDocument/typeDefinition",
            "params":{
                "textDocument":{"uri":test_uri},
                "position":{"line":0,"character":55}
            }
        }));

        assert_eq!(reply(&type_definition)["result"][0]["uri"], response_uri);
        assert!(server.external_origins.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn indexes_closed_workspace_sources_for_cross_file_definition() {
        let root = std::env::temp_dir().join(format!("jman-java-workspace-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("Target.java");
        std::fs::write(&target, "class Target {}").unwrap();
        let target_uri = format!("file://{}", target.display());
        let mut server = Server::with_backend(WorkspaceBackend {
            sources: vec![target],
        });

        server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        server.dispatch(json!({
            "jsonrpc":"2.0",
            "method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":"file:///Use.java",
                "version":1,
                "text":"class Use { Target field; }"
            }}
        }));
        let definition = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":"file:///Use.java"},
                "position":{"line":0,"character":13}
            }
        }));

        assert_eq!(reply(&definition)["result"][0]["uri"], target_uri);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn computes_import_edits_after_package_and_existing_imports() {
        let source = "package demo;\n\nimport java.util.List;\n\nclass Use {}\n";
        let edit = import_edit(source, "java.util.HashMap").unwrap();
        assert_eq!(edit["range"]["start"]["line"], 3);
        assert_eq!(edit["range"]["start"]["character"], 0);
        assert_eq!(edit["newText"], "import java.util.HashMap;\n");
        assert!(import_edit(source, "java.util.List").is_none());
        assert!(import_edit(source, "demo.Model").is_none());
        assert!(import_edit(source, "java.lang.String").is_none());
    }

    #[test]
    fn extracts_missing_type_names_from_javac_diagnostics() {
        assert_eq!(
            missing_type_name("cannot find symbol\n  symbol:   class HashMap"),
            Some("HashMap")
        );
        assert_eq!(
            missing_type_name("cannot find symbol\n  symbol: interface Handler"),
            Some("Handler")
        );
        assert_eq!(missing_type_name("';' expected"), None);
    }

    #[test]
    fn source_actions_generate_members_organize_imports_and_handle_exceptions() {
        let source = "package demo;\n\nimport java.util.Set;\nimport java.util.List;\nimport java.util.Set;\n\nclass Person {\n    private final String name;\n    private int age;\n    private Set<String> tags;\n    void load() {\n        read();\n    }\n}\n";
        let organized = organize_imports_edit(source).unwrap();
        assert_eq!(organized["newText"], "import java.util.Set;\n");
        let actions = generation_actions(source, "file:///Person.java");
        assert_eq!(actions.len(), 3);
        let constructor = actions
            .iter()
            .find(|action| action["title"] == "Generate constructor for Person")
            .unwrap()["edit"]["changes"]["file:///Person.java"][0]["newText"]
            .as_str()
            .unwrap();
        assert!(constructor.contains("Person(String name, int age, Set<String> tags)"));
        assert!(constructor.contains("this.name = name;"));
        let accessors = actions
            .iter()
            .find(|action| action["title"] == "Generate getters and setters")
            .unwrap()["edit"]["changes"]["file:///Person.java"][0]["newText"]
            .as_str()
            .unwrap();
        assert!(accessors.contains("getName()"));
        assert!(!accessors.contains("setName("));
        assert!(accessors.contains("setAge(int age)"));

        let diagnostic = json!({
            "range": {
                "start": {"line": 11, "character": 8},
                "end": {"line": 11, "character": 12}
            }
        });
        let throws = add_throws_edit(source, &diagnostic, "java.io.IOException").unwrap();
        assert_eq!(throws["newText"], " throws java.io.IOException");
        let wrapped =
            surround_with_try_catch_edit(source, &diagnostic, "java.io.IOException").unwrap();
        assert!(
            wrapped["newText"]
                .as_str()
                .unwrap()
                .contains("catch (java.io.IOException exception)")
        );
        assert_eq!(
            unreported_exception(
                "unreported exception java.io.IOException; must be caught or declared to be thrown"
            ),
            Some("java.io.IOException")
        );
    }

    #[test]
    fn change_signature_argument_parser_preserves_nested_expressions() {
        let source = "service.run(first, factory.create(\"a,b\", 2), values.get(0));";
        let (_, _, arguments) =
            parenthesized_after(source, source.find("run").unwrap() + 3).unwrap();
        assert_eq!(
            arguments,
            ["first", " factory.create(\"a,b\", 2)", " values.get(0)"]
        );
        assert!(parenthesized_after("this::run", 9).is_none());
    }

    #[test]
    fn maps_canonical_method_ids_to_structural_workspace_ids() {
        assert_eq!(
            structural_method_identity(
                "<unnamed>|com/example/TransactionManagementPort#executeWrite(Ljava/util/function/Supplier;)Ljava/lang/Object;"
            ),
            Some("com.example.TransactionManagementPort#executeWrite".to_owned())
        );
        assert_eq!(
            structural_method_identity("demo.module|com/example/Outer$Inner#run()V"),
            Some("com.example.Outer.Inner#run".to_owned())
        );
        assert_eq!(
            structural_method_identity("<unnamed>|Lcom/example/TransactionManagementPort;"),
            None
        );
    }

    #[test]
    fn on_demand_semantics_select_exact_identifier_candidates() {
        assert!(contains_java_identifier(
            "port::executeWrite",
            "executeWrite"
        ));
        assert!(contains_java_identifier(
            "port.executeWrite(action)",
            "executeWrite"
        ));
        assert!(!contains_java_identifier(
            "port.executeWriter(action)",
            "executeWrite"
        ));
        assert!(!contains_java_identifier(
            "port.myexecuteWrite(action)",
            "executeWrite"
        ));

        let mut server = Server::with_backend(FakeBackend);
        server.indexed_sources.insert(
            "file:///Call.java".to_owned(),
            "class Call { void run() { port.executeWrite(action); } }".to_owned(),
        );
        server.indexed_sources.insert(
            "file:///MethodReference.java".to_owned(),
            "class MethodReference { Object call = port::executeWrite; }".to_owned(),
        );
        server.indexed_sources.insert(
            "file:///Different.java".to_owned(),
            "class Different { void executeWriter() {} }".to_owned(),
        );
        for document in ["file:///Call.java", "file:///MethodReference.java"] {
            server.analyses.insert(
                document.to_owned(),
                SemanticResult {
                    package_name: String::new(),
                    diagnostics: Vec::new(),
                    symbols: Vec::new(),
                },
            );
        }

        server.ensure_workspace_symbol_semantics("executeWrite");

        assert!(server.analyses.contains_key("file:///Call.java"));
        assert!(server.analyses.contains_key("file:///MethodReference.java"));
        assert!(!server.analyses.contains_key("file:///Different.java"));
        assert!(server.project.contains_document("file:///Call.java"));
        assert!(
            server
                .project
                .contains_document("file:///MethodReference.java")
        );
        assert!(!server.project.contains_document("file:///Different.java"));
    }

    #[test]
    fn references_refine_closed_candidates_and_follow_override_families() {
        let mut server = Server::with_backend(OverrideReferencesBackend);
        server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":OVERRIDE_INTERFACE_URI,
                "languageId":"java",
                "version":1,
                "text":OVERRIDE_INTERFACE_SOURCE
            }}
        }));

        assert!(!server.project.contains_document(OVERRIDE_CALLER_URI));
        let references = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/references",
            "params":{
                "textDocument":{"uri":OVERRIDE_INTERFACE_URI},
                "position":{
                    "line":0,
                    "character":OVERRIDE_INTERFACE_SOURCE.find("executeWrite").unwrap() + 1
                },
                "context":{"includeDeclaration":false}
            }
        }));

        assert!(
            server
                .project
                .contains_document(OVERRIDE_IMPLEMENTATION_URI)
        );
        assert!(server.project.contains_document(OVERRIDE_CALLER_URI));
        assert_eq!(reply(&references)["result"].as_array().unwrap().len(), 1);
        assert_eq!(reply(&references)["result"][0]["uri"], OVERRIDE_CALLER_URI);

        let with_declarations = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"textDocument/references",
            "params":{
                "textDocument":{"uri":OVERRIDE_INTERFACE_URI},
                "position":{
                    "line":0,
                    "character":OVERRIDE_INTERFACE_SOURCE.find("executeWrite").unwrap() + 1
                },
                "context":{"includeDeclaration":true}
            }
        }));
        let locations = reply(&with_declarations)["result"].as_array().unwrap();
        assert_eq!(locations.len(), 3);
        assert!(
            locations
                .iter()
                .any(|location| location["uri"] == OVERRIDE_INTERFACE_URI)
        );
        assert!(
            locations
                .iter()
                .any(|location| location["uri"] == OVERRIDE_IMPLEMENTATION_URI)
        );
        assert!(
            locations
                .iter()
                .any(|location| location["uri"] == OVERRIDE_CALLER_URI)
        );
    }

    #[test]
    fn declaration_of_semantic_override_finds_structural_interface_method() {
        let interface_uri = "file:///TransactionManagementPort.java";
        let implementation_uri = "file:///JdbcTransactionManagementAdapter.java";
        let interface_source = "package com.example; interface TransactionManagementPort { <T> T executeWrite(java.util.function.Supplier<T> action); }";
        let implementation_source = "package com.example; class JdbcTransactionManagementAdapter implements TransactionManagementPort { public <T> T executeWrite(java.util.function.Supplier<T> action) { return action.get(); } void use() { executeWrite(() -> \"x\"); } }";
        let interface_method = "com.example.TransactionManagementPort#executeWrite";
        let canonical_interface_method = "<unnamed>|com/example/TransactionManagementPort#executeWrite(Ljava/util/function/Supplier;)Ljava/lang/Object;";
        let implementation_method = "<unnamed>|com/example/JdbcTransactionManagementAdapter#executeWrite(Ljava/util/function/Supplier;)Ljava/lang/Object;";
        let interface_start = interface_source.find("executeWrite").unwrap() as u64;
        let implementation_start = implementation_source.find("executeWrite").unwrap() as u64;
        let mut server = Server::with_backend(FakeBackend);
        server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        for (uri, source) in [
            (interface_uri, interface_source),
            (implementation_uri, implementation_source),
        ] {
            server.dispatch(json!({
                "jsonrpc":"2.0","method":"textDocument/didOpen",
                "params":{"textDocument":{"uri":uri,"version":1,"text":source}}
            }));
        }
        server.structural_project.update_document(
            interface_uri,
            &SemanticResult {
                package_name: "com.example".to_owned(),
                diagnostics: Vec::new(),
                symbols: vec![SemanticSymbol {
                    role: "declaration".to_owned(),
                    kind: "method".to_owned(),
                    name: "executeWrite".to_owned(),
                    qualified_name: interface_method.to_owned(),
                    symbol_id: interface_method.to_owned(),
                    start: interface_start,
                    end: interface_start + "executeWrite".len() as u64,
                }],
            },
        );
        server.project.update_document(
            implementation_uri,
            &SemanticResult {
                package_name: "com.example".to_owned(),
                diagnostics: Vec::new(),
                symbols: vec![
                    SemanticSymbol {
                        role: "declaration".to_owned(),
                        kind: "method".to_owned(),
                        name: "executeWrite".to_owned(),
                        qualified_name: "com.example.JdbcTransactionManagementAdapter#executeWrite"
                            .to_owned(),
                        symbol_id: implementation_method.to_owned(),
                        start: implementation_start,
                        end: implementation_start + "executeWrite".len() as u64,
                    },
                    SemanticSymbol {
                        role: "override_family".to_owned(),
                        kind: "method".to_owned(),
                        name: "executeWrite".to_owned(),
                        qualified_name: canonical_interface_method.to_owned(),
                        symbol_id: implementation_method.to_owned(),
                        start: implementation_start,
                        end: implementation_start + "executeWrite".len() as u64,
                    },
                    SemanticSymbol {
                        role: "reference".to_owned(),
                        kind: "method".to_owned(),
                        name: "executeWrite".to_owned(),
                        qualified_name: "com.example.JdbcTransactionManagementAdapter#executeWrite"
                            .to_owned(),
                        symbol_id: implementation_method.to_owned(),
                        start: implementation_source
                            .match_indices("executeWrite")
                            .nth(1)
                            .unwrap()
                            .0 as u64,
                        end: implementation_source
                            .match_indices("executeWrite")
                            .nth(1)
                            .unwrap()
                            .0 as u64
                            + "executeWrite".len() as u64,
                    },
                ],
            },
        );

        let position = implementation_source.find("executeWrite").unwrap() + 1;
        let definition = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":implementation_uri},
                "position":{"line":0,"character":position}
            }
        }));
        assert_eq!(reply(&definition)["result"][0]["uri"], implementation_uri);

        let declaration = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"textDocument/declaration",
            "params":{
                "textDocument":{"uri":implementation_uri},
                "position":{"line":0,"character":position}
            }
        }));
        assert_eq!(reply(&declaration)["result"][0]["uri"], interface_uri);

        let references = server.dispatch(json!({
            "jsonrpc":"2.0","id":4,"method":"textDocument/references",
            "params":{
                "textDocument":{"uri":interface_uri},
                "position":{"line":0,"character":interface_source.find("executeWrite").unwrap() + 1},
                "context":{"includeDeclaration":false}
            }
        }));
        assert_eq!(reply(&references)["result"].as_array().unwrap().len(), 1);
        assert_eq!(reply(&references)["result"][0]["uri"], implementation_uri);
    }

    #[test]
    fn change_signature_rewrites_override_family_declarations_and_calls() {
        let service_uri = "file:///Service.java";
        let impl_uri = "file:///Impl.java";
        let service_source = "interface Service { void run(String name, int count); }";
        let impl_source = "class Impl implements Service { public void run(String name, int count) {} void use(){ run(\"x\", 2); } }";
        let service_id = "<unnamed>|Service#run(Ljava/lang/String;I)V";
        let impl_id = "<unnamed>|Impl#run(Ljava/lang/String;I)V";
        let use_id = "<unnamed>|Impl#use()V";
        let service_type_id = "<unnamed>|LService;";
        let impl_type_id = "<unnamed>|LImpl;";
        let symbol =
            |role: &str, source: &str, occurrence: usize, symbol_id: &str, qualified_name: &str| {
                let start = source.match_indices("run").nth(occurrence).unwrap().0 as u64;
                SemanticSymbol {
                    role: role.to_owned(),
                    kind: "method".to_owned(),
                    name: "run".to_owned(),
                    qualified_name: qualified_name.to_owned(),
                    symbol_id: symbol_id.to_owned(),
                    start,
                    end: start + 3,
                }
            };
        let mut server = Server::with_backend(FakeBackend);
        server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        for (uri, source) in [(service_uri, service_source), (impl_uri, impl_source)] {
            server.dispatch(json!({
                "jsonrpc":"2.0","method":"textDocument/didOpen",
                "params":{"textDocument":{"uri":uri,"version":1,"text":source}}
            }));
        }
        server.project.update_document(
            service_uri,
            &SemanticResult {
                package_name: String::new(),
                diagnostics: Vec::new(),
                symbols: vec![
                    SemanticSymbol {
                        role: "declaration".to_owned(),
                        kind: "interface".to_owned(),
                        name: "Service".to_owned(),
                        qualified_name: "Service".to_owned(),
                        symbol_id: service_type_id.to_owned(),
                        start: service_source.find("Service").unwrap() as u64,
                        end: service_source.find("Service").unwrap() as u64 + 7,
                    },
                    symbol("declaration", service_source, 0, service_id, "Service#run"),
                    symbol("override_family", service_source, 0, service_id, service_id),
                ],
            },
        );
        server.project.update_document(
            impl_uri,
            &SemanticResult {
                package_name: String::new(),
                diagnostics: Vec::new(),
                symbols: vec![
                    SemanticSymbol {
                        role: "declaration".to_owned(),
                        kind: "class".to_owned(),
                        name: "Impl".to_owned(),
                        qualified_name: "Impl".to_owned(),
                        symbol_id: impl_type_id.to_owned(),
                        start: impl_source.find("Impl").unwrap() as u64,
                        end: impl_source.find("Impl").unwrap() as u64 + 4,
                    },
                    SemanticSymbol {
                        role: "type_edge".to_owned(),
                        kind: "type".to_owned(),
                        name: "Service".to_owned(),
                        qualified_name: impl_type_id.to_owned(),
                        symbol_id: service_type_id.to_owned(),
                        start: impl_source.find("Impl").unwrap() as u64,
                        end: impl_source.find("Impl").unwrap() as u64 + 4,
                    },
                    symbol("declaration", impl_source, 0, impl_id, "Impl#run"),
                    symbol("override_family", impl_source, 0, impl_id, service_id),
                    symbol("reference", impl_source, 1, impl_id, "Impl#run"),
                    symbol("override_family", impl_source, 1, impl_id, service_id),
                    SemanticSymbol {
                        role: "declaration".to_owned(),
                        kind: "method".to_owned(),
                        name: "use".to_owned(),
                        qualified_name: "Impl#use".to_owned(),
                        symbol_id: use_id.to_owned(),
                        start: impl_source.find("use").unwrap() as u64,
                        end: impl_source.find("use").unwrap() as u64 + 3,
                    },
                    SemanticSymbol {
                        role: "call_edge".to_owned(),
                        kind: "method".to_owned(),
                        name: "run".to_owned(),
                        qualified_name: use_id.to_owned(),
                        symbol_id: impl_id.to_owned(),
                        start: impl_source.match_indices("run").nth(1).unwrap().0 as u64,
                        end: impl_source.match_indices("run").nth(1).unwrap().0 as u64 + 3,
                    },
                ],
            },
        );
        let definition = server.dispatch(json!({
            "jsonrpc":"2.0","id":8,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":impl_uri},
                "position":{"line":0,"character":impl_source.find("run").unwrap() + 1}
            }
        }));
        assert_eq!(reply(&definition)["result"][0]["uri"], impl_uri);

        let declaration = server.dispatch(json!({
            "jsonrpc":"2.0","id":9,"method":"textDocument/declaration",
            "params":{
                "textDocument":{"uri":impl_uri},
                "position":{"line":0,"character":impl_source.find("run").unwrap() + 1}
            }
        }));
        assert_eq!(reply(&declaration)["result"][0]["uri"], service_uri);

        let references = server.dispatch(json!({
            "jsonrpc":"2.0","id":91,"method":"textDocument/references",
            "params":{
                "textDocument":{"uri":service_uri},
                "position":{"line":0,"character":service_source.find("run").unwrap() + 1},
                "context":{"includeDeclaration":false}
            }
        }));
        assert_eq!(reply(&references)["result"].as_array().unwrap().len(), 1);
        assert_eq!(reply(&references)["result"][0]["uri"], impl_uri);

        let implementation = server.dispatch(json!({
            "jsonrpc":"2.0","id":10,"method":"textDocument/implementation",
            "params":{
                "textDocument":{"uri":service_uri},
                "position":{"line":0,"character":25}
            }
        }));
        assert_eq!(
            reply(&implementation)["result"].as_array().unwrap().len(),
            1
        );
        assert_eq!(reply(&implementation)["result"][0]["uri"], impl_uri);

        let prepared = server.dispatch(json!({
            "jsonrpc":"2.0","id":11,"method":"textDocument/prepareCallHierarchy",
            "params":{
                "textDocument":{"uri":impl_uri},
                "position":{"line":0,"character":impl_source.find("use").unwrap() + 1}
            }
        }));
        let use_item = reply(&prepared)["result"][0].clone();
        assert_eq!(use_item["data"]["symbolId"], use_id);
        let outgoing = server.dispatch(json!({
            "jsonrpc":"2.0","id":12,"method":"callHierarchy/outgoingCalls",
            "params":{"item":use_item}
        }));
        assert_eq!(reply(&outgoing)["result"][0]["to"]["name"], "run");
        let run_item = reply(&outgoing)["result"][0]["to"].clone();
        let incoming = server.dispatch(json!({
            "jsonrpc":"2.0","id":13,"method":"callHierarchy/incomingCalls",
            "params":{"item":run_item}
        }));
        assert_eq!(reply(&incoming)["result"][0]["from"]["name"], "use");
        let hints = server.dispatch(json!({
            "jsonrpc":"2.0","id":17,"method":"textDocument/inlayHint",
            "params":{
                "textDocument":{"uri":impl_uri},
                "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":impl_source.len()}}
            }
        }));
        assert_eq!(reply(&hints)["result"][0]["label"], "name:");
        assert_eq!(reply(&hints)["result"][1]["label"], "count:");

        let prepared_type = server.dispatch(json!({
            "jsonrpc":"2.0","id":14,"method":"textDocument/prepareTypeHierarchy",
            "params":{
                "textDocument":{"uri":service_uri},
                "position":{"line":0,"character":service_source.find("Service").unwrap() + 1}
            }
        }));
        let service_item = reply(&prepared_type)["result"][0].clone();
        let subtypes = server.dispatch(json!({
            "jsonrpc":"2.0","id":15,"method":"typeHierarchy/subtypes",
            "params":{"item":service_item}
        }));
        assert_eq!(reply(&subtypes)["result"][0]["name"], "Impl");
        let impl_item = reply(&subtypes)["result"][0].clone();
        let supertypes = server.dispatch(json!({
            "jsonrpc":"2.0","id":16,"method":"typeHierarchy/supertypes",
            "params":{"item":impl_item}
        }));
        assert_eq!(reply(&supertypes)["result"][0]["name"], "Service");

        let response = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"jman.java/changeSignature",
            "params":{
                "textDocument":{"uri":service_uri},
                "position":{"line":0,"character":25},
                "newParameters":["int count","String label"],
                "argumentOrder":[1,0]
            }
        }));
        let result = &reply(&response)["result"]["changes"];
        assert_eq!(result[service_uri][0]["newText"], "int count, String label");
        assert_eq!(result[impl_uri][0]["newText"], "int count, String label");
        assert_eq!(result[impl_uri][1]["newText"], "2, \"x\"");
    }

    #[test]
    fn generated_members_compile_with_javac() {
        let source =
            "public class Person {\n    private final String name;\n    private int age;\n}\n";
        let actions = generation_actions(source, "file:///Person.java");
        let mut members = String::new();
        for action in &actions {
            members.push_str(
                action["edit"]["changes"]["file:///Person.java"][0]["newText"]
                    .as_str()
                    .unwrap(),
            );
        }
        let body_end = source.rfind('}').unwrap();
        let generated = format!("{}{}{}", &source[..body_end], members, &source[body_end..]);
        let root = std::env::temp_dir().join(format!(
            "jman-java-generated-members-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("Person.java");
        std::fs::write(&file, generated).unwrap();
        let javac = std::env::var_os("JAVA_HOME")
            .map(PathBuf::from)
            .map(|home| home.join("bin/javac"))
            .unwrap_or_else(|| PathBuf::from("javac"));
        let output = std::process::Command::new(javac)
            .arg("-d")
            .arg(&root)
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removes_only_the_invalid_override_annotation() {
        let source = "class Demo {\n  @Override\n  void value() {}\n  void other() {}\n}\n";
        let edit = remove_override_edit(
            source,
            &json!({
                "range": {
                    "start": {"line": 2, "character": 7},
                    "end": {"line": 2, "character": 12}
                }
            }),
        )
        .unwrap();
        assert_eq!(edit["range"]["start"], json!({"line": 1, "character": 2}));
        assert_eq!(edit["range"]["end"], json!({"line": 2, "character": 0}));
        assert_eq!(edit["newText"], "");
    }

    #[test]
    fn module_descriptors_support_navigation_completion_and_quick_fixes() {
        let root =
            std::env::temp_dir().join(format!("jman-java-jpms-editor-{}", std::process::id()));
        let current = root.join("current/src/main/java");
        let target = root.join("target/src/main/java");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        let current_descriptor = current.join("module-info.java");
        let target_descriptor = target.join("module-info.java");
        let service = current.join("demo/current/api/DemoService.java");
        let provider = current.join("demo/current/internal/DemoProvider.java");
        std::fs::create_dir_all(service.parent().unwrap()).unwrap();
        std::fs::create_dir_all(provider.parent().unwrap()).unwrap();
        std::fs::write(
            &service,
            "package demo.current.api; public interface DemoService {}",
        )
        .unwrap();
        std::fs::write(
            &provider,
            "package demo.current.internal; public class DemoProvider {}",
        )
        .unwrap();
        std::fs::write(
            &current_descriptor,
            "module demo.current {\n  requires demo.target;\n}\n",
        )
        .unwrap();
        std::fs::write(
            &target_descriptor,
            "module demo.target {\n  exports demo.target.api;\n}\n",
        )
        .unwrap();
        let current_uri = format!("file://{}", current_descriptor.display());
        let target_uri = format!("file://{}", target_descriptor.display());
        let mut server = Server::with_backend(JpmsBackend {
            sources: vec![
                current_descriptor.clone(),
                target_descriptor,
                service,
                provider,
            ],
        });
        server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        let current_source = std::fs::read_to_string(&current_descriptor).unwrap();
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":current_uri,"version":1,"text":current_source}}
        }));

        let definition = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":current_uri},
                "position":{"line":1,"character":15}
            }
        }));
        assert_eq!(reply(&definition)["result"][0]["uri"], target_uri);

        let completion_source = "module demo.current {\n  requires demo.\n}";
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didChange",
            "params":{
                "textDocument":{"uri":current_uri,"version":2},
                "contentChanges":[{"text":completion_source}]
            }
        }));
        let completion = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"textDocument/completion",
            "params":{
                "textDocument":{"uri":current_uri},
                "position":{"line":1,"character":16}
            }
        }));
        assert!(
            reply(&completion)["result"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["label"] == "demo.target")
        );

        let qualified_target_source =
            "module demo.current {\n  exports demo.current.api to demo.target;\n}";
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didChange",
            "params":{
                "textDocument":{"uri":current_uri,"version":3},
                "contentChanges":[{"text":qualified_target_source}]
            }
        }));
        let qualified_definition = server.dispatch(json!({
            "jsonrpc":"2.0","id":20,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":current_uri},
                "position":{"line":1,"character":35}
            }
        }));
        assert_eq!(reply(&qualified_definition)["result"][0]["uri"], target_uri);

        for (index, (text, expected)) in [
            (
                "module demo.current {\n  exports demo.current.\n}",
                "demo.current.api",
            ),
            (
                "module demo.current {\n  uses demo.current.api.\n}",
                "demo.current.api.DemoService",
            ),
            (
                "module demo.current {\n  provides demo.current.api.DemoService with demo.current.internal.\n}",
                "demo.current.internal.DemoProvider",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            server.dispatch(json!({
                "jsonrpc":"2.0","method":"textDocument/didChange",
                "params":{
                    "textDocument":{"uri":current_uri,"version":4 + index},
                    "contentChanges":[{"text":text}]
                }
            }));
            let character = text.lines().nth(1).unwrap().len();
            let completion = server.dispatch(json!({
                "jsonrpc":"2.0","id":30,"method":"textDocument/completion",
                "params":{
                    "textDocument":{"uri":current_uri},
                    "position":{"line":1,"character":character}
                }
            }));
            assert!(
                reply(&completion)["result"]["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["label"] == expected),
                "{text}: {}",
                reply(&completion)
            );
        }

        let use_uri = format!("file://{}", current.join("demo/current/Use.java").display());
        let diagnostic = json!({
            "code":"compiler.err.package.not.visible",
            "message":"package demo.extra.api is not visible\n  (package demo.extra.api is declared in module demo.extra, but module demo.current does not read it)",
            "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}}
        });
        let actions = server.dispatch(json!({
            "jsonrpc":"2.0","id":4,"method":"textDocument/codeAction",
            "params":{
                "textDocument":{"uri":use_uri},
                "context":{"diagnostics":[diagnostic]}
            }
        }));
        let titles: Vec<_> = reply(&actions)["result"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|action| action["title"].as_str())
            .collect();
        assert!(titles.contains(&"Add requires demo.extra"));
        assert!(titles.contains(&"Add requires transitive demo.extra"));

        let export_diagnostic = json!({
            "code":"compiler.err.package.not.visible",
            "message":"package demo.target.secret is not visible\n  (package demo.target.secret is declared in module demo.target, which does not export it)",
            "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}}
        });
        let export_actions = server.dispatch(json!({
            "jsonrpc":"2.0","id":5,"method":"textDocument/codeAction",
            "params":{
                "textDocument":{"uri":use_uri},
                "context":{"diagnostics":[export_diagnostic]}
            }
        }));
        let export_titles: Vec<_> = reply(&export_actions)["result"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|action| action["title"].as_str())
            .collect();
        assert!(export_titles.contains(&"Add exports demo.target.secret; to demo.target"));
        assert!(export_titles.contains(&"Add opens demo.target.secret; to demo.target"));

        let unnamed_diagnostic = json!({
            "code":"compiler.err.package.not.visible",
            "message":"package demo.legacy is not visible\n  (package demo.legacy is declared in the unnamed module, but module demo.current does not read it)",
            "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}}
        });
        let unnamed_actions = server.dispatch(json!({
            "jsonrpc":"2.0","id":6,"method":"textDocument/codeAction",
            "params":{
                "textDocument":{"uri":use_uri},
                "context":{"diagnostics":[unnamed_diagnostic]}
            }
        }));
        let move_action = reply(&unnamed_actions)["result"]
            .as_array()
            .unwrap()
            .iter()
            .find(|action| {
                action["title"] == "Move the dependency from the classpath to the module path"
            })
            .unwrap();
        assert!(
            move_action["disabled"]["reason"]
                .as_str()
                .unwrap()
                .contains("Maven or Gradle")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completion_and_quickfix_add_an_unambiguous_type_import() {
        let uri = "file:///Use.java";
        let source = "package demo;\nclass Use { HashM value; }\n";
        let mut server = Server::with_backend(FakeBackend);
        server.dispatch(json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"capabilities":{"textDocument":{"completion":{"completionItem":{
                "resolveSupport":{"properties":["additionalTextEdits"]}
            }}}}}
        }));
        server.structural_project.update_document(
            "jrt:/java.base/java/util/HashMap.java",
            &SemanticResult {
                package_name: "java.util".to_owned(),
                symbols: vec![javac_frontend::SemanticSymbol {
                    role: "declaration".to_owned(),
                    kind: "class".to_owned(),
                    name: "HashMap".to_owned(),
                    qualified_name: "java.util.HashMap".to_owned(),
                    symbol_id: "java.base|java/util/HashMap".to_owned(),
                    start: 0,
                    end: 7,
                }],
                diagnostics: vec![],
            },
        );
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":uri,"version":1,"text":source}}
        }));
        let completion = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/completion",
            "params":{"textDocument":{"uri":uri},"position":{"line":1,"character":18}}
        }));
        let item = reply(&completion)["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["label"] == "HashMap")
            .unwrap()
            .clone();
        assert!(item.get("additionalTextEdits").is_none());
        let resolved = server.dispatch(json!({
            "jsonrpc":"2.0","id":4,"method":"completionItem/resolve","params":item
        }));
        assert_eq!(
            reply(&resolved)["result"]["additionalTextEdits"][0]["newText"],
            "import java.util.HashMap;\n"
        );

        let actions = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"textDocument/codeAction",
            "params":{
                "textDocument":{"uri":uri},
                "range":{"start":{"line":1,"character":12},"end":{"line":1,"character":17}},
                "context":{"diagnostics":[{
                    "range":{"start":{"line":1,"character":12},"end":{"line":1,"character":19}},
                    "code":"compiler.err.cant.resolve.location",
                    "message":"cannot find symbol\n  symbol:   class HashMap"
                }]}
            }
        }));
        assert!(
            reply(&actions)["result"]
                .as_array()
                .unwrap()
                .iter()
                .any(|action| {
                    action["title"] == "Import java.util.HashMap"
                        && action["edit"]["changes"][uri][0]["newText"]
                            == "import java.util.HashMap;\n"
                })
        );
    }

    #[test]
    fn serves_completion_hover_signature_tokens_and_quickfixes() {
        let uri = "file:///Editor.java";
        let source = "class Editor { void run() { run(); } }";
        let mut server = Server::with_backend(EditorBackend);
        let initialized = server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        let capabilities = &reply(&initialized)["result"]["capabilities"];
        assert!(capabilities["completionProvider"].is_object());
        assert_eq!(capabilities["hoverProvider"], true);
        assert_eq!(capabilities["declarationProvider"], true);
        assert_eq!(capabilities["implementationProvider"], true);
        assert_eq!(capabilities["typeDefinitionProvider"], true);
        assert_eq!(capabilities["callHierarchyProvider"], true);
        assert_eq!(capabilities["typeHierarchyProvider"], true);
        assert_eq!(capabilities["documentSymbolProvider"], true);
        assert_eq!(capabilities["documentHighlightProvider"], true);
        assert_eq!(capabilities["documentFormattingProvider"], true);
        assert!(capabilities["documentRangeFormattingProvider"].is_null());
        assert!(capabilities["documentOnTypeFormattingProvider"].is_null());
        assert_eq!(capabilities["inlayHintProvider"], true);
        assert_eq!(capabilities["selectionRangeProvider"], true);
        assert_eq!(capabilities["foldingRangeProvider"], true);
        assert!(capabilities["signatureHelpProvider"].is_object());
        assert!(capabilities["semanticTokensProvider"].is_object());
        assert!(capabilities["codeActionProvider"].is_object());
        assert_eq!(
            capabilities["executeCommandProvider"]["commands"][0],
            "jman.java.status"
        );
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":uri,"version":1,"text":source}}
        }));

        let outline = server.dispatch(json!({
            "jsonrpc":"2.0","id":8,"method":"textDocument/documentSymbol",
            "params":{"textDocument":{"uri":uri}}
        }));
        let symbols = reply(&outline)["result"].as_array().unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0]["name"], "Editor");
        assert_eq!(symbols[0]["kind"], 5);
        assert_eq!(symbols[0]["children"][0]["name"], "run");
        assert_eq!(symbols[0]["children"][0]["kind"], 6);

        let completion = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/completion",
            "params":{"textDocument":{"uri":uri},"position":{"line":0,"character":35}}
        }));
        assert!(
            reply(&completion)["result"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["label"] == "add"
                    && item["detail"].as_str().unwrap().contains("boolean"))
        );

        let type_definition = server.dispatch(json!({
            "jsonrpc":"2.0","id":9,"method":"textDocument/typeDefinition",
            "params":{"textDocument":{"uri":uri},"position":{"line":0,"character":7}}
        }));
        assert_eq!(reply(&type_definition)["result"][0]["uri"], uri);
        assert!(
            reply(&completion)["result"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["documentation"]["value"] == "Adds an element.")
        );

        let hover = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"textDocument/hover",
            "params":{"textDocument":{"uri":uri},"position":{"line":0,"character":7}}
        }));
        assert!(
            reply(&hover)["result"]["contents"]["value"]
                .as_str()
                .unwrap()
                .contains("Editor documentation.")
        );

        let signature = server.dispatch(json!({
            "jsonrpc":"2.0","id":4,"method":"textDocument/signatureHelp",
            "params":{"textDocument":{"uri":uri},"position":{"line":0,"character":34}}
        }));
        assert!(
            reply(&signature)["result"]["signatures"][0]["label"]
                .as_str()
                .unwrap()
                .contains("run(java.lang.String value)")
        );
        assert_eq!(
            reply(&signature)["result"]["signatures"][0]["parameters"][0]["label"],
            "java.lang.String value"
        );
        assert_eq!(
            reply(&signature)["result"]["signatures"][0]["documentation"]["value"],
            "Runs the operation."
        );

        let tokens = server.dispatch(json!({
            "jsonrpc":"2.0","id":5,"method":"textDocument/semanticTokens/full",
            "params":{"textDocument":{"uri":uri}}
        }));
        assert!(reply(&tokens)["result"]["data"].as_array().unwrap().len() >= 10);

        let actions = server.dispatch(json!({
            "jsonrpc":"2.0","id":6,"method":"textDocument/codeAction",
            "params":{
                "textDocument":{"uri":uri},
                "range":{"start":{"line":0,"character":39},"end":{"line":0,"character":39}},
                "context":{"diagnostics":[{
                    "range":{"start":{"line":0,"character":39},"end":{"line":0,"character":39}},
                    "code":"compiler.err.expected",
                    "message":"';' expected"
                }]}
            }
        }));
        assert_eq!(reply(&actions)["result"][0]["title"], "Insert missing ';'");

        let status = server.dispatch(json!({
            "jsonrpc":"2.0","id":7,"method":"workspace/executeCommand",
            "params":{"command":"jman.java.status","arguments":[]}
        }));
        assert_eq!(reply(&status)["result"]["state"], "ready");
        assert_eq!(reply(&status)["result"]["openDocuments"], 1);
        assert_eq!(reply(&status)["result"]["semanticDocuments"], 1);
    }

    #[test]
    fn lazily_resolves_completion_and_code_action_properties() {
        let uri = "file:///Editor.java";
        let source = "class Editor { void run() { run(); } }";
        let mut server = Server::with_backend(EditorBackend);
        let initialized = server.dispatch(json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"capabilities":{"textDocument":{
                "completion":{"completionItem":{"resolveSupport":{"properties":[
                    "detail", "documentation", "additionalTextEdits"
                ]}}},
                "codeAction":{"resolveSupport":{"properties":["edit"]}}
            }}}
        }));
        let capabilities = &reply(&initialized)["result"]["capabilities"];
        assert_eq!(capabilities["completionProvider"]["resolveProvider"], true);
        assert_eq!(capabilities["codeActionProvider"]["resolveProvider"], true);
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":uri,"version":1,"text":source}}
        }));

        let completion = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/completion",
            "params":{"textDocument":{"uri":uri},"position":{"line":0,"character":35}}
        }));
        let item = reply(&completion)["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["label"] == "add")
            .unwrap()
            .clone();
        assert!(item.get("detail").is_none());
        assert!(item.get("documentation").is_none());
        assert_eq!(item["data"]["jmanResolve"]["kind"], "completion");
        let resolved = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"completionItem/resolve","params":item
        }));
        assert!(
            reply(&resolved)["result"]["detail"]
                .as_str()
                .unwrap()
                .contains("boolean")
        );
        assert_eq!(
            reply(&resolved)["result"]["documentation"]["value"],
            "Adds an element."
        );

        let actions = server.dispatch(json!({
            "jsonrpc":"2.0","id":4,"method":"textDocument/codeAction",
            "params":{
                "textDocument":{"uri":uri},
                "range":{"start":{"line":0,"character":39},"end":{"line":0,"character":39}},
                "context":{"diagnostics":[{
                    "range":{"start":{"line":0,"character":39},"end":{"line":0,"character":39}},
                    "code":"compiler.err.expected","message":"';' expected"
                }]}
            }
        }));
        let action = reply(&actions)["result"][0].clone();
        assert!(action.get("edit").is_none());
        assert_eq!(action["data"]["jmanResolve"]["kind"], "codeAction");
        let resolved = server.dispatch(json!({
            "jsonrpc":"2.0","id":5,"method":"codeAction/resolve","params":action
        }));
        assert_eq!(
            reply(&resolved)["result"]["edit"]["changes"][uri][0]["newText"],
            ";"
        );

        let actions = server.dispatch(json!({
            "jsonrpc":"2.0","id":6,"method":"textDocument/codeAction",
            "params":{
                "textDocument":{"uri":uri},
                "context":{"diagnostics":[{
                    "range":{"start":{"line":0,"character":39},"end":{"line":0,"character":39}},
                    "code":"compiler.err.expected","message":"';' expected"
                }]}
            }
        }));
        let stale_action = reply(&actions)["result"][0].clone();
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didChange",
            "params":{
                "textDocument":{"uri":uri,"version":2},
                "contentChanges":[{"text":source}]
            }
        }));
        let stale = server.dispatch(json!({
            "jsonrpc":"2.0","id":7,"method":"codeAction/resolve","params":stale_action
        }));
        assert!(reply(&stale)["result"].get("edit").is_none());
    }

    #[test]
    fn semantic_tokens_support_ranges_deltas_and_richer_java_kinds() {
        let uri = "file:///Test.java";
        let source = "class Test {}\nclass More {}\n";
        let mut server = Server::with_backend(FakeBackend);
        let initialized = server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        let provider = &reply(&initialized)["result"]["capabilities"]["semanticTokensProvider"];
        assert_eq!(provider["range"], true);
        assert_eq!(provider["full"]["delta"], true);
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":uri,"version":1,"text":source}}
        }));

        let full = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/semanticTokens/full",
            "params":{"textDocument":{"uri":uri}}
        }));
        let previous_id = reply(&full)["result"]["resultId"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(reply(&full)["result"]["data"].as_array().unwrap().len(), 5);

        let more = source.find("More").unwrap() as u64;
        server
            .analyses
            .get_mut(uri)
            .unwrap()
            .symbols
            .push(SemanticSymbol {
                role: "declaration".to_owned(),
                kind: "record".to_owned(),
                name: "More".to_owned(),
                qualified_name: "More".to_owned(),
                symbol_id: "More".to_owned(),
                start: more,
                end: more + 4,
            });
        let delta = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"textDocument/semanticTokens/full/delta",
            "params":{"textDocument":{"uri":uri},"previousResultId":previous_id}
        }));
        assert_eq!(reply(&delta)["result"]["edits"][0]["start"], 5);
        assert_eq!(
            reply(&delta)["result"]["edits"][0]["data"]
                .as_array()
                .unwrap()
                .len(),
            5
        );

        let range = server.dispatch(json!({
            "jsonrpc":"2.0","id":4,"method":"textDocument/semanticTokens/range",
            "params":{
                "textDocument":{"uri":uri},
                "range":{"start":{"line":1,"character":0},"end":{"line":2,"character":0}}
            }
        }));
        let data = reply(&range)["result"]["data"].as_array().unwrap();
        assert_eq!(data.len(), 5);
        assert_eq!(data[3], 8);

        assert_eq!(semantic_token_type("enum_constant", "reference"), Some(10));
        assert_eq!(
            semantic_token_type("annotation_type", "reference"),
            Some(11)
        );
        assert_eq!(
            semantic_token_modifiers(&SemanticSymbol {
                role: "write".to_owned(),
                kind: "local_variable".to_owned(),
                name: "value".to_owned(),
                qualified_name: "value".to_owned(),
                symbol_id: "value".to_owned(),
                start: 0,
                end: 5,
            }),
            1 << 6
        );
    }

    #[test]
    fn document_highlights_distinguish_declarations_writes_and_reads() {
        let uri = "file:///Counter.java";
        let source = "class Counter { void run() { int value = 0; value = 1; use(value); } }";
        let positions: Vec<_> = source
            .match_indices("value")
            .map(|(offset, _)| offset)
            .collect();
        let mut server = Server::with_backend(FakeBackend);
        let initialized = server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        assert_eq!(
            reply(&initialized)["result"]["capabilities"]["documentHighlightProvider"],
            true
        );
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":uri,"version":1,"text":source}}
        }));
        server.project.update_document(
            uri,
            &SemanticResult {
                package_name: String::new(),
                diagnostics: Vec::new(),
                symbols: ["declaration", "write", "reference"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, role)| SemanticSymbol {
                        role: role.to_owned(),
                        kind: "local_variable".to_owned(),
                        name: "value".to_owned(),
                        qualified_name: "Counter#run:value".to_owned(),
                        symbol_id: "Counter#run:value".to_owned(),
                        start: positions[index] as u64,
                        end: positions[index] as u64 + 5,
                    })
                    .collect(),
            },
        );
        let highlights = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/documentHighlight",
            "params":{"textDocument":{"uri":uri},"position":{"line":0,"character":positions[0] + 1}}
        }));
        assert_eq!(
            reply(&highlights)["result"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["kind"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![3, 3, 2]
        );
    }

    #[test]
    fn formatting_uses_the_canonical_backend_for_a_full_document_edit() {
        let uri = "file:///Format.java";
        let source = "class Format {  \nvoid run() {\nString value = \"{\"; // }\nif (true) {\nuse();   \n}\n}\n}";
        let expected = "class Format {\n    void run() {\n        String value = \"{\"; // }\n        if (true) {\n            use();\n        }\n    }\n}";
        let mut server = Server::with_backend(FormattingBackend(expected));
        let initialized = server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        assert_eq!(
            reply(&initialized)["result"]["capabilities"]["documentFormattingProvider"],
            true
        );
        assert!(
            reply(&initialized)["result"]["capabilities"]["documentRangeFormattingProvider"]
                .is_null()
        );
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":uri,"version":1,"text":source}}
        }));
        let formatted = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/formatting",
            "params":{"textDocument":{"uri":uri},"options":{"tabSize":4,"insertSpaces":true}}
        }));
        assert_eq!(reply(&formatted)["result"][0]["newText"], expected);
        let selections = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"textDocument/selectionRange",
            "params":{
                "textDocument":{"uri":uri},
                "positions":[{"line":4,"character":2}]
            }
        }));
        assert_eq!(reply(&selections)["result"][0]["range"]["start"]["line"], 4);
        assert!(reply(&selections)["result"][0]["parent"].is_object());
        let folds = server.dispatch(json!({
            "jsonrpc":"2.0","id":4,"method":"textDocument/foldingRange",
            "params":{"textDocument":{"uri":uri}}
        }));
        assert!(reply(&folds)["result"].as_array().unwrap().len() >= 3);
    }

    #[test]
    fn discovers_junit_tests_prepares_runs_and_serves_code_lenses() {
        let uri = "file:///GreetingTest.java";
        let source = "class GreetingTest {\n @org.junit.jupiter.api.Test\n void greets() {}\n @ParameterizedTest\n void many() {}\n}";
        let mut server = Server::with_backend(TestDiscoveryBackend);
        let initialized = server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
        let capabilities = &reply(&initialized)["result"]["capabilities"];
        assert!(capabilities["codeLensProvider"].is_object());
        assert_eq!(
            capabilities["experimental"]["jmanJavaTesting"]["protocolVersion"],
            2
        );
        assert_eq!(
            capabilities["experimental"]["jmanJavaTesting"]["coverage"],
            true
        );
        server.dispatch(json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":uri,"version":1,"text":source}}
        }));

        let discovered = server.dispatch(json!({
            "jsonrpc":"2.0","id":2,"method":"jman.java/tests/discover","params":{}
        }));
        let items = reply(&discovered)["result"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["selector"], "com.example.jman_test.GreetingTest");
        assert_eq!(
            items[1]["selector"],
            "com.example.jman_test.GreetingTest#greets"
        );
        assert_eq!(
            items[2]["selector"],
            "com.example.jman_test.GreetingTest#many"
        );

        let lenses = server.dispatch(json!({
            "jsonrpc":"2.0","id":3,"method":"textDocument/codeLens",
            "params":{"textDocument":{"uri":uri}}
        }));
        assert_eq!(reply(&lenses)["result"].as_array().unwrap().len(), 3);
        assert_eq!(
            reply(&lenses)["result"][0]["command"]["title"],
            "Run Test Class"
        );
        assert_eq!(reply(&lenses)["result"][1]["command"]["title"], "Run Test");
        assert_eq!(
            reply(&lenses)["result"][0]["command"]["command"],
            "jman.java.test"
        );

        let run = server.dispatch(json!({
            "jsonrpc":"2.0","id":4,"method":"jman.java/tests/run",
            "params":{"selectors":["com.example.jman_test.GreetingTest#greets"]}
        }));
        assert_eq!(reply(&run)["result"]["protocolVersion"], 2);
        assert_eq!(reply(&run)["result"]["program"], "jman");
        assert_eq!(
            reply(&run)["result"]["arguments"],
            json!([
                "--no-progress",
                "test",
                "--report",
                "json",
                "--tests",
                "com.example.jman_test.GreetingTest#greets"
            ])
        );
        assert_eq!(reply(&run)["result"]["coverage"]["supported"], true);

        let coverage = server.dispatch(json!({
            "jsonrpc":"2.0","id":5,"method":"jman.java/tests/run",
            "params":{
                "selectors":["com.example.jman_test.GreetingTest#greets"],
                "coverage":true
            }
        }));
        assert_eq!(
            reply(&coverage)["result"]["arguments"],
            json!([
                "--no-progress",
                "test",
                "--report",
                "json",
                "--tests",
                "com.example.jman_test.GreetingTest#greets",
                "--coverage"
            ])
        );
    }

    #[test]
    fn prepares_maven_and_gradle_test_provider_commands() {
        let cases = [
            (
                "maven",
                json!([
                    "test",
                    "-Dtest=com.example.GreetingTest#greets",
                    "-Dsurefire.failIfNoSpecifiedTests=false"
                ]),
            ),
            (
                "gradle",
                json!(["test", "--tests", "com.example.GreetingTest.greets"]),
            ),
        ];
        for (build_system, expected) in cases {
            let mut server = Server::with_backend(ExternalTestDiscoveryBackend(build_system));
            server.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}));
            let run = server.dispatch(json!({
                "jsonrpc":"2.0","id":2,"method":"jman.java/tests/run",
                "params":{
                    "selectors":["com.example.GreetingTest#greets"],
                    "uri":"file:///workspace/src/test/java/com/example/GreetingTest.java"
                }
            }));
            assert_eq!(reply(&run)["result"]["buildSystem"], build_system);
            assert_eq!(reply(&run)["result"]["buildJavaHome"], "/jdks/21");
            assert_eq!(reply(&run)["result"]["arguments"], expected);
            assert_eq!(reply(&run)["result"]["report"], "junit-xml");
            assert_eq!(reply(&run)["result"]["coverage"]["supported"], false);
        }
    }

    struct CacheBackend {
        calls: Arc<Mutex<Vec<&'static str>>>,
    }

    struct SyncBackend {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        fail: Arc<Mutex<bool>>,
    }

    impl AnalysisBackend for SyncBackend {
        fn analyze(
            &mut self,
            _uri: &str,
            _file_name: &str,
            _source: &str,
        ) -> Result<SemanticResult, String> {
            Ok(SemanticResult {
                package_name: String::new(),
                symbols: Vec::new(),
                diagnostics: Vec::new(),
            })
        }

        fn reload(&mut self, changed_uris: &[String]) -> Result<bool, String> {
            self.calls.lock().unwrap().push(changed_uris.to_vec());
            if *self.fail.lock().unwrap() {
                Err("synthetic sync failure".to_owned())
            } else {
                Ok(true)
            }
        }
    }

    impl AnalysisBackend for CacheBackend {
        fn build_system(&self) -> Option<String> {
            Some("jman".to_owned())
        }

        fn analyze(
            &mut self,
            _uri: &str,
            _file_name: &str,
            _source: &str,
        ) -> Result<SemanticResult, String> {
            Ok(SemanticResult {
                package_name: String::new(),
                symbols: Vec::new(),
                diagnostics: Vec::new(),
            })
        }

        fn cache_status(&self) -> CacheStatus {
            CacheStatus {
                project_id: "project-1".to_owned(),
                directory: "/cache/project-1".to_owned(),
                structural_entries: 10,
                structural_hits: 7,
                structural_misses: 3,
                semantic_entries: 8,
                semantic_hits: 5,
                semantic_misses: 3,
                external_entries: 100,
                bytes: 4096,
                indexing_milliseconds: 12,
                build_tool_version: Some("8.7".to_owned()),
                build_java_home: Some("/jdks/21".to_owned()),
                build_java_version: Some("21.0.2".to_owned()),
                build_java_major: Some(21),
                build_java_source: Some("JMAN-managed".to_owned()),
            }
        }

        fn rebuild_index(&mut self) -> Result<(), String> {
            self.calls.lock().unwrap().push("rebuild");
            Ok(())
        }

        fn clear_project_cache(&mut self) -> Result<(), String> {
            self.calls.lock().unwrap().push("clear");
            Ok(())
        }
    }

    struct LifecycleBackend {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl AnalysisBackend for LifecycleBackend {
        fn initialize(
            &mut self,
            root_uri: Option<&str>,
            _options: Option<&Value>,
        ) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("initialize:{}", root_uri.unwrap_or_default()));
            Ok(())
        }

        fn analyze(
            &mut self,
            _uri: &str,
            _file_name: &str,
            source: &str,
        ) -> Result<SemanticResult, String> {
            Ok(SemanticResult {
                package_name: String::new(),
                symbols: Vec::new(),
                diagnostics: vec![SemanticDiagnostic {
                    kind: "warning".to_owned(),
                    code: "lifecycle.warning".to_owned(),
                    start: 0,
                    end: source.encode_utf16().count() as u64,
                    line: 1,
                    column: 1,
                    message: "lifecycle warning".to_owned(),
                }],
            })
        }

        fn workspace_folders_changed(
            &mut self,
            added: &[String],
            removed: &[String],
        ) -> Result<(), String> {
            self.calls.lock().unwrap().push(format!(
                "folders:+{}:-{}",
                added.join(","),
                removed.join(",")
            ));
            Ok(())
        }

        fn update_configuration(&mut self, settings: &Value) -> Result<(), String> {
            self.calls.lock().unwrap().push(format!(
                "configuration:{}",
                settings["buildSystem"].as_str().unwrap_or_default()
            ));
            Ok(())
        }

        fn cancel_request(&mut self, id: &Value) {
            self.calls.lock().unwrap().push(format!("cancel:{id}"));
        }

        fn rebuild_index(&mut self) -> Result<(), String> {
            self.calls.lock().unwrap().push("rebuild".to_owned());
            Ok(())
        }

        fn source_metadata(&self, uri: &str) -> SourceMetadata {
            let generated = uri.contains("/generated/");
            SourceMetadata {
                generated,
                read_only: generated,
                origin: generated.then(|| "fixture:compileJava".to_owned()),
            }
        }
    }

    struct FakeBackend;

    struct FormattingBackend(&'static str);

    impl AnalysisBackend for FormattingBackend {
        fn analyze(
            &mut self,
            _uri: &str,
            _file_name: &str,
            _source: &str,
        ) -> Result<SemanticResult, String> {
            Ok(SemanticResult {
                package_name: String::new(),
                symbols: Vec::new(),
                diagnostics: Vec::new(),
            })
        }

        fn format(
            &mut self,
            _uri: &str,
            _file_name: &str,
            _source: &str,
        ) -> Result<FormatResult, String> {
            Ok(FormatResult {
                source: self.0.to_owned(),
                diagnostics: Vec::new(),
            })
        }
    }

    impl AnalysisBackend for FakeBackend {
        fn analyze(
            &mut self,
            _uri: &str,
            _file_name: &str,
            source: &str,
        ) -> Result<SemanticResult, String> {
            Ok(SemanticResult {
                package_name: String::new(),
                symbols: vec![SemanticSymbol {
                    role: "declaration".to_owned(),
                    kind: "class".to_owned(),
                    name: "Test".to_owned(),
                    qualified_name: "Test".to_owned(),
                    symbol_id: "Test".to_owned(),
                    start: 6,
                    end: 10,
                }],
                diagnostics: vec![SemanticDiagnostic {
                    kind: "error".to_owned(),
                    code: "test.error".to_owned(),
                    start: 0,
                    end: source.encode_utf16().count() as u64,
                    line: 1,
                    column: 1,
                    message: "test diagnostic".to_owned(),
                }],
            })
        }
    }

    struct WorkspaceBackend {
        sources: Vec<PathBuf>,
    }

    struct OverrideReferencesBackend;

    const OVERRIDE_INTERFACE_URI: &str = "file:///TransactionManagementPort.java";
    const OVERRIDE_IMPLEMENTATION_URI: &str = "file:///JdbcTransactionManagementAdapter.java";
    const OVERRIDE_CALLER_URI: &str = "file:///UserService.java";
    const OVERRIDE_INTERFACE_SOURCE: &str = "package com.example; interface TransactionManagementPort { <T> T executeWrite(java.util.function.Supplier<T> action); }";
    const OVERRIDE_IMPLEMENTATION_SOURCE: &str = "package com.example; class JdbcTransactionManagementAdapter implements TransactionManagementPort { public <T> T executeWrite(java.util.function.Supplier<T> action) { return action.get(); } }";
    const OVERRIDE_CALLER_SOURCE: &str = "package com.example; class UserService { TransactionManagementPort transactions; Object save() { return transactions.executeWrite(() -> new Object()); } }";

    struct EditorBackend;

    struct TestDiscoveryBackend;

    struct ExternalTestDiscoveryBackend(&'static str);

    struct ExternalDefinitionBackend;

    struct WorkspaceShadowBackend {
        source: PathBuf,
    }

    struct PendingWorkspaceShadowBackend;

    struct JpmsBackend {
        sources: Vec<PathBuf>,
    }

    impl AnalysisBackend for TestDiscoveryBackend {
        fn analyze(
            &mut self,
            _uri: &str,
            _file_name: &str,
            source: &str,
        ) -> Result<SemanticResult, String> {
            let symbol = |name: &str, kind: &str| {
                let start = source.find(name).unwrap() as u64;
                SemanticSymbol {
                    role: "declaration".to_owned(),
                    kind: kind.to_owned(),
                    name: name.to_owned(),
                    qualified_name: if kind == "class" {
                        format!("com.example.jman_test.{name}")
                    } else {
                        // Mirror javac's executable qualified-name shape that
                        // previously made discovery mistake the package for
                        // the owning test class.
                        format!("com.example.jman_test.{name}")
                    },
                    symbol_id: format!("com.example.jman_test.{name}"),
                    start,
                    end: start + name.len() as u64,
                }
            };
            Ok(SemanticResult {
                package_name: String::new(),
                symbols: vec![
                    symbol("GreetingTest", "class"),
                    symbol("greets", "method"),
                    symbol("many", "method"),
                ],
                diagnostics: Vec::new(),
            })
        }
    }

    impl AnalysisBackend for ExternalTestDiscoveryBackend {
        fn build_system(&self) -> Option<String> {
            Some(self.0.to_owned())
        }

        fn cache_status(&self) -> CacheStatus {
            CacheStatus {
                build_java_home: Some("/jdks/21".to_owned()),
                build_java_major: Some(21),
                ..CacheStatus::default()
            }
        }

        fn analyze(
            &mut self,
            uri: &str,
            file_name: &str,
            source: &str,
        ) -> Result<SemanticResult, String> {
            TestDiscoveryBackend.analyze(uri, file_name, source)
        }
    }

    impl AnalysisBackend for JpmsBackend {
        fn workspace_source_files(&self) -> Vec<PathBuf> {
            self.sources.clone()
        }

        fn jpms_catalog(&self, _uri: &str) -> JpmsCatalog {
            JpmsCatalog {
                modules: vec!["demo.current".to_owned(), "demo.target".to_owned()],
                packages: vec!["demo.current.api".to_owned()],
            }
        }

        fn analyze(
            &mut self,
            _uri: &str,
            _file_name: &str,
            source: &str,
        ) -> Result<SemanticResult, String> {
            let declaration = if source.contains("interface DemoService") {
                Some(("interface", "DemoService", "demo.current.api.DemoService"))
            } else if source.contains("class DemoProvider") {
                Some((
                    "class",
                    "DemoProvider",
                    "demo.current.internal.DemoProvider",
                ))
            } else {
                None
            };
            Ok(SemanticResult {
                package_name: String::new(),
                symbols: declaration
                    .map(|(kind, name, qualified_name)| {
                        vec![SemanticSymbol {
                            role: "declaration".to_owned(),
                            kind: kind.to_owned(),
                            name: name.to_owned(),
                            qualified_name: qualified_name.to_owned(),
                            symbol_id: qualified_name.to_owned(),
                            start: source.find(name).unwrap_or_default() as u64,
                            end: source.find(name).unwrap_or_default() as u64 + name.len() as u64,
                        }]
                    })
                    .unwrap_or_default(),
                diagnostics: Vec::new(),
            })
        }
    }

    impl AnalysisBackend for ExternalDefinitionBackend {
        fn analyze(
            &mut self,
            _uri: &str,
            _file_name: &str,
            _source: &str,
        ) -> Result<SemanticResult, String> {
            Err("external source must not be analyzed".to_owned())
        }

        fn editor_query(
            &mut self,
            uri: &str,
            _file_name: &str,
            _source: &str,
            _cursor: u32,
        ) -> Result<EditorQueryResult, String> {
            assert_eq!(uri, "file:///workspace/Use.java");
            let source = "public class CloneNotSupportedException {}";
            let start = source.find("CloneNotSupportedException").unwrap() as u64;
            Ok(EditorQueryResult {
                completions: Vec::new(),
                signatures: Vec::new(),
                hover: None,
                definition: Some(javac_frontend::EditorDefinition {
                    symbol_id: "java.base|Ljava/lang/CloneNotSupportedException;".to_owned(),
                    module: "java.base".to_owned(),
                    owner: "java.lang.CloneNotSupportedException".to_owned(),
                    name: "CloneNotSupportedException".to_owned(),
                    descriptor: "java.lang.CloneNotSupportedException".to_owned(),
                    source_name: "CloneNotSupportedException.java".to_owned(),
                    source: source.to_owned(),
                    start,
                    end: start + "CloneNotSupportedException".len() as u64,
                    decompiled: false,
                }),
                type_definition: None,
            })
        }
    }

    impl AnalysisBackend for WorkspaceShadowBackend {
        fn workspace_source_files(&self) -> Vec<PathBuf> {
            vec![self.source.clone()]
        }

        fn analyze(
            &mut self,
            _uri: &str,
            _file_name: &str,
            source: &str,
        ) -> Result<SemanticResult, String> {
            let symbols = if source.contains("record HelloResponse") {
                let start = source.find("HelloResponse").unwrap() as u64;
                vec![SemanticSymbol {
                    role: "declaration".to_owned(),
                    kind: "record".to_owned(),
                    name: "HelloResponse".to_owned(),
                    qualified_name: "com.example.HelloResponse".to_owned(),
                    symbol_id: "com.example.HelloResponse".to_owned(),
                    start,
                    end: start + "HelloResponse".len() as u64,
                }]
            } else {
                Vec::new()
            };
            Ok(SemanticResult {
                package_name: "com.example".to_owned(),
                symbols,
                diagnostics: Vec::new(),
            })
        }

        fn editor_query(
            &mut self,
            _uri: &str,
            _file_name: &str,
            _source: &str,
            _cursor: u32,
        ) -> Result<EditorQueryResult, String> {
            let definition = javac_frontend::EditorDefinition {
                symbol_id: "unnamed|Lcom/example/HelloResponse;".to_owned(),
                module: String::new(),
                owner: "com.example.HelloResponse".to_owned(),
                name: "<init>".to_owned(),
                descriptor: "(java.lang.String)void".to_owned(),
                source_name: "HelloResponse.java".to_owned(),
                source: "package com.example; record HelloResponse() {}".to_owned(),
                start: 28,
                end: 41,
                decompiled: true,
            };
            Ok(EditorQueryResult {
                completions: Vec::new(),
                signatures: Vec::new(),
                hover: None,
                definition: Some(definition.clone()),
                type_definition: Some(definition),
            })
        }
    }

    impl AnalysisBackend for PendingWorkspaceShadowBackend {
        fn analyze(
            &mut self,
            _uri: &str,
            _file_name: &str,
            _source: &str,
        ) -> Result<SemanticResult, String> {
            Ok(SemanticResult {
                package_name: "com.example".to_owned(),
                symbols: Vec::new(),
                diagnostics: Vec::new(),
            })
        }

        fn workspace_index_pending(&self) -> bool {
            true
        }

        fn editor_query(
            &mut self,
            _uri: &str,
            _file_name: &str,
            _source: &str,
            _cursor: u32,
        ) -> Result<EditorQueryResult, String> {
            let definition = javac_frontend::EditorDefinition {
                symbol_id: "unnamed|Lcom/example/HelloResponse;".to_owned(),
                module: String::new(),
                owner: "com.example.HelloResponse".to_owned(),
                name: "<init>".to_owned(),
                descriptor: "(java.lang.String)void".to_owned(),
                source_name: "HelloResponse.java".to_owned(),
                source: "package com.example; record HelloResponse() {}".to_owned(),
                start: 28,
                end: 41,
                decompiled: true,
            };
            Ok(EditorQueryResult {
                completions: Vec::new(),
                signatures: Vec::new(),
                hover: None,
                definition: Some(definition.clone()),
                type_definition: Some(definition),
            })
        }
    }

    impl AnalysisBackend for EditorBackend {
        fn analyze(
            &mut self,
            _uri: &str,
            _file_name: &str,
            _source: &str,
        ) -> Result<SemanticResult, String> {
            Ok(SemanticResult {
                package_name: String::new(),
                symbols: vec![
                    SemanticSymbol {
                        role: "declaration".to_owned(),
                        kind: "class".to_owned(),
                        name: "Editor".to_owned(),
                        qualified_name: "Editor".to_owned(),
                        symbol_id: "Editor".to_owned(),
                        start: 6,
                        end: 12,
                    },
                    SemanticSymbol {
                        role: "declaration".to_owned(),
                        kind: "method".to_owned(),
                        name: "run".to_owned(),
                        qualified_name: "Editor.run".to_owned(),
                        symbol_id: "Editor.run".to_owned(),
                        start: 20,
                        end: 23,
                    },
                    SemanticSymbol {
                        role: "reference".to_owned(),
                        kind: "method".to_owned(),
                        name: "run".to_owned(),
                        qualified_name: "Editor.run".to_owned(),
                        symbol_id: "Editor.run".to_owned(),
                        start: 28,
                        end: 31,
                    },
                ],
                diagnostics: Vec::new(),
            })
        }

        fn editor_query(
            &mut self,
            _uri: &str,
            _file_name: &str,
            _source: &str,
            _cursor: u32,
        ) -> Result<EditorQueryResult, String> {
            Ok(EditorQueryResult {
                completions: vec![javac_frontend::EditorCompletion {
                    label: "add".to_owned(),
                    kind: "method".to_owned(),
                    detail: "boolean add(java.lang.String value)".to_owned(),
                    insert_text: "add(".to_owned(),
                    documentation: "Adds an element.".to_owned(),
                }],
                signatures: vec![javac_frontend::EditorSignature {
                    label: "void run(java.lang.String value)".to_owned(),
                    parameters: vec!["java.lang.String value".to_owned()],
                    return_type: "void".to_owned(),
                    documentation: "Runs the operation.".to_owned(),
                }],
                hover: Some(javac_frontend::EditorHover {
                    detail: "Editor".to_owned(),
                    documentation: "Editor documentation.".to_owned(),
                }),
                definition: None,
                type_definition: Some(javac_frontend::EditorDefinition {
                    symbol_id: "Editor".to_owned(),
                    module: String::new(),
                    owner: "Editor".to_owned(),
                    name: "Editor".to_owned(),
                    descriptor: "Editor".to_owned(),
                    source_name: "Editor.java".to_owned(),
                    source: "class Editor {}".to_owned(),
                    start: 6,
                    end: 12,
                    decompiled: false,
                }),
            })
        }
    }

    impl AnalysisBackend for WorkspaceBackend {
        fn workspace_source_files(&self) -> Vec<PathBuf> {
            self.sources.clone()
        }

        fn analyze(
            &mut self,
            _uri: &str,
            _file_name: &str,
            source: &str,
        ) -> Result<SemanticResult, String> {
            let declaration = source.contains("class Target");
            Ok(SemanticResult {
                package_name: String::new(),
                symbols: vec![SemanticSymbol {
                    role: if declaration {
                        "declaration".to_owned()
                    } else {
                        "reference".to_owned()
                    },
                    kind: "class".to_owned(),
                    name: "Target".to_owned(),
                    qualified_name: "Target".to_owned(),
                    symbol_id: "Target".to_owned(),
                    start: if declaration { 6 } else { 12 },
                    end: if declaration { 12 } else { 18 },
                }],
                diagnostics: Vec::new(),
            })
        }
    }

    impl AnalysisBackend for OverrideReferencesBackend {
        fn workspace_structural_documents(&self) -> Vec<(String, String, SemanticResult)> {
            [
                (
                    OVERRIDE_INTERFACE_URI,
                    OVERRIDE_INTERFACE_SOURCE,
                    "com.example.TransactionManagementPort#executeWrite",
                    "declaration",
                ),
                (
                    OVERRIDE_IMPLEMENTATION_URI,
                    OVERRIDE_IMPLEMENTATION_SOURCE,
                    "com.example.JdbcTransactionManagementAdapter#executeWrite",
                    "declaration",
                ),
                (
                    OVERRIDE_CALLER_URI,
                    OVERRIDE_CALLER_SOURCE,
                    "executeWrite",
                    "reference",
                ),
            ]
            .into_iter()
            .map(|(uri, source, identity, role)| {
                let start = source.find("executeWrite").unwrap() as u64;
                (
                    uri.to_owned(),
                    source.to_owned(),
                    SemanticResult {
                        package_name: "com.example".to_owned(),
                        diagnostics: Vec::new(),
                        symbols: vec![SemanticSymbol {
                            role: role.to_owned(),
                            kind: "method".to_owned(),
                            name: "executeWrite".to_owned(),
                            qualified_name: identity.to_owned(),
                            symbol_id: identity.to_owned(),
                            start,
                            end: start + "executeWrite".len() as u64,
                        }],
                    },
                )
            })
            .collect()
        }

        fn analyze(
            &mut self,
            uri: &str,
            _file_name: &str,
            source: &str,
        ) -> Result<SemanticResult, String> {
            let family = "<unnamed>|com/example/TransactionManagementPort#executeWrite(Ljava/util/function/Supplier;)Ljava/lang/Object;";
            let implementation = "<unnamed>|com/example/JdbcTransactionManagementAdapter#executeWrite(Ljava/util/function/Supplier;)Ljava/lang/Object;";
            let start = source.find("executeWrite").unwrap() as u64;
            let symbol = |role: &str, symbol_id: &str, qualified_name: &str| SemanticSymbol {
                role: role.to_owned(),
                kind: "method".to_owned(),
                name: "executeWrite".to_owned(),
                qualified_name: qualified_name.to_owned(),
                symbol_id: symbol_id.to_owned(),
                start,
                end: start + "executeWrite".len() as u64,
            };
            let symbols = match uri {
                OVERRIDE_INTERFACE_URI => vec![
                    symbol(
                        "declaration",
                        family,
                        "com.example.TransactionManagementPort#executeWrite",
                    ),
                    symbol("override_family", family, family),
                ],
                OVERRIDE_IMPLEMENTATION_URI => vec![
                    symbol(
                        "declaration",
                        implementation,
                        "com.example.JdbcTransactionManagementAdapter#executeWrite",
                    ),
                    symbol("override_family", implementation, family),
                ],
                OVERRIDE_CALLER_URI => vec![symbol(
                    "reference",
                    family,
                    "com.example.TransactionManagementPort#executeWrite",
                )],
                _ => Vec::new(),
            };
            Ok(SemanticResult {
                package_name: "com.example".to_owned(),
                diagnostics: Vec::new(),
                symbols,
            })
        }
    }

    fn reply(dispatch: &Dispatch) -> &Value {
        let Dispatch::Reply(reply) = dispatch else {
            panic!("expected reply")
        };
        reply
    }
}
