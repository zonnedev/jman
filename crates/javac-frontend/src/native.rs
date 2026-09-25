use std::ffi::{c_int, c_void};
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;

use crate::ABI_VERSION;
use crate::{
    EditorCompletion, EditorDefinition, EditorHover, EditorQueryResult, EditorSignature,
    FormatResult, SemanticDiagnostic, SemanticResult, SemanticSymbol, StructuralFile,
    WorkspaceParseResult, WorkspaceSource,
};

#[repr(C)]
struct GraalCreateIsolateParams {
    _private: [u8; 0],
}

#[repr(C)]
struct GraalIsolate {
    _private: [u8; 0],
}

#[repr(C)]
struct GraalIsolateThread {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn graal_create_isolate(
        params: *const GraalCreateIsolateParams,
        isolate: *mut *mut GraalIsolate,
        thread: *mut *mut GraalIsolateThread,
    ) -> c_int;
    fn graal_tear_down_isolate(thread: *mut GraalIsolateThread) -> c_int;
    fn javac_frontend_abi_version(thread: *mut GraalIsolateThread) -> u32;
    fn javac_frontend_configure_platform(
        thread: *mut GraalIsolateThread,
        home: *const u8,
        home_len: usize,
    ) -> c_int;
    fn javac_frontend_parse(
        thread: *mut GraalIsolateThread,
        source: *const u8,
        source_len: usize,
    ) -> *mut u8;
    fn javac_frontend_analyze(
        thread: *mut GraalIsolateThread,
        source: *const u8,
        source_len: usize,
        file_name: *const u8,
        file_name_len: usize,
        classpath: *const u8,
        classpath_len: usize,
        source_path: *const u8,
        source_path_len: usize,
        release: c_int,
    ) -> *mut u8;
    fn javac_frontend_workspace_parse(
        thread: *mut GraalIsolateThread,
        input: *const u8,
        input_len: usize,
        release: c_int,
        enable_preview: c_int,
    ) -> *mut u8;
    fn javac_frontend_free(thread: *mut GraalIsolateThread, result: *mut c_void);
    fn javac_frontend_session_create(
        thread: *mut GraalIsolateThread,
        classpath: *const u8,
        classpath_len: usize,
        source_path: *const u8,
        source_path_len: usize,
        module_path: *const u8,
        module_path_len: usize,
        module_info: *const u8,
        module_info_len: usize,
        compiler_options: *const u8,
        compiler_options_len: usize,
        release: c_int,
    ) -> u64;
    fn javac_frontend_session_analyze(
        thread: *mut GraalIsolateThread,
        session_id: u64,
        source: *const u8,
        source_len: usize,
        file_name: *const u8,
        file_name_len: usize,
    ) -> *mut u8;
    fn javac_frontend_session_editor_query(
        thread: *mut GraalIsolateThread,
        session_id: u64,
        source: *const u8,
        source_len: usize,
        file_name: *const u8,
        file_name_len: usize,
        cursor: c_int,
    ) -> *mut u8;
    fn javac_frontend_session_format(
        thread: *mut GraalIsolateThread,
        session_id: u64,
        source: *const u8,
        source_len: usize,
        file_name: *const u8,
        file_name_len: usize,
    ) -> *mut u8;
    fn javac_frontend_session_destroy(thread: *mut GraalIsolateThread, session_id: u64) -> c_int;
    fn javac_frontend_session_invalidate(
        thread: *mut GraalIsolateThread,
        session_id: u64,
        file_name: *const u8,
        file_name_len: usize,
    ) -> c_int;
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrontendError {
    IsolateCreation(c_int),
    AbiMismatch { expected: u32, actual: u32 },
    PlatformUnavailable(PathBuf),
    PlatformConfiguration(c_int),
    NullResult,
    InvalidResult(&'static str),
    InvalidClasspath,
    SessionCreation,
    SessionDestroy(c_int),
    SessionInvalidate(c_int),
    TearDown(c_int),
}

impl std::fmt::Display for FrontendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IsolateCreation(status) => {
                write!(
                    formatter,
                    "native isolate creation failed with status {status}"
                )
            }
            Self::AbiMismatch { expected, actual } => write!(
                formatter,
                "native frontend ABI mismatch: expected {expected}, found {actual}"
            ),
            Self::PlatformUnavailable(home) => write!(
                formatter,
                "native javac platform is missing at {}/lib/ct.sym; keep the platform directory beside the jman executable",
                home.display()
            ),
            Self::PlatformConfiguration(status) => write!(
                formatter,
                "native javac platform configuration failed with status {status}"
            ),
            Self::NullResult => formatter.write_str("native frontend returned no result"),
            Self::InvalidResult(message) => write!(formatter, "invalid native result: {message}"),
            Self::InvalidClasspath => formatter.write_str("invalid native frontend path list"),
            Self::SessionCreation => formatter.write_str("native session creation failed"),
            Self::SessionDestroy(status) => {
                write!(
                    formatter,
                    "native session destruction failed with status {status}"
                )
            }
            Self::SessionInvalidate(status) => {
                write!(
                    formatter,
                    "native session invalidation failed with status {status}"
                )
            }
            Self::TearDown(status) => {
                write!(
                    formatter,
                    "native isolate teardown failed with status {status}"
                )
            }
        }
    }
}

