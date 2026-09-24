use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{
    AnalysisBackend, CacheStatus, CompileModel, JpmsCatalog, ProcessorRequest, ProcessorWorker,
    SourceMetadata,
    build_runtime::BuildRuntime,
    cache::{FileLock, cache_root, directory_size, project_cache_directory, stable_project_id},
    file_uri_to_path, load_project,
    processed_semantics::{ProcessedSemanticRequest, ProcessedSemanticWorker},
    select_compile_model,
};
use javac_frontend::{
    ABI_VERSION, EditorQueryResult, Frontend, ProjectSession, SemanticResult, WorkspaceSource,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub struct NativeBackend {
    frontend: Frontend,
    models: Vec<CompileModel>,
    sessions: HashMap<String, ProjectSession>,
    root: Option<std::path::PathBuf>,
    preferred_build_system: Option<String>,
    build_runtime: Option<BuildRuntime>,
    processor_workers: HashMap<PathBuf, ProcessorWorker>,
    processed_semantic_workers: HashMap<PathBuf, ProcessedSemanticWorker>,
    structural_documents: Vec<(String, String, SemanticResult)>,
    structural_revision: u64,
    cancellation_generation: Arc<AtomicU64>,
    editor_queries: EditorQueryCache,
    cache_status: CacheStatus,
}

const STRUCTURAL_CACHE_VERSION: u32 = 3;
const SEMANTIC_CACHE_VERSION: u32 = 4;
const PARSE_BATCH_FILES: usize = 128;
const PARSE_BATCH_BYTES: usize = 4 * 1024 * 1024;
const EDITOR_QUERY_CACHE_CAPACITY: usize = 512;
type StructuralParseGroup = Vec<(PathBuf, String, u64)>;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct EditorQueryKey {
    uri: String,
    source_fingerprint: u64,
    cursor: u32,
}

#[derive(Default)]
struct EditorQueryCache {
    entries: HashMap<EditorQueryKey, EditorQueryResult>,
}

impl EditorQueryCache {
    fn key(uri: &str, source: &str, cursor: u32) -> EditorQueryKey {
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        EditorQueryKey {
            uri: uri.to_owned(),
            source_fingerprint: hasher.finish(),
            cursor,
        }
    }

    fn get(&self, key: &EditorQueryKey) -> Option<EditorQueryResult> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: EditorQueryKey, result: EditorQueryResult) {
        if self.entries.len() >= EDITOR_QUERY_CACHE_CAPACITY {
            self.entries.clear();
        }
        self.entries.insert(key, result);
    }

    fn invalidate(&mut self, uri: &str) {
        self.entries.retain(|key, _| key.uri != uri);
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

#[derive(Default, Serialize, Deserialize)]
struct StructuralCache {
    version: u32,
    documents: HashMap<String, CachedStructuralDocument>,
    #[serde(default)]
    external_fingerprint: u64,
    #[serde(default)]
    external_catalog: Option<SemanticResult>,
}

#[derive(Serialize, Deserialize)]
struct CachedStructuralDocument {
    fingerprint: u64,
    result: SemanticResult,
}

#[derive(Default, Serialize, Deserialize)]
struct SemanticCache {
    version: u32,
    documents: HashMap<String, CachedSemanticDocument>,
}

#[derive(Serialize, Deserialize)]
struct CachedSemanticDocument {
    fingerprint: u64,
    result: SemanticResult,
}

#[derive(Clone)]
struct PendingSemanticDocument {
    uri: String,
    key: String,
    source: String,
    fingerprint: u64,
    fallback: SemanticResult,
    model: CompileModel,
}

impl NativeBackend {
    pub fn new(frontend: Frontend) -> Self {
        Self::with_cancellation(frontend, Arc::new(AtomicU64::new(0)))
    }

    pub(crate) fn with_cancellation(
        frontend: Frontend,
        cancellation_generation: Arc<AtomicU64>,
    ) -> Self {
        Self {
            frontend,
            models: Vec::new(),
            sessions: HashMap::new(),
            root: None,
            preferred_build_system: None,
            build_runtime: None,
            processor_workers: HashMap::new(),
            processed_semantic_workers: HashMap::new(),
            structural_documents: Vec::new(),
            structural_revision: 0,
            cancellation_generation,
            editor_queries: EditorQueryCache::default(),
            cache_status: CacheStatus::default(),
        }
    }

    fn refresh_cache_bytes(&mut self) {
        let Some(root) = self.root.as_deref() else {
            return;
        };
        self.cache_status.bytes = directory_size(&project_cache_directory(root));
    }

    fn record_build_runtime(&mut self, runtime: Option<&BuildRuntime>) {
        self.cache_status.build_tool_version =
            runtime.and_then(|runtime| runtime.build_tool_version.clone());
        self.cache_status.build_java_home =
            runtime.map(|runtime| runtime.java_home.to_string_lossy().into_owned());
        self.cache_status.build_java_version = runtime.map(|runtime| runtime.java_version.clone());
        self.cache_status.build_java_major = runtime.map(|runtime| runtime.java_major);
        self.cache_status.build_java_source = runtime.map(|runtime| runtime.source.to_owned());
    }

    pub(crate) fn defer_semantic_refinement(&self) -> bool {
        self.models
            .iter()
            .any(|model| module_info_file(model).is_some())
    }

    fn session_for(&mut self, uri: &str) -> Result<&ProjectSession, String> {
        let source = file_uri_to_path(uri).ok_or_else(|| "URI is not a file URI".to_owned())?;
        let model = select_compile_model(&self.models, &source)
            .ok_or_else(|| format!("no compile unit owns {}", source.display()))?;
        let key = format!("{}:{}", model.project_path, model.task_path);
        if !self.sessions.contains_key(&key) {
            let release = model
                .java_release()
                .ok_or_else(|| format!("compile unit {key} has no Java release"))?;
            let mut source_path = model.source_path();
            // Resolve sibling sources only when their classpath artifact has
            // not been built. Adding every reactor root makes javac compile
            // unrelated modules with the current unit's narrower classpath.
            if module_info_file(model).is_none() {
                for candidate in dependency_source_roots(model, &self.models) {
                    if !source_path.contains(&candidate) {
                        source_path.push(candidate);
                    }
                }
            }
            for attachment in source_attachments(&model.classpath) {
                if !source_path.contains(&attachment) {
                    source_path.push(attachment);
                }
            }
            let session = self
                .frontend
                .create_module_session(
                    &semantic_classpath(model, &self.models),
                    &semantic_module_path(model, &self.models),
                    &source_path,
                    module_info_file(model).as_deref(),
                    &jpms_compiler_options(model, &self.models),
                    release,
                )
                .map_err(|error| format!("cannot create session for {key}: {error:?}"))?;
            self.sessions.insert(key.clone(), session);
        }
        Ok(self.sessions.get(&key).unwrap())
    }

    fn processed_request(
        &self,
        model: &CompileModel,
        file_name: &str,
        source: &str,
    ) -> Result<ProcessedSemanticRequest, String> {
        let release = model.java_release().ok_or_else(|| {
            format!(
                "compile unit {}:{} has no Java release",
                model.project_path, model.task_path
            )
        })?;
        let mut source_path = model.source_path();
        if module_info_file(model).is_none() {
            for root in dependency_source_roots(model, &self.models) {
                if !source_path.contains(&root) {
                    source_path.push(root);
                }
            }
        }
        for attachment in source_attachments(&model.classpath) {
            if !source_path.contains(&attachment) {
                source_path.push(attachment);
            }
        }
        Ok(ProcessedSemanticRequest {
            file_name: file_name.to_owned(),
            source: source.to_owned(),
            release,
            classpath: semantic_classpath(model, &self.models),
            module_path: semantic_module_path(model, &self.models),
            source_path,
            processor_path: model.annotation_processor_path.clone(),
            processor_options: model.annotation_processor_options.clone(),
            compiler_options: jpms_compiler_options(model, &self.models),
        })
    }

    fn processed_request_file(&self, model: &CompileModel) -> PathBuf {
        self.root
            .as_deref()
            .map(project_cache_directory)
            .unwrap_or_else(cache_root)
            .join("processed-semantics")
            .join(safe_key(&compile_model_key(model)))
            .join(format!("request-{}.properties", std::process::id()))
    }

    fn ensure_processed_worker(&mut self, java: &Path) -> Result<(), String> {
        if self.processed_semantic_workers.contains_key(java) {
            return Ok(());
        }
        let classpath = processor_worker_classpath()?;
        let worker = ProcessedSemanticWorker::start(java, &classpath)?;
        self.processed_semantic_workers
            .insert(java.to_path_buf(), worker);
        Ok(())
    }

    fn analyze_processed(
        &mut self,
        model: &CompileModel,
        file_name: &str,
        source: &str,
    ) -> Result<SemanticResult, String> {
        let java = processor_java(model, self.build_runtime.as_ref())?;
        let request = self.processed_request(model, file_name, source)?;
        let request_file = self.processed_request_file(model);
        self.ensure_processed_worker(&java)?;
        let result = self
            .processed_semantic_workers
            .get_mut(&java)
            .expect("processed semantic worker was started")
            .analyze(&request, &request_file);
        if result.is_err() {
            self.processed_semantic_workers.remove(&java);
        }
        result
    }

    fn query_processed(
        &mut self,
        model: &CompileModel,
        file_name: &str,
        source: &str,
        cursor: u32,
    ) -> Result<EditorQueryResult, String> {
        let java = processor_java(model, self.build_runtime.as_ref())?;
        let request = self.processed_request(model, file_name, source)?;
        let request_file = self.processed_request_file(model);
        self.ensure_processed_worker(&java)?;
        let result = self
            .processed_semantic_workers
            .get_mut(&java)
            .expect("processed semantic worker was started")
            .editor_query(&request, cursor, &request_file);
        if result.is_err() {
            self.processed_semantic_workers.remove(&java);
        }
        result
    }

    fn run_annotation_processors(&mut self) -> Result<(), String> {
        if std::env::var_os("JMAN_JAVA_LSP_DISABLE_ANNOTATION_PROCESSING").is_some() {
            return Ok(());
        }
        let started = std::time::Instant::now();
        let mut models: Vec<_> = self
            .models
            .iter()
            .filter(|model| {
                is_java_compile_model(model)
                    && !model.annotation_processor_path.is_empty()
                    && processor_inputs_available(model, &self.models)
            })
            .cloned()
            .collect();
        order_processor_models(&mut models);
        if models.is_empty() {
            self.processor_workers.clear();
            return Ok(());
        }
        let model_count = models.len();
        let mut processor_classes: Vec<(String, PathBuf)> = Vec::new();
        let mut used_java_executables = std::collections::HashSet::new();
        for model in models {
            let key = format!("{}:{}", model.project_path, model.task_path);
            let java = processor_java(&model, self.build_runtime.as_ref())?;
            used_java_executables.insert(java.clone());
            let generated_directory = model
                .generated_source_directories
                .first()
                .cloned()
                .or_else(|| model.generated_sources_directory.clone())
                .unwrap_or_else(|| {
                    model
                        .project_directory
                        .join(".jman-java/generated")
                        .join(safe_key(&key))
                });
            let state = self
                .root
                .as_deref()
                .map(project_cache_directory)
                .unwrap_or_else(cache_root)
                .join("processors")
                .join(safe_key(&key));
            std::fs::create_dir_all(&state)
                .map_err(|error| format!("cannot create processor state: {error}"))?;
            let _processor_lock = FileLock::acquire(&state.join("processor-cache"))?;
            let sources = processor_source_files(&model);
            if sources.is_empty() {
                continue;
            }
            let classes_directory = state
                .join("current/build/classes/java")
                .join(compile_source_set_name(&model.task_path));
            let mut classpath: Vec<_> = model
                .classpath
                .iter()
                .filter(|path| *path != &classes_directory)
                .cloned()
                .collect();
            // When a reactor artifact has not been built yet javac may compile
            // its sources implicitly from source_path. Those sources require
            // the dependency compile unit's own compile-only and transitive
            // classpath, not only the consumer's published runtime surface.
            for dependency in dependency_compile_models(&model, &self.models) {
                for path in dependency.classpath.iter().chain(&dependency.module_path) {
                    if !classpath.contains(path) {
                        classpath.push(path.clone());
                    }
                }
            }
            let mut processor_classpath = Vec::new();
            for (processed_key, path) in &processor_classes {
                if processor_output_is_dependency(&model, processed_key, &self.models)
                    && !processor_classpath.contains(path)
                {
                    processor_classpath.push(path.clone());
                }
            }
            processor_classpath.extend(classpath);
            classpath = processor_classpath;
            let mut source_path = model.source_path();
            for root in dependency_source_roots(&model, &self.models) {
                if !source_path.contains(&root) {
                    source_path.push(root);
                }
            }
            if !is_main_compile_model(&model) {
                for main in self.models.iter().filter(|candidate| {
                    is_main_compile_model(candidate) && candidate.project_path == model.project_path
                }) {
                    for root in main.source_path() {
                        if !source_path.contains(&root) {
                            source_path.push(root);
                        }
                    }
                }
            }
            let request = ProcessorRequest {
                java_executable: java.clone(),
                sources,
                source_path,
                classpath,
                processor_path: model.annotation_processor_path.clone(),
                processor_options: model.annotation_processor_options.clone(),
                release: model
                    .java_release()
                    .ok_or_else(|| format!("compile unit {key} has no processor Java release"))?,
                generated_directory,
                classes_directory,
            };
            let request_file = state.join("request.properties");
            if matches!(request.persistent_cache_hit(&request_file), Ok(Some(_))) {
                for classes in processor_output_directories(&request) {
                    processor_classes.push((key.clone(), classes));
                }
                continue;
            }
            if !self.processor_workers.contains_key(&java) {
                let worker = processor_worker_classpath()
                    .and_then(|classpath| ProcessorWorker::start(&java, &classpath));
                match worker {
                    Ok(worker) => {
                        self.processor_workers.insert(java.clone(), worker);
                    }
                    Err(message) => {
                        eprintln!(
                            "jman-java-lsp annotation processor warning for {key} using {}: {message}",
                            java.display()
                        );
                        continue;
                    }
                }
            }
            let result = self
                .processor_workers
                .get_mut(&java)
                .expect("processor worker was started")
                .process(&key, &request, &request_file);
            if let Err(message) = result {
                eprintln!(
                    "jman-java-lsp annotation processor warning for {key} using {}: {message}",
                    java.display()
                );
                self.processor_workers.remove(&java);
            }
            for classes in processor_output_directories(&request) {
                processor_classes.push((key.clone(), classes));
            }
        }
        self.processor_workers
            .retain(|java, _| used_java_executables.contains(java));
        for model in &mut self.models {
            let key = format!("{}:{}", model.project_path, model.task_path);
            let outputs: Vec<_> = processor_classes
                .iter()
                .filter(|(candidate, _)| candidate == &key)
                .map(|(_, classes)| classes.clone())
                .collect();
            for classes in outputs.into_iter().rev() {
                if !model.classpath.contains(&classes) {
                    model.classpath.insert(0, classes);
                }
            }
        }
        crate::metrics::emit(
            "annotation-processors",
            serde_json::json!({
                "milliseconds": started.elapsed().as_millis(),
                "compileUnits": model_count
            }),
        );
        Ok(())
    }

    fn rebuild_structural_index(&mut self) -> Result<(), String> {
        let started = std::time::Instant::now();
        let generation = self.cancellation_generation.load(Ordering::Acquire);
        let mut documents = Vec::new();
        let cache_path = self
            .root
            .as_deref()
            .map(structural_cache_path)
            .unwrap_or_else(|| std::env::temp_dir().join("jman-java-structural-index.json"));
        let previous_cache = read_structural_cache(&cache_path);
        let mut next_cache = StructuralCache {
            version: STRUCTURAL_CACHE_VERSION,
            documents: HashMap::new(),
            external_fingerprint: 0,
            external_catalog: None,
        };
        let mut cache_hits = 0usize;
        let mut cache_misses = 0usize;
        let mut grouped: std::collections::BTreeMap<(u8, bool), StructuralParseGroup> =
            std::collections::BTreeMap::new();
        for source in self.workspace_source_files() {
            let Some(model) = select_compile_model(&self.models, &source) else {
                continue;
            };
            let Some(release) = model.java_release() else {
                continue;
            };
            let preview = model
                .compiler_args
                .iter()
                .any(|argument| argument == "--enable-preview");
            let Ok(text) = std::fs::read_to_string(&source) else {
                continue;
            };
            let key = source.to_string_lossy().into_owned();
            let fingerprint = structural_fingerprint(&key, &text, release, preview);
            if let Some(cached) = previous_cache.documents.get(&key)
                && cached.fingerprint == fingerprint
            {
                cache_hits += 1;
                documents.push((format!("file://{key}"), text, cached.result.clone()));
                next_cache.documents.insert(
                    key,
                    CachedStructuralDocument {
                        fingerprint,
                        result: cached.result.clone(),
                    },
                );
            } else {
                cache_misses += 1;
                grouped
                    .entry((release, preview))
                    .or_default()
                    .push((source, text, fingerprint));
            }
        }
        for ((release, preview), pending) in grouped {
            let sources: Vec<_> = pending
                .iter()
                .map(|(path, source, _)| WorkspaceSource {
                    file_name: path.to_string_lossy().into_owned(),
                    source: source.clone(),
                })
                .collect();
            let parsed = parse_workspace_parallel(
                &sources,
                release,
                preview,
                &self.cancellation_generation,
                generation,
            )?;
            for ((input, file), (_, _, fingerprint)) in
                sources.into_iter().zip(parsed.files).zip(pending)
            {
                let uri = format!("file://{}", input.file_name);
                let result = SemanticResult {
                    package_name: file.package_name,
                    symbols: file.symbols,
                    diagnostics: file.diagnostics,
                };
                next_cache.documents.insert(
                    input.file_name.clone(),
                    CachedStructuralDocument {
                        fingerprint,
                        result: result.clone(),
                    },
                );
                documents.push((uri, input.source, result));
            }
        }
        let external_fingerprint = external_catalog_fingerprint(&self.models);
        let catalog = if previous_cache.external_fingerprint == external_fingerprint {
            previous_cache
                .external_catalog
                .clone()
                .unwrap_or_else(|| external_type_catalog(&self.models))
        } else {
            external_type_catalog(&self.models)
        };
        next_cache.external_fingerprint = external_fingerprint;
        next_cache.external_catalog = Some(catalog.clone());
        let external_entries = catalog.symbols.len();
        if !catalog.symbols.is_empty() {
            documents.push((
                "jman-java-catalog://external-types".to_owned(),
                String::new(),
                catalog,
            ));
        }
        if self.cancellation_generation.load(Ordering::Acquire) != generation {
            return Err("workspace parse cancelled by a newer project state".to_owned());
        }
        write_structural_cache(&cache_path, &next_cache)?;
        self.cache_status.structural_entries = next_cache.documents.len();
        self.cache_status.structural_hits = cache_hits;
        self.cache_status.structural_misses = cache_misses;
        self.cache_status.external_entries = external_entries;
        self.cache_status.indexing_milliseconds = started.elapsed().as_millis();
        self.refresh_cache_bytes();
        crate::metrics::emit(
            "workspace-parse",
            serde_json::json!({
                "milliseconds": started.elapsed().as_millis(),
                "sourceFiles": documents.len(),
                "cacheHits": cache_hits,
                "cacheMisses": cache_misses,
                "batchFileLimit": PARSE_BATCH_FILES,
                "batchByteLimit": PARSE_BATCH_BYTES
            }),
        );
        self.structural_documents = documents;
        self.structural_revision = self.structural_revision.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn refine_semantic_index(&mut self) -> Result<(), String> {
        let started = std::time::Instant::now();
        let generation = self.cancellation_generation.load(Ordering::Acquire);
        let root = self
            .root
            .clone()
            .ok_or_else(|| "project has not been initialized".to_owned())?;
        let cache_path = semantic_cache_path(&root);
        let previous = read_semantic_cache(&cache_path);
        let structural = self.structural_documents.clone();
        let mut refined = Vec::with_capacity(structural.len());
        let mut next = SemanticCache {
            version: SEMANTIC_CACHE_VERSION,
            documents: HashMap::new(),
        };
        let mut cache_hits = 0usize;
        let mut cache_misses = 0usize;
        let mut pending = Vec::new();
        for (uri, source, fallback) in structural {
            if self.cancellation_generation.load(Ordering::Acquire) != generation {
                return Err("semantic indexing cancelled by a newer project state".to_owned());
            }
            let Some(path) = file_uri_to_path(&uri) else {
                refined.push((uri, source, fallback));
                continue;
            };
            let Some(model) = select_compile_model(&self.models, &path) else {
                refined.push((uri, source, fallback));
                continue;
            };
            let key = path.to_string_lossy().into_owned();
            let fingerprint = semantic_fingerprint(&key, &source, model);
            if let Some(cached) = previous.documents.get(&key)
                && cached.fingerprint == fingerprint
            {
                cache_hits += 1;
                next.documents.insert(
                    key,
                    CachedSemanticDocument {
                        fingerprint,
                        result: cached.result.clone(),
                    },
                );
                refined.push((uri, source, cached.result.clone()));
                continue;
            }
            // A modular javac task must attribute the complete owning module so
            // every source is associated with its module descriptor. Doing
            // that once per workspace file would be quadratic; keep the fast
            // structural result here and run full attribution on editor
            // demand, where the session cache amortizes repeated features.
            if module_info_file(model).is_some() {
                refined.push((uri, source, fallback));
                continue;
            }
            cache_misses += 1;
            let document = PendingSemanticDocument {
                uri,
                key,
                source,
                fingerprint,
                fallback,
                model: model.clone(),
            };
            if !model.annotation_processor_path.is_empty()
                && std::env::var_os("JMAN_JAVA_LSP_DISABLE_ANNOTATION_PROCESSING").is_none()
            {
                // Running an open-world processor once per closed file is both
                // incorrect for aggregating processors and prohibitively
                // expensive. Keep the processor-neutral structural index for
                // closed files; open documents use the authoritative processed
                // javac lane below through AnalysisBackend::analyze/query.
                next.documents.insert(
                    document.key,
                    CachedSemanticDocument {
                        fingerprint: document.fingerprint,
                        result: document.fallback.clone(),
                    },
                );
                refined.push((document.uri, document.source, document.fallback));
            } else {
                pending.push(document);
            }
        }
        let semantic_workers = semantic_worker_count(pending.len());
        for document in analyze_semantic_parallel(
            pending,
            &self.models,
            &self.cancellation_generation,
            generation,
            semantic_workers,
        )? {
            next.documents.insert(
                document.key,
                CachedSemanticDocument {
                    fingerprint: document.fingerprint,
                    result: document.fallback.clone(),
                },
            );
            refined.push((document.uri, document.source, document.fallback));
        }
        if self.cancellation_generation.load(Ordering::Acquire) != generation {
            return Err("semantic indexing cancelled by a newer project state".to_owned());
        }
        write_semantic_cache(&cache_path, &next)?;
        self.cache_status.semantic_entries = next.documents.len();
        self.cache_status.semantic_hits = cache_hits;
        self.cache_status.semantic_misses = cache_misses;
        self.cache_status.indexing_milliseconds = self
            .cache_status
            .indexing_milliseconds
            .saturating_add(started.elapsed().as_millis());
        self.refresh_cache_bytes();
        self.structural_documents = refined;
        self.structural_revision = self.structural_revision.wrapping_add(1);
        crate::metrics::emit(
            "workspace-semantic-index",
            serde_json::json!({
                "milliseconds": started.elapsed().as_millis(),
                "sourceFiles": self.structural_documents.len(),
                "cacheHits": cache_hits,
                "cacheMisses": cache_misses,
                "cacheVersion": SEMANTIC_CACHE_VERSION,
                "workers": semantic_workers
            }),
        );
        Ok(())
    }

    pub(crate) fn resume_structural_index(&mut self) -> Result<(), String> {
        self.rebuild_structural_index()
    }
}

fn structural_cache_path(root: &Path) -> PathBuf {
    project_cache_directory(root).join("structural.json")
}

fn semantic_cache_path(root: &Path) -> PathBuf {
    project_cache_directory(root).join("semantic.json")
}

fn structural_fingerprint(file_name: &str, source: &str, release: u8, preview: bool) -> u64 {
    let mut hasher = DefaultHasher::new();
    STRUCTURAL_CACHE_VERSION.hash(&mut hasher);
    ABI_VERSION.hash(&mut hasher);
    file_name.hash(&mut hasher);
    source.hash(&mut hasher);
    release.hash(&mut hasher);
    preview.hash(&mut hasher);
    hasher.finish()
}

fn semantic_fingerprint(file_name: &str, source: &str, model: &CompileModel) -> u64 {
    let mut hasher = DefaultHasher::new();
    SEMANTIC_CACHE_VERSION.hash(&mut hasher);
    ABI_VERSION.hash(&mut hasher);
    file_name.hash(&mut hasher);
    source.hash(&mut hasher);
    model.schema_version.hash(&mut hasher);
    model.build_system.hash(&mut hasher);
    model.project_path.hash(&mut hasher);
    model.task_path.hash(&mut hasher);
    model.java_release().hash(&mut hasher);
    model.compiler_args.hash(&mut hasher);
    model.source_path().hash(&mut hasher);
    for path in model
        .classpath
        .iter()
        .chain(&model.module_path)
        .chain(&model.annotation_processor_path)
    {
        path.hash(&mut hasher);
        if let Ok(metadata) = std::fs::metadata(path)
            && metadata.is_file()
        {
            metadata.len().hash(&mut hasher);
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn external_catalog_fingerprint(models: &[CompileModel]) -> u64 {
    let mut hasher = DefaultHasher::new();
    STRUCTURAL_CACHE_VERSION.hash(&mut hasher);
    ABI_VERSION.hash(&mut hasher);
    std::env::var_os("JAVA_HOME").hash(&mut hasher);
    let mut paths: Vec<_> = models
        .iter()
        .flat_map(|model| model.classpath.iter().chain(&model.module_path))
        .collect();
    paths.sort();
    paths.dedup();
    for path in paths {
        path.hash(&mut hasher);
        if let Ok(metadata) = std::fs::metadata(path) {
            metadata.len().hash(&mut hasher);
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn read_structural_cache(path: &Path) -> StructuralCache {
    let Ok(bytes) = std::fs::read(path) else {
        return StructuralCache::default();
    };
    let Ok(cache) = serde_json::from_slice::<StructuralCache>(&bytes) else {
        return StructuralCache::default();
    };
    if cache.version == STRUCTURAL_CACHE_VERSION {
        cache
    } else {
        StructuralCache::default()
    }
}

fn write_structural_cache(path: &Path, cache: &StructuralCache) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("cache path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create structural cache directory: {error}"))?;
    let _lock = FileLock::acquire(path)?;
    let bytes = serde_json::to_vec(cache)
        .map_err(|error| format!("cannot serialize structural cache: {error}"))?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = std::fs::File::create(&temporary)
        .map_err(|error| format!("cannot create structural cache: {error}"))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("cannot persist structural cache: {error}"))?;
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("cannot publish structural cache: {error}"))?;
    if let Ok(directory) = std::fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn read_semantic_cache(path: &Path) -> SemanticCache {
    let Ok(bytes) = std::fs::read(path) else {
        return SemanticCache::default();
    };
    let Ok(cache) = serde_json::from_slice::<SemanticCache>(&bytes) else {
        return SemanticCache::default();
    };
    if cache.version == SEMANTIC_CACHE_VERSION {
        cache
    } else {
        SemanticCache::default()
    }
}

fn write_semantic_cache(path: &Path, cache: &SemanticCache) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("cache path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create semantic cache directory: {error}"))?;
    let _lock = FileLock::acquire(path)?;
    let bytes = serde_json::to_vec(cache)
        .map_err(|error| format!("cannot serialize semantic cache: {error}"))?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = std::fs::File::create(&temporary)
        .map_err(|error| format!("cannot create semantic cache: {error}"))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("cannot persist semantic cache: {error}"))?;
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("cannot publish semantic cache: {error}"))?;
    if let Ok(directory) = std::fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn semantic_worker_count(document_count: usize) -> usize {
    std::env::var("JMAN_JAVA_LSP_SEMANTIC_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(2)
                .div_ceil(2)
                .clamp(1, 4)
        })
        .clamp(1, document_count.max(1))
}

fn analyze_semantic_parallel(
    pending: Vec<PendingSemanticDocument>,
    models: &[CompileModel],
    cancellation_generation: &AtomicU64,
    generation: u64,
    worker_count: usize,
) -> Result<Vec<PendingSemanticDocument>, String> {
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let assignments: Vec<Vec<PendingSemanticDocument>> = (0..worker_count)
        .map(|worker| {
            pending
                .iter()
                .skip(worker)
                .step_by(worker_count)
                .cloned()
                .collect()
        })
        .collect();
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = assignments
            .into_iter()
            .map(|assignment| {
                scope.spawn(move || -> Result<Vec<PendingSemanticDocument>, String> {
                    let frontend = Frontend::new()
                        .map_err(|error| format!("cannot create semantic isolate: {error:?}"))?;
                    let mut sessions = HashMap::new();
                    let mut completed = Vec::with_capacity(assignment.len());
                    for mut document in assignment {
                        if cancellation_generation.load(Ordering::Acquire) != generation {
                            return Err(
                                "semantic indexing cancelled by a newer project state".to_owned()
                            );
                        }
                        let model_key = format!(
                            "{}:{}",
                            document.model.project_path, document.model.task_path
                        );
                        if !sessions.contains_key(&model_key) {
                            let Some(release) = document.model.java_release() else {
                                completed.push(document);
                                continue;
                            };
                            let mut source_path = document.model.source_path();
                            if module_info_file(&document.model).is_none() {
                                for root in dependency_source_roots(&document.model, models) {
                                    if !source_path.contains(&root) {
                                        source_path.push(root);
                                    }
                                }
                            }
                            for attachment in source_attachments(&document.model.classpath) {
                                if !source_path.contains(&attachment) {
                                    source_path.push(attachment);
                                }
                            }
                            let Ok(session) = frontend.create_module_session(
                                &semantic_classpath(&document.model, models),
                                &semantic_module_path(&document.model, models),
                                &source_path,
                                module_info_file(&document.model).as_deref(),
                                &jpms_compiler_options(&document.model, models),
                                release,
                            ) else {
                                completed.push(document);
                                continue;
                            };
                            sessions.insert(model_key.clone(), session);
                        }
                        let file_name = document
                            .key
                            .rsplit(std::path::MAIN_SEPARATOR)
                            .next()
                            .unwrap_or("Input.java");
                        if let Ok(result) =
                            sessions[&model_key].analyze(file_name, &document.source)
                        {
                            document.fallback = result;
                        }
                        completed.push(document);
                    }
                    Ok(completed)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| "semantic index worker panicked".to_owned())?
            })
            .collect::<Result<Vec<_>, String>>()
    })?;
    let mut completed: Vec<_> = results.into_iter().flatten().collect();
    completed.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(completed)
}

fn parse_workspace_parallel(
    sources: &[WorkspaceSource],
    release: u8,
    preview: bool,
    cancellation_generation: &AtomicU64,
    generation: u64,
) -> Result<javac_frontend::WorkspaceParseResult, String> {
    let batches = workspace_batches(sources);
    let requested_workers = std::env::var("JMAN_JAVA_LSP_PARSE_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(2)
                .div_ceil(2)
                .clamp(1, 4)
        });
    let worker_count = requested_workers.clamp(1, batches.len().max(1));
    if worker_count == 1 {
        let frontend =
            Frontend::new().map_err(|error| format!("cannot create parse isolate: {error:?}"))?;
        let mut files = Vec::new();
        for batch in batches {
            if cancellation_generation.load(Ordering::Acquire) != generation {
                return Err("workspace parse cancelled by a newer project state".to_owned());
            }
            files.extend(
                frontend
                    .parse_workspace(&batch, release, preview)
                    .map_err(|error| format!("workspace parse failed: {error:?}"))?
                    .files,
            );
        }
        return Ok(javac_frontend::WorkspaceParseResult { files });
    }

    let assignments: Vec<Vec<(usize, Vec<WorkspaceSource>)>> = (0..worker_count)
        .map(|worker| {
            batches
                .iter()
                .enumerate()
                .filter(|(index, _)| index % worker_count == worker)
                .map(|(index, batch)| (index, batch.clone()))
                .collect()
        })
        .collect();
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = assignments
            .into_iter()
            .map(|assignment| {
                scope.spawn(
                    move || -> Result<Vec<(usize, Vec<javac_frontend::StructuralFile>)>, String> {
                        let frontend = Frontend::new()
                            .map_err(|error| format!("cannot create parse isolate: {error:?}"))?;
                        let mut parsed = Vec::new();
                        for (index, batch) in assignment {
                            let result = frontend
                                .parse_workspace(&batch, release, preview)
                                .map_err(|error| format!("workspace parse failed: {error:?}"))?;
                            parsed.push((index, result.files));
                        }
                        Ok(parsed)
                    },
                )
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| "workspace parse worker panicked".to_owned())?
            })
            .collect::<Result<Vec<_>, String>>()
    })?;
    if cancellation_generation.load(Ordering::Acquire) != generation {
        return Err("workspace parse cancelled by a newer project state".to_owned());
    }
    let mut ordered: Vec<_> = results.into_iter().flatten().collect();
    ordered.sort_by_key(|(index, _)| *index);
    Ok(javac_frontend::WorkspaceParseResult {
        files: ordered.into_iter().flat_map(|(_, files)| files).collect(),
    })
}

