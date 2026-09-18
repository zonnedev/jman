//! Native Java toolchain discovery and incremental workspace compilation.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    fmt::Write as _,
    io::Write as _,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use futures::{stream, StreamExt, TryStreamExt};
use jman_config::{Lockfile, Manifest};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{fs, process::Command, sync::Semaphore};

pub mod toolchain;

const STATE_VERSION: &str = "jman-build-v2";

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

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
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
    pub duration_millis: u64,
    pub message: Option<String>,
    pub details: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TestCaseStatus {
    Passed,
    Failed,
    Skipped,
    Errored,
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
        packages.push(package_module(module, manifest).await?);
        if options.sources {
            packages.push(package_sources(module, manifest).await?);
        }
        if options.javadoc {
            packages.push(
                package_javadoc(module, manifest, &build.toolchain, cache_dir, &upstream).await?,
            );
        }
        if options.fat && manifest.project.main_class.is_some() {
            packages.push(package_fat(module, manifest, cache_dir, &upstream).await?);
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
) -> Result<Vec<TestModuleResult>, BuildError> {
    let modules = discover_modules(root).await?;
    validate_test_modules(&modules, &options.modules)?;
    let selection = junit_selection(&options.patterns)?;
    let build = check_workspace(root, cache_dir, jobs, offline).await?;
    let executable = if cfg!(windows) { "java.exe" } else { "java" };
    let java = build.toolchain.javac.parent().map_or_else(
        || PathBuf::from(executable),
        |parent| parent.join(executable),
    );
    let runner = ensure_test_runner(&build.toolchain, cache_dir).await?;
    let mut results = stream::iter(modules.iter().enumerate())
        .map(|(index, module)| {
            run_test_module(
                index, module, &modules, &build, cache_dir, &java, &runner, &selection, options,
            )
        })
        .buffer_unordered(jobs.max(1))
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    results.sort_by(|left, right| left.module.cmp(&right.module));
    Ok(results)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn run_test_module(
    index: usize,
    module: &Module,
    modules: &[Module],
    build: &CheckResult,
    cache_dir: &Path,
    java: &Path,
    runner: &Path,
    selection: &JunitSelection,
    options: &TestOptions,
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
    let processors = artifact_paths(&lock.classpath.processors, &lock, cache_dir)?;
    let (test_output, rebuilt) =
        compile_test_sources(module, &build.toolchain, &sources, &classpath, &processors).await?;
    let mut runtime = vec![runner.to_owned(), test_output.clone()];
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
    command
        .arg(format!("-Djman.test.port={port}"))
        .arg("-classpath")
        .arg(join_paths(&runtime)?)
        .args([
            "io.github.zonnedev.jman.runner.JmanRunner",
            "--protocol",
            "2",
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
    let output = command
        .args(&options.console_arguments)
        .output()
        .await
        .map_err(|source| BuildError::Javac {
            path: java.to_owned(),
            source,
        })?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(Some(TestModuleResult {
        module: module.manifest.project.name.clone(),
        sources: sources.len(),
        rebuilt,
        status: output.status,
        summary: parse_junit_summary(&text),
        test_cases: parse_junit_reports(reports.path())?,
        output: text,
    }))
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

async fn ensure_test_runner(
    toolchain: &Toolchain,
    cache_dir: &Path,
) -> Result<PathBuf, BuildError> {
    const SOURCE: &str = include_str!(
        "../../../tools/jman-runner/src/main/java/io/github/zonnedev/jman/runner/JmanRunner.java"
    );
    let digest = hex::encode(Sha256::digest(
        format!("jman-runner-v2\n{}\n{SOURCE}", toolchain.version).as_bytes(),
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
    let source = staging.join("src/io/github/zonnedev/jman/runner/JmanRunner.java");
    let classes = staging.join("classes");
    if let Some(parent) = source.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|error| io_error(parent, error))?;
    }
    fs::create_dir_all(&classes)
        .await
        .map_err(|error| io_error(&classes, error))?;
    fs::write(&source, SOURCE)
        .await
        .map_err(|error| io_error(&source, error))?;
    let result = Command::new(&toolchain.javac)
        .args(["-Werror", "-Xlint:all", "-d"])
        .arg(&classes)
        .arg(&source)
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
    checksums
        .iter()
        .map(|checksum| {
            let package = lock
                .packages
                .iter()
                .find(|package| package.artifact_checksum.as_deref() == Some(checksum))
                .ok_or_else(|| {
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
    async fn builds_and_reuses_the_versioned_java_test_runner() {
        let directory = tempfile::tempdir().expect("directory");
        let toolchain = discover_toolchain(8).await.expect("JDK");
        let first = ensure_test_runner(&toolchain, directory.path())
            .await
            .expect("build runner");
        let bytes = std::fs::read(&first).expect("runner bytes");
        let second = ensure_test_runner(&toolchain, directory.path())
            .await
            .expect("reuse runner");
        assert_eq!(first, second);
        assert_eq!(bytes, std::fs::read(second).expect("cached runner bytes"));

        let file = std::fs::File::open(first).expect("open runner");
        let mut archive = zip::ZipArchive::new(file).expect("runner JAR");
        assert!(archive
            .by_name("io/github/zonnedev/jman/runner/JmanRunner.class")
            .is_ok());
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