impl std::error::Error for FrontendError {}

pub struct Frontend {
    inner: Rc<FrontendInner>,
}

struct FrontendInner {
    thread: NonNull<GraalIsolateThread>,
}

pub struct ProjectSession {
    inner: Rc<FrontendInner>,
    id: NonZeroU64,
}

impl Frontend {
    pub fn new() -> Result<Self, FrontendError> {
        let mut isolate = std::ptr::null_mut();
        let mut thread = std::ptr::null_mut();
        // SAFETY: output pointers are valid and Native Image accepts null defaults.
        let status = unsafe { graal_create_isolate(std::ptr::null(), &mut isolate, &mut thread) };
        if status != 0 {
            return Err(FrontendError::IsolateCreation(status));
        }
        let thread = NonNull::new(thread).ok_or(FrontendError::IsolateCreation(status))?;
        // SAFETY: the thread belongs to the newly created live isolate.
        let actual = unsafe { javac_frontend_abi_version(thread.as_ptr()) };
        if actual != ABI_VERSION {
            // SAFETY: the isolate is live and owned by this function.
            unsafe { graal_tear_down_isolate(thread.as_ptr()) };
            return Err(FrontendError::AbiMismatch {
                expected: ABI_VERSION,
                actual,
            });
        }
        let platform_home = match native_platform_home() {
            Ok(home) => home,
            Err(error) => {
                // SAFETY: the isolate is live and still owned by this function.
                unsafe { graal_tear_down_isolate(thread.as_ptr()) };
                return Err(error);
            }
        };
        let encoded_home = match platform_home.to_str() {
            Some(home) => home,
            None => {
                // SAFETY: the isolate is live and still owned by this function.
                unsafe { graal_tear_down_isolate(thread.as_ptr()) };
                return Err(FrontendError::PlatformUnavailable(platform_home));
            }
        };
        // SAFETY: the platform path remains live for the duration of the native call.
        let status = unsafe {
            javac_frontend_configure_platform(
                thread.as_ptr(),
                encoded_home.as_ptr(),
                encoded_home.len(),
            )
        };
        if status != 0 {
            // SAFETY: the isolate is live and still owned by this function.
            unsafe { graal_tear_down_isolate(thread.as_ptr()) };
            return Err(FrontendError::PlatformConfiguration(status));
        }
        Ok(Self {
            inner: Rc::new(FrontendInner { thread }),
        })
    }

    pub fn parse_raw(&self, source: &str) -> Result<Vec<u8>, FrontendError> {
        // SAFETY: the source slice remains alive for the duration of the call.
        let result = unsafe {
            javac_frontend_parse(self.inner.thread.as_ptr(), source.as_ptr(), source.len())
        };
        let result = NonNull::new(result).ok_or(FrontendError::NullResult)?;

        // The allocation starts with a little-endian u32 payload length.
        let mut length_bytes = [0_u8; 4];
        // SAFETY: every successful bridge allocation contains the four-byte header.
        unsafe {
            std::ptr::copy_nonoverlapping(result.as_ptr(), length_bytes.as_mut_ptr(), 4);
        }
        let length = u32::from_le_bytes(length_bytes) as usize;
        if length > 64 * 1024 * 1024 {
            // SAFETY: the result was allocated by this isolate.
            unsafe { javac_frontend_free(self.inner.thread.as_ptr(), result.as_ptr().cast()) };
            return Err(FrontendError::InvalidResult("payload exceeds safety limit"));
        }

        let mut payload = vec![0_u8; length];
        // SAFETY: the length header describes the immediately following allocation.
        unsafe {
            std::ptr::copy_nonoverlapping(result.as_ptr().add(4), payload.as_mut_ptr(), length);
            javac_frontend_free(self.inner.thread.as_ptr(), result.as_ptr().cast());
        }
        Ok(payload)
    }