fn workspace_batches(sources: &[WorkspaceSource]) -> Vec<Vec<WorkspaceSource>> {
    let mut batches = Vec::new();
    let mut batch = Vec::new();
    let mut bytes = 0usize;
    for source in sources {
        let source_bytes = source.file_name.len() + source.source.len();
        if !batch.is_empty()
            && (batch.len() == PARSE_BATCH_FILES || bytes + source_bytes > PARSE_BATCH_BYTES)
        {
            batches.push(std::mem::take(&mut batch));
            bytes = 0;
        }
        bytes += source_bytes;
        batch.push(source.clone());
    }
    if !batch.is_empty() {
        batches.push(batch);
    }
    batches
}

fn processor_java(
    model: &CompileModel,
    build_runtime: Option<&BuildRuntime>,
) -> Result<PathBuf, String> {
    let executable_name = if cfg!(windows) { "java.exe" } else { "java" };
    let compiler_runtime = model
        .java_compiler_executable
        .as_deref()
        .and_then(Path::parent)
        .map(|bin| bin.join(executable_name));
    let build_runtime =
        build_runtime.map(|runtime| runtime.java_home.join("bin").join(executable_name));

    compiler_runtime
        .into_iter()
        .chain(build_runtime)
        .chain(environment_java())
        .find(|candidate| candidate.is_file())
        .map(|candidate| std::fs::canonicalize(&candidate).unwrap_or(candidate))
        .ok_or_else(|| {
            format!(
                "no Java runtime is available for annotation processors in {}:{}",
                model.project_path, model.task_path
            )
        })
}

