//! Native Java toolchain discovery and incremental workspace compilation.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env,
    ffi::OsString,
    fmt::Write as _,
    io::Write as _,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use fs2::FileExt;
use futures::{stream, StreamExt, TryStreamExt};
use jman_config::{Lockfile, Manifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader},
    process::Command,
    sync::{mpsc, Semaphore},
};

pub mod toolchain;

const STATE_VERSION: &str = "jman-build-v2";
const PACKAGE_STATE_VERSION: &str = "jman-package-v1";
static PACKAGE_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum BuildError {
    #[error("{0}")]
    Invalid(String),
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse {path}: {message}")]
    Config { path: PathBuf, message: String },
    #[error("could not execute Java compiler `{path}`: {source}")]
    Javac {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Java compilation failed for {module}\n{diagnostics}")]
    Compilation { module: String, diagnostics: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Toolchain {
    pub javac: PathBuf,
    pub version: String,
    pub major: u16,
    pub managed: Option<toolchain::ManagedJdk>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleResult {
    pub name: String,
    pub directory: PathBuf,
    pub sources: usize,
    pub rebuilt: bool,
    pub output: PathBuf,
    pub generated_sources: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckResult {
    pub toolchain: Toolchain,
    pub modules: Vec<ModuleResult>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageResult {
    pub module: String,
    pub kind: &'static str,
    pub artifact: PathBuf,
    pub size: u64,
    pub checksum: String,
    pub unchanged: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PackageCacheRecord {
    key: String,
    checksum: String,
    size: u64,
    warnings: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FatEntry {
    bytes: Vec<u8>,
    source: String,
}

#[derive(Default)]
struct FatArchive {
    entries: BTreeMap<String, FatEntry>,
    warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResourceMergeStrategy {
    KeepFirst,
    Lines,
    Properties,
    PropertyLists,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ArtifactOptions {
    pub fat: bool,
    pub sources: bool,
    pub javadoc: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunResult {
    pub module: String,
    pub main_class: String,
    pub status: std::process::ExitStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TestModuleResult {
    pub module: String,
    pub sources: usize,
    pub rebuilt: bool,
    pub status: std::process::ExitStatus,
    pub summary: Option<TestSummary>,
    pub test_cases: Vec<TestCaseResult>,
    pub output: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TestRunResult {
    pub modules: Vec<TestModuleResult>,
    pub coverage: Option<CoverageReport>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCaseResult {
    pub id: String,
    pub selector: String,
    pub class_name: String,
    pub method_name: String,
    pub display_name: String,
    pub invocation: Option<String>,
    pub attempt: u32,
    pub status: TestCaseStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_nanos: Option<u64>,
    pub duration_millis: u64,
    pub message: Option<String>,
    pub details: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TestCaseStatus {
    Passed,
    Failed,
    Skipped,
    Errored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TestEventReason {
    ContainerStarted,
    ContainerFinished,
    TestStarted,
    TestFinished,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestEvent {
    pub protocol_version: u16,
    pub reason: TestEventReason,
    #[serde(default)]
    pub module: String,
    pub id: String,
    pub parent_id: Option<String>,
    pub selector: Option<String>,
    pub class_name: Option<String>,
    pub method_name: Option<String>,
    pub display_name: String,
    pub status: Option<TestCaseStatus>,
    pub duration_nanos: Option<u64>,
    pub duration_millis: Option<u64>,
    pub lifecycle_nanos: Option<u64>,
    pub lifecycle_millis: Option<u64>,
    pub message: Option<String>,
    pub details: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TestSourceSet {
    #[default]
    All,
    Unit,
    Integration,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestSummary {
    pub duration_millis: u64,
    pub containers_found: usize,
    pub containers_started: usize,
    pub containers_successful: usize,
    pub containers_skipped: usize,
    pub containers_aborted: usize,
    pub containers_failed: usize,
    pub tests_found: usize,
    pub tests_started: usize,
    pub tests_successful: usize,
    pub tests_skipped: usize,
    pub tests_aborted: usize,
    pub tests_failed: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TestOptions {
    pub modules: BTreeSet<String>,
    pub patterns: Vec<String>,
    pub console_arguments: Vec<String>,
    pub source_set: TestSourceSet,
    pub debug: bool,
    pub coverage: Option<CoverageOptions>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverageOptions {
    pub output_directory: PathBuf,
    pub formats: BTreeSet<CoverageOutputFormat>,
    pub minimum_line: Option<u8>,
    pub minimum_branch: Option<u8>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CoverageOutputFormat {
    Summary,
    Json,
    Xml,
    Html,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageCount {
    pub missed: u64,
    pub covered: u64,
}

impl CoverageCount {
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.missed + self.covered
    }

    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn percentage(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            100.0
        } else {
            self.covered as f64 * 100.0 / total as f64
        }
    }

    fn add(&mut self, other: &Self) {
        self.missed += other.missed;
        self.covered += other.covered;
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageCounters {
    pub instruction: CoverageCount,
    pub line: CoverageCount,
    pub branch: CoverageCount,
    pub complexity: CoverageCount,
    pub method: CoverageCount,
    pub class: CoverageCount,
}

impl CoverageCounters {
    fn add(&mut self, other: &Self) {
        self.instruction.add(&other.instruction);
        self.line.add(&other.line);
        self.branch.add(&other.branch);
        self.complexity.add(&other.complexity);
        self.method.add(&other.method);
        self.class.add(&other.class);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageLine {
    pub number: u32,
    pub instruction: CoverageCount,
    pub branch: CoverageCount,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageFile {
    pub module: String,
    pub package: String,
    pub name: String,
    pub path: PathBuf,
    pub counters: CoverageCounters,
    pub lines: Vec<CoverageLine>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageModule {
    pub module: String,
    pub counters: CoverageCounters,
    pub files: Vec<CoverageFile>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageThresholdFailure {
    pub metric: String,
    pub required: u8,
    pub actual_basis_points: u64,
}

impl CoverageThresholdFailure {
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn actual_percentage(&self) -> f64 {
        self.actual_basis_points as f64 / 100.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageReport {
    pub protocol_version: u16,
    pub engine: String,
    pub output_directory: PathBuf,
    pub html: Option<PathBuf>,
    pub xml: Option<PathBuf>,
    pub json: Option<PathBuf>,
    pub modules: Vec<CoverageModule>,
    pub totals: CoverageCounters,
    pub threshold_failures: Vec<CoverageThresholdFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum JunitSelection {
    Scan,
    Classes(String),
    Methods(Vec<String>),
}

#[derive(Clone)]
struct Module {
    directory: PathBuf,
    manifest: Manifest,
    dependencies: Vec<usize>,
}

/// Locate `javac`, query its version, and validate it against a requested release.
///
/// # Errors
///
/// Returns an error when `javac` cannot be executed, its version is malformed,
/// or it is older than the requested Java release.
pub async fn discover_toolchain(requested_release: u16) -> Result<Toolchain, BuildError> {
    let javac = env::var_os("JAVA_HOME").map_or_else(
        || PathBuf::from("javac"),
        |home| {
            let executable = if cfg!(windows) { "javac.exe" } else { "javac" };
            PathBuf::from(home).join("bin").join(executable)
        },
    );
    probe_javac(javac, requested_release).await
}

pub(crate) async fn probe_javac(
    javac: PathBuf,
    requested_release: u16,
) -> Result<Toolchain, BuildError> {
    let output = Command::new(&javac)
        .arg("-version")
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .await
        .map_err(|source| BuildError::Javac {
            path: javac.clone(),
            source,
        })?;
    if !output.status.success() {
        return Err(BuildError::Invalid(format!(
            "`{}` -version exited with {}",
            javac.display(),
            output.status
        )));
    }
    let version = String::from_utf8_lossy(if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    })
    .trim()
    .to_owned();
    let major = parse_javac_major(&version).ok_or_else(|| {
        BuildError::Invalid(format!("could not parse Java compiler version `{version}`"))
    })?;
    if major < requested_release {
        return Err(BuildError::Invalid(format!(
            "project requires Java {requested_release}, but {version} was found"
        )));
    }
    Ok(Toolchain {
        javac,
        version,
        major,
        managed: None,
    })
}

/// Compile every module in dependency order, rebuilding only invalidated modules.
///
/// # Errors
///
/// Returns an error for invalid workspace graphs, missing lock/cache inputs,
/// unavailable toolchains, filesystem failures, or Java compilation diagnostics.
pub async fn check_workspace(
    root: &Path,
    cache_dir: &Path,
    jobs: usize,
    offline: bool,
) -> Result<CheckResult, BuildError> {
    let modules = discover_modules(root).await?;
    let required_release = modules
        .iter()
        .map(|module| module.manifest.project.java_release)
        .max()
        .unwrap_or(17);
    let requests = modules
        .iter()
        .filter_map(|module| module.manifest.toolchain.as_ref())
        .map(|toolchain| (toolchain.jdk.clone(), toolchain.vendor.clone()))
        .collect::<BTreeSet<_>>();
    if requests.len() > 1 {
        return Err(BuildError::Invalid(format!(
            "workspace modules request conflicting JDK toolchains: {}",
            requests
                .iter()
                .map(|(version, vendor)| format!("{vendor}:{version}"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let toolchain = if let Some((version, vendor)) = requests.into_iter().next() {
        toolchain::resolve(
            &toolchain::ToolchainRequest { version, vendor },
            cache_dir,
            true,
            offline,
        )
        .await?
    } else {
        toolchain::resolve_default(required_release, cache_dir).await?
    };
    let layers = topological_layers(&modules)?;
    let semaphore = Arc::new(Semaphore::new(jobs.max(1)));
    let mut results = BTreeMap::new();
    let mut fingerprints = BTreeMap::<usize, String>::new();

    for layer in layers {
        let completed = stream::iter(layer.into_iter().map(|index| {
            let module = modules[index].clone();
            let toolchain = toolchain.clone();
            let cache_dir = cache_dir.to_owned();
            let semaphore = Arc::clone(&semaphore);
            let upstream = module
                .dependencies
                .iter()
                .map(|dependency| {
                    fingerprints.get(dependency).map_or_else(
                        || {
                            Err(BuildError::Invalid(
                                "module scheduler lost a predecessor result".to_owned(),
                            ))
                        },
                        |fingerprint| {
                            Ok((modules[*dependency].directory.clone(), fingerprint.clone()))
                        },
                    )
                })
                .collect::<Result<Vec<_>, _>>();
            async move {
                let upstream = upstream?;
                let _permit = semaphore.acquire_owned().await.map_err(|_| {
                    BuildError::Invalid("compilation scheduler closed unexpectedly".to_owned())
                })?;
                compile_module(&module, &toolchain, &cache_dir, &upstream)
                    .await
                    .map(|(result, fingerprint)| (index, result, fingerprint))
            }
        }))
        .buffer_unordered(jobs.max(1))
        .try_collect::<Vec<_>>()
        .await?;
        for (index, result, fingerprint) in completed {
            results.insert(index, result);
            fingerprints.insert(index, fingerprint);
        }
    }

    Ok(CheckResult {
        toolchain,
        modules: results.into_values().collect(),
    })
}

/// Build and package every non-aggregator module as a deterministic thin JAR.
///
/// # Errors
///
/// Returns an error for build failures, unsafe archive paths, or filesystem and
/// ZIP creation failures.
pub async fn package_workspace(
    root: &Path,
    cache_dir: &Path,
    jobs: usize,
    offline: bool,
    options: ArtifactOptions,
) -> Result<Vec<PackageResult>, BuildError> {
    let modules = discover_modules(root).await?;
    if options.fat
        && !modules
            .iter()
            .any(|module| module.manifest.project.main_class.is_some())
    {
        return Err(BuildError::Invalid(
            "`--fat` requires at least one module with `project.main-class`".to_owned(),
        ));
    }
    let build = check_workspace(root, cache_dir, jobs, offline).await?;
    if modules.len() != build.modules.len() {
        return Err(BuildError::Invalid(
            "workspace changed while artifacts were being built".to_owned(),
        ));
    }
    let mut packages = Vec::new();
    for (index, module) in build.modules.iter().enumerate() {
        let manifest = &modules[index].manifest;
        if manifest.project.packaging == "pom" {
            continue;
        }
        let upstream = transitive_dependencies(index, &modules)
            .into_iter()
            .map(|dependency| build.modules[dependency].output.clone())
            .collect::<Vec<_>>();
        packages.push(
            package_cached(
                module,
                manifest,
                "thin",
                &build.toolchain,
                cache_dir,
                &upstream,
            )
            .await?,
        );
        if options.sources {
            packages.push(
                package_cached(
                    module,
                    manifest,
                    "sources",
                    &build.toolchain,
                    cache_dir,
                    &upstream,
                )
                .await?,
            );
        }
        if options.javadoc {
            packages.push(
                package_cached(
                    module,
                    manifest,
                    "javadoc",
                    &build.toolchain,
                    cache_dir,
                    &upstream,
                )
                .await?,
            );
        }
        if options.fat && manifest.project.main_class.is_some() {
            packages.push(
                package_cached(
                    module,
                    manifest,
                    "fat",
                    &build.toolchain,
                    cache_dir,
                    &upstream,
                )
                .await?,
            );
        }
    }
    Ok(packages)
}

/// Compile and run the single application module in a workspace.
///
/// # Errors
///
/// Returns an error when no unique application entry point exists, compilation
/// fails, runtime artifacts are unavailable, or the JVM cannot be started.
pub async fn run_workspace(
    root: &Path,
    cache_dir: &Path,
    jobs: usize,
    offline: bool,
    arguments: &[String],
) -> Result<RunResult, BuildError> {
    let modules = discover_modules(root).await?;
    let applications = modules
        .iter()
        .enumerate()
        .filter_map(|(index, module)| {
            module
                .manifest
                .project
                .main_class
                .as_ref()
                .map(|main_class| (index, module, main_class))
        })
        .collect::<Vec<_>>();
    let [(index, application, main_class)] = applications.as_slice() else {
        return Err(BuildError::Invalid(if applications.is_empty() {
            "no application module found; set `project.main-class` in jman.toml".to_owned()
        } else {
            format!(
                "multiple application modules found: {}; run from a selected module is not supported yet",
                applications
                    .iter()
                    .map(|(_, module, _)| module.manifest.project.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }));
    };
    let build = check_workspace(root, cache_dir, jobs, offline).await?;
    let mut classpath = vec![build.modules[*index].output.clone()];
    classpath.extend(
        transitive_dependencies(*index, &modules)
            .into_iter()
            .map(|dependency| build.modules[dependency].output.clone()),
    );
    let lock = read_lock(&application.directory.join("jman.lock"))?;
    classpath.extend(artifact_paths(&lock.classpath.runtime, &lock, cache_dir)?);
    let executable = if cfg!(windows) { "java.exe" } else { "java" };
    let java = build.toolchain.javac.parent().map_or_else(
        || PathBuf::from(executable),
        |parent| parent.join(executable),
    );
    let status = Command::new(&java)
        .current_dir(&application.directory)
        .arg("-classpath")
        .arg(join_paths(&classpath)?)
        .arg(main_class)
        .args(arguments)
        .status()
        .await
        .map_err(|source| BuildError::Javac { path: java, source })?;
    Ok(RunResult {
        module: application.manifest.project.name.clone(),
        main_class: (*main_class).clone(),
        status,
    })
}

/// Compile and execute `JUnit` Platform tests in isolated per-module JVMs.
///
/// The project's test classpath must include `junit-platform-console` or
/// `junit-platform-console-standalone`; JMAN never downloads hidden test
/// dependencies.
///
/// # Errors
///
/// Returns an error for compilation failures, missing `JUnit` Platform support,
/// unavailable artifacts, or JVM launch failures.
pub async fn test_workspace(
    root: &Path,
    cache_dir: &Path,
    jobs: usize,
    offline: bool,
    options: &TestOptions,
    events: Option<mpsc::UnboundedSender<TestEvent>>,
) -> Result<TestRunResult, BuildError> {
    let modules = discover_modules(root).await?;
    validate_test_modules(&modules, &options.modules)?;
    let selection = junit_selection(&options.patterns)?;
    let build = check_workspace(root, cache_dir, jobs, offline).await?;
    let executable = if cfg!(windows) { "java.exe" } else { "java" };
    let java = build.toolchain.javac.parent().map_or_else(
        || PathBuf::from(executable),
        |parent| parent.join(executable),
    );
    let coverage_runtime = if options.coverage.is_some() {
        Some(prepare_coverage_runtime(root, cache_dir).await?)
    } else {
        None
    };
    let mut results = stream::iter(modules.iter().enumerate())
        .map(|(index, module)| {
            run_test_module(
                index,
                module,
                &modules,
                &build,
                cache_dir,
                &java,
                &selection,
                options,
                coverage_runtime.as_ref(),
                events.clone(),
            )
        })
        .buffer_unordered(jobs.max(1))
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    results.sort_by(|left, right| left.module.cmp(&right.module));
    let coverage = if let (Some(coverage_options), Some(runtime)) =
        (&options.coverage, coverage_runtime.as_ref())
    {
        if results.is_empty() {
            None
        } else {
            Some(
                generate_coverage_report(
                    &modules,
                    &build,
                    &results,
                    &java,
                    runtime,
                    coverage_options,
                )
                .await?,
            )
        }
    } else {
        None
    };
    if let Some(runtime) = coverage_runtime {
        let _ = fs::remove_dir_all(runtime.work_directory).await;
    }
    Ok(TestRunResult {
        modules: results,
        coverage,
    })
}

#[derive(Clone, Debug)]
struct CoverageRuntime {
    agent: PathBuf,
    cli: PathBuf,
    work_directory: PathBuf,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn run_test_module(
    index: usize,
    module: &Module,
    modules: &[Module],
    build: &CheckResult,
    cache_dir: &Path,
    java: &Path,
    selection: &JunitSelection,
    options: &TestOptions,
    coverage: Option<&CoverageRuntime>,
    events: Option<mpsc::UnboundedSender<TestEvent>>,
) -> Result<Option<TestModuleResult>, BuildError> {
    if module.manifest.project.packaging == "pom"
        || (!options.modules.is_empty() && !options.modules.contains(&module.manifest.project.name))
    {
        return Ok(None);
    }
    let source_roots: &[&str] = match options.source_set {
        TestSourceSet::All => &["src/test/java", "src/integrationTest/java"],
        TestSourceSet::Unit => &["src/test/java"],
        TestSourceSet::Integration => &["src/integrationTest/java"],
    };
    let mut sources = Vec::new();
    for source_root in source_roots {
        sources.extend(java_sources(&module.directory.join(source_root)).await?);
    }
    sources.sort();
    if sources.is_empty() {
        return Ok(None);
    }
    let lock = read_lock(&module.directory.join("jman.lock"))?;
    ensure_junit_console(&lock)?;
    let mut classpath = vec![build.modules[index].output.clone()];
    classpath.extend(
        transitive_dependencies(index, modules)
            .into_iter()
            .map(|dependency| build.modules[dependency].output.clone()),
    );
    classpath.extend(artifact_paths(&lock.classpath.test, &lock, cache_dir)?);
    let runner = ensure_test_runner(&build.toolchain, cache_dir, &classpath).await?;
    let processors = artifact_paths(&lock.classpath.processors, &lock, cache_dir)?;
    let (test_output, rebuilt) =
        compile_test_sources(module, &build.toolchain, &sources, &classpath, &processors).await?;
    let mut runtime = vec![runner, test_output.clone()];
    for resource_root in source_roots {
        let resources = module
            .directory
            .join(resource_root.replace("/java", "/resources"));
        if resources.is_dir() {
            runtime.push(resources);
        }
    }
    runtime.extend(classpath);
    let port = env::var("JMAN_TEST_PORT").unwrap_or_else(|_| "0".to_owned());
    let mut command = Command::new(java);
    command.kill_on_drop(true).current_dir(&module.directory);
    if options.debug {
        command.arg("-agentlib:jdwp=transport=dt_socket,server=y,suspend=y,address=0");
    }
    if let Some(coverage) = coverage {
        let execution_file = coverage.work_directory.join(format!(
            "{}.exec",
            safe_file_name(&module.manifest.project.name)
        ));
        let mut agent_options = vec![
            format!("destfile={}", execution_file.display()),
            "append=false".to_owned(),
            "dumponexit=true".to_owned(),
            format!(
                "sessionid={}",
                safe_file_name(&module.manifest.project.name)
            ),
        ];
        if let Some(options) = &options.coverage {
            if !options.include.is_empty() {
                agent_options.push(format!("includes={}", options.include.join(":")));
            }
            if !options.exclude.is_empty() {
                agent_options.push(format!("excludes={}", options.exclude.join(":")));
            }
        }
        command.arg(format!(
            "-javaagent:{}={}",
            coverage.agent.display(),
            agent_options.join(",")
        ));
    }
    command
        .arg(format!("-Djman.test.port={port}"))
        .arg("-classpath")
        .arg(join_paths(&runtime)?)
        .args([
            "io.github.zonnedev.jman.runner.JmanRunner",
            "--protocol",
            "3",
            "--",
            "execute",
            "--disable-banner",
            "--details=summary",
        ]);
    let reports = tempfile::tempdir().map_err(|source| BuildError::Io {
        path: module.directory.clone(),
        source,
    })?;
    command.arg(format!("--reports-dir={}", reports.path().display()));
    match selection {
        JunitSelection::Scan => {
            command.arg(format!("--scan-class-path={}", test_output.display()));
        }
        JunitSelection::Classes(pattern) => {
            command
                .arg(format!("--scan-class-path={}", test_output.display()))
                .arg(format!("--include-classname={pattern}"));
        }
        JunitSelection::Methods(methods) => {
            for method in methods {
                command.arg(format!("--select-method={method}"));
            }
        }
    }
    if !options.patterns.is_empty() {
        command.arg("--fail-if-no-tests");
    }
    command
        .args(&options.console_arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|source| BuildError::Javac {
        path: java.to_owned(),
        source,
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| BuildError::Invalid("JUnit worker stdout was not captured".to_owned()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| BuildError::Invalid("JUnit worker stderr was not captured".to_owned()))?;
    let module_name = module.manifest.project.name.clone();
    let stdout_task = tokio::spawn(read_test_stdout(stdout, module_name, events));
    let stderr_task = tokio::spawn(read_test_stderr(stderr));
    let status = child.wait().await.map_err(|source| BuildError::Javac {
        path: java.to_owned(),
        source,
    })?;
    let stdout = stdout_task
        .await
        .map_err(|error| BuildError::Invalid(format!("JUnit stdout reader failed: {error}")))??;
    let stderr = stderr_task
        .await
        .map_err(|error| BuildError::Invalid(format!("JUnit stderr reader failed: {error}")))??;
    let mut text = String::from_utf8_lossy(&stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&stderr));
    Ok(Some(TestModuleResult {
        module: module.manifest.project.name.clone(),
        sources: sources.len(),
        rebuilt,
        status,
        summary: parse_junit_summary(&text),
        test_cases: parse_junit_reports(reports.path())?,
        output: text,
    }))
}

async fn read_test_stdout<R: AsyncRead + Unpin>(
    reader: R,
    module: String,
    events: Option<mpsc::UnboundedSender<TestEvent>>,
) -> Result<Vec<u8>, BuildError> {
    let mut reader = BufReader::new(reader);
    let mut output = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        let bytes = reader
            .read_until(b'\n', &mut line)
            .await
            .map_err(|source| io_error(Path::new("JUnit worker stdout"), source))?;
        if bytes == 0 {
            break;
        }
        let event = line
            .strip_prefix(&[0x1e])
            .and_then(|json| serde_json::from_slice::<TestEvent>(json).ok());
        if let Some(mut event) = event {
            event.module.clone_from(&module);
            if event.protocol_version == 3 {
                if let Some(events) = &events {
                    let _ = events.send(event);
                }
                continue;
            }
        }
        output.extend_from_slice(&line);
    }
    Ok(output)
}

async fn read_test_stderr<R: AsyncRead + Unpin>(mut reader: R) -> Result<Vec<u8>, BuildError> {
    let mut output = Vec::new();
    reader
        .read_to_end(&mut output)
        .await
        .map_err(|source| io_error(Path::new("JUnit worker stderr"), source))?;
    Ok(output)
}

async fn prepare_coverage_runtime(
    root: &Path,
    cache_dir: &Path,
) -> Result<CoverageRuntime, BuildError> {
    let (agent, cli) = coverage_tool_paths()?;
    let identity = hex::encode(Sha256::digest(root.to_string_lossy().as_bytes()));
    let work_directory = cache_dir.join("tools/coverage/runs").join(format!(
        "{}-{}",
        std::process::id(),
        &identity[..12]
    ));
    if work_directory.exists() {
        fs::remove_dir_all(&work_directory)
            .await
            .map_err(|source| io_error(&work_directory, source))?;
    }
    fs::create_dir_all(&work_directory)
        .await
        .map_err(|source| io_error(&work_directory, source))?;
    Ok(CoverageRuntime {
        agent,
        cli,
        work_directory,
    })
}

fn coverage_tool_paths() -> Result<(PathBuf, PathBuf), BuildError> {
    match (
        env::var_os("JMAN_JACOCO_AGENT"),
        env::var_os("JMAN_JACOCO_CLI"),
    ) {
        (Some(agent), Some(cli)) => {
            let agent = PathBuf::from(agent);
            let cli = PathBuf::from(cli);
            if agent.is_file() && cli.is_file() {
                return Ok((agent, cli));
            }
            return Err(BuildError::Invalid(format!(
                "configured JaCoCo tools are missing: {} and {}",
                agent.display(),
                cli.display()
            )));
        }
        (None, None) => {}
        _ => {
            return Err(BuildError::Invalid(
                "JMAN_JACOCO_AGENT and JMAN_JACOCO_CLI must be configured together".to_owned(),
            ));
        }
    }
    if let Ok(executable) = env::current_exe() {
        if let Some(directory) = executable.parent() {
            let agent = directory.join("jacocoagent.jar");
            let cli = directory.join("jacococli.jar");
            if agent.is_file() && cli.is_file() {
                return Ok((agent, cli));
            }
        }
    }
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let agent = project.join("target/jacoco-0.8.15-agent.jar");
    let cli = project.join("target/jacoco-0.8.15-cli.jar");
    if agent.is_file() && cli.is_file() {
        return Ok((agent, cli));
    }
    Err(BuildError::Invalid(
        "JaCoCo coverage tools are unavailable; reinstall JMAN or run `make jacoco`".to_owned(),
    ))
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn generate_coverage_report(
    modules: &[Module],
    build: &CheckResult,
    results: &[TestModuleResult],
    java: &Path,
    runtime: &CoverageRuntime,
    options: &CoverageOptions,
) -> Result<CoverageReport, BuildError> {
    let output_directory = &options.output_directory;
    fs::create_dir_all(output_directory)
        .await
        .map_err(|source| io_error(output_directory, source))?;

    let mut coverage_modules = Vec::new();
    let mut execution_files = Vec::new();
    let mut class_files = Vec::new();
    let mut source_directories = Vec::new();
    for result in results {
        let Some((index, module)) = modules
            .iter()
            .enumerate()
            .find(|(_, module)| module.manifest.project.name == result.module)
        else {
            return Err(BuildError::Invalid(format!(
                "coverage module `{}` disappeared from the workspace",
                result.module
            )));
        };
        let execution_file = runtime
            .work_directory
            .join(format!("{}.exec", safe_file_name(&result.module)));
        if !execution_file.is_file() {
            return Err(BuildError::Invalid(format!(
                "JaCoCo did not produce execution data for module `{}`",
                result.module
            )));
        }
        let module_classes = coverage_class_files(
            &build.modules[index].output,
            &options.include,
            &options.exclude,
        )
        .await?;
        if module_classes.is_empty() {
            coverage_modules.push(CoverageModule {
                module: result.module.clone(),
                counters: CoverageCounters::default(),
                files: Vec::new(),
            });
            continue;
        }
        let source_directory = module.directory.join("src/main/java");
        let module_xml = runtime
            .work_directory
            .join(format!("{}.xml", safe_file_name(&result.module)));
        run_jacoco_report(
            java,
            &runtime.cli,
            std::slice::from_ref(&execution_file),
            &module_classes,
            std::slice::from_ref(&source_directory),
            Some(&module_xml),
            None,
            &result.module,
        )
        .await?;
        let xml = fs::read_to_string(&module_xml)
            .await
            .map_err(|source| io_error(&module_xml, source))?;
        coverage_modules.push(parse_jacoco_xml(&result.module, &source_directory, &xml)?);
        execution_files.push(execution_file);
        class_files.extend(module_classes);
        if source_directory.is_dir() {
            source_directories.push(source_directory);
        }
    }
    coverage_modules.sort_by(|left, right| left.module.cmp(&right.module));
    let mut totals = CoverageCounters::default();
    for module in &coverage_modules {
        totals.add(&module.counters);
    }

    let xml = options
        .formats
        .contains(&CoverageOutputFormat::Xml)
        .then(|| output_directory.join("coverage.xml"));
    let html = options
        .formats
        .contains(&CoverageOutputFormat::Html)
        .then(|| output_directory.join("index.html"));
    if !execution_files.is_empty() && (xml.is_some() || html.is_some()) {
        run_jacoco_report(
            java,
            &runtime.cli,
            &execution_files,
            &class_files,
            &source_directories,
            xml.as_deref(),
            html.as_ref().map(|_| output_directory.as_path()),
            "JMAN workspace",
        )
        .await?;
    }
    let json = options
        .formats
        .contains(&CoverageOutputFormat::Json)
        .then(|| output_directory.join("coverage.json"));
    let threshold_failures =
        coverage_threshold_failures(&totals, options.minimum_line, options.minimum_branch);
    let report = CoverageReport {
        protocol_version: 1,
        engine: "jacoco".to_owned(),
        output_directory: output_directory.clone(),
        html,
        xml,
        json: json.clone(),
        modules: coverage_modules,
        totals,
        threshold_failures,
    };
    if let Some(json) = json {
        let bytes = serde_json::to_vec_pretty(&report).map_err(|error| {
            BuildError::Invalid(format!("could not serialize coverage report: {error}"))
        })?;
        fs::write(&json, bytes)
            .await
            .map_err(|source| io_error(&json, source))?;
    }
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
async fn run_jacoco_report(
    java: &Path,
    cli: &Path,
    execution_files: &[PathBuf],
    class_files: &[PathBuf],
    source_directories: &[PathBuf],
    xml: Option<&Path>,
    html: Option<&Path>,
    name: &str,
) -> Result<(), BuildError> {
    let mut command = Command::new(java);
    command.arg("-jar").arg(cli).arg("report");
    for execution_file in execution_files {
        command.arg(execution_file);
    }
    for class_file in class_files {
        command.arg("--classfiles").arg(class_file);
    }
    for source_directory in source_directories {
        if source_directory.is_dir() {
            command.arg("--sourcefiles").arg(source_directory);
        }
    }
    command.args(["--name", name, "--quiet"]);
    if let Some(xml) = xml {
        command.arg("--xml").arg(xml);
    }
    if let Some(html) = html {
        command.arg("--html").arg(html);
    }
    let result = command.output().await.map_err(|source| BuildError::Javac {
        path: java.to_owned(),
        source,
    })?;
    if !result.status.success() {
        return Err(BuildError::Invalid(format!(
            "JaCoCo report generation failed: {}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        )));
    }
    Ok(())
}

async fn coverage_class_files(
    output: &Path,
    includes: &[String],
    excludes: &[String],
) -> Result<Vec<PathBuf>, BuildError> {
    let mut classes = Vec::new();
    for path in input_files(output, Some("class")).await? {
        let name = path
            .strip_prefix(output)
            .map_err(|error| BuildError::Invalid(error.to_string()))?
            .with_extension("")
            .to_string_lossy()
            .replace(['/', '\\'], ".");
        let matches_include = includes.is_empty()
            || includes
                .iter()
                .any(|pattern| wildcard_matches(pattern, &name));
        let matches_exclude = excludes
            .iter()
            .any(|pattern| wildcard_matches(pattern, &name));
        if matches_include && !matches_exclude {
            classes.push(path);
        }
    }
    classes.sort();
    Ok(classes)
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.replace(['/', '\\'], ".");
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let (mut pattern_index, mut value_index) = (0, 0);
    let (mut star, mut retry) = (None, 0);
    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == value[value_index])
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star = Some(pattern_index);
            pattern_index += 1;
            retry = value_index;
        } else if let Some(star_index) = star {
            pattern_index = star_index + 1;
            retry += 1;
            value_index = retry;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

fn parse_jacoco_xml(
    module: &str,
    source_directory: &Path,
    xml: &str,
) -> Result<CoverageModule, BuildError> {
    let document = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..roxmltree::ParsingOptions::default()
        },
    )
    .map_err(|error| BuildError::Invalid(format!("invalid JaCoCo XML: {error}")))?;
    let report = document.root_element();
    let mut files = Vec::new();
    for package in report
        .children()
        .filter(|node| node.has_tag_name("package"))
    {
        let package_name = package.attribute("name").unwrap_or_default();
        for source_file in package
            .children()
            .filter(|node| node.has_tag_name("sourcefile"))
        {
            let name = source_file.attribute("name").unwrap_or_default().to_owned();
            let mut path = source_directory.to_owned();
            if !package_name.is_empty() {
                path.push(package_name);
            }
            path.push(&name);
            let mut lines = source_file
                .children()
                .filter(|node| node.has_tag_name("line"))
                .map(|line| {
                    Ok(CoverageLine {
                        number: xml_u32(&line, "nr")?,
                        instruction: CoverageCount {
                            missed: xml_u64(&line, "mi")?,
                            covered: xml_u64(&line, "ci")?,
                        },
                        branch: CoverageCount {
                            missed: xml_u64(&line, "mb")?,
                            covered: xml_u64(&line, "cb")?,
                        },
                    })
                })
                .collect::<Result<Vec<_>, BuildError>>()?;
            lines.sort_by_key(|line| line.number);
            files.push(CoverageFile {
                module: module.to_owned(),
                package: package_name.replace('/', "."),
                name,
                path,
                counters: xml_counters(&source_file)?,
                lines,
            });
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(CoverageModule {
        module: module.to_owned(),
        counters: xml_counters(&report)?,
        files,
    })
}

fn xml_counters(node: &roxmltree::Node<'_, '_>) -> Result<CoverageCounters, BuildError> {
    let mut counters = CoverageCounters::default();
    for counter in node
        .children()
        .filter(|child| child.has_tag_name("counter"))
    {
        let value = CoverageCount {
            missed: xml_u64(&counter, "missed")?,
            covered: xml_u64(&counter, "covered")?,
        };
        match counter.attribute("type") {
            Some("INSTRUCTION") => counters.instruction = value,
            Some("LINE") => counters.line = value,
            Some("BRANCH") => counters.branch = value,
            Some("COMPLEXITY") => counters.complexity = value,
            Some("METHOD") => counters.method = value,
            Some("CLASS") => counters.class = value,
            _ => {}
        }
    }
    Ok(counters)
}

fn xml_u64(node: &roxmltree::Node<'_, '_>, attribute: &str) -> Result<u64, BuildError> {
    node.attribute(attribute)
        .ok_or_else(|| BuildError::Invalid(format!("JaCoCo XML is missing `{attribute}`")))?
        .parse()
        .map_err(|error| BuildError::Invalid(format!("invalid JaCoCo `{attribute}`: {error}")))
}

fn xml_u32(node: &roxmltree::Node<'_, '_>, attribute: &str) -> Result<u32, BuildError> {
    let value = xml_u64(node, attribute)?;
    u32::try_from(value)
        .map_err(|error| BuildError::Invalid(format!("invalid JaCoCo `{attribute}`: {error}")))
}

fn coverage_threshold_failures(
    counters: &CoverageCounters,
    minimum_line: Option<u8>,
    minimum_branch: Option<u8>,
) -> Vec<CoverageThresholdFailure> {
    let mut failures = Vec::new();
    for (metric, minimum, count) in [
        ("line", minimum_line, &counters.line),
        ("branch", minimum_branch, &counters.branch),
    ] {
        let Some(required) = minimum else {
            continue;
        };
        let total = count.total();
        if total > 0 && count.covered * 100 < u64::from(required) * total {
            failures.push(CoverageThresholdFailure {
                metric: metric.to_owned(),
                required,
                actual_basis_points: count.covered * 10_000 / total,
            });
        }
    }
    failures
}

fn safe_file_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn validate_test_modules(
    modules: &[Module],
    requested: &BTreeSet<String>,
) -> Result<(), BuildError> {
    let module_names = modules
        .iter()
        .map(|module| module.manifest.project.name.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = requested
        .iter()
        .find(|module| !module_names.contains(module.as_str()))
    {
        return Err(BuildError::Invalid(format!(
            "unknown test module `{unknown}`; available modules: {}",
            module_names.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(())
}

fn parse_junit_reports(directory: &Path) -> Result<Vec<TestCaseResult>, BuildError> {
    let entries = std::fs::read_dir(directory).map_err(|source| BuildError::Io {
        path: directory.to_owned(),
        source,
    })?;
    let mut cases = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| BuildError::Io {
            path: directory.to_owned(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("xml") {
            continue;
        }
        let xml = std::fs::read_to_string(&path).map_err(|source| BuildError::Io {
            path: path.clone(),
            source,
        })?;
        let document = roxmltree::Document::parse(&xml).map_err(|error| BuildError::Config {
            path: path.clone(),
            message: error.to_string(),
        })?;
        for testcase in document
            .descendants()
            .filter(|node| node.has_tag_name("testcase"))
        {
            let class_name = testcase
                .attribute("classname")
                .unwrap_or_default()
                .to_owned();
            let raw_name = testcase.attribute("name").unwrap_or_default();
            let method_name = raw_name.split('(').next().unwrap_or(raw_name).to_owned();
            let selector = format!("{class_name}#{method_name}");
            let invocation = (raw_name != method_name).then(|| raw_name.to_owned());
            let failure = testcase
                .children()
                .find(|node| node.has_tag_name("failure"));
            let error = testcase.children().find(|node| node.has_tag_name("error"));
            let skipped = testcase
                .children()
                .find(|node| node.has_tag_name("skipped"));
            let problem = failure.or(error);
            let status = if failure.is_some() {
                TestCaseStatus::Failed
            } else if error.is_some() {
                TestCaseStatus::Errored
            } else if skipped.is_some() {
                TestCaseStatus::Skipped
            } else {
                TestCaseStatus::Passed
            };
            let duration_millis = testcase
                .attribute("time")
                .and_then(|value| value.parse::<f64>().ok())
                .and_then(duration_millis_from_seconds)
                .unwrap_or(0);
            cases.push(TestCaseResult {
                id: format!(
                    "{}::{selector}::{raw_name}",
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("junit")
                ),
                selector,
                class_name,
                method_name,
                display_name: raw_name.to_owned(),
                invocation,
                attempt: 1,
                status,
                duration_nanos: None,
                duration_millis,
                message: problem
                    .and_then(|node| node.attribute("message"))
                    .map(str::to_owned),
                details: problem.and_then(|node| node.text()).map(str::to_owned),
            });
        }
    }
    cases.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(cases)
}

#[allow(clippy::too_many_lines)]
async fn ensure_test_runner(
    toolchain: &Toolchain,
    cache_dir: &Path,
    classpath: &[PathBuf],
) -> Result<PathBuf, BuildError> {
    const RUNNER_SOURCE: &str = include_str!(
        "../../../tools/jman-runner/src/main/java/io/github/zonnedev/jman/runner/JmanRunner.java"
    );
    const LISTENER_SOURCE: &str = include_str!(
        "../../../tools/jman-runner/src/main/java/io/github/zonnedev/jman/runner/JmanExecutionListener.java"
    );
    let classpath_identity = classpath
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n");
    let digest = hex::encode(Sha256::digest(
        format!(
            "jman-runner-v3\n{}\n{classpath_identity}\n{RUNNER_SOURCE}\n{LISTENER_SOURCE}",
            toolchain.version
        )
        .as_bytes(),
    ));
    let directory = cache_dir.join("tools/jman-runner").join(digest);
    let artifact = directory.join("jman-runner.jar");
    if artifact.is_file() {
        return Ok(artifact);
    }
    let staging = cache_dir.join("tools/jman-runner").join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        &hex::encode(Sha256::digest(directory.to_string_lossy().as_bytes()))[..12]
    ));
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .await
            .map_err(|source| io_error(&staging, source))?;
    }
    let source_directory = staging.join("src/io/github/zonnedev/jman/runner");
    let runner_source = source_directory.join("JmanRunner.java");
    let listener_source = source_directory.join("JmanExecutionListener.java");
    let classes = staging.join("classes");
    fs::create_dir_all(&source_directory)
        .await
        .map_err(|error| io_error(&source_directory, error))?;
    fs::create_dir_all(&classes)
        .await
        .map_err(|error| io_error(&classes, error))?;
    fs::write(&runner_source, RUNNER_SOURCE)
        .await
        .map_err(|error| io_error(&runner_source, error))?;
    fs::write(&listener_source, LISTENER_SOURCE)
        .await
        .map_err(|error| io_error(&listener_source, error))?;
    let mut compiler = Command::new(&toolchain.javac);
    compiler.args(["-Werror", "-Xlint:all", "-d"]).arg(&classes);
    if !classpath.is_empty() {
        compiler.arg("-classpath").arg(join_paths(classpath)?);
    }
    let result = compiler
        .arg(&runner_source)
        .arg(&listener_source)
        .output()
        .await
        .map_err(|source| BuildError::Javac {
            path: toolchain.javac.clone(),
            source,
        })?;
    if !result.status.success() {
        let _ = fs::remove_dir_all(&staging).await;
        return Err(BuildError::Compilation {
            module: "jman-runner".to_owned(),
            diagnostics: String::from_utf8_lossy(&result.stderr).trim().to_owned(),
        });
    }
    let mut entries = Vec::new();
    for class in input_files(&classes, Some("class")).await? {
        let name = class
            .strip_prefix(&classes)
            .map_err(|error| BuildError::Invalid(error.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        entries.push((
            name,
            fs::read(&class)
                .await
                .map_err(|error| io_error(&class, error))?,
        ));
    }
    entries.push((
        "META-INF/services/org.junit.platform.launcher.TestExecutionListener".to_owned(),
        b"io.github.zonnedev.jman.runner.JmanExecutionListener\n".to_vec(),
    ));
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let staged_artifact = staging.join("jman-runner.jar");
    write_jar(staged_artifact.clone(), entries).await?;
    fs::create_dir_all(directory.parent().unwrap_or(cache_dir))
        .await
        .map_err(|error| io_error(cache_dir, error))?;
    if directory.exists() {
        fs::remove_dir_all(&directory)
            .await
            .map_err(|error| io_error(&directory, error))?;
    }
    replace_directory(&staging, &directory).await?;
    Ok(artifact)
}

fn parse_junit_summary(output: &str) -> Option<TestSummary> {
    let mut summary = TestSummary::default();
    let duration = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Test run finished after "))?;
    summary.duration_millis = parse_junit_duration(duration)?;
    for line in output.lines() {
        let line = line.trim();
        let Some(line) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        else {
            continue;
        };
        let mut words = line.split_whitespace();
        let Some(count) = words.next().and_then(|count| count.parse::<usize>().ok()) else {
            continue;
        };
        let Some(kind) = words.next() else {
            continue;
        };
        let Some(state) = words.next() else {
            continue;
        };
        match (kind, state) {
            ("containers", "found") => summary.containers_found = count,
            ("containers", "started") => summary.containers_started = count,
            ("containers", "successful") => summary.containers_successful = count,
            ("containers", "skipped") => summary.containers_skipped = count,
            ("containers", "aborted") => summary.containers_aborted = count,
            ("containers", "failed") => summary.containers_failed = count,
            ("tests", "found") => summary.tests_found = count,
            ("tests", "started") => summary.tests_started = count,
            ("tests", "successful") => summary.tests_successful = count,
            ("tests", "skipped") => summary.tests_skipped = count,
            ("tests", "aborted") => summary.tests_aborted = count,
            ("tests", "failed") => summary.tests_failed = count,
            _ => {}
        }
    }
    Some(summary)
}

fn parse_junit_duration(duration: &str) -> Option<u64> {
    let mut words = duration.split_whitespace();
    let value = words.next()?.parse::<f64>().ok()?;
    match words.next()? {
        "ms" => duration_millis_from_seconds(value / 1_000.0),
        "s" => duration_millis_from_seconds(value),
        _ => None,
    }
}

fn duration_millis_from_seconds(seconds: f64) -> Option<u64> {
    let duration = std::time::Duration::try_from_secs_f64(seconds).ok()?;
    ((duration.as_nanos() + 500_000) / 1_000_000)
        .try_into()
        .ok()
}

fn junit_selection(patterns: &[String]) -> Result<JunitSelection, BuildError> {
    if patterns.is_empty() {
        return Ok(JunitSelection::Scan);
    }
    let methods = patterns
        .iter()
        .filter(|pattern| pattern.contains('#'))
        .collect::<Vec<_>>();
    if !methods.is_empty() {
        if methods.len() != patterns.len() {
            return Err(BuildError::Invalid(
                "cannot mix class patterns and method selectors in one test run".to_owned(),
            ));
        }
        if let Some(pattern) = methods
            .iter()
            .find(|pattern| pattern.contains('*') || pattern.contains('?'))
        {
            return Err(BuildError::Invalid(format!(
                "method wildcards are not supported yet: `{pattern}`; use an exact Class#method selector"
            )));
        }
        return Ok(JunitSelection::Methods(patterns.to_vec()));
    }
    let patterns = patterns
        .iter()
        .map(|pattern| glob_class_pattern(pattern))
        .collect::<Vec<_>>()
        .join("|");
    Ok(JunitSelection::Classes(format!("^(?:{patterns})$")))
}

fn glob_class_pattern(pattern: &str) -> String {
    let mut regex = String::new();
    for character in pattern.chars() {
        match character {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '.' => regex.push_str("\\."),
            '\\' | '+' | '(' | ')' | '|' | '^' | '$' | '[' | ']' | '{' | '}' => {
                regex.push('\\');
                regex.push(character);
            }
            _ => regex.push(character),
        }
    }
    regex
}

fn ensure_junit_console(lock: &Lockfile) -> Result<(), BuildError> {
    if lock.packages.iter().any(|package| {
        package.group == "org.junit.platform"
            && matches!(
                package.artifact.as_str(),
                "junit-platform-console" | "junit-platform-console-standalone"
            )
    }) {
        Ok(())
    } else {
        Err(BuildError::Invalid(
            "JUnit Platform Console is missing; run `jman add \
             org.junit.platform:junit-platform-console-standalone@<version> --scope test`"
                .to_owned(),
        ))
    }
}

#[allow(clippy::too_many_lines)]
async fn compile_test_sources(
    module: &Module,
    toolchain: &Toolchain,
    sources: &[PathBuf],
    classpath: &[PathBuf],
    processors: &[PathBuf],
) -> Result<(PathBuf, bool), BuildError> {
    let state_dir = module.directory.join(".jman");
    let output = state_dir.join("output/test-classes");
    let state_file = state_dir.join("test.sha256");
    let mut hash = Sha256::new();
    hash.update(STATE_VERSION);
    hash.update(toolchain.version.as_bytes());
    for source in sources {
        hash.update(source.to_string_lossy().as_bytes());
        hash.update(
            fs::read(source)
                .await
                .map_err(|error| io_error(source, error))?,
        );
    }
    for entry in classpath.iter().chain(processors) {
        hash.update(entry.to_string_lossy().as_bytes());
        if entry.is_file() {
            hash.update(
                fs::metadata(entry)
                    .await
                    .map_err(|error| io_error(entry, error))?
                    .len()
                    .to_le_bytes(),
            );
        } else if entry.is_dir() {
            for file in input_files(entry, None).await? {
                hash.update(file.to_string_lossy().as_bytes());
                hash.update(
                    fs::read(&file)
                        .await
                        .map_err(|error| io_error(&file, error))?,
                );
            }
        }
    }
    let fingerprint = format!("sha256:{}", hex::encode(hash.finalize()));
    if fs::read_to_string(&state_file)
        .await
        .is_ok_and(|current| current.trim() == fingerprint)
        && output.is_dir()
    {
        return Ok((output, false));
    }
    let temporary = state_dir.join(format!("test-classes.tmp.{}", std::process::id()));
    if temporary.exists() {
        fs::remove_dir_all(&temporary)
            .await
            .map_err(|error| io_error(&temporary, error))?;
    }
    fs::create_dir_all(&temporary)
        .await
        .map_err(|error| io_error(&temporary, error))?;
    let generated = state_dir.join("output/generated/test-sources/annotations");
    if generated.exists() {
        fs::remove_dir_all(&generated)
            .await
            .map_err(|error| io_error(&generated, error))?;
    }
    fs::create_dir_all(&generated)
        .await
        .map_err(|error| io_error(&generated, error))?;
    let mut command = Command::new(&toolchain.javac);
    command
        .arg("-d")
        .arg(&temporary)
        .arg("-s")
        .arg(&generated)
        .arg("--release")
        .arg(module.manifest.project.java_release.to_string())
        .arg("-encoding")
        .arg(
            module
                .manifest
                .build
                .as_ref()
                .map_or("UTF-8", |build| build.encoding.as_str()),
        )
        .arg("-classpath")
        .arg(join_paths(classpath)?)
        .args(
            module
                .manifest
                .build
                .as_ref()
                .map_or(&[][..], |build| build.compiler_args.as_slice()),
        );
    if toolchain.major >= 23 && module.manifest.project.java_release < 23 {
        command.arg("-proc:full");
    }
    if !processors.is_empty() {
        command.arg("-processorpath").arg(join_paths(processors)?);
    }
    let result = command
        .args(sources)
        .output()
        .await
        .map_err(|source| BuildError::Javac {
            path: toolchain.javac.clone(),
            source,
        })?;
    if !result.status.success() {
        let _ = fs::remove_dir_all(&temporary).await;
        return Err(BuildError::Compilation {
            module: format!("{} tests", module.manifest.project.name),
            diagnostics: String::from_utf8_lossy(&result.stderr).trim().to_owned(),
        });
    }
    replace_directory(&temporary, &output).await?;
    fs::write(&state_file, format!("{fingerprint}\n"))
        .await
        .map_err(|error| io_error(&state_file, error))?;
    Ok((output, true))
}

fn transitive_dependencies(index: usize, modules: &[Module]) -> BTreeSet<usize> {
    let mut found = BTreeSet::new();
    let mut pending = modules[index].dependencies.clone();
    while let Some(dependency) = pending.pop() {
        if found.insert(dependency) {
            pending.extend(modules[dependency].dependencies.iter().copied());
        }
    }
    found
}

async fn discover_modules(root: &Path) -> Result<Vec<Module>, BuildError> {
    let root = canonicalize(root).await?;
    let mut pending = vec![root.clone()];
    let mut seen = BTreeSet::new();
    let mut raw = Vec::new();
    while let Some(directory) = pending.pop() {
        if !seen.insert(directory.clone()) {
            return Err(BuildError::Invalid(format!(
                "duplicate or cyclic module declaration at {}",
                directory.display()
            )));
        }
        let manifest = read_manifest(&directory.join("jman.toml"))?;
        for child in manifest.project.modules.iter().rev() {
            let child = canonicalize(&directory.join(child)).await?;
            if !child.starts_with(&root) {
                return Err(BuildError::Invalid(format!(
                    "module {} escapes workspace {}",
                    child.display(),
                    root.display()
                )));
            }
            pending.push(child);
        }
        raw.push((directory, manifest));
    }
    let indices = raw
        .iter()
        .enumerate()
        .map(|(index, (directory, _))| (directory.clone(), index))
        .collect::<BTreeMap<_, _>>();
    stream::iter(raw.into_iter().map(|(directory, manifest)| {
        let indices = &indices;
        async move {
            let mut dependencies = Vec::new();
            for relative in manifest.path_dependencies.values() {
                let dependency = canonicalize(&directory.join(relative)).await?;
                let index = indices.get(&dependency).ok_or_else(|| {
                    BuildError::Invalid(format!(
                        "path dependency {} from {} is not a declared workspace module",
                        dependency.display(),
                        directory.display()
                    ))
                })?;
                dependencies.push(*index);
            }
            dependencies.sort_unstable();
            dependencies.dedup();
            Ok(Module {
                directory,
                manifest,
                dependencies,
            })
        }
    }))
    .buffered(1)
    .try_collect()
    .await
}

fn topological_layers(modules: &[Module]) -> Result<Vec<Vec<usize>>, BuildError> {
    let mut remaining = (0..modules.len()).collect::<BTreeSet<_>>();
    let mut complete = BTreeSet::new();
    let mut layers = Vec::new();
    while !remaining.is_empty() {
        let layer = remaining
            .iter()
            .copied()
            .filter(|index| {
                modules[*index]
                    .dependencies
                    .iter()
                    .all(|dependency| complete.contains(dependency))
            })
            .collect::<Vec<_>>();
        if layer.is_empty() {
            let names = remaining
                .iter()
                .map(|index| modules[*index].manifest.project.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(BuildError::Invalid(format!(
                "module dependency cycle detected involving: {names}"
            )));
        }
        for index in &layer {
            remaining.remove(index);
            complete.insert(*index);
        }
        layers.push(layer);
    }
    Ok(layers)
}

#[allow(clippy::too_many_lines)]
async fn compile_module(
    module: &Module,
    toolchain: &Toolchain,
    cache_dir: &Path,
    upstream: &[(PathBuf, String)],
) -> Result<(ModuleResult, String), BuildError> {
    let sources = java_sources(&module.directory.join("src/main/java")).await?;
    let resources = input_files(&module.directory.join("src/main/resources"), None).await?;
    let lock = read_lock(&module.directory.join("jman.lock"))?;
    let classpath = artifact_paths(&lock.classpath.compile, &lock, cache_dir)?;
    let processors = artifact_paths(&lock.classpath.processors, &lock, cache_dir)?;
    let fingerprint = module_fingerprint(
        module,
        toolchain,
        &sources,
        &resources,
        &classpath,
        &processors,
        upstream,
    )
    .await?;
    let state_dir = module.directory.join(".jman");
    let output_root = state_dir.join("output");
    let output = output_root.join("classes");
    let generated_sources = output_root.join("generated/sources/annotations");
    let state_file = state_dir.join("check.sha256");
    if fs::read_to_string(&state_file)
        .await
        .is_ok_and(|current| current.trim() == fingerprint)
        && output.is_dir()
    {
        return Ok((
            ModuleResult {
                name: module.manifest.project.name.clone(),
                sources: sources.len(),
                rebuilt: false,
                output,
                generated_sources,
                directory: module.directory.clone(),
            },
            fingerprint,
        ));
    }
    let temporary_root = state_dir.join(format!("output.tmp.{}", std::process::id()));
    if temporary_root.exists() {
        fs::remove_dir_all(&temporary_root)
            .await
            .map_err(|source| io_error(&temporary_root, source))?;
    }
    let temporary_classes = temporary_root.join("classes");
    let temporary_generated = temporary_root.join("generated/sources/annotations");
    fs::create_dir_all(&temporary_classes)
        .await
        .map_err(|source| io_error(&temporary_classes, source))?;
    fs::create_dir_all(&temporary_generated)
        .await
        .map_err(|source| io_error(&temporary_generated, source))?;
    copy_resources(
        &module.directory.join("src/main/resources"),
        &resources,
        &temporary_classes,
    )
    .await?;
    if !sources.is_empty() {
        let mut full_classpath = upstream
            .iter()
            .map(|(directory, _)| directory.join(".jman/output/classes"))
            .collect::<Vec<_>>();
        full_classpath.extend(classpath);
        let mut command = Command::new(&toolchain.javac);
        command
            .arg("-d")
            .arg(&temporary_classes)
            .arg("-s")
            .arg(&temporary_generated)
            .arg("--release")
            .arg(module.manifest.project.java_release.to_string())
            .arg("-encoding")
            .arg(
                module
                    .manifest
                    .build
                    .as_ref()
                    .map_or("UTF-8", |build| build.encoding.as_str()),
            );
        // JDK 23 disabled implicit classpath processor discovery. Preserve the
        // compiler behavior expected by projects targeting older Java releases.
        if toolchain.major >= 23 && module.manifest.project.java_release < 23 {
            command.arg("-proc:full");
        }
        if !full_classpath.is_empty() {
            command.arg("-classpath").arg(join_paths(&full_classpath)?);
        }
        if !processors.is_empty() {
            command.arg("-processorpath").arg(join_paths(&processors)?);
        }
        if let Some(build) = &module.manifest.build {
            command.args(&build.compiler_args);
        }
        command.args(&sources);
        let result = command.output().await.map_err(|source| BuildError::Javac {
            path: toolchain.javac.clone(),
            source,
        })?;
        if !result.status.success() {
            let _ = fs::remove_dir_all(&temporary_root).await;
            let mut diagnostics = String::from_utf8_lossy(&result.stderr).trim().to_owned();
            if diagnostics.contains("annotation processor")
                && toolchain.major > module.manifest.project.java_release
            {
                let _ = write!(
                    diagnostics,
                    "\n\nhint: this project targets Java {}, but javac {} is active; \
                     older annotation processors may require a matching JDK (set JAVA_HOME)",
                    module.manifest.project.java_release, toolchain.major
                );
            }
            return Err(BuildError::Compilation {
                module: module.manifest.project.name.clone(),
                diagnostics,
            });
        }
    }
    replace_directory(&temporary_root, &output_root).await?;
    fs::write(&state_file, format!("{fingerprint}\n"))
        .await
        .map_err(|source| io_error(&state_file, source))?;
    Ok((
        ModuleResult {
            name: module.manifest.project.name.clone(),
            sources: sources.len(),
            rebuilt: true,
            output,
            generated_sources,
            directory: module.directory.clone(),
        },
        fingerprint,
    ))
}

async fn copy_resources(
    resource_root: &Path,
    resources: &[PathBuf],
    output: &Path,
) -> Result<(), BuildError> {
    for resource in resources {
        let relative = resource.strip_prefix(resource_root).map_err(|_| {
            BuildError::Invalid(format!(
                "resource {} is outside {}",
                resource.display(),
                resource_root.display()
            ))
        })?;
        let destination = output.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|source| io_error(parent, source))?;
        }
        fs::copy(resource, &destination)
            .await
            .map_err(|source| io_error(&destination, source))?;
    }
    Ok(())
}

fn package_filename(manifest: &Manifest, kind: &str) -> String {
    let base = format!("{}-{}", manifest.project.name, manifest.project.version);
    match kind {
        "thin" => format!("{base}.jar"),
        "sources" | "javadoc" | "fat" => format!("{base}-{kind}.jar"),
        _ => unreachable!("package kind is selected internally"),
    }
}

async fn package_cached(
    module: &ModuleResult,
    manifest: &Manifest,
    kind: &'static str,
    toolchain: &Toolchain,
    cache_dir: &Path,
    upstream: &[PathBuf],
) -> Result<PackageResult, BuildError> {
    let artifact_dir = module.directory.join(".jman/artifacts");
    fs::create_dir_all(&artifact_dir)
        .await
        .map_err(|source| io_error(&artifact_dir, source))?;
    let artifact = artifact_dir.join(package_filename(manifest, kind));
    let state = artifact.with_extension("jar.cache.json");
    let lock_path = artifact.with_extension("jar.lock");
    let lock = tokio::task::spawn_blocking(move || {
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| io_error(&lock_path, source))?;
        lock.lock_exclusive()
            .map_err(|source| io_error(&lock_path, source))?;
        Ok::<_, BuildError>(lock)
    })
    .await
    .map_err(|error| BuildError::Invalid(format!("package lock task failed: {error}")))??;
    let key = package_input_key(module, manifest, kind, toolchain, cache_dir, upstream).await?;
    if let Ok(contents) = fs::read(&state).await {
        if let Ok(record) = serde_json::from_slice::<PackageCacheRecord>(&contents) {
            if record.key == key {
                if let Ok((checksum, size)) = digest_file(&artifact).await {
                    if size == record.size && checksum == record.checksum {
                        drop(lock);
                        return Ok(PackageResult {
                            module: manifest.project.name.clone(),
                            kind,
                            artifact,
                            size: record.size,
                            checksum: record.checksum,
                            unchanged: true,
                            warnings: record.warnings,
                        });
                    }
                }
            }
        }
    }
    let result = match kind {
        "thin" => package_module(module, manifest).await?,
        "sources" => package_sources(module, manifest).await?,
        "javadoc" => package_javadoc(module, manifest, toolchain, cache_dir, upstream).await?,
        "fat" => package_fat(module, manifest, cache_dir, upstream).await?,
        _ => unreachable!("package kind is selected internally"),
    };
    // An input changed while packaging must never be recorded under its old key.
    if package_input_key(module, manifest, kind, toolchain, cache_dir, upstream).await? != key {
        drop(lock);
        return Ok(result);
    }
    let record = PackageCacheRecord {
        key,
        checksum: result.checksum.clone(),
        size: result.size,
        warnings: result.warnings.clone(),
    };
    let temporary = state.with_extension(format!(
        "cache.tmp.{}.{}",
        std::process::id(),
        PACKAGE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let contents = serde_json::to_vec(&record).map_err(|error| {
        BuildError::Invalid(format!("could not serialize package cache: {error}"))
    })?;
    fs::write(&temporary, contents)
        .await
        .map_err(|source| io_error(&temporary, source))?;
    replace_file(&temporary, &state).await?;
    drop(lock);
    Ok(result)
}

async fn package_input_key(
    module: &ModuleResult,
    manifest: &Manifest,
    kind: &str,
    toolchain: &Toolchain,
    cache_dir: &Path,
    upstream: &[PathBuf],
) -> Result<String, BuildError> {
    let mut hash = Sha256::new();
    hash_part(&mut hash, PACKAGE_STATE_VERSION.as_bytes());
    hash_part(&mut hash, env!("CARGO_PKG_VERSION").as_bytes());
    hash_part(&mut hash, kind.as_bytes());
    hash_part(&mut hash, package_filename(manifest, kind).as_bytes());
    hash_part(
        &mut hash,
        manifest
            .to_toml()
            .map_err(|error| BuildError::Config {
                path: module.directory.join("jman.toml"),
                message: error.to_string(),
            })?
            .as_bytes(),
    );
    match kind {
        "thin" => hash_tree(&mut hash, "classes", &module.output, None).await?,
        "sources" => {
            hash_tree(
                &mut hash,
                "sources",
                &module.directory.join("src/main/java"),
                Some("java"),
            )
            .await?;
            hash_tree(
                &mut hash,
                "generated",
                &module.generated_sources,
                Some("java"),
            )
            .await?;
        }
        "javadoc" => {
            hash_part(&mut hash, toolchain.version.as_bytes());
            hash_part(&mut hash, toolchain.javac.to_string_lossy().as_bytes());
            for name in [
                "LANG",
                "LANGUAGE",
                "LC_ALL",
                "LC_CTYPE",
                "LC_MESSAGES",
                "TZ",
                "JAVA_TOOL_OPTIONS",
                "JDK_JAVA_OPTIONS",
                "_JAVA_OPTIONS",
            ] {
                hash_part(&mut hash, name.as_bytes());
                hash_part(
                    &mut hash,
                    env::var_os(name)
                        .unwrap_or_default()
                        .to_string_lossy()
                        .as_bytes(),
                );
            }
            hash_tree(
                &mut hash,
                "sources",
                &module.directory.join("src/main/java"),
                None,
            )
            .await?;
            hash_tree(
                &mut hash,
                "generated",
                &module.generated_sources,
                Some("java"),
            )
            .await?;
            hash_classpath(&mut hash, module, cache_dir, upstream, true).await?;
        }
        "fat" => {
            hash_tree(&mut hash, "classes", &module.output, None).await?;
            hash_classpath(&mut hash, module, cache_dir, upstream, false).await?;
        }
        _ => unreachable!("package kind is selected internally"),
    }
    Ok(format!("sha256:{}", hex::encode(hash.finalize())))
}

fn hash_part(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

async fn hash_file(hash: &mut Sha256, path: &Path) -> Result<(), BuildError> {
    let mut file = fs::File::open(path)
        .await
        .map_err(|source| io_error(path, source))?;
    let length = file
        .metadata()
        .await
        .map_err(|source| io_error(path, source))?
        .len();
    hash.update(length.to_le_bytes());
    let mut count = 0;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|source| io_error(path, source))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        count += read as u64;
    }
    if count != length {
        return Err(BuildError::Invalid(format!(
            "package input changed while reading {}",
            path.display()
        )));
    }
    Ok(())
}

async fn digest_file(path: &Path) -> Result<(String, u64), BuildError> {
    let mut hash = Sha256::new();
    let mut file = fs::File::open(path)
        .await
        .map_err(|source| io_error(path, source))?;
    let mut size = 0;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|source| io_error(path, source))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((format!("sha256:{}", hex::encode(hash.finalize())), size))
}

async fn hash_tree(
    hash: &mut Sha256,
    label: &str,
    root: &Path,
    extension: Option<&str>,
) -> Result<(), BuildError> {
    hash_part(hash, label.as_bytes());
    let files = input_files(root, extension).await?;
    hash.update((files.len() as u64).to_le_bytes());
    for path in files {
        let relative = path.strip_prefix(root).map_err(|_| {
            BuildError::Invalid(format!("package input escaped {}", root.display()))
        })?;
        hash_part(hash, archive_name(relative)?.as_bytes());
        hash_file(hash, &path).await?;
    }
    Ok(())
}

async fn hash_classpath(
    hash: &mut Sha256,
    module: &ModuleResult,
    cache_dir: &Path,
    upstream: &[PathBuf],
    compile: bool,
) -> Result<(), BuildError> {
    let lock_path = module.directory.join("jman.lock");
    hash_file(hash, &lock_path).await?;
    for (index, output) in upstream.iter().enumerate() {
        hash_part(hash, &(index as u64).to_le_bytes());
        hash_tree(hash, "upstream", output, None).await?;
    }
    let lock = read_lock(&lock_path)?;
    let checksums = if compile {
        &lock.classpath.compile
    } else {
        &lock.classpath.runtime
    };
    for (index, dependency) in artifact_paths(checksums, &lock, cache_dir)?
        .iter()
        .enumerate()
    {
        hash_part(hash, &(index as u64).to_le_bytes());
        hash_part(hash, dependency.to_string_lossy().as_bytes());
        hash_file(hash, dependency).await?;
    }
    Ok(())
}

async fn package_module(
    module: &ModuleResult,
    manifest: &Manifest,
) -> Result<PackageResult, BuildError> {
    let files = input_files(&module.output, None).await?;
    let mut entries = Vec::with_capacity(files.len() + 1);
    entries.push((
        "META-INF/MANIFEST.MF".to_owned(),
        jar_manifest(manifest).into_bytes(),
    ));
    for file in files {
        let relative = file.strip_prefix(&module.output).map_err(|_| {
            BuildError::Invalid(format!(
                "package input {} is outside {}",
                file.display(),
                module.output.display()
            ))
        })?;
        let name = archive_name(relative)?;
        if name.eq_ignore_ascii_case("META-INF/MANIFEST.MF") {
            continue;
        }
        let bytes = fs::read(&file)
            .await
            .map_err(|source| io_error(&file, source))?;
        entries.push((name, bytes));
    }
    entries[1..].sort_by(|left, right| left.0.cmp(&right.0));
    finalize_artifact(
        module,
        manifest,
        "thin",
        format!("{}-{}.jar", manifest.project.name, manifest.project.version),
        entries,
    )
    .await
}

async fn package_sources(
    module: &ModuleResult,
    manifest: &Manifest,
) -> Result<PackageResult, BuildError> {
    let roots = [
        module.directory.join("src/main/java"),
        module.generated_sources.clone(),
    ];
    let mut archive = BTreeMap::new();
    for root in roots {
        for file in java_sources(&root).await? {
            let relative = file.strip_prefix(&root).map_err(|_| {
                BuildError::Invalid(format!(
                    "source {} is outside {}",
                    file.display(),
                    root.display()
                ))
            })?;
            insert_entry(
                &mut archive,
                archive_name(relative)?,
                fs::read(&file)
                    .await
                    .map_err(|source| io_error(&file, source))?,
                "source JAR",
            )?;
        }
    }
    let mut entries = vec![(
        "META-INF/MANIFEST.MF".to_owned(),
        jar_manifest(manifest).into_bytes(),
    )];
    entries.extend(archive);
    finalize_artifact(
        module,
        manifest,
        "sources",
        format!(
            "{}-{}-sources.jar",
            manifest.project.name, manifest.project.version
        ),
        entries,
    )
    .await
}

async fn package_javadoc(
    module: &ModuleResult,
    manifest: &Manifest,
    toolchain: &Toolchain,
    cache_dir: &Path,
    upstream: &[PathBuf],
) -> Result<PackageResult, BuildError> {
    let mut sources = java_sources(&module.directory.join("src/main/java")).await?;
    sources.extend(java_sources(&module.generated_sources).await?);
    sources.sort();
    sources.dedup();
    let temporary = module
        .directory
        .join(format!(".jman/javadoc.tmp.{}", std::process::id()));
    if temporary.exists() {
        fs::remove_dir_all(&temporary)
            .await
            .map_err(|source| io_error(&temporary, source))?;
    }
    fs::create_dir_all(&temporary)
        .await
        .map_err(|source| io_error(&temporary, source))?;
    if !sources.is_empty() {
        let executable = if cfg!(windows) {
            "javadoc.exe"
        } else {
            "javadoc"
        };
        let javadoc = toolchain.javac.parent().map_or_else(
            || PathBuf::from(executable),
            |parent| parent.join(executable),
        );
        let lock = read_lock(&module.directory.join("jman.lock"))?;
        let mut classpath = upstream.to_vec();
        classpath.extend(artifact_paths(&lock.classpath.compile, &lock, cache_dir)?);
        let mut command = Command::new(&javadoc);
        command
            .arg("-quiet")
            .arg("-notimestamp")
            .arg("-d")
            .arg(&temporary)
            .arg("--release")
            .arg(manifest.project.java_release.to_string())
            .arg("-encoding")
            .arg(
                manifest
                    .build
                    .as_ref()
                    .map_or("UTF-8", |build| build.encoding.as_str()),
            );
        if !classpath.is_empty() {
            command.arg("-classpath").arg(join_paths(&classpath)?);
        }
        command.args(&sources);
        let result = command.output().await.map_err(|source| BuildError::Javac {
            path: javadoc,
            source,
        })?;
        if !result.status.success() {
            let _ = fs::remove_dir_all(&temporary).await;
            return Err(BuildError::Compilation {
                module: format!("{} Javadoc", manifest.project.name),
                diagnostics: String::from_utf8_lossy(&result.stderr).trim().to_owned(),
            });
        }
    }
    let files = input_files(&temporary, None).await?;
    let mut entries = vec![(
        "META-INF/MANIFEST.MF".to_owned(),
        jar_manifest(manifest).into_bytes(),
    )];
    for file in files {
        let relative = file
            .strip_prefix(&temporary)
            .map_err(|_| BuildError::Invalid("Javadoc output escaped its directory".to_owned()))?;
        entries.push((
            archive_name(relative)?,
            fs::read(&file)
                .await
                .map_err(|source| io_error(&file, source))?,
        ));
    }
    let _ = fs::remove_dir_all(&temporary).await;
    finalize_artifact(
        module,
        manifest,
        "javadoc",
        format!(
            "{}-{}-javadoc.jar",
            manifest.project.name, manifest.project.version
        ),
        entries,
    )
    .await
}

async fn package_fat(
    module: &ModuleResult,
    manifest: &Manifest,
    cache_dir: &Path,
    upstream: &[PathBuf],
) -> Result<PackageResult, BuildError> {
    let mut archive = FatArchive::default();
    add_directory_entries(&mut archive, &module.output, "application output").await?;
    for output in upstream {
        add_directory_entries(&mut archive, output, "workspace dependency").await?;
    }
    let lock = read_lock(&module.directory.join("jman.lock"))?;
    for dependency in artifact_paths(&lock.classpath.runtime, &lock, cache_dir)? {
        let coordinate = lock
            .packages
            .iter()
            .find(|package| {
                package
                    .artifact_checksum
                    .as_deref()
                    .is_some_and(|checksum| {
                        checksum.strip_prefix("sha256:").is_some_and(|digest| {
                            dependency
                                .file_name()
                                .is_some_and(|name| name.to_string_lossy().starts_with(digest))
                        })
                    })
            })
            .map_or_else(
                || "unknown".to_owned(),
                |package| format!("{}:{}:{}", package.group, package.artifact, package.version),
            );
        merge_dependency_jar(&mut archive, dependency, coordinate).await?;
    }
    let warnings = archive.warnings;
    let mut entries = vec![(
        "META-INF/MANIFEST.MF".to_owned(),
        jar_manifest(manifest).into_bytes(),
    )];
    entries.extend(
        archive
            .entries
            .into_iter()
            .map(|(name, entry)| (name, entry.bytes)),
    );
    let mut result = finalize_artifact(
        module,
        manifest,
        "fat",
        format!(
            "{}-{}-fat.jar",
            manifest.project.name, manifest.project.version
        ),
        entries,
    )
    .await?;
    result.warnings = warnings;
    Ok(result)
}

async fn add_directory_entries(
    archive: &mut FatArchive,
    root: &Path,
    context: &str,
) -> Result<(), BuildError> {
    for file in input_files(root, None).await? {
        let relative = file.strip_prefix(root).map_err(|_| {
            BuildError::Invalid(format!(
                "input {} is outside {}",
                file.display(),
                root.display()
            ))
        })?;
        let name = archive_name(relative)?;
        if ignored_fat_entry(&name) {
            continue;
        }
        merge_fat_entry(
            archive,
            name,
            fs::read(&file)
                .await
                .map_err(|source| io_error(&file, source))?,
            context,
        )?;
    }
    Ok(())
}

async fn merge_dependency_jar(
    archive: &mut FatArchive,
    jar: PathBuf,
    coordinate: String,
) -> Result<(), BuildError> {
    let entries = tokio::task::spawn_blocking(move || read_jar_entries(&jar, &coordinate))
        .await
        .map_err(|error| BuildError::Invalid(format!("fat JAR merge task failed: {error}")))??;
    for (name, bytes, context) in entries {
        merge_fat_entry(archive, name, bytes, &context)?;
    }
    Ok(())
}

fn read_jar_entries(
    jar: &Path,
    coordinate: &str,
) -> Result<Vec<(String, Vec<u8>, String)>, BuildError> {
    use std::io::Read;

    let file = std::fs::File::open(jar).map_err(|source| io_error(jar, source))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| BuildError::Invalid(format!("invalid JAR {}: {error}", jar.display())))?;
    let mut entries = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| BuildError::Invalid(format!("invalid JAR entry: {error}")))?;
        if entry.is_dir() {
            continue;
        }
        let mut name = safe_zip_name(entry.name())?;
        if ignored_fat_entry(&name) {
            continue;
        }
        name = relocated_dependency_entry(&name, coordinate);
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|source| io_error(jar, source))?;
        entries.push((name, bytes, format!("dependency {coordinate}")));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(entries)
}

fn safe_zip_name(name: &str) -> Result<String, BuildError> {
    if name.is_empty() || name.starts_with('/') || name.contains('\\') {
        return Err(BuildError::Invalid(format!(
            "unsafe JAR entry path `{name}`"
        )));
    }
    let parts = name.split('/').collect::<Vec<_>>();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(BuildError::Invalid(format!(
            "unsafe JAR entry path `{name}`"
        )));
    }
    Ok(parts.join("/"))
}

fn ignored_fat_entry(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let signature = Path::new(&upper)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "SF" | "RSA" | "DSA" | "EC"));
    upper == "META-INF/MANIFEST.MF"
        || upper == "META-INF/INDEX.LIST"
        || upper == "MODULE-INFO.CLASS"
        || upper.ends_with("/MODULE-INFO.CLASS")
        || signature
}

fn is_license_entry(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.starts_with("META-INF/LICENSE")
        || upper.starts_with("META-INF/NOTICE")
        || upper.starts_with("META-INF/COPYING")
}

fn relocated_dependency_entry(name: &str, coordinate: &str) -> String {
    let coordinate = coordinate.replace(':', "/");
    let leaf = name.rsplit('/').next().unwrap_or("metadata");
    if is_license_entry(name) {
        format!("META-INF/jman/licenses/{coordinate}/{leaf}")
    } else {
        name.to_owned()
    }
}

fn resource_merge_strategy(name: &str) -> ResourceMergeStrategy {
    if name.starts_with("META-INF/services/")
        || (name.starts_with("META-INF/spring/")
            && (name.ends_with(".imports") || name.ends_with(".includes")))
    {
        ResourceMergeStrategy::Lines
    } else if matches!(
        name,
        "META-INF/spring.factories"
            | "META-INF/spring/aot.factories"
            | "META-INF/spring.components"
    ) {
        ResourceMergeStrategy::PropertyLists
    } else if matches!(
        name,
        "META-INF/io.netty.versions.properties"
            | "META-INF/spring.handlers"
            | "META-INF/spring.schemas"
            | "META-INF/spring.tooling"
            | "META-INF/spring-autoconfigure-metadata.properties"
    ) {
        ResourceMergeStrategy::Properties
    } else {
        ResourceMergeStrategy::KeepFirst
    }
}

fn merge_fat_entry(
    archive: &mut FatArchive,
    name: String,
    bytes: Vec<u8>,
    source: &str,
) -> Result<(), BuildError> {
    match resource_merge_strategy(&name) {
        ResourceMergeStrategy::KeepFirst => insert_fat_entry(archive, name, bytes, source),
        ResourceMergeStrategy::Lines => {
            merge_line_entry(archive, name, &bytes);
            Ok(())
        }
        ResourceMergeStrategy::Properties => merge_properties_entry(archive, name, &bytes, source),
        ResourceMergeStrategy::PropertyLists => {
            merge_property_list_entry(archive, name, &bytes, source)
        }
    }
}

fn merge_line_entry(archive: &mut FatArchive, name: String, bytes: &[u8]) {
    let existing = archive
        .entries
        .get(&name)
        .map(|entry| entry.bytes.as_slice())
        .unwrap_or_default();
    let lines = String::from_utf8_lossy(existing)
        .lines()
        .chain(String::from_utf8_lossy(bytes).lines())
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut merged = lines.into_iter().collect::<Vec<_>>().join("\n");
    if !merged.is_empty() {
        merged.push('\n');
    }
    archive.entries.insert(
        name,
        FatEntry {
            bytes: merged.into_bytes(),
            source: "merged line registry".to_owned(),
        },
    );
}

fn merge_properties_entry(
    archive: &mut FatArchive,
    name: String,
    bytes: &[u8],
    source: &str,
) -> Result<(), BuildError> {
    let mut properties = BTreeMap::new();
    if let Some(existing) = archive.entries.get(&name) {
        parse_properties(&mut properties, &existing.bytes, &existing.source, &name)?;
    }
    parse_properties(&mut properties, bytes, source, &name)?;
    let mut merged = String::from("# Merged by JMAN\n");
    for (key, value) in properties {
        merged.push_str(&key);
        merged.push('=');
        merged.push_str(&value);
        merged.push('\n');
    }
    archive.entries.insert(
        name,
        FatEntry {
            bytes: merged.into_bytes(),
            source: "merged properties".to_owned(),
        },
    );
    Ok(())
}

fn merge_property_list_entry(
    archive: &mut FatArchive,
    name: String,
    bytes: &[u8],
    source: &str,
) -> Result<(), BuildError> {
    let mut properties = BTreeMap::<String, BTreeSet<String>>::new();
    if let Some(existing) = archive.entries.get(&name) {
        parse_property_lists(&mut properties, &existing.bytes, &existing.source, &name)?;
    }
    parse_property_lists(&mut properties, bytes, source, &name)?;
    let mut merged = String::from("# Merged by JMAN\n");
    for (key, values) in properties {
        merged.push_str(&key);
        merged.push('=');
        merged.push_str(&values.into_iter().collect::<Vec<_>>().join(","));
        merged.push('\n');
    }
    archive.entries.insert(
        name,
        FatEntry {
            bytes: merged.into_bytes(),
            source: "merged property list registry".to_owned(),
        },
    );
    Ok(())
}

fn parse_property_lists(
    properties: &mut BTreeMap<String, BTreeSet<String>>,
    bytes: &[u8],
    source: &str,
    name: &str,
) -> Result<(), BuildError> {
    for (key, value) in property_entries(bytes, source, name)? {
        properties.entry(key).or_default().extend(
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
        );
    }
    Ok(())
}

fn parse_properties(
    properties: &mut BTreeMap<String, String>,
    bytes: &[u8],
    source: &str,
    name: &str,
) -> Result<(), BuildError> {
    for (key, value) in property_entries(bytes, source, name)? {
        match properties.get(&key) {
            Some(existing) if existing != &value => {
                return Err(BuildError::Invalid(format!(
                    "conflicting property `{key}` in `{name}` from {source}"
                )));
            }
            Some(_) => {}
            None => {
                properties.insert(key, value);
            }
        }
    }
    Ok(())
}

fn property_entries(
    bytes: &[u8],
    source: &str,
    name: &str,
) -> Result<Vec<(String, String)>, BuildError> {
    let text = String::from_utf8_lossy(bytes);
    let mut logical = String::new();
    let mut entries = Vec::new();
    for physical in text.lines() {
        if logical.is_empty() {
            logical.push_str(physical);
        } else {
            logical.push_str(physical.trim_start());
        }
        let trailing_slashes = logical
            .chars()
            .rev()
            .take_while(|character| *character == '\\')
            .count();
        if trailing_slashes % 2 == 1 {
            logical.pop();
            continue;
        }
        if let Some(entry) = parse_property_entry(&logical, source, name)? {
            entries.push(entry);
        }
        logical.clear();
    }
    if !logical.is_empty() {
        return Err(BuildError::Invalid(format!(
            "unterminated property continuation in `{name}` from {source}"
        )));
    }
    Ok(entries)
}

fn parse_property_entry(
    line: &str,
    source: &str,
    name: &str,
) -> Result<Option<(String, String)>, BuildError> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
        return Ok(None);
    }
    let mut escaped = false;
    let separator = line.char_indices().find(|(_, character)| {
        if escaped {
            escaped = false;
            false
        } else if *character == '\\' {
            escaped = true;
            false
        } else {
            *character == '=' || *character == ':' || character.is_whitespace()
        }
    });
    let Some((index, separator)) = separator else {
        return Err(BuildError::Invalid(format!(
            "unsupported property syntax in `{name}` from {source}: `{line}`"
        )));
    };
    let key = line[..index].trim_end();
    if key.is_empty() {
        return Err(BuildError::Invalid(format!(
            "empty property key in `{name}` from {source}"
        )));
    }
    let mut value = &line[index + separator.len_utf8()..];
    if separator.is_whitespace() {
        value = value.trim_start();
        if value.starts_with(['=', ':']) {
            value = &value[1..];
        }
    }
    Ok(Some((key.to_owned(), value.trim().to_owned())))
}

fn insert_fat_entry(
    archive: &mut FatArchive,
    name: String,
    bytes: Vec<u8>,
    source: &str,
) -> Result<(), BuildError> {
    if let Some(existing) = archive.entries.get(&name) {
        if existing.bytes == bytes {
            return Ok(());
        }
        if Path::new(&name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("class"))
        {
            return Err(BuildError::Invalid(format!(
                "conflicting class `{name}` from {} and {source}",
                existing.source
            )));
        }
        archive.warnings.push(format!(
            "`{name}`: kept {}, ignored {source}",
            existing.source
        ));
        return Ok(());
    }
    archive.entries.insert(
        name,
        FatEntry {
            bytes,
            source: source.to_owned(),
        },
    );
    Ok(())
}

fn insert_entry(
    archive: &mut BTreeMap<String, Vec<u8>>,
    name: String,
    bytes: Vec<u8>,
    context: &str,
) -> Result<(), BuildError> {
    if let Some(existing) = archive.get(&name) {
        if existing != &bytes {
            return Err(BuildError::Invalid(format!(
                "conflicting JAR entry `{name}` from {context}"
            )));
        }
    } else {
        archive.insert(name, bytes);
    }
    Ok(())
}

async fn finalize_artifact(
    module: &ModuleResult,
    manifest: &Manifest,
    kind: &'static str,
    filename: String,
    entries: Vec<(String, Vec<u8>)>,
) -> Result<PackageResult, BuildError> {
    let artifact_dir = module.directory.join(".jman/artifacts");
    fs::create_dir_all(&artifact_dir)
        .await
        .map_err(|source| io_error(&artifact_dir, source))?;
    let artifact = artifact_dir.join(filename);
    let temporary = artifact.with_extension(format!("jar.tmp-{}", std::process::id()));
    write_jar(temporary.clone(), entries).await?;
    let bytes = fs::read(&temporary)
        .await
        .map_err(|source| io_error(&temporary, source))?;
    let checksum = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    let unchanged = fs::read(&artifact)
        .await
        .is_ok_and(|existing| Sha256::digest(existing) == Sha256::digest(&bytes));
    if unchanged {
        fs::remove_file(&temporary)
            .await
            .map_err(|source| io_error(&temporary, source))?;
    } else {
        replace_file(&temporary, &artifact).await?;
    }
    Ok(PackageResult {
        module: manifest.project.name.clone(),
        kind,
        artifact,
        size: bytes.len() as u64,
        checksum,
        unchanged,
        warnings: Vec::new(),
    })
}

async fn write_jar(
    destination: PathBuf,
    entries: Vec<(String, Vec<u8>)>,
) -> Result<(), BuildError> {
    tokio::task::spawn_blocking(move || {
        let file =
            std::fs::File::create(&destination).map_err(|source| io_error(&destination, source))?;
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(0o644);
        let directory_options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(0o755);
        let mut directories = BTreeSet::new();
        for (name, bytes) in entries {
            let mut offset = 0;
            while let Some(relative) = name[offset..].find('/') {
                offset += relative + 1;
                let directory = name[..offset].to_owned();
                if directories.insert(directory.clone()) {
                    archive
                        .add_directory(&directory, directory_options)
                        .map_err(|error| {
                            BuildError::Invalid(format!(
                                "could not add directory `{directory}`: {error}"
                            ))
                        })?;
                }
            }
            archive
                .start_file(&name, options)
                .map_err(|error| BuildError::Invalid(format!("could not add `{name}`: {error}")))?;
            archive
                .write_all(&bytes)
                .map_err(|source| io_error(&destination, source))?;
        }
        archive
            .finish()
            .map_err(|error| BuildError::Invalid(format!("could not finish JAR: {error}")))?;
        Ok(())
    })
    .await
    .map_err(|error| BuildError::Invalid(format!("JAR packaging task failed: {error}")))?
}

fn jar_manifest(manifest: &Manifest) -> String {
    let mut output = String::new();
    append_manifest_header(&mut output, "Manifest-Version", "1.0");
    append_manifest_header(&mut output, "Created-By", "JMAN");
    if let Some(main_class) = &manifest.project.main_class {
        append_manifest_header(&mut output, "Main-Class", main_class);
    }
    output.push_str("\r\n");
    output
}

fn append_manifest_header(output: &mut String, name: &str, value: &str) {
    let prefix = format!("{name}: ");
    let mut line = prefix;
    for character in value.chars() {
        if line.len() + character.len_utf8() > 70 {
            output.push_str(&line);
            output.push_str("\r\n");
            line.clear();
            line.push(' ');
        }
        line.push(character);
    }
    output.push_str(&line);
    output.push_str("\r\n");
}

fn archive_name(path: &Path) -> Result<String, BuildError> {
    let mut parts = Vec::new();
    for component in path.components() {
        let std::path::Component::Normal(part) = component else {
            return Err(BuildError::Invalid(format!(
                "unsafe JAR entry path {}",
                path.display()
            )));
        };
        let part = part.to_str().ok_or_else(|| {
            BuildError::Invalid(format!("JAR entry path is not UTF-8: {}", path.display()))
        })?;
        if part.is_empty() || part.contains('\\') {
            return Err(BuildError::Invalid(format!(
                "unsafe JAR entry path {}",
                path.display()
            )));
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err(BuildError::Invalid("empty JAR entry path".to_owned()));
    }
    Ok(parts.join("/"))
}

async fn replace_file(temporary: &Path, output: &Path) -> Result<(), BuildError> {
    // On platforms that support replacement by rename, keep the published file
    // continuously available. The backup path remains the Windows fallback.
    if fs::rename(temporary, output).await.is_ok() {
        return Ok(());
    }
    let backup = output.with_extension(format!("old.{}", std::process::id()));
    if backup.exists() {
        fs::remove_file(&backup)
            .await
            .map_err(|source| io_error(&backup, source))?;
    }
    if output.exists() {
        fs::rename(output, &backup)
            .await
            .map_err(|source| io_error(output, source))?;
    }
    if let Err(source) = fs::rename(temporary, output).await {
        if backup.exists() {
            let _ = fs::rename(&backup, output).await;
        }
        return Err(io_error(output, source));
    }
    if backup.exists() {
        fs::remove_file(&backup)
            .await
            .map_err(|source| io_error(&backup, source))?;
    }
    Ok(())
}

async fn module_fingerprint(
    module: &Module,
    toolchain: &Toolchain,
    sources: &[PathBuf],
    resources: &[PathBuf],
    classpath: &[PathBuf],
    processors: &[PathBuf],
    upstream: &[(PathBuf, String)],
) -> Result<String, BuildError> {
    let mut hash = Sha256::new();
    hash.update(STATE_VERSION);
    hash.update(toolchain.version.as_bytes());
    hash.update(
        module
            .manifest
            .to_toml()
            .map_err(|error| BuildError::Config {
                path: module.directory.join("jman.toml"),
                message: error.to_string(),
            })?,
    );
    for path in sources.iter().chain(resources) {
        hash.update(path.to_string_lossy().as_bytes());
        hash.update(
            fs::read(path)
                .await
                .map_err(|source| io_error(path, source))?,
        );
    }
    for path in classpath.iter().chain(processors) {
        hash.update(path.to_string_lossy().as_bytes());
    }
    for (directory, fingerprint) in upstream {
        hash.update(directory.to_string_lossy().as_bytes());
        hash.update(fingerprint);
    }
    Ok(format!("sha256:{}", hex::encode(hash.finalize())))
}

fn artifact_paths(
    checksums: &[String],
    lock: &Lockfile,
    cache_dir: &Path,
) -> Result<Vec<PathBuf>, BuildError> {
    let mut packages_by_checksum = HashMap::with_capacity(lock.packages.len());
    for package in &lock.packages {
        if let Some(checksum) = package.artifact_checksum.as_deref() {
            packages_by_checksum.entry(checksum).or_insert(package);
        }
    }
    checksums
        .iter()
        .map(|checksum| {
            let package = packages_by_checksum.get(checksum.as_str()).ok_or_else(|| {
                BuildError::Invalid(format!(
                    "classpath checksum `{checksum}` has no package in jman.lock; run `jman sync`"
                ))
            })?;
            let digest = checksum.strip_prefix("sha256:").ok_or_else(|| {
                BuildError::Invalid(format!("invalid classpath checksum `{checksum}`"))
            })?;
            let path = cache_dir
                .join("artifacts")
                .join(format!("{digest}.{}", package.extension));
            if !path.is_file() {
                return Err(BuildError::Invalid(format!(
                    "cached artifact is missing at {}; run `jman sync`",
                    path.display()
                )));
            }
            Ok(path)
        })
        .collect()
}

async fn java_sources(root: &Path) -> Result<Vec<PathBuf>, BuildError> {
    input_files(root, Some("java")).await
}

async fn input_files(root: &Path, extension: Option<&str>) -> Result<Vec<PathBuf>, BuildError> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut pending = vec![root.to_owned()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(&directory)
            .await
            .map_err(|source| io_error(&directory, source))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|source| io_error(&directory, source))?
        {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .await
                .map_err(|source| io_error(&path, source))?;
            if file_type.is_symlink() {
                return Err(BuildError::Invalid(format!(
                    "symbolic links are not allowed in build inputs: {}",
                    path.display()
                )));
            }
            if file_type.is_dir() {
                pending.push(path);
            } else if extension.is_none_or(|expected| {
                path.extension()
                    .is_some_and(|extension| extension == expected)
            }) {
                sources.push(path);
            }
        }
    }
    sources.sort();
    Ok(sources)
}

async fn replace_directory(temporary: &Path, output: &Path) -> Result<(), BuildError> {
    let backup = output.with_extension(format!("old.{}", std::process::id()));
    if backup.exists() {
        fs::remove_dir_all(&backup)
            .await
            .map_err(|source| io_error(&backup, source))?;
    }
    if output.exists() {
        fs::rename(output, &backup)
            .await
            .map_err(|source| io_error(output, source))?;
    }
    if let Err(source) = fs::rename(temporary, output).await {
        if backup.exists() {
            let _ = fs::rename(&backup, output).await;
        }
        return Err(io_error(output, source));
    }
    if backup.exists() {
        fs::remove_dir_all(&backup)
            .await
            .map_err(|source| io_error(&backup, source))?;
    }
    Ok(())
}

fn join_paths(paths: &[PathBuf]) -> Result<OsString, BuildError> {
    env::join_paths(paths).map_err(|error| {
        BuildError::Invalid(format!("could not construct Java classpath: {error}"))
    })
}

fn read_manifest(path: &Path) -> Result<Manifest, BuildError> {
    Manifest::read(path).map_err(|error| BuildError::Config {
        path: path.to_owned(),
        message: error.to_string(),
    })
}

fn read_lock(path: &Path) -> Result<Lockfile, BuildError> {
    Lockfile::read(path).map_err(|error| BuildError::Config {
        path: path.to_owned(),
        message: format!("{error}; run `jman sync`"),
    })
}

async fn canonicalize(path: &Path) -> Result<PathBuf, BuildError> {
    fs::canonicalize(path)
        .await
        .map_err(|source| io_error(path, source))
}

fn io_error(path: &Path, source: std::io::Error) -> BuildError {
    BuildError::Io {
        path: path.to_owned(),
        source,
    }
}

fn parse_javac_major(output: &str) -> Option<u16> {
    let version = output.split_whitespace().nth(1)?;
    let first = version.split('.').next()?.parse().ok()?;
    if first == 1 {
        version.split('.').nth(1)?.parse().ok()
    } else {
        Some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package_fixture(root: &Path) -> (Manifest, ModuleResult, Toolchain) {
        let manifest = Manifest {
            manifest_version: 1,
            project: jman_config::Project {
                group: "example".to_owned(),
                name: "app".to_owned(),
                version: "1".to_owned(),
                java_release: 21,
                packaging: "jar".to_owned(),
                modules: Vec::new(),
                main_class: Some("example.App".to_owned()),
            },
            toolchain: None,
            maven: None,
            build: None,
            test: None,
            publishing: None,
            audit: None,
            repositories: Vec::new(),
            dependencies: jman_config::Dependencies::default(),
            annotation_processors: BTreeMap::new(),
            path_dependencies: BTreeMap::new(),
        };
        let module = ModuleResult {
            name: "app".to_owned(),
            directory: root.to_owned(),
            sources: 1,
            rebuilt: false,
            output: root.join(".jman/output/classes"),
            generated_sources: root.join(".jman/output/generated/sources/annotations"),
        };
        let toolchain = Toolchain {
            javac: PathBuf::from("/test/jdk/bin/javac"),
            version: "21.0.1".to_owned(),
            major: 21,
            managed: None,
        };
        (manifest, module, toolchain)
    }

    fn empty_package_lock(root: &Path) {
        let lock = Lockfile {
            lock_version: jman_config::LOCK_VERSION,
            manifest_hash: "sha256:manifest".to_owned(),
            workspace_hash: "sha256:workspace".to_owned(),
            platform: "test".to_owned(),
            toolchain: jman_config::LockedToolchain { java_release: 21 },
            packages: Vec::new(),
            classpath: jman_config::LockedClasspaths::default(),
        };
        std::fs::write(root.join("jman.lock"), lock.to_toml().expect("lock TOML"))
            .expect("lockfile");
    }

    #[tokio::test]
    async fn thin_package_cache_reuses_verified_output_and_repairs_damage() {
        let directory = tempfile::tempdir().expect("package fixture");
        let (mut manifest, module, toolchain) = package_fixture(directory.path());
        std::fs::create_dir_all(&module.output).expect("classes");
        let class = module.output.join("App.class");
        std::fs::write(&class, b"first").expect("class");
        let build = || async {
            package_cached(
                &module,
                &manifest,
                "thin",
                &toolchain,
                directory.path(),
                &[],
            )
            .await
        };
        let first = build().await.expect("first package");
        assert!(!first.unchanged);
        let state = first.artifact.with_extension("jar.cache.json");
        let mut record = serde_json::from_slice::<PackageCacheRecord>(
            &std::fs::read(&state).expect("package state"),
        )
        .expect("valid package state");
        record.warnings.push("cache-hit-marker".to_owned());
        std::fs::write(&state, serde_json::to_vec(&record).expect("state JSON"))
            .expect("cache state marker");
        let second = build().await.expect("cached package");
        assert!(second.unchanged);
        assert_eq!(first.checksum, second.checksum);
        assert_eq!(second.warnings, ["cache-hit-marker"]);

        std::fs::write(&first.artifact, b"damage").expect("damage artifact");
        let repaired = build().await.expect("repaired package");
        assert_eq!(repaired.checksum, first.checksum);
        assert_ne!(
            std::fs::read(&repaired.artifact).expect("repaired bytes"),
            b"damage"
        );

        std::fs::write(&state, b"incomplete state").expect("interrupt cache publication");
        let recovered = build().await.expect("recovered package state");
        assert_eq!(recovered.checksum, first.checksum);
        assert!(serde_json::from_slice::<PackageCacheRecord>(
            &std::fs::read(&state).expect("recovered state")
        )
        .is_ok());

        std::fs::write(&class, b"other").expect("changed class");
        let changed = build().await.expect("changed package");
        assert_ne!(changed.checksum, first.checksum);
        manifest.project.main_class = None;
        let changed_manifest = package_cached(
            &module,
            &manifest,
            "thin",
            &toolchain,
            directory.path(),
            &[],
        )
        .await
        .expect("changed manifest");
        assert_ne!(changed_manifest.checksum, changed.checksum);
    }

    #[tokio::test]
    async fn sources_and_fat_package_keys_track_generated_and_upstream_outputs() {
        let directory = tempfile::tempdir().expect("package fixture");
        let (manifest, module, toolchain) = package_fixture(directory.path());
        std::fs::create_dir_all(&module.output).expect("classes");
        std::fs::write(module.output.join("App.class"), b"app").expect("app class");
        let source = directory.path().join("src/main/java/App.java");
        std::fs::create_dir_all(source.parent().expect("source parent")).expect("source directory");
        std::fs::write(&source, b"class App {}").expect("source");
        let generated = module.generated_sources.join("Generated.java");
        std::fs::create_dir_all(&module.generated_sources).expect("generated directory");
        std::fs::write(&generated, b"class Generated {}").expect("generated source");
        let first_sources = package_cached(
            &module,
            &manifest,
            "sources",
            &toolchain,
            directory.path(),
            &[],
        )
        .await
        .expect("sources package");
        let cached_sources = package_cached(
            &module,
            &manifest,
            "sources",
            &toolchain,
            directory.path(),
            &[],
        )
        .await
        .expect("cached sources");
        assert!(cached_sources.unchanged);
        std::fs::write(&generated, b"class Generated { int x; }").expect("generated change");
        let changed_sources = package_cached(
            &module,
            &manifest,
            "sources",
            &toolchain,
            directory.path(),
            &[],
        )
        .await
        .expect("changed sources");
        assert_ne!(first_sources.checksum, changed_sources.checksum);

        empty_package_lock(directory.path());
        let upstream = directory.path().join("dependency/classes");
        std::fs::create_dir_all(&upstream).expect("upstream directory");
        let upstream_class = upstream.join("Dependency.class");
        std::fs::write(&upstream_class, b"one").expect("upstream class");
        let first_fat = package_cached(
            &module,
            &manifest,
            "fat",
            &toolchain,
            directory.path(),
            std::slice::from_ref(&upstream),
        )
        .await
        .expect("fat package");
        let cached_fat = package_cached(
            &module,
            &manifest,
            "fat",
            &toolchain,
            directory.path(),
            std::slice::from_ref(&upstream),
        )
        .await
        .expect("cached fat package");
        assert!(cached_fat.unchanged);
        std::fs::write(&upstream_class, b"two").expect("upstream change");
        let changed_fat = package_cached(
            &module,
            &manifest,
            "fat",
            &toolchain,
            directory.path(),
            &[upstream],
        )
        .await
        .expect("changed fat package");
        assert_ne!(first_fat.checksum, changed_fat.checksum);
    }

    #[tokio::test]
    async fn package_keys_track_ordered_runtime_jars_and_javadoc_toolchain() {
        let directory = tempfile::tempdir().expect("package fixture");
        let (manifest, module, mut toolchain) = package_fixture(directory.path());
        std::fs::create_dir_all(&module.output).expect("classes");
        std::fs::write(module.output.join("App.class"), b"app").expect("class");
        let source = directory.path().join("src/main/java/App.java");
        std::fs::create_dir_all(source.parent().expect("source parent")).expect("source directory");
        std::fs::write(&source, b"class App {}").expect("source");
        let artifacts = directory.path().join("cache/artifacts");
        std::fs::create_dir_all(&artifacts).expect("artifact cache");
        let mut packages = Vec::new();
        let mut checksums = Vec::new();
        for (name, bytes) in [("one", b"one".as_slice()), ("two", b"two")] {
            let checksum = format!("sha256:{}", hex::encode(Sha256::digest(bytes)));
            let path = artifacts.join(format!("{}.jar", checksum.trim_start_matches("sha256:")));
            std::fs::write(path, bytes).expect("runtime JAR bytes");
            packages.push(jman_config::LockedPackage {
                group: "example".to_owned(),
                artifact: name.to_owned(),
                version: "1".to_owned(),
                extension: "jar".to_owned(),
                classifier: None,
                source: "test".to_owned(),
                pom_checksum: "sha256:pom".to_owned(),
                artifact_checksum: Some(checksum.clone()),
                artifact_size: Some(bytes.len() as u64),
                dependencies: Vec::new(),
                scopes: Vec::new(),
                selected_parent: None,
            });
            checksums.push(checksum);
        }
        let mut lock = Lockfile {
            lock_version: jman_config::LOCK_VERSION,
            manifest_hash: "sha256:manifest".to_owned(),
            workspace_hash: "sha256:workspace".to_owned(),
            platform: "test".to_owned(),
            toolchain: jman_config::LockedToolchain { java_release: 21 },
            packages,
            classpath: jman_config::LockedClasspaths {
                compile: checksums.clone(),
                runtime: checksums,
                ..jman_config::LockedClasspaths::default()
            },
        };
        let lock_path = directory.path().join("jman.lock");
        std::fs::write(&lock_path, lock.to_toml().expect("lock TOML")).expect("lockfile");
        let cache = directory.path().join("cache");
        let first = package_input_key(&module, &manifest, "fat", &toolchain, &cache, &[])
            .await
            .expect("fat key");
        lock.classpath.runtime.reverse();
        std::fs::write(&lock_path, lock.to_toml().expect("reordered lock TOML"))
            .expect("reordered lockfile");
        let reordered = package_input_key(&module, &manifest, "fat", &toolchain, &cache, &[])
            .await
            .expect("reordered fat key");
        assert_ne!(first, reordered);
        let javadoc = package_input_key(&module, &manifest, "javadoc", &toolchain, &cache, &[])
            .await
            .expect("Javadoc key");
        let doc_file = directory
            .path()
            .join("src/main/java/doc-files/overview.html");
        std::fs::create_dir_all(doc_file.parent().expect("doc-files parent"))
            .expect("doc-files directory");
        std::fs::write(&doc_file, b"documentation").expect("doc-file");
        let changed_docs =
            package_input_key(&module, &manifest, "javadoc", &toolchain, &cache, &[])
                .await
                .expect("changed Javadoc key");
        assert_ne!(javadoc, changed_docs);
        toolchain.version = "21.0.2".to_owned();
        assert_ne!(
            changed_docs,
            package_input_key(&module, &manifest, "javadoc", &toolchain, &cache, &[])
                .await
                .expect("new toolchain key")
        );
    }

    #[tokio::test]
    async fn concurrent_package_requests_publish_one_valid_artifact() {
        let directory = tempfile::tempdir().expect("package fixture");
        let (manifest, module, toolchain) = package_fixture(directory.path());
        std::fs::create_dir_all(&module.output).expect("classes");
        std::fs::write(module.output.join("App.class"), b"app").expect("class");
        let (first, second) = tokio::join!(
            package_cached(
                &module,
                &manifest,
                "thin",
                &toolchain,
                directory.path(),
                &[]
            ),
            package_cached(
                &module,
                &manifest,
                "thin",
                &toolchain,
                directory.path(),
                &[]
            ),
        );
        let first = first.expect("first concurrent package");
        let second = second.expect("second concurrent package");
        assert_eq!(first.checksum, second.checksum);
        assert!(first.unchanged || second.unchanged);
        assert_eq!(
            digest_file(&first.artifact).await.expect("valid JAR").0,
            first.checksum
        );
    }

    #[test]
    fn classpath_uses_locked_artifacts_in_order_and_reports_missing_entries() {
        let cache = tempfile::tempdir().expect("artifact cache");
        let artifacts = cache.path().join("artifacts");
        std::fs::create_dir(&artifacts).expect("artifact directory");
        let package = |name: &str, checksum: &str| jman_config::LockedPackage {
            group: "example".to_owned(),
            artifact: name.to_owned(),
            version: "1".to_owned(),
            extension: "jar".to_owned(),
            classifier: None,
            source: "test".to_owned(),
            pom_checksum: "sha256:pom".to_owned(),
            artifact_checksum: Some(format!("sha256:{checksum}")),
            artifact_size: Some(1),
            dependencies: Vec::new(),
            scopes: Vec::new(),
            selected_parent: None,
        };
        let lock = Lockfile {
            lock_version: jman_config::LOCK_VERSION,
            manifest_hash: String::new(),
            workspace_hash: String::new(),
            platform: String::new(),
            toolchain: jman_config::LockedToolchain { java_release: 21 },
            packages: vec![package("first", "aaa"), package("second", "bbb")],
            classpath: jman_config::LockedClasspaths::default(),
        };
        std::fs::write(artifacts.join("aaa.jar"), b"a").expect("first artifact");
        std::fs::write(artifacts.join("bbb.jar"), b"b").expect("second artifact");
        let paths = artifact_paths(
            &["sha256:bbb".to_owned(), "sha256:aaa".to_owned()],
            &lock,
            cache.path(),
        )
        .expect("locked classpath");
        assert_eq!(
            paths,
            vec![artifacts.join("bbb.jar"), artifacts.join("aaa.jar")]
        );
        assert!(
            artifact_paths(&["sha256:missing".to_owned()], &lock, cache.path())
                .expect_err("missing lock entry")
                .to_string()
                .contains("has no package in jman.lock")
        );
    }

    #[test]
    fn parses_modern_and_legacy_javac_versions() {
        assert_eq!(parse_javac_major("javac 21.0.2"), Some(21));
        assert_eq!(parse_javac_major("javac 1.8.0_402"), Some(8));
        assert_eq!(parse_javac_major("unexpected"), None);
    }

    #[test]
    fn detects_dependency_cycle() {
        let manifest = |name: &str| Manifest {
            manifest_version: 1,
            project: jman_config::Project {
                group: "com.example".to_owned(),
                name: name.to_owned(),
                version: "1".to_owned(),
                java_release: 17,
                packaging: "jar".to_owned(),
                modules: Vec::new(),
                main_class: None,
            },
            toolchain: None,
            maven: None,
            build: None,
            test: None,
            publishing: None,
            audit: None,
            repositories: Vec::new(),
            dependencies: jman_config::Dependencies::default(),
            annotation_processors: BTreeMap::new(),
            path_dependencies: BTreeMap::new(),
        };
        let modules = vec![
            Module {
                directory: PathBuf::from("a"),
                manifest: manifest("a"),
                dependencies: vec![1],
            },
            Module {
                directory: PathBuf::from("b"),
                manifest: manifest("b"),
                dependencies: vec![0],
            },
        ];
        assert!(topological_layers(&modules)
            .expect_err("cycle")
            .to_string()
            .contains("a, b"));
    }

    #[test]
    fn fat_jar_metadata_rules_are_safe_and_deterministic() {
        assert!(ignored_fat_entry("META-INF/MANIFEST.MF"));
        assert!(ignored_fat_entry("META-INF/versions/9/module-info.class"));
        assert!(ignored_fat_entry("META-INF/SIGNATURE.RSA"));
        assert!(!ignored_fat_entry("com/example/App.class"));
        assert!(is_license_entry("META-INF/LICENSE.txt"));
        assert_eq!(
            relocated_dependency_entry(
                "META-INF/LICENSE.txt",
                "io.micronaut:micronaut-http-server"
            ),
            "META-INF/jman/licenses/io.micronaut/micronaut-http-server/LICENSE.txt"
        );
        assert_eq!(
            relocated_dependency_entry(
                "META-INF/config-properties.adoc",
                "io.micronaut:micronaut-http-server"
            ),
            "META-INF/config-properties.adoc"
        );
        assert!(safe_zip_name("../escape.class").is_err());
        assert!(safe_zip_name("/absolute.class").is_err());

        let mut archive = FatArchive::default();
        merge_fat_entry(
            &mut archive,
            "META-INF/services/example.Service".to_owned(),
            b"example.B\nexample.A\n".to_vec(),
            "first dependency",
        )
        .expect("first service registry");
        merge_fat_entry(
            &mut archive,
            "META-INF/services/example.Service".to_owned(),
            b"# provider comment\nexample.A\nexample.C\n".to_vec(),
            "second dependency",
        )
        .expect("second service registry");
        assert_eq!(
            archive.entries["META-INF/services/example.Service"].bytes,
            b"example.A\nexample.B\nexample.C\n"
        );
        insert_fat_entry(
            &mut archive,
            "same.txt".to_owned(),
            vec![1],
            "first dependency",
        )
        .expect("first entry");
        insert_fat_entry(
            &mut archive,
            "same.txt".to_owned(),
            vec![1],
            "second dependency",
        )
        .expect("identical duplicate");
        insert_fat_entry(
            &mut archive,
            "same.txt".to_owned(),
            vec![2],
            "conflicting dependency",
        )
        .expect("unknown resources keep first");
        assert_eq!(archive.entries["same.txt"].bytes, vec![1]);
        assert_eq!(archive.warnings.len(), 1);
        assert!(archive.warnings[0].contains("first dependency"));
        assert!(archive.warnings[0].contains("conflicting dependency"));
        insert_fat_entry(
            &mut archive,
            "same.class".to_owned(),
            vec![1],
            "first class",
        )
        .expect("first class");
        assert!(insert_fat_entry(
            &mut archive,
            "same.class".to_owned(),
            vec![2],
            "conflicting class",
        )
        .is_err());

        merge_properties_entry(
            &mut archive,
            "META-INF/io.netty.versions.properties".to_owned(),
            b"# generated\nnetty-codec.version=4.1\n",
            "netty-codec",
        )
        .expect("first properties");
        merge_properties_entry(
            &mut archive,
            "META-INF/io.netty.versions.properties".to_owned(),
            b"netty-buffer.version=4.1\n",
            "netty-buffer",
        )
        .expect("merged properties");
        assert_eq!(
            archive.entries["META-INF/io.netty.versions.properties"].bytes,
            b"# Merged by JMAN\nnetty-buffer.version=4.1\nnetty-codec.version=4.1\n"
        );
    }

    #[test]
    fn fat_jar_merges_line_property_and_property_list_registries() {
        let mut archive = FatArchive::default();
        let imports =
            "META-INF/spring/org.springframework.boot.autoconfigure.AutoConfiguration.imports";
        merge_fat_entry(
            &mut archive,
            imports.to_owned(),
            b"# first dependency\nexample.JpaAutoConfiguration\nexample.WebAutoConfiguration\n"
                .to_vec(),
            "spring-data-jpa",
        )
        .expect("first line registry");
        merge_fat_entry(
            &mut archive,
            imports.to_owned(),
            b"example.DevToolsAutoConfiguration\nexample.WebAutoConfiguration\n".to_vec(),
            "spring-boot-devtools",
        )
        .expect("second line registry");
        assert_eq!(
            archive.entries[imports].bytes,
            b"example.DevToolsAutoConfiguration\nexample.JpaAutoConfiguration\nexample.WebAutoConfiguration\n"
        );

        merge_fat_entry(
            &mut archive,
            "META-INF/spring.factories".to_owned(),
            b"example.Factory=\\\n example.Second,\\\n example.First\n".to_vec(),
            "first factory dependency",
        )
        .expect("first property list registry");
        merge_fat_entry(
            &mut archive,
            "META-INF/spring.factories".to_owned(),
            b"example.Factory=example.First,example.Third\nexample.Other=example.Value\n".to_vec(),
            "second factory dependency",
        )
        .expect("second property list registry");
        assert_eq!(
            archive.entries["META-INF/spring.factories"].bytes,
            b"# Merged by JMAN\nexample.Factory=example.First,example.Second,example.Third\nexample.Other=example.Value\n"
        );

        merge_fat_entry(
            &mut archive,
            "META-INF/spring.handlers".to_owned(),
            b"http\\://example.org/schema/one=example.OneHandler\n".to_vec(),
            "first handler dependency",
        )
        .expect("first property registry");
        merge_fat_entry(
            &mut archive,
            "META-INF/spring.handlers".to_owned(),
            b"http\\://example.org/schema/two=example.TwoHandler\n".to_vec(),
            "second handler dependency",
        )
        .expect("second property registry");
        assert_eq!(
            archive.entries["META-INF/spring.handlers"].bytes,
            b"# Merged by JMAN\nhttp\\://example.org/schema/one=example.OneHandler\nhttp\\://example.org/schema/two=example.TwoHandler\n"
        );
    }

    #[test]
    fn jar_paths_and_manifest_lines_are_safe() {
        assert_eq!(
            archive_name(Path::new("com/example/App.class")).expect("entry"),
            "com/example/App.class"
        );
        assert!(archive_name(Path::new("../secret")).is_err());
        let mut manifest = String::new();
        append_manifest_header(&mut manifest, "Main-Class", &"a".repeat(180));
        assert!(manifest
            .split_inclusive("\r\n")
            .all(|line| line.len() <= 72));
        assert!(manifest.lines().skip(1).all(|line| line.starts_with(' ')));
    }

    #[test]
    fn translates_typed_test_selectors_without_broadening_the_run() {
        assert_eq!(junit_selection(&[]).expect("scan"), JunitSelection::Scan);
        assert_eq!(
            junit_selection(&["com.example.*Test".to_owned(), "*Spec".to_owned()])
                .expect("class globs"),
            JunitSelection::Classes("^(?:com\\.example\\..*Test|.*Spec)$".to_owned())
        );
        assert_eq!(
            junit_selection(&[
                "com.example.OrderTest#createsOrder".to_owned(),
                "com.example.OrderTest#rejectsInvalidOrder".to_owned(),
            ])
            .expect("methods"),
            JunitSelection::Methods(vec![
                "com.example.OrderTest#createsOrder".to_owned(),
                "com.example.OrderTest#rejectsInvalidOrder".to_owned(),
            ])
        );
        assert!(junit_selection(&[
            "com.example.*Test".to_owned(),
            "com.example.OrderTest#createsOrder".to_owned(),
        ])
        .is_err());
        assert!(junit_selection(&["com.example.OrderTest#create*".to_owned()]).is_err());
    }

    #[test]
    fn parses_junit_summary_without_exposing_its_table() {
        let summary = parse_junit_summary(
            r"
Test run finished after 758 ms
[         4 containers found      ]
[         0 containers skipped    ]
[         4 containers started    ]
[         0 containers aborted    ]
[         4 containers successful ]
[         0 containers failed     ]
[         1 tests found           ]
[         0 tests skipped         ]
[         1 tests started         ]
[         0 tests aborted         ]
[         1 tests successful      ]
[         0 tests failed          ]
",
        )
        .expect("summary");
        assert_eq!(
            summary,
            TestSummary {
                duration_millis: 758,
                containers_found: 4,
                containers_started: 4,
                containers_successful: 4,
                tests_found: 1,
                tests_started: 1,
                tests_successful: 1,
                ..TestSummary::default()
            }
        );
        assert_eq!(parse_junit_duration("1.25 s"), Some(1_250));
        assert!(parse_junit_summary("unrelated output").is_none());
    }

    #[test]
    fn parses_individual_junit_xml_results_for_editor_reporting() {
        let directory = tempfile::tempdir().expect("directory");
        std::fs::write(
            directory.path().join("TEST-junit-jupiter.xml"),
            r#"<testsuite>
              <testcase name="passes()" classname="com.example.GreetingTest" time="0.012"/>
              <testcase name="fails()" classname="com.example.GreetingTest" time="0.034">
                <failure message="expected: &lt;Ada&gt; but was: &lt;Bob&gt;">stack trace</failure>
              </testcase>
              <testcase name="disabled()" classname="com.example.GreetingTest" time="0">
                <skipped/>
              </testcase>
              <testcase name="greets(String)[1]" classname="com.example.ParameterTest" time="0.005"/>
            </testsuite>"#,
        )
        .expect("report");

        let cases = parse_junit_reports(directory.path()).expect("parse reports");
        assert_eq!(cases.len(), 4);
        assert_eq!(cases[1].selector, "com.example.GreetingTest#fails");
        assert_eq!(cases[1].status, TestCaseStatus::Failed);
        assert_eq!(cases[1].duration_millis, 34);
        assert_eq!(
            cases[1].message.as_deref(),
            Some("expected: <Ada> but was: <Bob>")
        );
        assert_eq!(cases[0].status, TestCaseStatus::Skipped);
        assert_eq!(cases[2].status, TestCaseStatus::Passed);
        assert_eq!(cases[3].selector, "com.example.ParameterTest#greets");
        assert_eq!(cases[3].display_name, "greets(String)[1]");
        assert_eq!(cases[3].invocation.as_deref(), Some("greets(String)[1]"));
        assert_eq!(cases[3].attempt, 1);
    }

    #[tokio::test]
    async fn streams_framed_test_events_and_preserves_ordinary_output() {
        use tokio::io::AsyncWriteExt as _;

        let (mut writer, reader) = tokio::io::duplex(64);
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let read = tokio::spawn(read_test_stdout(
            reader,
            "application".to_owned(),
            Some(sender),
        ));
        writer.write_all(b"\x1e{\"protocolVersion\":3,\"reason\":\"test-finished\",\"id\":\"test-1\",\"parentId\":null,\"selector\":\"example.AppTest#works\",\"className\":\"example.AppTest\",\"methodName\":\"works\",\"displayName\":\"works()\",\"status\":\"passed\",\"durationMillis\":12,\"message\":null,\"details\":null}\n")
            .await
            .expect("write stream");

        let event = receiver.recv().await.expect("test event");
        assert!(
            !read.is_finished(),
            "event must arrive before the stream closes"
        );
        assert_eq!(event.module, "application");
        assert_eq!(event.reason, TestEventReason::TestFinished);
        assert_eq!(event.status, Some(TestCaseStatus::Passed));
        assert_eq!(event.duration_millis, Some(12));
        writer
            .write_all(b"ordinary output\n")
            .await
            .expect("write ordinary output");
        drop(writer);
        assert_eq!(
            read.await.expect("reader task").expect("read output"),
            b"ordinary output\n"
        );
    }

    #[tokio::test]
    async fn preserves_malformed_and_unknown_test_event_frames() {
        use tokio::io::AsyncWriteExt as _;

        let (mut writer, reader) = tokio::io::duplex(64);
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let read = tokio::spawn(read_test_stdout(
            reader,
            "application".to_owned(),
            Some(sender),
        ));
        writer
            .write_all(b"\x1enot-json\n\x1e{\"protocolVersion\":99}\n")
            .await
            .expect("write stream");
        drop(writer);

        assert_eq!(
            read.await.expect("reader task").expect("read output"),
            b"\x1enot-json\n\x1e{\"protocolVersion\":99}\n"
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn coverage_parses_jacoco_xml_into_provider_neutral_model() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<report name="app">
  <package name="com/example">
    <sourcefile name="Greeting.java">
      <line nr="4" mi="0" ci="3" mb="1" cb="1"/>
      <line nr="5" mi="2" ci="0" mb="0" cb="0"/>
      <counter type="INSTRUCTION" missed="2" covered="3"/>
      <counter type="LINE" missed="1" covered="1"/>
      <counter type="BRANCH" missed="1" covered="1"/>
      <counter type="COMPLEXITY" missed="1" covered="1"/>
      <counter type="METHOD" missed="0" covered="1"/>
      <counter type="CLASS" missed="0" covered="1"/>
    </sourcefile>
  </package>
  <counter type="INSTRUCTION" missed="2" covered="3"/>
  <counter type="LINE" missed="1" covered="1"/>
  <counter type="BRANCH" missed="1" covered="1"/>
  <counter type="COMPLEXITY" missed="1" covered="1"/>
  <counter type="METHOD" missed="0" covered="1"/>
  <counter type="CLASS" missed="0" covered="1"/>
</report>"#;
        let report = parse_jacoco_xml("app", Path::new("/workspace/src/main/java"), xml)
            .expect("coverage report");

        assert_eq!(report.module, "app");
        assert_eq!(report.counters.line.covered, 1);
        assert_eq!(report.counters.line.missed, 1);
        assert!((report.counters.branch.percentage() - 50.0).abs() < f64::EPSILON);
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].package, "com.example");
        assert_eq!(
            report.files[0].path,
            PathBuf::from("/workspace/src/main/java/com/example/Greeting.java")
        );
        assert_eq!(report.files[0].lines[0].number, 4);
        assert_eq!(report.files[0].lines[0].branch.covered, 1);
    }

    #[test]
    fn coverage_thresholds_use_exact_integer_comparisons() {
        let counters = CoverageCounters {
            line: CoverageCount {
                missed: 1,
                covered: 2,
            },
            branch: CoverageCount {
                missed: 1,
                covered: 3,
            },
            ..CoverageCounters::default()
        };
        let failures = coverage_threshold_failures(&counters, Some(67), Some(75));

        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].metric, "line");
        assert_eq!(failures[0].required, 67);
        assert_eq!(failures[0].actual_basis_points, 6_666);
        assert!((failures[0].actual_percentage() - 66.66).abs() < f64::EPSILON);
    }

    #[test]
    fn coverage_class_patterns_support_jacoco_wildcards() {
        assert!(wildcard_matches("com.example.*", "com.example.Service"));
        assert!(wildcard_matches("com/example/*", "com.example.Service"));
        assert!(wildcard_matches("*Service?", "com.example.Service1"));
        assert!(!wildcard_matches(
            "com.example.api.*",
            "com.example.Service"
        ));
    }

    #[tokio::test]
    async fn jar_writer_preserves_resource_directory_entries() {
        let directory = tempfile::tempdir().expect("directory");
        let jar = directory.path().join("application.jar");
        write_jar(
            jar.clone(),
            vec![(
                "META-INF/micronaut/example.Service/provider".to_owned(),
                Vec::new(),
            )],
        )
        .await
        .expect("write JAR");

        let file = std::fs::File::open(jar).expect("open JAR");
        let mut archive = zip::ZipArchive::new(file).expect("read JAR");
        for entry in [
            "META-INF/",
            "META-INF/micronaut/",
            "META-INF/micronaut/example.Service/",
            "META-INF/micronaut/example.Service/provider",
        ] {
            assert!(archive.by_name(entry).is_ok(), "missing `{entry}`");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symbolic_link_build_inputs() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("directory");
        let external = directory.path().join("external.txt");
        std::fs::write(&external, "external").expect("external");
        let inputs = directory.path().join("inputs");
        std::fs::create_dir(&inputs).expect("inputs");
        symlink(&external, inputs.join("linked.txt")).expect("symlink");

        assert!(input_files(&inputs, None)
            .await
            .expect_err("reject symlink")
            .to_string()
            .contains("symbolic links"));
    }
}