    pub fn analyze(
        &self,
        file_name: &str,
        source: &str,
        classpath: &[PathBuf],
        source_path: &[PathBuf],
        release: u8,
    ) -> Result<SemanticResult, FrontendError> {
        let classpath = encode_paths(classpath)?;
        let source_path = encode_paths(source_path)?;
        // SAFETY: every input buffer remains live for the complete native call.
        let result = unsafe {
            javac_frontend_analyze(
                self.inner.thread.as_ptr(),
                source.as_ptr(),
                source.len(),
                file_name.as_ptr(),
                file_name.len(),
                classpath.as_ptr(),
                classpath.len(),
                source_path.as_ptr(),
                source_path.len(),
                release.into(),
            )
        };
        let payload = copy_and_free(self.inner.thread, result)?;
        decode_semantic_result(&payload)
    }

    pub fn parse_workspace(
        &self,
        sources: &[WorkspaceSource],
        release: u8,
        enable_preview: bool,
    ) -> Result<WorkspaceParseResult, FrontendError> {
        let input = encode_workspace_sources(sources)?;
        // SAFETY: the encoded batch remains live for the complete native call.
        let result = unsafe {
            javac_frontend_workspace_parse(
                self.inner.thread.as_ptr(),
                input.as_ptr(),
                input.len(),
                release.into(),
                c_int::from(enable_preview),
            )
        };
        let payload = copy_and_free(self.inner.thread, result)?;
        decode_workspace_result(&payload)
    }

    pub fn parse_workspace_batched(
        &self,
        sources: &[WorkspaceSource],
        release: u8,
        enable_preview: bool,
        max_files: usize,
        max_source_bytes: usize,
    ) -> Result<WorkspaceParseResult, FrontendError> {
        if max_files == 0 || max_source_bytes == 0 {
            return Err(FrontendError::InvalidResult(
                "batch limits must be positive",
            ));
        }
        let mut files = Vec::with_capacity(sources.len());
        let mut start = 0;
        while start < sources.len() {
            let mut end = start;
            let mut bytes = 0_usize;
            while end < sources.len() && end - start < max_files {
                let next = sources[end].source.len();
                if end > start && bytes.saturating_add(next) > max_source_bytes {
                    break;
                }
                bytes = bytes.saturating_add(next);
                end += 1;
            }
            let mut parsed = self.parse_workspace(&sources[start..end], release, enable_preview)?;
            files.append(&mut parsed.files);
            start = end;
        }
        Ok(WorkspaceParseResult { files })
    }

    pub fn create_session(
        &self,
        classpath: &[PathBuf],
        source_path: &[PathBuf],
        release: u8,
    ) -> Result<ProjectSession, FrontendError> {
        self.create_module_session(classpath, &[], source_path, None, &[], release)
    }

    pub fn create_module_session(
        &self,
        classpath: &[PathBuf],
        module_path: &[PathBuf],
        source_path: &[PathBuf],
        module_info: Option<&std::path::Path>,
        compiler_options: &[String],
        release: u8,
    ) -> Result<ProjectSession, FrontendError> {
        let classpath = encode_paths(classpath)?;
        let module_path = encode_paths(module_path)?;
        let source_path = encode_paths(source_path)?;
        let module_info = module_info
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let compiler_options = encode_strings(compiler_options)?;
        // SAFETY: both encoded path lists remain live during session creation.
        let id = unsafe {
            javac_frontend_session_create(
                self.inner.thread.as_ptr(),
                classpath.as_ptr(),
                classpath.len(),
                source_path.as_ptr(),
                source_path.len(),
                module_path.as_ptr(),
                module_path.len(),
                module_info.as_ptr(),
                module_info.len(),
                compiler_options.as_ptr(),
                compiler_options.len(),
                release.into(),
            )
        };
        Ok(ProjectSession {
            inner: Rc::clone(&self.inner),
            id: NonZeroU64::new(id).ok_or(FrontendError::SessionCreation)?,
        })
    }
}