fn environment_java() -> Option<PathBuf> {
    let executable_name = if cfg!(windows) { "java.exe" } else { "java" };
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .map(|home| home.join("bin").join(executable_name))
        .filter(|candidate| candidate.is_file())
        .or_else(|| {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|directory| directory.join(executable_name))
                    .find(|candidate| candidate.is_file())
            })
        })
        .map(|candidate| std::fs::canonicalize(&candidate).unwrap_or(candidate))
}

fn processor_worker_classpath() -> Result<std::path::PathBuf, String> {
    if let Some(path) = std::env::var_os("JMAN_JAVA_LSP_PROCESSOR_WORKER_CLASSPATH") {
        return Ok(path.into());
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        let packaged = directory.join("processor-worker.jar");
        if packaged.is_file() {
            return Ok(packaged);
        }
    }
    let development =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/java-test-classes");
    if development.is_dir() {
        return Ok(development);
    }
    Err("processor worker classes are unavailable".to_owned())
}

fn safe_key(key: &str) -> String {
    key.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn order_processor_models(models: &mut Vec<CompileModel>) {
    let mut remaining = std::mem::take(models);
    while !remaining.is_empty() {
        let next = remaining
            .iter()
            .enumerate()
            .filter(|(_, candidate)| {
                !remaining.iter().any(|dependency| {
                    (candidate
                        .project_dependencies
                        .iter()
                        .any(|project| project == &dependency.project_path)
                        || (!is_main_compile_model(candidate)
                            && is_main_compile_model(dependency)
                            && candidate.project_path == dependency.project_path))
                        && compile_model_key(candidate) != compile_model_key(dependency)
                })
            })
            .min_by_key(|(_, candidate)| {
                (
                    !is_main_compile_model(candidate),
                    candidate.project_path.as_str(),
                    candidate.task_path.as_str(),
                )
            })
            .map(|(index, _)| index)
            .unwrap_or_else(|| {
                remaining
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, candidate)| {
                        (
                            !is_main_compile_model(candidate),
                            candidate.project_path.as_str(),
                            candidate.task_path.as_str(),
                        )
                    })
                    .map_or(0, |(index, _)| index)
            });
        models.push(remaining.remove(next));
    }
}

fn processor_output_directories(request: &ProcessorRequest) -> Vec<PathBuf> {
    [
        partial_processor_classes_directory(&request.classes_directory),
        request.classes_directory.clone(),
    ]
    .into_iter()
    .filter(|directory| directory.is_dir())
    .collect()
}

fn partial_processor_classes_directory(target: &Path) -> PathBuf {
    let mut partial = PathBuf::new();
    let mut replaced = false;
    for component in target.components() {
        if !replaced && component.as_os_str() == "current" {
            partial.push("partial");
            replaced = true;
        } else {
            partial.push(component.as_os_str());
        }
    }
    if replaced {
        partial
    } else {
        target.with_file_name(format!(
            "{}.jman-java-partial",
            target.file_name().unwrap_or_default().to_string_lossy()
        ))
    }
}

fn dependency_source_roots(
    owner: &CompileModel,
    models: &[CompileModel],
) -> Vec<std::path::PathBuf> {
    let missing_classpath: Vec<_> = owner
        .classpath
        .iter()
        .chain(&owner.module_path)
        .filter(|path| !path.exists())
        .collect();
    let mut roots = Vec::new();
    for model in models.iter().filter(|model| {
        is_main_compile_model(model)
            && !compiled_output_available(model)
            && module_info_file(model).is_none()
            && (owner
                .project_dependencies
                .iter()
                .any(|project| project == &model.project_path)
                || missing_classpath
                    .iter()
                    .any(|artifact| artifact.starts_with(&model.project_directory)))
    }) {
        for root in model.source_path() {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    roots
}

fn processor_inputs_available(model: &CompileModel, models: &[CompileModel]) -> bool {
    if !model
        .annotation_processor_path
        .iter()
        .all(|path| path.exists())
        || !model.module_path.iter().all(|path| path.exists())
    {
        return false;
    }
    let dependencies = dependency_compile_models(model, models);
    model.classpath.iter().all(|path| {
        path.exists()
            || dependencies
                .iter()
                .any(|dependency| path.starts_with(&dependency.project_directory))
    })
}

fn dependency_compile_models<'a>(
    owner: &CompileModel,
    models: &'a [CompileModel],
) -> Vec<&'a CompileModel> {
    let mut pending = owner.project_dependencies.clone();
    let mut visited = std::collections::HashSet::new();
    let mut dependencies = Vec::new();
    while let Some(project) = pending.pop() {
        if !visited.insert(project.clone()) {
            continue;
        }
        for model in models.iter().filter(|candidate| {
            is_main_compile_model(candidate) && candidate.project_path == project
        }) {
            pending.extend(model.project_dependencies.iter().cloned());
            dependencies.push(model);
        }
    }
    dependencies
}

fn module_info_file(model: &CompileModel) -> Option<PathBuf> {
    model
        .source_files
        .iter()
        .find(|source| {
            source.file_name().and_then(|name| name.to_str()) == Some("module-info.java")
                && model
                    .source_roots
                    .iter()
                    .any(|root| source.starts_with(root))
        })
        .cloned()
        .or_else(|| {
            model
                .source_roots
                .iter()
                .map(|root| root.join("module-info.java"))
                .find(|source| source.is_file())
        })
}

fn jpms_compiler_options(model: &CompileModel, _models: &[CompileModel]) -> Vec<String> {
    const OPTIONS_WITH_VALUE: &[&str] = &[
        "--add-exports",
        "--add-modules",
        "--add-reads",
        "--limit-modules",
        "--module-version",
        "--patch-module",
        "--upgrade-module-path",
    ];
    let mut selected = Vec::new();
    let mut index = 0;
    while index < model.compiler_args.len() {
        let argument = &model.compiler_args[index];
        if argument == "--enable-preview"
            || OPTIONS_WITH_VALUE
                .iter()
                .any(|option| argument.starts_with(&format!("{option}=")))
        {
            selected.push(argument.clone());
        } else if OPTIONS_WITH_VALUE.contains(&argument.as_str())
            && let Some(value) = model.compiler_args.get(index + 1)
        {
            selected.push(argument.clone());
            selected.push(value.clone());
            index += 1;
        }
        index += 1;
    }
    selected
}