fn native_platform_home() -> Result<PathBuf, FrontendError> {
    if let Some(configured) = std::env::var_os("JMAN_JAVAC_FRONTEND_PLATFORM_HOME") {
        let home = PathBuf::from(configured);
        return home
            .join("lib/ct.sym")
            .is_file()
            .then_some(home.clone())
            .ok_or(FrontendError::PlatformUnavailable(home));
    }

    let mut candidates = Vec::new();
    if let Some(directory) = std::env::var_os("JMAN_JAVAC_FRONTEND_LIB_DIR") {
        candidates.push(PathBuf::from(directory).join("platform"));
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        candidates.push(directory.join("platform"));
    }
    if let Some(directory) = option_env!("JMAN_COMPILED_FRONTEND_PLATFORM_HOME") {
        candidates.push(PathBuf::from(directory));
    }
    candidates
        .into_iter()
        .find(|home| home.join("lib/ct.sym").is_file())
        .ok_or_else(|| FrontendError::PlatformUnavailable(PathBuf::from("platform")))
}

impl ProjectSession {
    pub fn analyze(&self, file_name: &str, source: &str) -> Result<SemanticResult, FrontendError> {
        // SAFETY: the session belongs to this live frontend and inputs outlive the call.
        let result = unsafe {
            javac_frontend_session_analyze(
                self.inner.thread.as_ptr(),
                self.id.get(),
                source.as_ptr(),
                source.len(),
                file_name.as_ptr(),
                file_name.len(),
            )
        };
        let payload = copy_and_free(self.inner.thread, result)?;
        decode_semantic_result(&payload)
    }

    pub fn editor_query(
        &self,
        file_name: &str,
        source: &str,
        cursor: u32,
    ) -> Result<EditorQueryResult, FrontendError> {
        // SAFETY: the session belongs to this live frontend and inputs outlive the call.
        let result = unsafe {
            javac_frontend_session_editor_query(
                self.inner.thread.as_ptr(),
                self.id.get(),
                source.as_ptr(),
                source.len(),
                file_name.as_ptr(),
                file_name.len(),
                cursor as c_int,
            )
        };
        let payload = copy_and_free(self.inner.thread, result)?;
        decode_editor_query_result(&payload)
    }

    pub fn format(&self, file_name: &str, source: &str) -> Result<FormatResult, FrontendError> {
        // SAFETY: the session belongs to this live frontend and inputs outlive the call.
        let result = unsafe {
            javac_frontend_session_format(
                self.inner.thread.as_ptr(),
                self.id.get(),
                source.as_ptr(),
                source.len(),
                file_name.as_ptr(),
                file_name.len(),
            )
        };
        let payload = copy_and_free(self.inner.thread, result)?;
        decode_format_result(&payload)
    }

    pub fn invalidate(&self, file_name: &str) -> Result<(), FrontendError> {
        // SAFETY: the session belongs to this frontend and the file name outlives the call.
        let status = unsafe {
            javac_frontend_session_invalidate(
                self.inner.thread.as_ptr(),
                self.id.get(),
                file_name.as_ptr(),
                file_name.len(),
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(FrontendError::SessionInvalidate(status))
        }
    }
}

fn copy_and_free(
    thread: NonNull<GraalIsolateThread>,
    result: *mut u8,
) -> Result<Vec<u8>, FrontendError> {
    let result = NonNull::new(result).ok_or(FrontendError::NullResult)?;
    let mut length_bytes = [0_u8; 4];
    // SAFETY: every successful bridge allocation contains the four-byte header.
    unsafe {
        std::ptr::copy_nonoverlapping(result.as_ptr(), length_bytes.as_mut_ptr(), 4);
    }
    let length = u32::from_le_bytes(length_bytes) as usize;
    if length > 64 * 1024 * 1024 {
        // SAFETY: the result was allocated by this isolate.
        unsafe { javac_frontend_free(thread.as_ptr(), result.as_ptr().cast()) };
        return Err(FrontendError::InvalidResult("payload exceeds safety limit"));
    }

    let mut payload = vec![0_u8; length];
    // SAFETY: the length header describes the immediately following allocation.
    unsafe {
        std::ptr::copy_nonoverlapping(result.as_ptr().add(4), payload.as_mut_ptr(), length);
        javac_frontend_free(thread.as_ptr(), result.as_ptr().cast());
    }
    Ok(payload)
}

impl Drop for ProjectSession {
    fn drop(&mut self) {
        // SAFETY: this handle is destroyed before its borrowed frontend.
        let status =
            unsafe { javac_frontend_session_destroy(self.inner.thread.as_ptr(), self.id.get()) };
        debug_assert_eq!(status, 0, "{:?}", FrontendError::SessionDestroy(status));
    }
}

fn encode_paths(paths: &[PathBuf]) -> Result<String, FrontendError> {
    std::env::join_paths(paths)
        .map_err(|_| FrontendError::InvalidClasspath)?
        .into_string()
        .map_err(|_| FrontendError::InvalidClasspath)
}

fn encode_strings(values: &[String]) -> Result<String, FrontendError> {
    if values.iter().any(|value| value.contains('\0')) {
        return Err(FrontendError::InvalidResult(
            "compiler option contains a null byte",
        ));
    }
    Ok(values.join("\0"))
}

fn encode_workspace_sources(sources: &[WorkspaceSource]) -> Result<Vec<u8>, FrontendError> {
    let count = u32::try_from(sources.len())
        .map_err(|_| FrontendError::InvalidResult("too many workspace sources"))?;
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&count.to_be_bytes());
    for source in sources {
        encode_wire_string(&mut encoded, &source.file_name)?;
        encode_wire_string(&mut encoded, &source.source)?;
    }
    Ok(encoded)
}