fn semantic_module_path(model: &CompileModel, models: &[CompileModel]) -> Vec<PathBuf> {
    let mut module_path = model.module_path.clone();
    if module_info_file(model).is_none() {
        return module_path;
    }
    for dependency in dependency_compile_models(model, models) {
        let Some(output) = dependency.destination_directory.as_ref() else {
            continue;
        };
        if output.join("module-info.class").is_file() && !module_path.contains(output) {
            module_path.push(output.clone());
        }
    }
    module_path
}

fn semantic_classpath(model: &CompileModel, models: &[CompileModel]) -> Vec<PathBuf> {
    let mut classpath = model.classpath.clone();
    if module_info_file(model).is_some() {
        return classpath;
    }
    for dependency in dependency_compile_models(model, models) {
        let Some(output) = dependency.destination_directory.as_ref() else {
            continue;
        };
        if compiled_output_available(dependency) && !classpath.contains(output) {
            classpath.push(output.clone());
        }
    }
    classpath
}

fn compiled_output_available(model: &CompileModel) -> bool {
    model
        .destination_directory
        .as_ref()
        .is_some_and(|output| output.is_dir())
}

fn java_module_name(source: &str) -> Option<&str> {
    let words: Vec<_> = source.split_whitespace().collect();
    words
        .windows(3)
        .find(|parts| parts[0] == "open" && parts[1] == "module")
        .map(|parts| parts[2].trim_end_matches('{'))
        .or_else(|| {
            words
                .windows(2)
                .find(|parts| parts[0] == "module")
                .map(|parts| parts[1].trim_end_matches('{'))
        })
}

fn is_main_compile_model(model: &CompileModel) -> bool {
    model.task_path == "compile" || model.task_path.ends_with(":compileJava")
}

fn is_java_compile_model(model: &CompileModel) -> bool {
    if matches!(model.task_path.as_str(), "compile" | "testCompile") {
        return true;
    }
    let task = model
        .task_path
        .rsplit(':')
        .next()
        .unwrap_or(&model.task_path);
    task.starts_with("compile") && task.ends_with("Java")
}

fn compile_source_set_name(task_path: &str) -> String {
    let task = task_path.rsplit(':').next().unwrap_or(task_path);
    match task {
        "compile" | "compileJava" => "main".to_owned(),
        "testCompile" | "compileTestJava" => "test".to_owned(),
        _ => task
            .strip_prefix("compile")
            .and_then(|name| name.strip_suffix("Java"))
            .map(|name| {
                let mut characters = name.chars();
                characters.next().map_or_else(String::new, |first| {
                    first.to_lowercase().chain(characters).collect()
                })
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| safe_key(task_path)),
    }
}

fn compile_model_key(model: &CompileModel) -> String {
    format!("{}:{}", model.project_path, model.task_path)
}

fn compile_model_fingerprint(model: &CompileModel) -> u64 {
    let mut hasher = DefaultHasher::new();
    model.schema_version.hash(&mut hasher);
    model.build_system.hash(&mut hasher);
    model.project_path.hash(&mut hasher);
    model.task_path.hash(&mut hasher);
    model.java_release().hash(&mut hasher);
    model.compiler_args.hash(&mut hasher);
    model.source_roots.hash(&mut hasher);
    model.classpath.hash(&mut hasher);
    model.module_path.hash(&mut hasher);
    model.annotation_processor_path.hash(&mut hasher);
    model.annotation_processor_options.hash(&mut hasher);
    hasher.finish()
}

fn processor_output_is_dependency(
    owner: &CompileModel,
    processed_key: &str,
    models: &[CompileModel],
) -> bool {
    let Some(processed) = models.iter().find(|candidate| {
        format!("{}:{}", candidate.project_path, candidate.task_path) == processed_key
    }) else {
        return false;
    };
    if !is_main_compile_model(processed) {
        return false;
    }
    if owner.project_path == processed.project_path && !is_main_compile_model(owner) {
        return true;
    }
    owner
        .classpath
        .iter()
        .chain(&owner.module_path)
        .any(|artifact| !artifact.exists() && artifact.starts_with(&processed.project_directory))
}

fn source_attachments(classpath: &[PathBuf]) -> Vec<PathBuf> {
    let mut attachments = Vec::new();
    for binary in classpath {
        let Some(stem) = binary.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if binary.extension().and_then(|extension| extension.to_str()) == Some("jar") {
            let source = binary.with_file_name(format!("{stem}-sources.jar"));
            if source.is_file() {
                attachments.push(source);
                continue;
            }
            // Gradle stores every artifact variant below a separate content
            // hash directory under the same module version.
            if let Some(version_directory) = binary.parent().and_then(Path::parent)
                && let Ok(hash_directories) = std::fs::read_dir(version_directory)
            {
                for hash_directory in hash_directories.flatten() {
                    let source = hash_directory.path().join(format!("{stem}-sources.jar"));
                    if source.is_file() {
                        attachments.push(source);
                        break;
                    }
                }
            }
        }
    }
    if let Some(java_home) = std::env::var_os("JAVA_HOME") {
        let source = PathBuf::from(java_home).join("lib/src.zip");
        if source.is_file() {
            attachments.push(source);
        }
    }
    attachments
}

fn collect_java_sources(directory: &std::path::Path, sources: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_java_sources(&path, sources);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("java")
            && path.file_name().and_then(|name| name.to_str()) != Some("module-info.java")
            && !sources.contains(&path)
        {
            sources.push(path);
        }
    }
}

fn external_type_catalog(models: &[CompileModel]) -> SemanticResult {
    let mut types = std::collections::BTreeMap::<String, (String, String)>::new();
    if let Some(java_home) = std::env::var_os("JAVA_HOME") {
        let source = PathBuf::from(java_home).join("lib/src.zip");
        collect_source_archive_types(&source, &mut types);
    }
    let mut archives = std::collections::BTreeMap::<PathBuf, u8>::new();
    for model in models {
        let release = model.java_release().unwrap_or(8);
        for archive in model
            .classpath
            .iter()
            .chain(&model.module_path)
            .chain(&model.annotation_processor_path)
        {
            if archive.extension().and_then(|extension| extension.to_str()) == Some("jar")
                && !archive
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem.ends_with("-sources") || stem.ends_with("-javadoc"))
            {
                archives
                    .entry(archive.clone())
                    .and_modify(|current| *current = (*current).min(release))
                    .or_insert(release);
            }
        }
    }
    for (archive, release) in archives {
        collect_class_archive_types(&archive, release, &mut types);
    }
    SemanticResult {
        package_name: String::new(),
        symbols: types
            .into_iter()
            .map(
                |(qualified_name, (module, binary_name))| javac_frontend::SemanticSymbol {
                    role: "reference".to_owned(),
                    kind: "class".to_owned(),
                    name: qualified_name
                        .rsplit('.')
                        .next()
                        .unwrap_or(&qualified_name)
                        .to_owned(),
                    symbol_id: format!("{module}|{binary_name}"),
                    qualified_name,
                    start: 0,
                    end: 0,
                },
            )
            .collect(),
        diagnostics: Vec::new(),
    }
}

fn collect_source_archive_types(
    archive_path: &Path,
    types: &mut std::collections::BTreeMap<String, (String, String)>,
) {
    let Ok(file) = std::fs::File::open(archive_path) else {
        return;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return;
    };
    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index(index) else {
            continue;
        };
        let name = entry.name();
        let Some(path) = name.strip_suffix(".java") else {
            continue;
        };
        let Some((module, binary_name)) = path.split_once('/') else {
            continue;
        };
        if binary_name.ends_with("module-info")
            || binary_name.ends_with("package-info")
            || binary_name.contains('$')
        {
            continue;
        }
        let qualified_name = binary_name.replace('/', ".");
        types
            .entry(qualified_name)
            .or_insert_with(|| (module.to_owned(), binary_name.to_owned()));
    }
}

fn collect_class_archive_types(
    archive_path: &Path,
    release: u8,
    types: &mut std::collections::BTreeMap<String, (String, String)>,
) {
    let Ok(file) = std::fs::File::open(archive_path) else {
        return;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return;
    };
    for (binary_name, _) in multi_release_entries(&mut archive, release, ".class") {
        let Some(binary_name) = binary_name.strip_suffix(".class") else {
            continue;
        };
        if binary_name.ends_with("module-info")
            || binary_name.ends_with("package-info")
            || binary_name.contains('$')
        {
            continue;
        }
        let qualified_name = binary_name.replace('/', ".");
        types
            .entry(qualified_name)
            .or_insert_with(|| ("<unnamed>".to_owned(), binary_name.to_owned()));
    }
}

fn processor_source_files(model: &CompileModel) -> Vec<PathBuf> {
    let generated: Vec<&Path> = model
        .generated_sources_directory
        .iter()
        .chain(&model.generated_source_directories)
        .map(PathBuf::as_path)
        .collect();
    let mut sources = Vec::new();
    for root in &model.source_roots {
        if !generated
            .iter()
            .any(|candidate| root.starts_with(candidate))
        {
            collect_java_sources(root, &mut sources);
        }
    }
    for source in &model.source_files {
        if source.extension().and_then(|extension| extension.to_str()) == Some("java")
            && source.file_name().and_then(|name| name.to_str()) != Some("module-info.java")
            && source.is_file()
            && !generated
                .iter()
                .any(|candidate| source.starts_with(candidate))
            && !sources.contains(source)
        {
            sources.push(source.clone());
        }
    }
    sources.sort();
    sources
}

fn decompile_definition(
    classpath: &[PathBuf],
    release: u8,
    definition: &mut javac_frontend::EditorDefinition,
) -> Result<(), String> {
    let (class_bytes, entry_name) = find_class_bytes(classpath, &definition.owner, release)?;
    let hash = hex::encode(Sha256::digest(&class_bytes));
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("io.github.zonnedev.jman.lsp/decompiled")
        .join(format!("vineflower-1.12.0-{hash}"));
    std::fs::create_dir_all(&cache)
        .map_err(|error| format!("cannot create decompiler cache: {error}"))?;
    let _lock = FileLock::acquire(&cache.join("decompiler-cache"))?;
    let simple_name = entry_name
        .rsplit('/')
        .next()
        .unwrap_or("External.class")
        .trim_end_matches(".class");
    let source_path = cache.join(format!("{simple_name}.java"));
    let source = if let Ok(source) = std::fs::read_to_string(&source_path) {
        source
    } else {
        let class_path = cache.join(&entry_name);
        let output = cache.join("output");
        if let Some(parent) = class_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create decompiler input: {error}"))?;
        }
        std::fs::create_dir_all(&output)
            .map_err(|error| format!("cannot create decompiler output: {error}"))?;
        std::fs::write(&class_path, &class_bytes)
            .map_err(|error| format!("cannot write decompiler input: {error}"))?;
        let vineflower = vineflower_jar()?;
        let process =
            std::process::Command::new(environment_java().unwrap_or_else(|| PathBuf::from("java")))
                .arg("-jar")
                .arg(vineflower)
                .args(["-dgs=1", "-asc=1", "-rbr=1", "-rsy=1"])
                .arg(&class_path)
                .arg(&output)
                .output()
                .map_err(|error| format!("cannot start Vineflower: {error}"))?;
        if !process.status.success() {
            return Err(format!(
                "Vineflower failed: {}",
                String::from_utf8_lossy(&process.stderr)
            ));
        }
        let generated = find_generated_source(&output, simple_name)
            .ok_or_else(|| "Vineflower produced no Java source".to_owned())?;
        let generated_source = std::fs::read_to_string(generated)
            .map_err(|error| format!("cannot read Vineflower source: {error}"))?;
        let header = format!(
            "/* Decompiled from {} with Vineflower 1.12.0. Read-only cache. */\n\n",
            entry_name
        );
        let source = header + &generated_source;
        std::fs::write(&source_path, &source)
            .map_err(|error| format!("cannot persist Vineflower source: {error}"))?;
        source
    };
    let (start, end) =
        find_decompiled_declaration(&source, &definition.name, &definition.descriptor).ok_or_else(
            || {
                format!(
                    "cannot locate {} in Vineflower output for {}",
                    definition.name, definition.owner
                )
            },
        )?;
    definition.source_name = format!("{simple_name}.java");
    definition.source = source;
    definition.start = start as u64;
    definition.end = end as u64;
    definition.decompiled = true;
    Ok(())
}

fn find_class_bytes(
    classpath: &[PathBuf],
    owner: &str,
    release: u8,
) -> Result<(Vec<u8>, String), String> {
    let mut candidates = vec![format!("{}.class", owner.replace('.', "/"))];
    let mut nested = owner.to_owned();
    while let Some(index) = nested.rfind('.') {
        nested.replace_range(index..=index, "$");
        candidates.push(format!("{}.class", nested.replace('.', "/")));
    }
    for entry_path in classpath {
        if entry_path.is_dir() {
            for candidate in &candidates {
                let class_file = entry_path.join(candidate);
                if let Ok(bytes) = std::fs::read(&class_file) {
                    return Ok((bytes, candidate.clone()));
                }
            }
            continue;
        }
        if entry_path
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("jar")
        {
            continue;
        }
        let Ok(file) = std::fs::File::open(entry_path) else {
            continue;
        };
        let Ok(mut archive) = zip::ZipArchive::new(file) else {
            continue;
        };
        for candidate in &candidates {
            let selected = multi_release_entry(&mut archive, candidate, release);
            if let Some(selected) = selected
                && let Ok(mut entry) = archive.by_name(&selected)
            {
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut bytes).map_err(|error| {
                    format!(
                        "cannot read {} from {}: {error}",
                        candidate,
                        entry_path.display()
                    )
                })?;
                return Ok((bytes, candidate.clone()));
            }
        }
    }
    Err(format!(
        "class bytes for {owner} are not on the compile classpath"
    ))
}

fn multi_release_entries<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    release: u8,
    suffix: &str,
) -> std::collections::BTreeMap<String, String> {
    let enabled = multi_release_manifest(archive);
    let mut selected = std::collections::BTreeMap::<String, (u16, String)>::new();
    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index(index) else {
            continue;
        };
        let name = entry.name().to_owned();
        if !name.ends_with(suffix) {
            continue;
        }
        let (version, logical) = if let Some(versioned) = name.strip_prefix("META-INF/versions/") {
            let Some((version, logical)) = versioned.split_once('/') else {
                continue;
            };
            let Ok(version) = version.parse::<u16>() else {
                continue;
            };
            if !enabled || version > u16::from(release) || version < 9 {
                continue;
            }
            (version, logical.to_owned())
        } else if name.starts_with("META-INF/") {
            continue;
        } else {
            (0, name.clone())
        };
        let current = selected.entry(logical).or_insert((version, name.clone()));
        if version > current.0 {
            *current = (version, name);
        }
    }
    selected
        .into_iter()
        .map(|(logical, (_, physical))| (logical, physical))
        .collect()
}

fn multi_release_entry<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    logical_name: &str,
    release: u8,
) -> Option<String> {
    multi_release_entries(archive, release, "").remove(logical_name)
}

fn multi_release_manifest<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> bool {
    let Ok(mut manifest) = archive.by_name("META-INF/MANIFEST.MF") else {
        return false;
    };
    let mut contents = String::new();
    if std::io::Read::read_to_string(&mut manifest, &mut contents).is_err() {
        return false;
    }
    contents.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("Multi-Release") && value.trim().eq_ignore_ascii_case("true")
        })
    })
}

fn jar_module_name(path: &Path, release: u8) -> Option<String> {
    if path.extension().and_then(|value| value.to_str()) != Some("jar") {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    if let Some(name) = manifest_attribute(&mut archive, "Automatic-Module-Name") {
        return Some(name);
    }
    if let Some(descriptor) = multi_release_entry(&mut archive, "module-info.class", release)
        && let Ok(mut entry) = archive.by_name(&descriptor)
    {
        let mut bytes = Vec::new();
        if std::io::Read::read_to_end(&mut entry, &mut bytes).is_ok()
            && let Some(name) = module_name_from_class(&bytes)
        {
            return Some(name);
        }
    }
    automatic_module_name(path.file_stem()?.to_str()?)
}

fn module_name_from_class(bytes: &[u8]) -> Option<String> {
    if bytes.get(..4)? != [0xca, 0xfe, 0xba, 0xbe] {
        return None;
    }
    let mut cursor = 8;
    let constant_pool_count = read_u16(bytes, &mut cursor)? as usize;
    let mut utf8 = vec![None; constant_pool_count];
    let mut modules = vec![None; constant_pool_count];
    let mut index = 1;
    while index < constant_pool_count {
        let tag = *bytes.get(cursor)?;
        cursor += 1;
        match tag {
            1 => {
                let length = read_u16(bytes, &mut cursor)? as usize;
                let value = std::str::from_utf8(bytes.get(cursor..cursor + length)?)
                    .ok()?
                    .to_owned();
                cursor += length;
                utf8[index] = Some(value);
            }
            3 | 4 => cursor += 4,
            5 | 6 => {
                cursor += 8;
                index += 1;
            }
            7 | 8 | 16 | 19 | 20 => {
                let name = read_u16(bytes, &mut cursor)? as usize;
                if tag == 19 {
                    modules[index] = Some(name);
                }
            }
            9 | 10 | 11 | 12 | 17 | 18 => cursor += 4,
            15 => cursor += 3,
            _ => return None,
        }
        bytes.get(cursor.saturating_sub(1))?;
        index += 1;
    }
    cursor += 6;
    let interfaces = read_u16(bytes, &mut cursor)? as usize;
    cursor += interfaces * 2;
    skip_class_members(bytes, &mut cursor)?;
    skip_class_members(bytes, &mut cursor)?;
    let attributes = read_u16(bytes, &mut cursor)? as usize;
    for _ in 0..attributes {
        let name_index = read_u16(bytes, &mut cursor)? as usize;
        let length = read_u32(bytes, &mut cursor)? as usize;
        if utf8.get(name_index)?.as_deref() == Some("Module") {
            let module_index = read_u16(bytes, &mut cursor)? as usize;
            let utf8_index = modules.get(module_index)?.as_ref()?;
            return utf8.get(*utf8_index)?.clone();
        }
        cursor += length;
        bytes.get(cursor.saturating_sub(1))?;
    }
    None
}

fn skip_class_members(bytes: &[u8], cursor: &mut usize) -> Option<()> {
    let count = read_u16(bytes, cursor)? as usize;
    for _ in 0..count {
        *cursor += 6;
        let attributes = read_u16(bytes, cursor)? as usize;
        for _ in 0..attributes {
            *cursor += 2;
            let length = read_u32(bytes, cursor)? as usize;
            *cursor += length;
            bytes.get(cursor.saturating_sub(1))?;
        }
    }
    Some(())
}

fn read_u16(bytes: &[u8], cursor: &mut usize) -> Option<u16> {
    let value = u16::from_be_bytes(bytes.get(*cursor..*cursor + 2)?.try_into().ok()?);
    *cursor += 2;
    Some(value)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Option<u32> {
    let value = u32::from_be_bytes(bytes.get(*cursor..*cursor + 4)?.try_into().ok()?);
    *cursor += 4;
    Some(value)
}

fn manifest_attribute<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    attribute: &str,
) -> Option<String> {
    let mut manifest = archive.by_name("META-INF/MANIFEST.MF").ok()?;
    let mut contents = String::new();
    std::io::Read::read_to_string(&mut manifest, &mut contents).ok()?;
    let mut unfolded = Vec::<String>::new();
    for line in contents.lines() {
        if let Some(continuation) = line.strip_prefix(' ')
            && let Some(previous) = unfolded.last_mut()
        {
            previous.push_str(continuation);
        } else {
            unfolded.push(line.to_owned());
        }
    }
    unfolded.into_iter().find_map(|line| {
        line.split_once(':').and_then(|(name, value)| {
            name.eq_ignore_ascii_case(attribute)
                .then(|| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        })
    })
}

fn automatic_module_name(file_stem: &str) -> Option<String> {
    let version = file_stem
        .char_indices()
        .find(|(index, character)| {
            *character == '-'
                && file_stem[*index + 1..]
                    .chars()
                    .next()
                    .is_some_and(|next| next.is_ascii_digit())
        })
        .map(|(index, _)| index)
        .unwrap_or(file_stem.len());
    let mut name = String::new();
    let mut separator = false;
    for character in file_stem[..version].chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !name.is_empty() {
                name.push('.');
            }
            name.push(character);
            separator = false;
        } else {
            separator = true;
        }
    }
    (!name.is_empty()).then_some(name)
}

fn java_package_name(source: &str) -> Option<&str> {
    let declaration = source.find("package ")? + "package ".len();
    let remaining = &source[declaration..];
    let end = remaining.find(';')?;
    let name = remaining[..end].trim();
    (!name.is_empty()
        && name
            .chars()
            .all(|character| character.is_alphanumeric() || character == '.' || character == '_'))
    .then_some(name)
}

fn vineflower_jar() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("JMAN_JAVA_LSP_VINEFLOWER_JAR") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        let packaged = directory.join("vineflower.jar");
        if packaged.is_file() {
            return Ok(packaged);
        }
    }
    let development =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/vineflower-1.12.0.jar");
    development
        .is_file()
        .then_some(development)
        .ok_or_else(|| "Vineflower 1.12.0 is unavailable".to_owned())
}

fn find_generated_source(directory: &Path, simple_name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(directory).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_generated_source(&path, simple_name) {
                return Some(found);
            }
        } else if path.file_stem().and_then(|stem| stem.to_str()) == Some(simple_name)
            && path.extension().and_then(|extension| extension.to_str()) == Some("java")
        {
            return Some(path);
        }
    }
    None
}

fn find_decompiled_declaration(
    source: &str,
    name: &str,
    descriptor: &str,
) -> Option<(usize, usize)> {
    if name.is_empty() || name.starts_with('<') {
        return None;
    }
    let needle = format!("{name}(");
    let expected_types = descriptor_parameter_types(descriptor);
    for (start, _) in source.match_indices(&needle) {
        let parameters_start = start + needle.len();
        let close = matching_parenthesis(source, parameters_start)?;
        let parameters = &source[parameters_start..close];
        let count = if parameters.trim().is_empty() {
            0
        } else {
            top_level_commas(parameters) + 1
        };
        if count == expected_types.len()
            && declaration_parameter_types(parameters) == expected_types
        {
            return Some((start, start + name.len()));
        }
    }
    source
        .match_indices(name)
        .find(|(start, _)| {
            let before = source[..*start].chars().next_back();
            let after = source[*start + name.len()..].chars().next();
            before.is_none_or(|character| !CharacterExt::java_identifier_part(character))
                && after.is_none_or(|character| !CharacterExt::java_identifier_part(character))
        })
        .map(|(start, _)| (start, start + name.len()))
}

fn descriptor_parameter_types(descriptor: &str) -> Vec<String> {
    let Some(open) = descriptor.find('(') else {
        return Vec::new();
    };
    let Some(close) = descriptor[open + 1..].find(')') else {
        return Vec::new();
    };
    let parameters = &descriptor[open + 1..open + 1 + close];
    if parameters
        .chars()
        .next()
        .is_some_and(|character| "ZBSIJCFDL[".contains(character))
    {
        return jvm_parameter_types(parameters);
    }
    split_top_level(parameters)
        .into_iter()
        .map(normalized_java_type)
        .collect()
}

fn jvm_parameter_types(parameters: &str) -> Vec<String> {
    let bytes = parameters.as_bytes();
    let mut index = 0usize;
    let mut types = Vec::new();
    while index < bytes.len() {
        let mut arrays = 0usize;
        while bytes.get(index) == Some(&b'[') {
            arrays += 1;
            index += 1;
        }
        let Some(kind) = bytes.get(index).copied() else {
            break;
        };
        index += 1;
        let name = match kind {
            b'Z' => "boolean".to_owned(),
            b'B' => "byte".to_owned(),
            b'S' => "short".to_owned(),
            b'I' => "int".to_owned(),
            b'J' => "long".to_owned(),
            b'C' => "char".to_owned(),
            b'F' => "float".to_owned(),
            b'D' => "double".to_owned(),
            b'L' => {
                let end = parameters[index..]
                    .find(';')
                    .map(|offset| index + offset)
                    .unwrap_or(parameters.len());
                let binary = &parameters[index..end];
                index = end.saturating_add(1);
                binary
                    .rsplit(['/', '$'])
                    .next()
                    .unwrap_or(binary)
                    .to_owned()
            }
            _ => "Object".to_owned(),
        };
        types.push(name + &"[]".repeat(arrays));
    }
    types
}

fn declaration_parameter_types(parameters: &str) -> Vec<String> {
    split_top_level(parameters)
        .into_iter()
        .map(|parameter| {
            let declaration = parameter.trim().trim_start_matches("final ");
            let type_end = declaration
                .rfind(char::is_whitespace)
                .unwrap_or(declaration.len());
            normalized_java_type(&declaration[..type_end])
        })
        .collect()
}

fn split_top_level(source: &str) -> Vec<&str> {
    if source.trim().is_empty() {
        return Vec::new();
    }
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut values = Vec::new();
    for (offset, character) in source.char_indices() {
        match character {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                values.push(&source[start..offset]);
                start = offset + 1;
            }
            _ => {}
        }
    }
    values.push(&source[start..]);
    values
}

fn normalized_java_type(source: &str) -> String {
    let mut result = String::new();
    let mut generic_depth = 0usize;
    for character in source.replace("...", "[]").chars() {
        match character {
            '<' => generic_depth += 1,
            '>' => generic_depth = generic_depth.saturating_sub(1),
            character if generic_depth == 0 && !character.is_whitespace() => result.push(character),
            _ => {}
        }
    }
    result
        .split(['.', '$'])
        .next_back()
        .unwrap_or(&result)
        .to_owned()
}