fn encode_wire_string(output: &mut Vec<u8>, value: &str) -> Result<(), FrontendError> {
    let length = u32::try_from(value.len())
        .map_err(|_| FrontendError::InvalidResult("workspace source is too large"))?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

pub fn decode_semantic_result(payload: &[u8]) -> Result<SemanticResult, FrontendError> {
    let mut input = WireReader::new(payload);
    if input.take(4)? != b"JFS1" {
        return Err(FrontendError::InvalidResult(
            "semantic wire version mismatch",
        ));
    }
    let package_name = input.string()?;
    let symbol_count = input.count()?;
    let mut symbols = Vec::with_capacity(symbol_count);
    for _ in 0..symbol_count {
        symbols.push(SemanticSymbol {
            role: input.string()?,
            kind: input.string()?,
            name: input.string()?,
            qualified_name: input.string()?,
            symbol_id: input.string()?,
            start: input.u64()?,
            end: input.u64()?,
        });
    }
    let diagnostic_count = input.count()?;
    let mut diagnostics = Vec::with_capacity(diagnostic_count);
    for _ in 0..diagnostic_count {
        diagnostics.push(SemanticDiagnostic {
            kind: input.string()?,
            code: input.string()?,
            start: input.u64()?,
            end: input.u64()?,
            line: input.u64()?,
            column: input.u64()?,
            message: input.string()?,
        });
    }
    if !input.remaining().is_empty() {
        return Err(FrontendError::InvalidResult("trailing semantic wire data"));
    }
    Ok(SemanticResult {
        package_name,
        symbols,
        diagnostics,
    })
}

pub fn decode_editor_query_result(payload: &[u8]) -> Result<EditorQueryResult, FrontendError> {
    let mut input = WireReader::new(payload);
    let version = input.take(4)?;
    if version != b"JFQ1" && version != b"JFQ2" {
        return Err(FrontendError::InvalidResult(
            "editor-query wire version mismatch",
        ));
    }
    let completion_count = input.count()?;
    let mut completions = Vec::with_capacity(completion_count);
    for _ in 0..completion_count {
        completions.push(EditorCompletion {
            label: input.string()?,
            kind: input.string()?,
            detail: input.string()?,
            insert_text: input.string()?,
            documentation: input.string()?,
        });
    }
    let signature_count = input.count()?;
    let mut signatures = Vec::with_capacity(signature_count);
    for _ in 0..signature_count {
        let label = input.string()?;
        let parameter_count = input.count()?;
        let mut parameters = Vec::with_capacity(parameter_count);
        for _ in 0..parameter_count {
            parameters.push(input.string()?);
        }
        signatures.push(EditorSignature {
            label,
            parameters,
            return_type: input.string()?,
            documentation: input.string()?,
        });
    }
    let hover = if input.take(1)? == [1] {
        Some(EditorHover {
            detail: input.string()?,
            documentation: input.string()?,
        })
    } else {
        None
    };
    let definition = if input.take(1)? == [1] {
        Some(EditorDefinition {
            symbol_id: input.string()?,
            module: input.string()?,
            owner: input.string()?,
            name: input.string()?,
            descriptor: input.string()?,
            source_name: input.string()?,
            source: input.string()?,
            start: input.u64()?,
            end: input.u64()?,
            decompiled: input.take(1)? == [1],
        })
    } else {
        None
    };
    let type_definition = if version == b"JFQ2" && input.take(1)? == [1] {
        Some(EditorDefinition {
            symbol_id: input.string()?,
            module: input.string()?,
            owner: input.string()?,
            name: input.string()?,
            descriptor: input.string()?,
            source_name: input.string()?,
            source: input.string()?,
            start: input.u64()?,
            end: input.u64()?,
            decompiled: input.take(1)? == [1],
        })
    } else {
        None
    };
    if !input.remaining().is_empty() {
        return Err(FrontendError::InvalidResult(
            "trailing editor-query wire data",
        ));
    }
    Ok(EditorQueryResult {
        completions,
        signatures,
        hover,
        definition,
        type_definition,
    })
}

pub fn decode_format_result(payload: &[u8]) -> Result<FormatResult, FrontendError> {
    let mut input = WireReader::new(payload);
    if input.take(4)? != b"JFF1" {
        return Err(FrontendError::InvalidResult(
            "formatter wire version mismatch",
        ));
    }
    let source = input.string()?;
    let diagnostic_count = input.count()?;
    let mut diagnostics = Vec::with_capacity(diagnostic_count);
    for _ in 0..diagnostic_count {
        diagnostics.push(SemanticDiagnostic {
            kind: input.string()?,
            code: input.string()?,
            start: input.u64()?,
            end: input.u64()?,
            line: input.u64()?,
            column: input.u64()?,
            message: input.string()?,
        });
    }
    if !input.remaining().is_empty() {
        return Err(FrontendError::InvalidResult("trailing formatter wire data"));
    }
    Ok(FormatResult {
        source,
        diagnostics,
    })
}

fn decode_workspace_result(payload: &[u8]) -> Result<WorkspaceParseResult, FrontendError> {
    let mut input = WireReader::new(payload);
    if input.take(4)? != b"JFB1" {
        return Err(FrontendError::InvalidResult(
            "workspace wire version mismatch",
        ));
    }
    let file_count = input.count()?;
    let mut files = Vec::with_capacity(file_count);
    for _ in 0..file_count {
        let file_name = input.string()?;
        let package_name = input.string()?;
        let import_count = input.count()?;
        let mut imports = Vec::with_capacity(import_count);
        for _ in 0..import_count {
            imports.push(input.string()?);
        }
        let symbol_count = input.count()?;
        let mut symbols = Vec::with_capacity(symbol_count);
        for _ in 0..symbol_count {
            symbols.push(SemanticSymbol {
                role: input.string()?,
                kind: input.string()?,
                name: input.string()?,
                qualified_name: input.string()?,
                symbol_id: input.string()?,
                start: input.u64()?,
                end: input.u64()?,
            });
        }
        let diagnostic_count = input.count()?;
        let mut diagnostics = Vec::with_capacity(diagnostic_count);
        for _ in 0..diagnostic_count {
            diagnostics.push(SemanticDiagnostic {
                kind: input.string()?,
                code: input.string()?,
                start: input.u64()?,
                end: input.u64()?,
                line: input.u64()?,
                column: input.u64()?,
                message: input.string()?,
            });
        }
        files.push(StructuralFile {
            file_name,
            package_name,
            imports,
            symbols,
            diagnostics,
        });
    }
    if !input.remaining().is_empty() {
        return Err(FrontendError::InvalidResult("trailing workspace wire data"));
    }
    Ok(WorkspaceParseResult { files })
}

struct WireReader<'a> {
    remaining: &'a [u8],
}