fn matching_parenthesis(source: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, character) in source[start..].char_indices() {
        match character {
            '(' | '<' | '[' => depth += 1,
            ')' if depth == 0 => return Some(start + offset),
            ')' | '>' | ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn top_level_commas(source: &str) -> usize {
    let mut depth = 0usize;
    let mut commas = 0usize;
    for character in source.chars() {
        match character {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => commas += 1,
            _ => {}
        }
    }
    commas
}

trait CharacterExt {
    fn java_identifier_part(self) -> bool;
}

impl CharacterExt for char {
    fn java_identifier_part(self) -> bool {
        self.is_alphanumeric() || matches!(self, '_' | '$')
    }
}

impl AnalysisBackend for NativeBackend {
    fn build_system(&self) -> Option<String> {
        self.models.first().map(|model| model.build_system.clone())
    }

    fn initialize(
        &mut self,
        root_uri: Option<&str>,
        options: Option<&Value>,
    ) -> Result<(), String> {
        let root_uri = root_uri.ok_or_else(|| "initialize has no rootUri".to_owned())?;
        let root =
            file_uri_to_path(root_uri).ok_or_else(|| "rootUri is not a file URI".to_owned())?;
        let preference = options.and_then(|value| value["buildSystem"].as_str());
        let loaded = load_project(&root, preference)?;
        self.cache_status = CacheStatus {
            project_id: stable_project_id(&root),
            directory: project_cache_directory(&root)
                .to_string_lossy()
                .into_owned(),
            ..CacheStatus::default()
        };
        self.record_build_runtime(loaded.build_runtime.as_ref());
        self.root = Some(root);
        self.preferred_build_system = preference.map(str::to_owned);
        self.build_runtime = loaded.build_runtime;
        self.models = loaded.models;
        self.sessions.clear();
        self.processed_semantic_workers.clear();
        self.editor_queries.clear();
        self.run_annotation_processors()?;
        self.rebuild_structural_index()?;
        Ok(())
    }

    fn analyze(
        &mut self,
        uri: &str,
        file_name: &str,
        source: &str,
    ) -> Result<SemanticResult, String> {
        let source_file =
            file_uri_to_path(uri).ok_or_else(|| "URI is not a file URI".to_owned())?;
        let model = select_compile_model(&self.models, &source_file).cloned();
        if let Some(model) = model
            && !model.annotation_processor_path.is_empty()
            && std::env::var_os("JMAN_JAVA_LSP_DISABLE_ANNOTATION_PROCESSING").is_none()
        {
            return self
                .analyze_processed(&model, file_name, source)
                .map_err(|error| format!("processed javac semantic analysis failed: {error}"));
        }
        self.session_for(uri)?
            .analyze(file_name, source)
            .map_err(|error| format!("native semantic analysis failed: {error:?}"))
    }

    fn editor_query(
        &mut self,
        uri: &str,
        file_name: &str,
        source: &str,
        cursor: u32,
    ) -> Result<EditorQueryResult, String> {
        let cache_key = EditorQueryCache::key(uri, source, cursor);
        if let Some(result) = self.editor_queries.get(&cache_key) {
            return Ok(result);
        }
        let source_file =
            file_uri_to_path(uri).ok_or_else(|| "URI is not a file URI".to_owned())?;
        let selected_model = select_compile_model(&self.models, &source_file).cloned();
        let classpath = selected_model
            .as_ref()
            .map(|model| {
                model
                    .classpath
                    .iter()
                    .chain(&model.module_path)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let release = selected_model
            .as_ref()
            .and_then(CompileModel::java_release)
            .unwrap_or(8);
        let mut result = if let Some(model) = selected_model.as_ref()
            && !model.annotation_processor_path.is_empty()
            && std::env::var_os("JMAN_JAVA_LSP_DISABLE_ANNOTATION_PROCESSING").is_none()
        {
            self.query_processed(model, file_name, source, cursor)
                .map_err(|error| format!("processed javac editor query failed: {error}"))?
        } else {
            self.session_for(uri)?
                .editor_query(file_name, source, cursor)
                .map_err(|error| format!("native editor query failed: {error:?}"))?
        };
        if let Some(definition) = result.definition.as_mut()
            && definition.source.is_empty()
        {
            let _ = decompile_definition(&classpath, release, definition);
        }
        self.editor_queries.insert(cache_key, result.clone());
        Ok(result)
    }

    fn workspace_source_files(&self) -> Vec<std::path::PathBuf> {
        workspace_source_files(&self.models)
    }

    fn jpms_catalog(&self, uri: &str) -> JpmsCatalog {
        let source_file = file_uri_to_path(uri);
        let selected = source_file
            .as_deref()
            .and_then(|source| select_compile_model(&self.models, source));
        let release = selected.and_then(CompileModel::java_release).unwrap_or(25);
        let mut modules = std::collections::BTreeSet::new();
        let mut packages = std::collections::BTreeSet::new();
        for model in &self.models {
            if let Some(descriptor) = module_info_file(model)
                && let Ok(source) = std::fs::read_to_string(descriptor)
                && let Some(name) = java_module_name(&source)
            {
                modules.insert(name.to_owned());
            }
        }
        if let Some(java_home) = std::env::var_os("JAVA_HOME")
            && let Ok(entries) = std::fs::read_dir(PathBuf::from(java_home).join("jmods"))
        {
            for entry in entries.flatten() {
                if entry.path().extension().and_then(|value| value.to_str()) == Some("jmod")
                    && let Some(name) = entry.path().file_stem().and_then(|value| value.to_str())
                {
                    modules.insert(name.to_owned());
                }
            }
        }
        if let Some(model) = selected {
            for archive in model.classpath.iter().chain(&model.module_path) {
                if let Some(name) = jar_module_name(archive, release) {
                    modules.insert(name);
                }
            }
            for source in &model.source_files {
                if let Ok(contents) = std::fs::read_to_string(source)
                    && let Some(package) = java_package_name(&contents)
                {
                    packages.insert(package.to_owned());
                }
            }
        }
        JpmsCatalog {
            modules: modules.into_iter().collect(),
            packages: packages.into_iter().collect(),
        }
    }

    fn workspace_structural_documents(&self) -> Vec<(String, String, SemanticResult)> {
        self.structural_documents.clone()
    }

    fn workspace_structural_revision(&self) -> u64 {
        self.structural_revision
    }

    fn close(&mut self, uri: &str) {
        self.editor_queries.invalidate(uri);
        if let Ok(session) = self.session_for(uri) {
            let file_name = uri.rsplit('/').next().unwrap_or(uri);
            let _ = session.invalidate(file_name);
        }
    }

    fn invalidate(&mut self, documents: &[String]) {
        for document in documents {
            self.editor_queries.invalidate(document);
            if let Ok(session) = self.session_for(document) {
                let file_name = document.rsplit('/').next().unwrap_or(document);
                let _ = session.invalidate(file_name);
            }
        }
    }

    fn reload(&mut self, changed_uris: &[String]) -> Result<bool, String> {
        if !changed_uris.is_empty() && !changed_uris.iter().any(|uri| is_build_file(uri)) {
            return Ok(false);
        }
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| "project has not been initialized".to_owned())?;
        let loaded = load_project(root, self.preferred_build_system.as_deref())?;
        let build_runtime = loaded.build_runtime;
        let previous: HashMap<_, _> = self
            .models
            .iter()
            .map(|model| (compile_model_key(model), compile_model_fingerprint(model)))
            .collect();
        let next: HashMap<_, _> = loaded
            .models
            .iter()
            .map(|model| (compile_model_key(model), compile_model_fingerprint(model)))
            .collect();
        let changed: std::collections::HashSet<_> = previous
            .keys()
            .chain(next.keys())
            .filter(|key| previous.get(*key) != next.get(*key))
            .cloned()
            .collect();
        let previous_models = std::mem::replace(&mut self.models, loaded.models);
        let previous_build_runtime =
            std::mem::replace(&mut self.build_runtime, build_runtime.clone());
        let previous_structural = self.structural_documents.clone();
        let previous_revision = self.structural_revision;
        let previous_status = self.cache_status.clone();
        if let Err(message) = self
            .run_annotation_processors()
            .and_then(|()| self.rebuild_structural_index())
        {
            self.models = previous_models;
            self.build_runtime = previous_build_runtime;
            self.structural_documents = previous_structural;
            self.structural_revision = previous_revision;
            self.cache_status = previous_status;
            return Err(message);
        }
        self.sessions.retain(|key, _| !changed.contains(key));
        self.processed_semantic_workers.clear();
        self.editor_queries.entries.retain(|key, _| {
            file_uri_to_path(&key.uri)
                .and_then(|path| select_compile_model(&previous_models, &path))
                .is_some_and(|model| !changed.contains(&compile_model_key(model)))
        });
        self.record_build_runtime(build_runtime.as_ref());
        Ok(true)
    }

    fn source_saved(&mut self, uri: &str) -> Result<bool, String> {
        let Some(path) = file_uri_to_path(uri) else {
            return Ok(false);
        };
        if path.extension().and_then(|extension| extension.to_str()) != Some("java")
            || select_compile_model(&self.models, &path).is_none()
        {
            return Ok(false);
        }
        self.run_annotation_processors()?;
        self.sessions.clear();
        self.processed_semantic_workers.clear();
        self.editor_queries.clear();
        self.rebuild_structural_index()?;
        Ok(true)
    }

    fn cache_status(&self) -> CacheStatus {
        self.cache_status.clone()
    }

    fn rebuild_index(&mut self) -> Result<(), String> {
        let root = self
            .root
            .clone()
            .ok_or_else(|| "project has not been initialized".to_owned())?;
        let loaded = load_project(&root, self.preferred_build_system.as_deref())?;
        let build_runtime = loaded.build_runtime;
        self.models = loaded.models;
        self.record_build_runtime(build_runtime.as_ref());
        self.build_runtime = build_runtime;
        for path in [structural_cache_path(&root), semantic_cache_path(&root)] {
            if path.is_file() {
                std::fs::remove_file(&path)
                    .map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
            }
        }
        self.sessions.clear();
        self.processed_semantic_workers.clear();
        self.editor_queries.clear();
        self.run_annotation_processors()?;
        self.rebuild_structural_index()?;
        self.refine_semantic_index()?;
        Ok(())
    }

    fn clear_project_cache(&mut self) -> Result<(), String> {
        let root = self
            .root
            .as_deref()
            .ok_or_else(|| "project has not been initialized".to_owned())?;
        let directory = project_cache_directory(root);
        if directory.is_dir() {
            std::fs::remove_dir_all(&directory)
                .map_err(|error| format!("cannot clear {}: {error}", directory.display()))?;
        }
        self.sessions.clear();
        self.processed_semantic_workers.clear();
        self.editor_queries.clear();
        self.structural_documents.clear();
        self.cache_status.structural_entries = 0;
        self.cache_status.semantic_entries = 0;
        self.cache_status.external_entries = 0;
        self.cache_status.bytes = 0;
        Ok(())
    }

    fn update_configuration(&mut self, settings: &Value) -> Result<(), String> {
        let preference = settings["buildSystem"]
            .as_str()
            .filter(|value| *value != "auto")
            .map(str::to_owned);
        if preference == self.preferred_build_system {
            return Ok(());
        }
        let previous = std::mem::replace(&mut self.preferred_build_system, preference);
        if let Err(error) = self.reload(&[]) {
            self.preferred_build_system = previous;
            return Err(error);
        }
        Ok(())
    }

    fn source_metadata(&self, uri: &str) -> SourceMetadata {
        let Some(path) = file_uri_to_path(uri) else {
            return SourceMetadata::default();
        };
        let Some(model) = select_compile_model(&self.models, &path) else {
            return SourceMetadata::default();
        };
        let conventional_generated = model.project_directory.join("target/generated-sources");
        let generated = model
            .generated_sources_directory
            .iter()
            .chain(&model.generated_source_directories)
            .chain(std::iter::once(&conventional_generated))
            .any(|root| path.starts_with(root));
        SourceMetadata {
            generated,
            read_only: generated,
            origin: generated.then(|| format!("{}:{}", model.project_path, model.task_path)),
        }
    }
}

fn workspace_source_files(models: &[CompileModel]) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    for model in models {
        if !is_java_compile_model(model) {
            continue;
        }
        for root in &model.source_roots {
            collect_java_sources(root, &mut sources);
        }
        for source in &model.source_files {
            if !sources.contains(source) {
                sources.push(source.clone());
            }
        }
        collect_java_sources(
            &model.project_directory.join("target/generated-sources"),
            &mut sources,
        );
        if let Some(generated) = &model.generated_sources_directory {
            collect_java_sources(generated, &mut sources);
        }
        for generated in &model.generated_source_directories {
            collect_java_sources(generated, &mut sources);
        }
    }
    sources.sort();
    sources.dedup();
    sources
}

fn is_build_file(uri: &str) -> bool {
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
mod decompiler_tests {
    use super::*;

    fn empty_editor_result() -> EditorQueryResult {
        EditorQueryResult {
            completions: Vec::new(),
            signatures: Vec::new(),
            hover: None,
            definition: None,
            type_definition: None,
        }
    }

    fn processor_model(root: &Path, compiler: Option<PathBuf>) -> CompileModel {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 2,
            "buildSystem": "gradle",
            "projectPath": ":app",
            "projectDirectory": root,
            "taskPath": ":app:compileJava",
            "sourceFiles": [],
            "sourceRoots": [],
            "classpath": [],
            "modulePath": [],
            "projectDependencies": [],
            "annotationProcessorPath": [],
            "annotationProcessorOptions": [],
            "compilerArgs": [],
            "release": 21,
            "encoding": "UTF-8",
            "generatedSourcesDirectory": null,
            "generatedSourceDirectories": [],
            "destinationDirectory": null,
            "javaCompilerExecutable": compiler,
            "javaLanguageVersion": 21,
            "resolutionErrors": []
        }))
        .unwrap()
    }

    fn fake_jdk(root: &Path, name: &str) -> PathBuf {
        let home = root.join(name);
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::write(home.join("bin/java"), "runtime").unwrap();
        std::fs::write(home.join("bin/javac"), "compiler").unwrap();
        home
    }

    #[test]
    fn annotation_processors_prefer_the_compile_units_jdk() {
        let root = tempfile::tempdir().unwrap();
        let compiler_home = fake_jdk(root.path(), "compiler-jdk");
        let build_home = fake_jdk(root.path(), "build-jdk");
        let model = processor_model(root.path(), Some(compiler_home.join("bin/javac")));
        let build_runtime = BuildRuntime {
            build_tool: "Gradle",
            build_tool_version: Some("8.7".to_owned()),
            java_home: build_home,
            java_version: "21.0.12".to_owned(),
            java_major: 21,
            source: "test",
        };

        assert_eq!(
            processor_java(&model, Some(&build_runtime)).unwrap(),
            std::fs::canonicalize(compiler_home.join("bin/java")).unwrap()
        );
    }

    #[test]
    fn annotation_processors_fall_back_to_the_selected_build_jdk() {
        let root = tempfile::tempdir().unwrap();
        let build_home = fake_jdk(root.path(), "build-jdk");
        let model = processor_model(root.path(), None);
        let build_runtime = BuildRuntime {
            build_tool: "Gradle",
            build_tool_version: Some("8.7".to_owned()),
            java_home: build_home.clone(),
            java_version: "21.0.12".to_owned(),
            java_major: 21,
            source: "test",
        };

        assert_eq!(
            processor_java(&model, Some(&build_runtime)).unwrap(),
            std::fs::canonicalize(build_home.join("bin/java")).unwrap()
        );
    }

    #[test]
    fn workspace_index_collects_sources_declared_only_through_source_roots() {
        let root = std::env::temp_dir().join(format!(
            "jman-java-workspace-source-roots-{}",
            std::process::id()
        ));
        let source_root = root.join("src/main/java");
        let source = source_root.join("com/example/HelloResponse.java");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "package com.example; record HelloResponse() {}").unwrap();
        let model: CompileModel = serde_json::from_value(serde_json::json!({
            "schemaVersion": 2,
            "buildSystem": "jman",
            "projectPath": "demo",
            "projectDirectory": root,
            "taskPath": "compile",
            "sourceFiles": [],
            "sourceRoots": [source_root],
            "classpath": [],
            "modulePath": [],
            "projectDependencies": [],
            "annotationProcessorPath": [],
            "annotationProcessorOptions": [],
            "compilerArgs": [],
            "release": 25,
            "encoding": "UTF-8",
            "generatedSourcesDirectory": null,
            "generatedSourceDirectories": [],
            "destinationDirectory": null,
            "javaCompilerExecutable": null,
            "javaLanguageVersion": 25,
            "resolutionErrors": []
        }))
        .unwrap();

        assert_eq!(workspace_source_files(&[model]), [source]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_queries_are_cached_by_document_content_and_cursor_and_invalidated_by_uri() {
        let mut cache = EditorQueryCache::default();
        let first = EditorQueryCache::key("file:///Demo.java", "class Demo {}", 7);
        cache.insert(first.clone(), empty_editor_result());
        assert!(cache.get(&first).is_some());

        let changed_source =
            EditorQueryCache::key("file:///Demo.java", "class Demo { int value; }", 7);
        let changed_cursor = EditorQueryCache::key("file:///Demo.java", "class Demo {}", 8);
        assert!(cache.get(&changed_source).is_none());
        assert!(cache.get(&changed_cursor).is_none());

        cache.invalidate("file:///Demo.java");
        assert!(cache.get(&first).is_none());
    }

    #[test]
    fn processor_sources_are_refreshed_from_roots_and_exclude_generated_output() {
        let root = std::env::temp_dir().join(format!(
            "jman-java-processor-sources-{}",
            std::process::id()
        ));
        let source_root = root.join("src/main/java");
        let generated = root.join("build/generated/sources/annotationProcessor/java/main");
        std::fs::create_dir_all(&source_root).unwrap();
        std::fs::create_dir_all(&generated).unwrap();
        let source = source_root.join("App.java");
        let stale = source_root.join("Removed.java");
        let generated_source = generated.join("Generated.java");
        std::fs::write(&source, "class App {}").unwrap();
        std::fs::write(&generated_source, "class Generated {}").unwrap();
        let model: CompileModel = serde_json::from_value(serde_json::json!({
            "schemaVersion": 2,
            "buildSystem": "gradle",
            "projectPath": ":app",
            "projectDirectory": root,
            "taskPath": ":app:compileJava",
            "sourceFiles": [source, stale, generated_source],
            "sourceRoots": [source_root],
            "classpath": [],
            "annotationProcessorPath": [],
            "compilerArgs": [],
            "release": 25,
            "encoding": "UTF-8",
            "generatedSourcesDirectory": generated,
            "generatedSourceDirectories": [],
            "destinationDirectory": null,
            "javaCompilerExecutable": null,
            "javaLanguageVersion": 25
        }))
        .unwrap();
        let sources = processor_source_files(&model);
        assert_eq!(sources.len(), 1);
        assert!(sources[0].ends_with("App.java"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn semantic_source_path_only_substitutes_missing_reactor_artifacts() {
        let root = std::env::temp_dir().join(format!(
            "jman-java-reactor-source-path-{}",
            std::process::id()
        ));
        let library_root = root.join("library");
        let library_source = library_root.join("src/main/java");
        let missing_jar = library_root.join("build/libs/library.jar");
        let existing_jar = root.join("existing.jar");
        std::fs::create_dir_all(&library_source).unwrap();
        std::fs::write(&existing_jar, "existing").unwrap();
        let model = |project: &str, directory: &Path, classpath: Vec<PathBuf>| {
            serde_json::from_value::<CompileModel>(serde_json::json!({
                "schemaVersion": 2,
                "buildSystem": "gradle",
                "projectPath": project,
                "projectDirectory": directory,
                "taskPath": format!("{project}:compileJava"),
                "sourceFiles": [],
                "sourceRoots": [directory.join("src/main/java")],
                "classpath": classpath,
                "annotationProcessorPath": [],
                "compilerArgs": [],
                "release": 25,
                "encoding": "UTF-8",
                "generatedSourcesDirectory": null,
                "destinationDirectory": null,
                "javaCompilerExecutable": null,
                "javaLanguageVersion": 25
            }))
            .unwrap()
        };
        let library = model(":library", &library_root, vec![]);
        let missing_owner = model(":app", &root.join("app"), vec![missing_jar.clone()]);
        let existing_owner = model(":app", &root.join("app"), vec![existing_jar]);

        assert_eq!(
            dependency_source_roots(&missing_owner, std::slice::from_ref(&library)),
            std::slice::from_ref(&library_source)
        );
        assert!(
            dependency_source_roots(&existing_owner, std::slice::from_ref(&library)).is_empty()
        );
        let mut same_project_test = model(":library", &library_root, vec![missing_jar.clone()]);
        same_project_test.task_path = ":library:compileTestJava".to_owned();
        assert_eq!(
            dependency_source_roots(&same_project_test, std::slice::from_ref(&library)),
            std::slice::from_ref(&library_source)
        );
        let processed_key = format!("{}:{}", library.project_path, library.task_path);
        let mut library_test = library.clone();
        library_test.task_path = ":library:compileTestJava".to_owned();
        assert!(processor_output_is_dependency(
            &library_test,
            &processed_key,
            std::slice::from_ref(&library)
        ));
        assert!(processor_output_is_dependency(
            &missing_owner,
            &processed_key,
            std::slice::from_ref(&library)
        ));
        assert!(!processor_output_is_dependency(
            &existing_owner,
            &processed_key,
            std::slice::from_ref(&library)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn modular_reactor_dependencies_use_compiled_outputs_not_module_source_path() {
        let root = std::env::temp_dir().join(format!(
            "jman-java-reactor-module-path-{}",
            std::process::id()
        ));
        let library = root.join("library");
        let library_sources = library.join("src/main/java");
        let library_output = library.join("target/classes");
        let app = root.join("app");
        std::fs::create_dir_all(&library_sources).unwrap();
        std::fs::create_dir_all(&library_output).unwrap();
        std::fs::create_dir_all(app.join("src/main/java")).unwrap();
        std::fs::write(
            library_sources.join("module-info.java"),
            "module com.google.gson { exports com.google.gson; }",
        )
        .unwrap();
        std::fs::write(library_output.join("module-info.class"), "compiled").unwrap();
        std::fs::write(
            app.join("src/main/java/module-info.java"),
            "module gson.extras { requires com.google.gson; }",
        )
        .unwrap();
        let dependency: CompileModel = serde_json::from_value(serde_json::json!({
            "schemaVersion": 2,
            "buildSystem": "maven",
            "projectPath": "gson",
            "projectDirectory": library,
            "taskPath": "compile",
            "sourceFiles": [library_sources.join("module-info.java")],
            "sourceRoots": [library_sources],
            "classpath": [],
            "modulePath": [],
            "projectDependencies": [],
            "annotationProcessorPath": [],
            "compilerArgs": [],
            "release": "25",
            "encoding": "UTF-8",
            "generatedSourcesDirectory": null,
            "destinationDirectory": library_output,
            "javaCompilerExecutable": null,
            "javaLanguageVersion": null
        }))
        .unwrap();
        let owner: CompileModel = serde_json::from_value(serde_json::json!({
            "schemaVersion": 2,
            "buildSystem": "maven",
            "projectPath": "gson-extras",
            "projectDirectory": app,
            "taskPath": "compile",
            "sourceFiles": [app.join("src/main/java/module-info.java")],
            "sourceRoots": [app.join("src/main/java")],
            "classpath": [],
            "modulePath": [],
            "projectDependencies": ["gson"],
            "annotationProcessorPath": [],
            "compilerArgs": [],
            "release": "25",
            "encoding": "UTF-8",
            "generatedSourcesDirectory": null,
            "destinationDirectory": app.join("target/classes"),
            "javaCompilerExecutable": null,
            "javaLanguageVersion": null
        }))
        .unwrap();

        assert_eq!(
            semantic_module_path(&owner, std::slice::from_ref(&dependency)),
            [dependency.destination_directory.clone().unwrap()]
        );
        assert!(
            !jpms_compiler_options(&owner, std::slice::from_ref(&dependency))
                .iter()
                .any(|option| option.starts_with("--module-source-path"))
        );
        std::fs::remove_dir_all(&library_output).unwrap();
        assert!(
            dependency_source_roots(&owner, std::slice::from_ref(&dependency)).is_empty(),
            "an unbuilt named module must not be injected into another javac SOURCE_PATH"
        );
        let mut test_model = owner.clone();
        test_model.task_path = "testCompile".to_owned();
        test_model.source_roots = vec![app.join("src/test/java")];
        test_model.source_files = vec![app.join("src/test/java/ExampleTest.java")];
        assert_eq!(
            module_info_file(&test_model),
            None,
            "a test compile unit must not inherit src/main/java/module-info.java"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn processor_dependency_models_include_transitive_compile_classpaths() {
        let model = |project: &str, dependencies: Vec<&str>| {
            serde_json::from_value::<CompileModel>(serde_json::json!({
                "schemaVersion": 2,
                "buildSystem": "gradle",
                "projectPath": project,
                "projectDirectory": format!("/workspace/{project}"),
                "taskPath": format!("{project}:compileJava"),
                "sourceFiles": [],
                "sourceRoots": [],
                "classpath": [format!("/deps/{project}.jar")],
                "modulePath": [],
                "projectDependencies": dependencies,
                "annotationProcessorPath": [],
                "compilerArgs": [],
                "release": 25,
                "encoding": "UTF-8",
                "generatedSourcesDirectory": null,
                "destinationDirectory": null,
                "javaCompilerExecutable": null,
                "javaLanguageVersion": 25
            }))
            .unwrap()
        };
        let app = model(":app", vec![":middle"]);
        let middle = model(":middle", vec![":base"]);
        let base = model(":base", vec![]);
        let models = vec![app.clone(), middle, base];
        let projects: std::collections::HashSet<_> = dependency_compile_models(&app, &models)
            .into_iter()
            .map(|model| model.project_path.as_str())
            .collect();
        assert_eq!(
            projects,
            std::collections::HashSet::from([":middle", ":base"])
        );
    }

    #[test]
    fn processor_inputs_allow_unbuilt_reactor_outputs_but_not_missing_external_inputs() {
        let root = tempfile::tempdir().unwrap();
        let domain = root.path().join("domain");
        let app = root.path().join("app");
        let processor = root.path().join("processor.jar");
        std::fs::create_dir_all(&domain).unwrap();
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(&processor, "processor").unwrap();
        let model =
            |project: &str, directory: &Path, classpath: Vec<PathBuf>, dependencies: Vec<&str>| {
                serde_json::from_value::<CompileModel>(serde_json::json!({
                    "schemaVersion": 2,
                    "buildSystem": "gradle",
                    "projectPath": project,
                    "projectDirectory": directory,
                    "taskPath": format!("{project}:compileJava"),
                    "sourceFiles": [],
                    "sourceRoots": [],
                    "classpath": classpath,
                    "modulePath": [],
                    "projectDependencies": dependencies,
                    "annotationProcessorPath": [processor],
                    "compilerArgs": [],
                    "release": 21,
                    "encoding": "UTF-8",
                    "generatedSourcesDirectory": null,
                    "destinationDirectory": null,
                    "javaCompilerExecutable": null,
                    "javaLanguageVersion": 21
                }))
                .unwrap()
            };
        let dependency = model(":domain", &domain, vec![], vec![]);
        let missing_reactor_output = domain.join("build/libs/domain.jar");
        let owner = model(":app", &app, vec![missing_reactor_output], vec![":domain"]);
        let models = vec![owner.clone(), dependency];
        assert!(processor_inputs_available(&owner, &models));

        let missing_external = model(
            ":app",
            &app,
            vec![root.path().join("repository/missing.jar")],
            vec![":domain"],
        );
        assert!(!processor_inputs_available(&missing_external, &models));
    }

    #[test]
    fn annotation_processors_run_dependencies_before_consumers_and_tests() {
        let model = |project: &str, task: &str, dependencies: Vec<&str>| {
            serde_json::from_value::<CompileModel>(serde_json::json!({
                "schemaVersion": 2,
                "buildSystem": "gradle",
                "projectPath": project,
                "projectDirectory": format!("/workspace/{project}"),
                "taskPath": task,
                "sourceFiles": [],
                "sourceRoots": [],
                "classpath": [],
                "modulePath": [],
                "projectDependencies": dependencies,
                "annotationProcessorPath": ["/processors/lombok.jar"],
                "compilerArgs": [],
                "release": 21,
                "encoding": "UTF-8",
                "generatedSourcesDirectory": null,
                "destinationDirectory": null,
                "javaCompilerExecutable": null,
                "javaLanguageVersion": 21
            }))
            .unwrap()
        };
        let mut models = vec![
            model(":app", ":app:compileTestJava", vec![":domain"]),
            model(":app", ":app:compileJava", vec![":domain"]),
            model(":domain", ":domain:compileJava", vec![]),
        ];

        order_processor_models(&mut models);

        assert_eq!(
            models
                .iter()
                .map(|model| model.task_path.as_str())
                .collect::<Vec<_>>(),
            [
                ":domain:compileJava",
                ":app:compileJava",
                ":app:compileTestJava"
            ]
        );
    }

    #[test]
    fn partial_processor_output_uses_a_parallel_cache_generation() {
        assert_eq!(
            partial_processor_classes_directory(Path::new(
                "/cache/processors/unit/current/build/classes/java/main"
            )),
            PathBuf::from("/cache/processors/unit/partial/build/classes/java/main")
        );
        assert_eq!(
            partial_processor_classes_directory(Path::new("/tmp/classes")),
            PathBuf::from("/tmp/classes.jman-java-partial")
        );
    }

    #[test]
    fn recognizes_maven_gradle_test_and_custom_java_compile_units() {
        let model = |task_path: &str| {
            serde_json::from_value::<CompileModel>(serde_json::json!({
                "schemaVersion": 2,
                "buildSystem": "gradle",
                "projectPath": ":app",
                "projectDirectory": "/work/app",
                "taskPath": task_path,
                "sourceFiles": [],
                "sourceRoots": [],
                "classpath": [],
                "annotationProcessorPath": [],
                "compilerArgs": [],
                "release": 25,
                "encoding": "UTF-8",
                "generatedSourcesDirectory": null,
                "destinationDirectory": null,
                "javaCompilerExecutable": null,
                "javaLanguageVersion": 25
            }))
            .unwrap()
        };
        for task in [
            "compile",
            "testCompile",
            ":app:compileJava",
            ":app:compileTestJava",
            ":app:compileIntegrationTestJava",
            ":app:compileProcessorTestJava",
        ] {
            assert!(is_java_compile_model(&model(task)), "{task}");
        }
        for task in [":app:classes", ":app:compileKotlin", "test"] {
            assert!(!is_java_compile_model(&model(task)), "{task}");
        }
    }

    #[test]
    fn extracts_named_and_open_module_declarations() {
        assert_eq!(
            java_module_name("module io.github.demo { requires java.sql; }"),
            Some("io.github.demo")
        );
        assert_eq!(
            java_module_name("open module io.github.opened\n{ exports demo; }"),
            Some("io.github.opened")
        );
        assert_eq!(java_module_name("package demo; class NotAModule {}"), None);
    }

    #[test]
    fn external_catalog_discovers_jdk_sources_and_dependency_classes_without_inner_noise() {
        let root =
            std::env::temp_dir().join(format!("jman-java-type-catalog-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let sources = root.join("src.zip");
        let classes = root.join("library.jar");
        for (archive_path, entries) in [
            (
                &sources,
                vec![
                    "java.base/java/util/HashMap.java",
                    "java.base/java/util/package-info.java",
                ],
            ),
            (
                &classes,
                vec![
                    "com/example/Widget.class",
                    "com/example/Widget$Builder.class",
                    "META-INF/versions/25/com/example/Hidden.class",
                ],
            ),
        ] {
            let file = std::fs::File::create(archive_path).unwrap();
            let mut archive = zip::ZipWriter::new(file);
            for entry in entries {
                archive
                    .start_file(entry, zip::write::SimpleFileOptions::default())
                    .unwrap();
                archive.write_all(b"x").unwrap();
            }
            archive.finish().unwrap();
        }
        let mut types = std::collections::BTreeMap::new();
        collect_source_archive_types(&sources, &mut types);
        collect_class_archive_types(&classes, 25, &mut types);
        assert_eq!(
            types.get("java.util.HashMap"),
            Some(&("java.base".to_owned(), "java/util/HashMap".to_owned()))
        );
        assert_eq!(
            types.get("com.example.Widget"),
            Some(&("<unnamed>".to_owned(), "com/example/Widget".to_owned()))
        );
        assert!(!types.contains_key("com.example.Widget$Builder"));
        assert!(!types.contains_key("java.util.package-info"));
        assert_eq!(types.len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn multi_release_jars_select_the_highest_enabled_version_for_the_project_release() {
        let root =
            std::env::temp_dir().join(format!("jman-java-multi-release-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let jar = root.join("library.jar");
        let file = std::fs::File::create(&jar).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("META-INF/MANIFEST.MF", options).unwrap();
        archive
            .write_all(b"Manifest-Version: 1.0\nMulti-Release: true\n\n")
            .unwrap();
        for (entry, bytes) in [
            ("demo/Feature.class", b"base".as_slice()),
            ("META-INF/versions/17/demo/Feature.class", b"java17"),
            ("META-INF/versions/21/demo/Feature.class", b"java21"),
            ("META-INF/versions/21/demo/TwentyOneOnly.class", b"only21"),
            ("META-INF/versions/25/module-info.class", b"module25"),
        ] {
            archive.start_file(entry, options).unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap();

        assert_eq!(
            find_class_bytes(std::slice::from_ref(&jar), "demo.Feature", 11)
                .unwrap()
                .0,
            b"base"
        );
        assert_eq!(
            find_class_bytes(std::slice::from_ref(&jar), "demo.Feature", 17)
                .unwrap()
                .0,
            b"java17"
        );
        assert_eq!(
            find_class_bytes(std::slice::from_ref(&jar), "demo.Feature", 25)
                .unwrap()
                .0,
            b"java21"
        );
        let mut java17 = std::collections::BTreeMap::new();
        collect_class_archive_types(&jar, 17, &mut java17);
        assert!(!java17.contains_key("demo.TwentyOneOnly"));
        let mut java21 = std::collections::BTreeMap::new();
        collect_class_archive_types(&jar, 21, &mut java21);
        assert!(java21.contains_key("demo.TwentyOneOnly"));
        assert!(!java21.contains_key("module-info"));
        let file = std::fs::File::open(&jar).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert_eq!(
            multi_release_entry(&mut archive, "module-info.class", 25).as_deref(),
            Some("META-INF/versions/25/module-info.class")
        );
        assert!(multi_release_entry(&mut archive, "module-info.class", 17).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn versioned_entries_are_ignored_without_the_multi_release_manifest_attribute() {
        let root = std::env::temp_dir().join(format!(
            "jman-java-not-multi-release-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let jar = root.join("library.jar");
        let file = std::fs::File::create(&jar).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("META-INF/MANIFEST.MF", options).unwrap();
        archive.write_all(b"Manifest-Version: 1.0\n\n").unwrap();
        archive.start_file("demo/Feature.class", options).unwrap();
        archive.write_all(b"base").unwrap();
        archive
            .start_file("META-INF/versions/21/demo/Feature.class", options)
            .unwrap();
        archive.write_all(b"java21").unwrap();
        archive.finish().unwrap();

        assert_eq!(
            find_class_bytes(&[jar], "demo.Feature", 25).unwrap().0,
            b"base"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_module_names_prefer_the_manifest_and_normalize_jar_filenames() {
        assert_eq!(
            automatic_module_name("example-library-2.4.1"),
            Some("example.library".to_owned())
        );
        assert_eq!(
            automatic_module_name("many---separators_1"),
            Some("many.separators.1".to_owned())
        );
        let root =
            std::env::temp_dir().join(format!("jman-java-automatic-module-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let jar = root.join("ignored-name-1.0.jar");
        let file = std::fs::File::create(&jar).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file(
                "META-INF/MANIFEST.MF",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive
            .write_all(
                b"Manifest-Version: 1.0\nAutomatic-Module-Name: io.github.zonnedev.jman.tests.library\n\n",
            )
            .unwrap();
        archive.finish().unwrap();
        assert_eq!(
            jar_module_name(&jar, 25),
            Some("io.github.zonnedev.jman.tests.library".to_owned())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_jar_module_name_comes_from_the_selected_descriptor() {
        let root =
            std::env::temp_dir().join(format!("jman-java-explicit-module-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let source = root.join("src");
        let classes = root.join("classes");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("module-info.java"),
            "module io.github.zonnedev.jman.tests.explicit { }",
        )
        .unwrap();
        let javac = std::env::var_os("JAVA_HOME")
            .map(PathBuf::from)
            .map(|home| home.join("bin/javac"))
            .unwrap_or_else(|| PathBuf::from("javac"));
        assert!(
            std::process::Command::new(javac)
                .args(["-d", classes.to_str().unwrap()])
                .arg(source.join("module-info.java"))
                .status()
                .unwrap()
                .success()
        );
        let jar = root.join("wrong-filename-1.0.jar");
        let file = std::fs::File::create(&jar).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file(
                "module-info.class",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive
            .write_all(&std::fs::read(classes.join("module-info.class")).unwrap())
            .unwrap();
        archive.finish().unwrap();
        assert_eq!(
            jar_module_name(&jar, 25),
            Some("io.github.zonnedev.jman.tests.explicit".to_owned())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn semantic_cache_is_versioned_atomic_and_drops_stale_entries() {
        let root =
            std::env::temp_dir().join(format!("jman-java-semantic-cache-{}", std::process::id()));
        let path = root.join("semantic.json");
        let mut documents = HashMap::new();
        documents.insert(
            "/workspace/Demo.java".to_owned(),
            CachedSemanticDocument {
                fingerprint: 42,
                result: SemanticResult {
                    package_name: "demo".to_owned(),
                    symbols: Vec::new(),
                    diagnostics: Vec::new(),
                },
            },
        );
        write_semantic_cache(
            &path,
            &SemanticCache {
                version: SEMANTIC_CACHE_VERSION,
                documents,
            },
        )
        .unwrap();
        let loaded = read_semantic_cache(&path);
        assert_eq!(loaded.documents.len(), 1);
        assert_eq!(loaded.documents["/workspace/Demo.java"].fingerprint, 42);

        let mut stale = loaded;
        stale.version = SEMANTIC_CACHE_VERSION + 1;
        write_semantic_cache(&path, &stale).unwrap();
        assert!(read_semantic_cache(&path).documents.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn structural_cache_persists_the_external_symbol_catalog() {
        let root =
            std::env::temp_dir().join(format!("jman-java-external-cache-{}", std::process::id()));
        let path = root.join("structural.json");
        let catalog = SemanticResult {
            package_name: String::new(),
            symbols: vec![javac_frontend::SemanticSymbol {
                role: "reference".to_owned(),
                kind: "class".to_owned(),
                name: "ArrayList".to_owned(),
                qualified_name: "java.util.ArrayList".to_owned(),
                symbol_id: "java.base|java/util/ArrayList".to_owned(),
                start: 0,
                end: 0,
            }],
            diagnostics: Vec::new(),
        };
        write_structural_cache(
            &path,
            &StructuralCache {
                version: STRUCTURAL_CACHE_VERSION,
                documents: HashMap::new(),
                external_fingerprint: 42,
                external_catalog: Some(catalog.clone()),
            },
        )
        .unwrap();
        let loaded = read_structural_cache(&path);
        assert_eq!(loaded.external_fingerprint, 42);
        assert_eq!(loaded.external_catalog, Some(catalog));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vineflower_fallback_is_content_addressed_and_locates_overload() {
        let root =
            std::env::temp_dir().join(format!("jman-java-vineflower-test-{}", std::process::id()));
        let source_directory = root.join("src/demo");
        let classes = root.join("classes");
        std::fs::create_dir_all(&source_directory).unwrap();
        std::fs::create_dir_all(&classes).unwrap();
        let source = source_directory.join("BinaryOnly.java");
        std::fs::write(
            &source,
            "package demo; public class BinaryOnly { public String value(Object object) { return object.toString(); } public String value(int index) { return \"x\" + index; } }",
        )
        .unwrap();
        let java_home = std::env::var_os("JAVA_HOME").map(PathBuf::from);
        let javac = java_home
            .as_ref()
            .map(|home| home.join("bin/javac"))
            .unwrap_or_else(|| PathBuf::from("javac"));
        assert!(
            std::process::Command::new(javac)
                .args(["-d", classes.to_str().unwrap()])
                .arg(&source)
                .status()
                .unwrap()
                .success()
        );
        let jar = root.join("binary-only.jar");
        let jar_tool = java_home
            .as_ref()
            .map(|home| home.join("bin/jar"))
            .unwrap_or_else(|| PathBuf::from("jar"));
        assert!(
            std::process::Command::new(jar_tool)
                .args([
                    "--create",
                    "--file",
                    jar.to_str().unwrap(),
                    "-C",
                    classes.to_str().unwrap(),
                    ".",
                ])
                .status()
                .unwrap()
                .success()
        );
        let (directory_bytes, directory_entry) =
            find_class_bytes(std::slice::from_ref(&classes), "demo.BinaryOnly", 25).unwrap();
        assert!(!directory_bytes.is_empty());
        assert_eq!(directory_entry, "demo/BinaryOnly.class");
        let mut definition = javac_frontend::EditorDefinition {
            symbol_id: "<unnamed>|demo/BinaryOnly#value(I)Ljava/lang/String;".to_owned(),
            module: "<unnamed>".to_owned(),
            owner: "demo.BinaryOnly".to_owned(),
            name: "value".to_owned(),
            descriptor: "(I)Ljava/lang/String;".to_owned(),
            source_name: String::new(),
            source: String::new(),
            start: 0,
            end: 0,
            decompiled: true,
        };
        decompile_definition(&[jar], 25, &mut definition).unwrap();
        assert!(definition.source.contains("Decompiled from"));
        assert_eq!(
            &definition.source[definition.start as usize..definition.end as usize],
            "value"
        );
        assert!(definition.source[definition.end as usize..].starts_with("(int"));
        let _ = std::fs::remove_dir_all(root);
    }
}