impl<'a> WireReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }

    fn remaining(&self) -> &'a [u8] {
        self.remaining
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], FrontendError> {
        if length > self.remaining.len() {
            return Err(FrontendError::InvalidResult("truncated semantic wire data"));
        }
        let (value, remaining) = self.remaining.split_at(length);
        self.remaining = remaining;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, FrontendError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| FrontendError::InvalidResult("invalid u32"))?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, FrontendError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| FrontendError::InvalidResult("invalid u64"))?;
        Ok(u64::from_be_bytes(bytes))
    }

    fn count(&mut self) -> Result<usize, FrontendError> {
        let count = self.u32()? as usize;
        if count > 1_000_000 {
            return Err(FrontendError::InvalidResult("wire collection is too large"));
        }
        Ok(count)
    }

    fn string(&mut self) -> Result<String, FrontendError> {
        let length = self.count()?;
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| FrontendError::InvalidResult("wire string is not UTF-8"))
    }
}

impl Drop for FrontendInner {
    fn drop(&mut self) {
        // Native Image owns all isolate allocations; teardown releases them together.
        let status = unsafe { graal_tear_down_isolate(self.thread.as_ptr()) };
        debug_assert_eq!(status, 0, "{:?}", FrontendError::TearDown(status));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn creates_versioned_isolate_and_parses_source() {
        let frontend = Frontend::new().expect("create frontend");
        let payload = frontend
            .parse_raw("package demo; final class Example {}")
            .expect("parse source");

        assert!(payload.starts_with(b"JFE1"));
        assert!(
            payload
                .windows("Example".len())
                .any(|window| window == b"Example")
        );
    }

    #[test]
    fn malformed_source_returns_diagnostics_instead_of_crashing() {
        let frontend = Frontend::new().expect("create frontend");
        let payload = frontend
            .parse_raw("class Broken { void test( }")
            .expect("parse malformed source");

        assert!(payload.starts_with(b"JFE1"));
        assert!(
            payload
                .windows("illegal.start.of.type".len())
                .any(|window| window == b"illegal.start.of.type")
                || payload.windows(5).any(|window| window == b"error")
        );
    }

    #[test]
    fn parses_workspace_batch_into_typed_structural_facts() {
        let frontend = Frontend::new().expect("create frontend");
        let result = frontend
            .parse_workspace(
                &[
                    WorkspaceSource {
                        file_name: "demo/Model.java".to_owned(),
                        source: "package demo; record Model(String value) {}".to_owned(),
                    },
                    WorkspaceSource {
                        file_name: "demo/Use.java".to_owned(),
                        source:
                            "package demo; import java.util.List; class Use { List<Model> values; }"
                                .to_owned(),
                    },
                ],
                25,
                false,
            )
            .expect("parse workspace");
        assert_eq!(result.files.len(), 2);
        assert!(
            result.files[0]
                .symbols
                .iter()
                .any(|symbol| symbol.qualified_name == "demo.Model")
        );
        assert!(
            result.files[1]
                .imports
                .iter()
                .any(|imported| imported.contains("java.util.List"))
        );
    }

    #[test]
    fn bounded_batches_match_individual_parse_facts() {
        let frontend = Frontend::new().expect("create frontend");
        let sources: Vec<_> = (0..7)
            .map(|index| WorkspaceSource {
                file_name: format!("demo/Type{index}.java"),
                source: format!(
                    "package demo; import java.util.List; class Type{index} {{ List<String> values; }}"
                ),
            })
            .collect();
        let batched = frontend
            .parse_workspace_batched(&sources, 25, false, 3, 160)
            .expect("parse bounded batches");
        let individual_files: Vec<_> = sources
            .iter()
            .map(|source| {
                frontend
                    .parse_workspace(std::slice::from_ref(source), 25, false)
                    .expect("parse individual source")
                    .files
                    .into_iter()
                    .next()
                    .unwrap()
            })
            .collect();
        assert_eq!(batched.files, individual_files);
    }

    #[test]
    fn attributes_symbols_into_typed_rust_results() {
        let frontend = Frontend::new().expect("create frontend");
        let result = frontend
            .analyze(
                "Example.java",
                "package demo; import java.util.List; class Example { List<String> names; }",
                &[],
                &[],
                25,
            )
            .expect("analyze source");

        assert_eq!(result.package_name, "demo");
        assert!(
            result
                .symbols
                .iter()
                .any(|symbol| symbol.qualified_name == "java.util.List")
        );
        assert!(
            result
                .symbols
                .iter()
                .any(|symbol| symbol.role == "declaration"
                    && symbol.qualified_name == "demo.Example")
        );
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.kind != "error")
        );
    }

    #[test]
    fn project_session_tracks_unsaved_buffer_changes() {
        let frontend = Frontend::new().expect("create frontend");
        let session = frontend
            .create_session(&[], &[], 25)
            .expect("create project session");
        let first = session
            .analyze("Overlay.java", "class Overlay { String before; }")
            .expect("analyze first overlay");
        let second = session
            .analyze("Overlay.java", "class Overlay { int after; }")
            .expect("analyze updated overlay");

        assert!(first.symbols.iter().any(|symbol| symbol.name == "before"));
        assert!(second.symbols.iter().any(|symbol| symbol.name == "after"));
        assert!(!second.symbols.iter().any(|symbol| symbol.name == "before"));
        session
            .invalidate("Overlay.java")
            .expect("invalidate overlay");
        let third = session
            .analyze("Overlay.java", "class Overlay { int after; }")
            .expect("reanalyze invalidated overlay");
        assert_eq!(second, third);
    }

    #[test]
    fn project_session_formats_source_and_preserves_comments_idempotently() {
        let frontend = Frontend::new().expect("create frontend");
        let session = frontend
            .create_session(&[], &[], 25)
            .expect("create project session");
        let source = "class Messy{ // keep\nvoid zebra(){} int value; void alpha(){if  (value== 1) {call( 1,2 );}}}";

        let first = session.format("Messy.java", source).expect("format source");
        assert!(first.diagnostics.is_empty());
        assert!(first.source.contains("// keep"));
        assert!(first.source.contains("if (value == 1)"));
        assert!(first.source.contains("call(1, 2);"));
        assert!(
            first.source.find("void zebra()").unwrap() < first.source.find("void alpha()").unwrap(),
            "{}",
            first.source
        );

        let second = session
            .format("Messy.java", &first.source)
            .expect("format canonical source");
        assert_eq!(second, first);
    }

    #[test]
    fn project_session_formats_older_releases_inside_the_native_image() {
        let frontend = Frontend::new().expect("create frontend");
        let session = frontend
            .create_session(&[], &[], 17)
            .expect("create Java 17 session");
        let result = session
            .format(
                "Legacy.java",
                "import java.util.*; class Legacy{List<String> values=new ArrayList<>();}\n",
            )
            .expect("format Java 17 source");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.source.contains("import java.util.ArrayList;"));
        assert!(result.source.contains("import java.util.List;"));
    }

    #[test]
    fn project_session_resolves_typed_member_completion_and_signatures() {
        let frontend = Frontend::new().expect("create frontend");
        let source_path: Vec<_> = std::env::var_os("JAVA_HOME")
            .map(PathBuf::from)
            .map(|home| home.join("lib/src.zip"))
            .filter(|path| path.is_file())
            .into_iter()
            .collect();
        let session = frontend
            .create_session(&[], &source_path, 25)
            .expect("create project session");
        let completion_source =
            "import java.util.*; class Demo { void f() { List<String> xs=null; xs.ad; } }";
        let cursor = completion_source.find("xs.ad").unwrap() + "xs.ad".len();
        let completion = session
            .editor_query("Demo.java", completion_source, cursor as u32)
            .expect("query member completion");
        assert!(
            completion
                .completions
                .iter()
                .any(|item| item.label == "add" && item.detail.contains("boolean"))
        );
        if !source_path.is_empty() {
            assert!(
                completion
                    .completions
                    .iter()
                    .find(|item| item.label == "add")
                    .is_some_and(|item| !item.documentation.is_empty())
            );
        }

        let signature_source =
            "import java.util.*; class Demo { void f() { List<String> xs=null; xs.add( } }";
        let cursor = signature_source.find("add(").unwrap() + "add(".len();
        let signature = session
            .editor_query("Demo.java", signature_source, cursor as u32)
            .expect("query signature");
        assert!(
            signature
                .signatures
                .iter()
                .any(|item| item.label.contains("add(") && !item.parameters.is_empty())
        );
    }

    #[test]
    fn supports_repeated_isolate_lifecycle() {
        for iteration in 0..32 {
            let frontend = Frontend::new().expect("create frontend");
            let source = format!("record Lifecycle{iteration}(int value) {{}}");
            let payload = frontend.parse_raw(&source).expect("parse lifecycle source");
            assert!(payload.starts_with(b"JFE1"));
        }
    }

    #[test]
    fn independent_isolates_parse_concurrently() {
        let workers: Vec<_> = (0..4)
            .map(|worker| {
                std::thread::spawn(move || {
                    let frontend = Frontend::new().expect("create concurrent frontend");
                    for iteration in 0..25 {
                        let source = format!(
                            "package worker{worker}; class Item{iteration} {{ String café; }}"
                        );
                        let payload = frontend.parse_raw(&source).expect("parse concurrently");
                        assert!(payload.starts_with(b"JFE1"));
                    }
                })
            })
            .collect();

        for worker in workers {
            worker.join().expect("worker must not panic");
        }
    }

    #[test]
    fn warm_parse_throughput_stays_interactive() {
        let frontend = Frontend::new().expect("create frontend");
        let source = "package benchmark; sealed interface Node permits Leaf {} final class Leaf implements Node { String value() { return \"ok\"; } }";
        frontend.parse_raw(source).expect("warm up parser");

        let started = Instant::now();
        for _ in 0..250 {
            frontend.parse_raw(source).expect("parse benchmark source");
        }
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(10),
            "250 warm parses exceeded the exploratory 10s budget: {elapsed:?}"
        );
    }
}
