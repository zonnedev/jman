use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsStr,
    fmt::Write as _,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use futures::{stream, StreamExt, TryStreamExt};
use jman_config::{
    Build, CoverageFormat, Dependencies, LockedClasspaths, LockedPackage, LockedToolchain,
    Lockfile, Manifest, MavenCompatibility, MavenDependencyMetadata, MavenManagedDependency,
    Project, Repository, Toolchain as ManifestToolchain, LOCK_VERSION, MANIFEST_VERSION,
};
use jman_resolver::{
    plugin_coordinates, AnnotationProcessor, Coordinate, Dependency, DependencyResolver,
    EffectivePom, Exclusion, MavenProject, RepositoryClient, ResolvedGraph,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

mod dependency_report;
mod ui;

use dependency_report::{dependency_path_tree, terminal_text};
use ui::Ui;

#[derive(Debug, Parser)]
#[command(name = "jman", version, about = "A fast, self-contained Java tool")]
struct Cli {
    /// Suppress status and progress output.
    #[arg(long, global = true, conflicts_with = "verbose")]
    quiet: bool,
    /// Show additional operation details. Repeat for more detail.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Disable animated progress while retaining plain status output.
    #[arg(long, global = true)]
    no_progress: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a JMAN project or import an existing build.
    Init(Init),
    /// Resolve dependencies, refresh the lockfile, and prepare project classpaths.
    Sync(Sync),
    /// Add a dependency and synchronize the lockfile.
    Add(Add),
    /// Remove a dependency and synchronize the lockfile.
    Remove(Remove),
    /// Check direct dependencies for newer repository versions.
    Outdated(Outdated),
    /// Update direct dependency versions and synchronize the workspace.
    Update(Update),
    /// Audit resolved dependencies for known vulnerabilities.
    Audit(AuditCommand),
    /// Display the resolved dependency tree.
    Tree(ProjectPath),
    /// Explain why a dependency is present.
    Why(Why),
    /// Type-check and compile main Java sources.
    Check(Check),
    /// Build standard and optional module artifacts.
    Build(BuildCommand),
    /// Publish Maven-compatible artifacts locally or to a remote repository.
    Publish(PublishCommand),
    /// Compile and run an application.
    Run(RunCommand),
    /// Compile and run Java tests.
    Test(TestCommand),
    /// Diagnose the Java toolchain required by this project.
    Doctor(ProjectPath),
    /// Install, select, and inspect Java toolchains.
    Java(Java),
    /// Start the bundled Java language server.
    Lsp(Lsp),
}

#[derive(Debug, Args)]
struct Lsp {
    /// Communicate over standard input/output.
    ///
    /// This is the default and is accepted explicitly for editor clients that
    /// append the transport flag when spawning a language server.
    #[arg(long)]
    stdio: bool,
}

#[derive(Debug, Args)]
struct Init {
    /// Directory to initialize.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Import a detected Maven project without prompting.
    #[arg(long)]
    import: bool,
    /// Create a reusable library instead of an application.
    #[arg(long, conflicts_with = "modules")]
    lib: bool,
    /// Java package group.
    #[arg(long, default_value = "com.example")]
    group: String,
    /// Java release used to compile the project.
    #[arg(long, default_value_t = 21)]
    java: u16,
    /// Initial project version.
    #[arg(long, default_value = "0.1.0-SNAPSHOT")]
    version: String,
    /// Comma-separated child modules.
    #[arg(long, value_delimiter = ',', conflicts_with = "lib")]
    modules: Vec<String>,
    /// Application entry point; defaults to <group>.<name>.Application.
    #[arg(long, conflicts_with = "lib")]
    main_class: Option<String>,
}

#[derive(Debug, Args)]
struct Sync {
    /// Project directory.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Dependency report written after synchronization.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    report: ReportFormat,
    /// Force effective-model reconstruction and lock regeneration.
    #[arg(long)]
    refresh: bool,
    /// Resolve only from the local dependency cache.
    #[arg(long)]
    offline: bool,
}

#[derive(Debug, Args)]
struct Add {
    /// Dependency in group:artifact@version notation.
    dependency: String,
    /// Project directory.
    #[arg(long, default_value = ".")]
    path: PathBuf,
    /// Dependency scope.
    #[arg(long, value_enum, default_value_t = DependencyScope::Compile)]
    scope: DependencyScope,
    /// Resolve only from the local dependency cache.
    #[arg(long)]
    offline: bool,
}

#[derive(Debug, Args)]
struct Remove {
    /// Dependency in group:artifact notation.
    dependency: String,
    /// Project directory.
    #[arg(long, default_value = ".")]
    path: PathBuf,
    /// Resolve only from the local dependency cache.
    #[arg(long)]
    offline: bool,
}

#[derive(Debug, Args)]
struct Outdated {
    /// Project directory.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Use only previously cached Maven metadata.
    #[arg(long)]
    offline: bool,
    /// Include alpha, beta, milestone, release-candidate, and snapshot versions.
    #[arg(long)]
    include_prerelease: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

#[derive(Debug, Args)]
struct Update {
    /// Update only this dependency in group:artifact notation.
    dependency: Option<String>,
    /// Project directory.
    #[arg(long, default_value = ".")]
    path: PathBuf,
    /// Highest version-change class that may be selected.
    #[arg(long, value_enum, default_value_t = UpdateLevel::Patch)]
    level: UpdateLevel,
    /// Use only previously cached Maven metadata and artifacts.
    #[arg(long)]
    offline: bool,
    /// Include alpha, beta, milestone, release-candidate, and snapshot versions.
    #[arg(long)]
    include_prerelease: bool,
    /// Show the update plan without modifying manifests or lockfiles.
    #[arg(long)]
    dry_run: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

#[derive(Debug, Args)]
struct AuditCommand {
    /// Project directory.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Use only a previously cached audit for this exact dependency graph.
    #[arg(long, conflicts_with = "refresh")]
    offline: bool,
    /// Bypass a fresh cached audit and query the provider again.
    #[arg(long)]
    refresh: bool,
    /// Hide findings below this severity.
    #[arg(long, value_enum, default_value_t = AuditSeverity::Unknown)]
    severity: AuditSeverity,
    /// Exit unsuccessfully when an active finding reaches this severity.
    #[arg(long, value_enum)]
    deny: Option<AuditSeverity>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, ValueEnum)]
enum AuditSeverity {
    Unknown,
    Low,
    Medium,
    High,
    Critical,
}

impl From<AuditSeverity> for jman_audit::Severity {
    fn from(value: AuditSeverity) -> Self {
        match value {
            AuditSeverity::Unknown => Self::Unknown,
            AuditSeverity::Low => Self::Low,
            AuditSeverity::Medium => Self::Medium,
            AuditSeverity::High => Self::High,
            AuditSeverity::Critical => Self::Critical,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
enum UpdateLevel {
    Patch,
    Minor,
    Major,
    Latest,
}

#[derive(Debug, Args)]
struct ProjectPath {
    /// Project directory.
    #[arg(default_value = ".")]
    path: PathBuf,
}

#[derive(Debug, Args)]
struct Why {
    /// Dependency in group:artifact notation.
    dependency: String,
    /// Project directory.
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

#[derive(Debug, Args)]
struct Check {
    /// Project directory.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Maximum number of modules compiled concurrently.
    #[arg(short, long)]
    jobs: Option<usize>,
    /// Do not download a missing managed JDK.
    #[arg(long)]
    offline: bool,
}

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)]
struct BuildCommand {
    /// Project directory.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Maximum number of modules compiled concurrently.
    #[arg(short, long)]
    jobs: Option<usize>,
    /// Do not download a missing managed JDK.
    #[arg(long)]
    offline: bool,
    /// Also build executable fat JARs for application modules.
    #[arg(long)]
    fat: bool,
    /// Also build source JARs.
    #[arg(long)]
    sources: bool,
    /// Also build Javadoc JARs.
    #[arg(long)]
    javadoc: bool,
    /// Build fat, source, and Javadoc artifacts.
    #[arg(long)]
    all: bool,
}

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)]
struct PublishCommand {
    /// Project directory.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Publication destination.
    #[arg(long, value_enum, default_value_t = PublishTargetArgument::Local)]
    to: PublishTargetArgument,
    /// Generic Maven repository base URL (required with --to repository).
    #[arg(long)]
    repository_url: Option<String>,
    /// Local Maven repository (defaults to ~/.m2/repository).
    #[arg(long)]
    local_repository: Option<PathBuf>,
    /// Maximum number of modules compiled concurrently.
    #[arg(short, long)]
    jobs: Option<usize>,
    /// Do not download a missing managed JDK or dependency.
    #[arg(long)]
    offline: bool,
    /// Build, validate, and stage artifacts without publishing them.
    #[arg(long)]
    dry_run: bool,
    /// Sign artifacts with GPG (always enabled for Maven Central).
    #[arg(long)]
    sign: bool,
    /// GPG key ID or fingerprint (defaults to the GPG default key).
    #[arg(long)]
    gpg_key: Option<String>,
    /// Allow remote publication from a dirty Git worktree.
    #[arg(long)]
    allow_dirty: bool,
    /// Permit a plain HTTP generic repository URL for local development.
    #[arg(long)]
    allow_insecure: bool,
    /// Ask Maven Central to publish immediately after validation.
    #[arg(long)]
    automatic: bool,
    /// Maximum seconds to wait for Maven Central validation/publication.
    #[arg(long, default_value_t = 300)]
    timeout_seconds: u64,
    /// Publication report format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum PublishTargetArgument {
    Local,
    Repository,
    Central,
}

#[derive(Debug, Args)]
struct RunCommand {
    /// Project directory.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Maximum number of modules compiled concurrently.
    #[arg(short, long)]
    jobs: Option<usize>,
    /// Do not download a missing managed JDK.
    #[arg(long)]
    offline: bool,
    /// Arguments passed to the Java application.
    #[arg(last = true)]
    arguments: Vec<String>,
}

#[derive(Debug, Args)]
struct TestCommand {
    /// Project directory.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Maximum number of modules compiled concurrently.
    #[arg(short, long)]
    jobs: Option<usize>,
    /// Do not download a missing managed JDK.
    #[arg(long)]
    offline: bool,
    /// Run tests only in the named module. May be repeated.
    #[arg(long = "module")]
    modules: Vec<String>,
    /// Run matching test classes or an exact Class#method. May be repeated.
    #[arg(long = "tests", value_name = "PATTERN")]
    tests: Vec<String>,
    /// Select unit, integration, or both test source sets.
    #[arg(long, value_enum, default_value_t = TestSourceSetArgument::All)]
    source_set: TestSourceSetArgument,
    /// Start each test JVM suspended on an ephemeral JDWP port.
    #[arg(long)]
    debug: bool,
    /// Collect code coverage for project classes.
    #[arg(long)]
    coverage: bool,
    /// Coverage output format. May be repeated.
    #[arg(long = "coverage-format", value_enum)]
    coverage_formats: Vec<CoverageFormatArgument>,
    /// Coverage report directory; defaults to .jman/reports/coverage.
    #[arg(long)]
    coverage_output: Option<PathBuf>,
    /// Fail when aggregate line coverage is below this percentage.
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    coverage_min_line: Option<u8>,
    /// Fail when aggregate branch coverage is below this percentage.
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    coverage_min_branch: Option<u8>,
    /// `JaCoCo` class-name pattern to include. May be repeated.
    #[arg(long)]
    coverage_include: Vec<String>,
    /// `JaCoCo` class-name pattern to exclude. May be repeated.
    #[arg(long)]
    coverage_exclude: Vec<String>,
    /// Test result report format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    report: ReportFormat,
    /// Additional test-runner arguments.
    #[arg(last = true)]
    arguments: Vec<String>,
}

#[derive(Debug, Args)]
struct Java {
    #[command(subcommand)]
    command: JavaCommand,
}

#[derive(Debug, Subcommand)]
enum JavaCommand {
    /// Download and install a verified JDK from a vendor.
    Install(JavaVersion),
    /// List installed and remotely available JDKs.
    List(JavaList),
    /// Remove a JDK installed by JMAN.
    Remove(JavaRemove),
    /// Pin this project to a JDK version.
    Use(JavaUse),
    /// Print the managed JDK selected for this project.
    Which(ProjectPath),
}

#[derive(Debug, Args)]
struct JavaList {
    #[command(flatten)]
    scope: JavaListScope,
    /// List only versions from this Java feature release.
    #[arg(long)]
    major: Option<u16>,
    /// List only long-term-support releases.
    #[arg(long)]
    lts: bool,
    /// List only this JDK vendor.
    #[arg(long)]
    vendor: Option<String>,
    /// Revalidate the remote catalog even when the cache is still fresh.
    #[arg(long, conflicts_with = "local")]
    refresh: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

#[derive(Debug, Args)]
struct JavaListScope {
    /// List only JDKs installed by JMAN without contacting the remote catalog.
    #[arg(long)]
    local: bool,
    /// Show every catalog release instead of one latest release per vendor.
    #[arg(long, conflicts_with = "local")]
    all: bool,
}

#[derive(Debug, Args)]
struct JavaVersion {
    /// JDK major or exact version, such as 17 or 17.0.20+8.
    version: String,
    /// JDK vendor to install.
    #[arg(long, default_value = "temurin")]
    vendor: String,
    /// Resolve only from installed JDKs.
    #[arg(long)]
    offline: bool,
}

#[derive(Debug, Args)]
struct JavaUse {
    /// JDK major or exact version, such as 17 or 17.0.20+8.
    version: String,
    /// JDK vendor to pin.
    #[arg(long, default_value = "temurin")]
    vendor: String,
    /// Project directory.
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

#[derive(Debug, Args)]
struct JavaRemove {
    /// JDK major or exact installed version.
    version: String,
    /// JDK vendor to remove.
    #[arg(long, default_value = "temurin")]
    vendor: String,
    /// Remove every installed version of the requested major.
    #[arg(long)]
    all: bool,
    /// Show targets without deleting them.
    #[arg(long)]
    dry_run: bool,
    /// Remove even when the selected project pins this JDK.
    #[arg(long)]
    force: bool,
    /// Project used for pin-safety checks.
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum DependencyScope {
    Compile,
    Runtime,
    Provided,
    Test,
    Processor,
}

impl DependencyScope {
    fn name(self) -> &'static str {
        match self {
            Self::Compile => "compile",
            Self::Runtime => "runtime",
            Self::Provided => "provided",
            Self::Test => "test",
            Self::Processor => "processor",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ReportFormat {
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum TestSourceSetArgument {
    All,
    Unit,
    Integration,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, ValueEnum)]
enum CoverageFormatArgument {
    Summary,
    Json,
    Xml,
    Html,
}

impl From<CoverageFormatArgument> for jman_build::CoverageOutputFormat {
    fn from(value: CoverageFormatArgument) -> Self {
        match value {
            CoverageFormatArgument::Summary => Self::Summary,
            CoverageFormatArgument::Json => Self::Json,
            CoverageFormatArgument::Xml => Self::Xml,
            CoverageFormatArgument::Html => Self::Html,
        }
    }
}

fn configured_coverage_format(value: CoverageFormat) -> jman_build::CoverageOutputFormat {
    match value {
        CoverageFormat::Summary => jman_build::CoverageOutputFormat::Summary,
        CoverageFormat::Json => jman_build::CoverageOutputFormat::Json,
        CoverageFormat::Xml => jman_build::CoverageOutputFormat::Xml,
        CoverageFormat::Html => jman_build::CoverageOutputFormat::Html,
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let structured = matches!(
        &cli.command,
        Command::Sync(Sync {
            report: ReportFormat::Json,
            ..
        }) | Command::Test(TestCommand {
            report: ReportFormat::Json,
            ..
        }) | Command::Java(Java {
            command: JavaCommand::List(JavaList {
                format: ReportFormat::Json,
                ..
            }),
        }) | Command::Publish(PublishCommand {
            format: ReportFormat::Json,
            ..
        }) | Command::Outdated(Outdated {
            format: ReportFormat::Json,
            ..
        }) | Command::Update(Update {
            format: ReportFormat::Json,
            ..
        }) | Command::Audit(AuditCommand {
            format: ReportFormat::Json,
            ..
        })
    );
    let ui = Ui::new(cli.quiet, cli.verbose, cli.no_progress, structured);
    if let Err(error) = run(cli, &ui).await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli, ui: &Ui) -> Result<()> {
    match cli.command {
        Command::Init(init) => init_project(&init, ui).await,
        Command::Sync(sync) => sync_project(&sync, ui).await,
        Command::Add(add) => add_dependency(&add, ui).await,
        Command::Remove(remove) => remove_dependency(&remove, ui).await,
        Command::Outdated(arguments) => outdated_dependencies(&arguments, ui).await,
        Command::Update(arguments) => update_dependencies(&arguments, ui).await,
        Command::Audit(arguments) => audit_dependencies(&arguments, ui).await,
        Command::Tree(arguments) => dependency_tree(&arguments.path),
        Command::Why(arguments) => dependency_why(&arguments),
        Command::Check(arguments) => compile_project(&arguments, ui, "Checking", "Checked").await,
        Command::Build(arguments) => build_project(&arguments, ui).await,
        Command::Publish(arguments) => publish_project(&arguments, ui).await,
        Command::Run(arguments) => run_project(&arguments, ui).await,
        Command::Test(arguments) => test_project(&arguments, ui).await,
        Command::Doctor(arguments) => doctor_project(&arguments.path, ui).await,
        Command::Java(arguments) => java_command(arguments, ui).await,
        Command::Lsp(arguments) => lsp_command(&arguments),
    }
}

fn lsp_command(_arguments: &Lsp) -> Result<()> {
    let status = jman_java_lsp::run_stdio();
    if status != 0 {
        bail!("Java language server exited with status {status}");
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn test_project(arguments: &TestCommand, ui: &Ui) -> Result<()> {
    let target = absolute_path(&arguments.path)?;
    if !target.join("jman.toml").is_file() {
        bail!("cannot test {}: jman.toml was not found", target.display());
    }
    let workspace_root = find_workspace_root(&target);
    let jobs = arguments
        .jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from));
    if jobs == 0 {
        bail!("--jobs must be at least 1");
    }
    let cache_dir = default_cache_dir();
    let activity = ui.activity(format!("Compiling {}", workspace_root.display()));
    jman_build::check_workspace(&workspace_root, &cache_dir, jobs, arguments.offline)
        .await
        .context("Java compilation failed")?;
    activity.finish("Main sources compiled");
    let coverage = coverage_options(arguments, &workspace_root)?;
    let test_options = jman_build::TestOptions {
        modules: arguments.modules.iter().cloned().collect(),
        patterns: arguments.tests.clone(),
        console_arguments: arguments.arguments.clone(),
        source_set: match arguments.source_set {
            TestSourceSetArgument::All => jman_build::TestSourceSet::All,
            TestSourceSetArgument::Unit => jman_build::TestSourceSet::Unit,
            TestSourceSetArgument::Integration => jman_build::TestSourceSet::Integration,
        },
        debug: arguments.debug,
        coverage,
    };
    let (event_sender, mut event_receiver) = tokio::sync::mpsc::unbounded_channel();
    let test_activity = ui.activity("Compiling test sources");
    let tests = jman_build::test_workspace(
        &workspace_root,
        &cache_dir,
        jobs,
        arguments.offline,
        &test_options,
        Some(event_sender),
    );
    tokio::pin!(tests);
    let cancellation = test_cancellation_signal();
    tokio::pin!(cancellation);
    let mut started_modules = BTreeSet::new();
    let mut streamed_tests = BTreeSet::new();
    let mut human_tree = HumanTestTree::default();
    let test_run = loop {
        tokio::select! {
            result = &mut tests => break result.context("JUnit test execution failed")?,
            event = event_receiver.recv() => {
                if let Some(event) = event {
                    report_live_test_event(
                        &event,
                        arguments.report,
                        &test_activity,
                        &mut started_modules,
                        &mut streamed_tests,
                        &mut human_tree,
                    )?;
                }
            }
            signal = &mut cancellation => {
                signal.context("could not install the test cancellation handler")?;
                bail!("test run cancelled; isolated JVM workers were terminated")
            }
        }
    };
    while let Ok(event) = event_receiver.try_recv() {
        report_live_test_event(
            &event,
            arguments.report,
            &test_activity,
            &mut started_modules,
            &mut streamed_tests,
            &mut human_tree,
        )?;
    }
    let results = &test_run.modules;
    if results.is_empty() {
        test_activity.finish("No test sources found");
        return Ok(());
    }
    for result in results {
        report_test_module(
            result,
            arguments.report,
            ui,
            &mut started_modules,
            &streamed_tests,
            &test_activity,
            &mut human_tree,
        )?;
    }
    test_activity.finish("Test execution finished");
    let coverage_result = if let Some(coverage) = &test_run.coverage {
        report_coverage(coverage, arguments.report, &test_options, ui)
    } else {
        Ok(())
    };
    finish_test_results(arguments, ui, results)?;
    coverage_result
}

fn coverage_options(
    arguments: &TestCommand,
    workspace_root: &Path,
) -> Result<Option<jman_build::CoverageOptions>> {
    let manifest = Manifest::read(&workspace_root.join("jman.toml"))?;
    let configured = manifest.test.and_then(|test| test.coverage);
    let explicitly_requested = arguments.coverage
        || !arguments.coverage_formats.is_empty()
        || arguments.coverage_output.is_some()
        || arguments.coverage_min_line.is_some()
        || arguments.coverage_min_branch.is_some()
        || !arguments.coverage_include.is_empty()
        || !arguments.coverage_exclude.is_empty();
    if !explicitly_requested && !configured.as_ref().is_some_and(|coverage| coverage.enabled) {
        return Ok(None);
    }
    let configured = configured.unwrap_or_default();
    let formats = if !arguments.coverage_formats.is_empty() {
        arguments
            .coverage_formats
            .iter()
            .copied()
            .map(Into::into)
            .collect()
    } else if !configured.formats.is_empty() {
        configured
            .formats
            .iter()
            .copied()
            .map(configured_coverage_format)
            .collect()
    } else {
        [
            jman_build::CoverageOutputFormat::Summary,
            jman_build::CoverageOutputFormat::Json,
            jman_build::CoverageOutputFormat::Xml,
            jman_build::CoverageOutputFormat::Html,
        ]
        .into_iter()
        .collect()
    };
    let output_directory = arguments.coverage_output.as_ref().map_or_else(
        || workspace_root.join(".jman/reports/coverage"),
        |path| {
            if path.is_absolute() {
                path.clone()
            } else {
                workspace_root.join(path)
            }
        },
    );
    Ok(Some(jman_build::CoverageOptions {
        output_directory,
        formats,
        minimum_line: arguments.coverage_min_line.or(configured.minimum_line),
        minimum_branch: arguments.coverage_min_branch.or(configured.minimum_branch),
        include: if arguments.coverage_include.is_empty() {
            configured.include
        } else {
            arguments.coverage_include.clone()
        },
        exclude: if arguments.coverage_exclude.is_empty() {
            configured.exclude
        } else {
            arguments.coverage_exclude.clone()
        },
    }))
}

#[derive(Clone, Debug)]
struct TestTreeContainer {
    module: String,
    parent_id: Option<String>,
    display_name: String,
    suite: bool,
}

#[derive(Debug, Default)]
struct HumanTestTree {
    containers: BTreeMap<String, TestTreeContainer>,
    announced_suites: BTreeSet<String>,
    announced_modules: BTreeSet<String>,
    active_suites: BTreeMap<String, String>,
    current_module: Option<String>,
}

impl HumanTestTree {
    fn container_started(&mut self, event: &jman_build::TestEvent, activity: &ui::Activity) {
        let suite = event.class_name.is_some() && event.method_name.is_none();
        self.containers.insert(
            event.id.clone(),
            TestTreeContainer {
                module: event.module.clone(),
                parent_id: event.parent_id.clone(),
                display_name: event.display_name.clone(),
                suite,
            },
        );
        if suite {
            self.ensure_suite_context(&event.id, false, activity);
        }
    }

    fn test_finished(&mut self, event: &jman_build::TestEvent, activity: &ui::Activity) {
        let suite = event
            .parent_id
            .as_deref()
            .and_then(|parent| self.nearest_suite(parent));
        let depth = suite.as_deref().map_or(1, |suite| {
            self.ensure_suite_context(suite, false, activity);
            self.suite_depth(suite) + 1
        });
        self.ensure_module(&event.module, activity);
        let status = event.status.unwrap_or(jman_build::TestCaseStatus::Errored);
        let symbol = match status {
            jman_build::TestCaseStatus::Passed => "✓",
            jman_build::TestCaseStatus::Skipped => "○",
            jman_build::TestCaseStatus::Failed | jman_build::TestCaseStatus::Errored => "✗",
        };
        let duration = format_event_duration(event.duration_nanos, event.duration_millis);
        activity.line(tree_line(
            depth,
            false,
            &format!("{symbol} {} ({duration})", event.display_name),
        ));
        if matches!(
            status,
            jman_build::TestCaseStatus::Failed | jman_build::TestCaseStatus::Errored
        ) {
            Self::failure_details(event, depth + 1, activity);
        }
    }

    fn container_finished(&mut self, event: &jman_build::TestEvent, activity: &ui::Activity) {
        let Some(container) = self.containers.get(&event.id).cloned() else {
            return;
        };
        if !container.suite {
            return;
        }
        self.ensure_suite_context(&event.id, false, activity);
        let depth = self.suite_depth(&event.id) + 1;
        let total = format_event_duration(event.duration_nanos, event.duration_millis);
        let lifecycle = format_event_duration(event.lifecycle_nanos, event.lifecycle_millis);
        let timing = if event.lifecycle_nanos.unwrap_or_default() == 0
            && event.lifecycle_millis.unwrap_or_default() == 0
        {
            format!("{total} total")
        } else {
            format!("{total} total · {lifecycle} lifecycle")
        };
        activity.line(tree_line(depth, true, &timing));
        if let Some(parent) = container
            .parent_id
            .as_deref()
            .and_then(|parent| self.nearest_suite(parent))
        {
            self.active_suites.insert(event.module.clone(), parent);
        } else {
            self.active_suites.remove(&event.module);
        }
    }

    fn module_finished(
        &mut self,
        module: &str,
        summary: &jman_build::TestSummary,
        activity: &ui::Activity,
    ) {
        self.ensure_module(module, activity);
        activity.line(tree_line(
            0,
            true,
            &format!(
                "{} passed · {} failed · {} skipped · {}",
                summary.tests_successful,
                summary.tests_failed,
                summary.tests_skipped,
                ui::format_duration(std::time::Duration::from_millis(summary.duration_millis))
            ),
        ));
        self.active_suites.remove(module);
    }

    fn ensure_suite_context(&mut self, suite: &str, force: bool, activity: &ui::Activity) {
        let Some(container) = self.containers.get(suite).cloned() else {
            return;
        };
        let switched_module = self.ensure_module(&container.module, activity);
        let active = self
            .active_suites
            .get(&container.module)
            .map(String::as_str);
        let announced = self.announced_suites.contains(suite);
        if force || switched_module || active != Some(suite) || !announced {
            let continued = announced;
            let suffix = if continued { " (continued)" } else { "" };
            activity.line(tree_line(
                self.suite_depth(suite),
                false,
                &format!("{}{}", container.display_name, suffix),
            ));
            self.announced_suites.insert(suite.to_owned());
        }
        self.active_suites
            .insert(container.module.clone(), suite.to_owned());
    }

    fn ensure_module(&mut self, module: &str, activity: &ui::Activity) -> bool {
        if self.current_module.as_deref() == Some(module) {
            return false;
        }
        let continued = self.announced_modules.contains(module);
        activity.line(if continued {
            format!("{module} (continued)")
        } else {
            module.to_owned()
        });
        self.announced_modules.insert(module.to_owned());
        self.current_module = Some(module.to_owned());
        true
    }

    fn nearest_suite(&self, identifier: &str) -> Option<String> {
        let mut current = Some(identifier);
        while let Some(identifier) = current {
            let container = self.containers.get(identifier)?;
            if container.suite {
                return Some(identifier.to_owned());
            }
            current = container.parent_id.as_deref();
        }
        None
    }

    fn suite_depth(&self, suite: &str) -> usize {
        let mut depth = 0;
        let mut current = self
            .containers
            .get(suite)
            .and_then(|container| container.parent_id.as_deref());
        while let Some(identifier) = current {
            let Some(container) = self.containers.get(identifier) else {
                break;
            };
            if container.suite {
                depth += 1;
            }
            current = container.parent_id.as_deref();
        }
        depth
    }

    fn failure_details(event: &jman_build::TestEvent, depth: usize, activity: &ui::Activity) {
        if let Some(message) = &event.message {
            activity.line(tree_detail(depth, message));
        }
        if let Some(details) = &event.details {
            for line in details.lines() {
                if event.message.as_deref() != Some(line.trim()) {
                    activity.line(tree_detail(depth, line));
                }
            }
        }
    }
}

fn tree_line(depth: usize, last: bool, content: &str) -> String {
    format!(
        "{}{} {content}",
        "│  ".repeat(depth),
        if last { "└─" } else { "├─" }
    )
}

fn tree_detail(depth: usize, content: &str) -> String {
    format!("{}   {content}", "│  ".repeat(depth))
}

fn format_event_duration(nanos: Option<u64>, millis: Option<u64>) -> String {
    nanos.map_or_else(
        || ui::format_duration(std::time::Duration::from_millis(millis.unwrap_or_default())),
        |nanos| ui::format_duration(std::time::Duration::from_nanos(nanos)),
    )
}

fn report_live_test_event(
    event: &jman_build::TestEvent,
    format: ReportFormat,
    activity: &ui::Activity,
    started_modules: &mut BTreeSet<String>,
    streamed_tests: &mut BTreeSet<(String, String, String)>,
    human_tree: &mut HumanTestTree,
) -> Result<()> {
    report_test_module_started(&event.module, format, started_modules)?;
    let selector = event.selector.as_deref().unwrap_or(&event.id);
    match event.reason {
        jman_build::TestEventReason::ContainerStarted => {
            if format == ReportFormat::Human {
                human_tree.container_started(event, activity);
            }
            if event.class_name.is_some() && event.method_name.is_none() {
                activity.set_message(format!(
                    "Preparing {} › {}",
                    event.module, event.display_name
                ));
                if format == ReportFormat::Json {
                    print_test_json(&serde_json::json!({
                        "protocolVersion": 3,
                        "reason": "test-suite-started",
                        "module": event.module,
                        "id": event.id,
                        "selector": selector,
                        "displayName": event.display_name,
                    }))?;
                }
            }
        }
        jman_build::TestEventReason::ContainerFinished => {
            if format == ReportFormat::Human {
                human_tree.container_finished(event, activity);
            } else if event.class_name.is_some() && event.method_name.is_none() {
                report_live_test_suite(event, selector)?;
            }
        }
        jman_build::TestEventReason::TestStarted => {
            activity.set_message(format!("Running {} › {}", event.module, event.display_name));
            if format == ReportFormat::Json {
                print_test_json(&serde_json::json!({
                    "protocolVersion": 3,
                    "reason": "test-case-started",
                    "module": event.module,
                    "id": event.id,
                    "selector": selector,
                    "displayName": event.display_name,
                }))?;
            }
        }
        jman_build::TestEventReason::TestFinished => {
            let status = event.status.unwrap_or(jman_build::TestCaseStatus::Errored);
            let test_case = jman_build::TestCaseResult {
                id: event.id.clone(),
                selector: selector.to_owned(),
                class_name: event.class_name.clone().unwrap_or_default(),
                method_name: event.method_name.clone().unwrap_or_default(),
                display_name: event.display_name.clone(),
                invocation: None,
                attempt: 1,
                status,
                duration_nanos: event.duration_nanos,
                duration_millis: event.duration_millis.unwrap_or_default(),
                message: event.message.clone(),
                details: event.details.clone(),
            };
            streamed_tests.insert((
                event.module.clone(),
                test_case.selector.clone(),
                test_case.display_name.clone(),
            ));
            if format == ReportFormat::Json {
                print_test_json(&serde_json::json!({
                    "protocolVersion": 3,
                    "reason": "test-case",
                    "module": event.module,
                    "test": test_case,
                }))?;
            } else {
                human_tree.test_finished(event, activity);
            }
        }
    }
    Ok(())
}

fn report_live_test_suite(event: &jman_build::TestEvent, selector: &str) -> Result<()> {
    let status = event.status.unwrap_or(jman_build::TestCaseStatus::Errored);
    let duration_millis = event.duration_millis.unwrap_or_default();
    let lifecycle_millis = event.lifecycle_millis.unwrap_or_default();
    print_test_json(&serde_json::json!({
        "protocolVersion": 3,
        "reason": "test-suite",
        "module": event.module,
        "suite": {
            "id": event.id,
            "selector": selector,
            "className": event.class_name,
            "displayName": event.display_name,
            "status": status,
            "durationNanos": event.duration_nanos,
            "durationMillis": duration_millis,
            "lifecycleNanos": event.lifecycle_nanos,
            "lifecycleMillis": lifecycle_millis,
            "message": event.message,
            "details": event.details,
        }
    }))?;
    Ok(())
}

fn report_test_module_started(
    module: &str,
    format: ReportFormat,
    started_modules: &mut BTreeSet<String>,
) -> Result<()> {
    if format == ReportFormat::Json && started_modules.insert(module.to_owned()) {
        print_test_json(&serde_json::json!({
            "protocolVersion": 3,
            "reason": "test-module-started",
            "module": module,
            "debug": {
                "supported": true,
                "transport": "java-debug-adapter",
                "suspend": true
            },
            "coverage": {
                "supported": true,
                "engine": "jacoco",
                "protocolVersion": 1
            }
        }))?;
    }
    Ok(())
}

fn print_test_json(event: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string(event)?);
    io::stdout().flush().context("could not flush test event")
}

async fn test_cancellation_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

fn report_test_module(
    result: &jman_build::TestModuleResult,
    format: ReportFormat,
    ui: &Ui,
    started_modules: &mut BTreeSet<String>,
    streamed_tests: &BTreeSet<(String, String, String)>,
    activity: &ui::Activity,
    human_tree: &mut HumanTestTree,
) -> Result<()> {
    if format == ReportFormat::Json {
        report_test_module_started(&result.module, format, started_modules)?;
        for test_case in result.test_cases.iter().filter(|test_case| {
            !streamed_tests.contains(&(
                result.module.clone(),
                test_case.selector.clone(),
                test_case.display_name.clone(),
            ))
        }) {
            print_test_json(&serde_json::json!({
                "protocolVersion": 3,
                "reason": "test-case",
                "module": result.module,
                "test": test_case,
            }))?;
        }
        print_test_json(&serde_json::json!({
            "protocolVersion": 3,
            "reason": "test-module-finished",
            "module": result.module,
            "successful": result.status.success(),
            "summary": result.summary
        }))?;
        return Ok(());
    }
    if result.summary.is_some() && (!result.status.success() || ui.is_verbose()) {
        if let Some((details, _)) = result.output.split_once("Test run finished after") {
            let details = details.trim();
            if !details.is_empty() {
                eprintln!("{details}");
            }
        }
    } else if result.summary.is_none() {
        print!("{}", result.output);
    }
    ui.detail(format!(
        "{}: {} test source files ({})",
        result.module,
        result.sources,
        if result.rebuilt { "compiled" } else { "cached" }
    ));
    if let Some(summary) = &result.summary {
        if ui.is_verbose() {
            ui.detail(format!(
                "JUnit containers: {} found / {} started / {} successful / {} skipped / {} aborted / {} failed",
                summary.containers_found,
                summary.containers_started,
                summary.containers_successful,
                summary.containers_skipped,
                summary.containers_aborted,
                summary.containers_failed
            ));
        }
        human_tree.module_finished(&result.module, summary, activity);
    }
    Ok(())
}

fn report_coverage(
    coverage: &jman_build::CoverageReport,
    report_format: ReportFormat,
    test_options: &jman_build::TestOptions,
    ui: &Ui,
) -> Result<()> {
    if report_format == ReportFormat::Json {
        for module in &coverage.modules {
            for file in &module.files {
                print_test_json(&serde_json::json!({
                    "protocolVersion": 3,
                    "reason": "coverage-file",
                    "file": file
                }))?;
            }
        }
        print_test_json(&serde_json::json!({
            "protocolVersion": 3,
            "reason": "coverage-summary",
            "coverage": coverage
        }))?;
    } else if test_options.coverage.as_ref().is_some_and(|options| {
        options
            .formats
            .contains(&jman_build::CoverageOutputFormat::Summary)
    }) {
        ui.line("");
        ui.line("Coverage");
        for module in &coverage.modules {
            ui.line(format!("├─ {}", module.module));
            ui.line(format!(
                "│  ├─ lines       {}",
                format_coverage_count(&module.counters.line)
            ));
            ui.line(format!(
                "│  ├─ branches    {}",
                format_coverage_count(&module.counters.branch)
            ));
            ui.line(format!(
                "│  └─ methods     {}",
                format_coverage_count(&module.counters.method)
            ));
        }
        ui.line("└─ workspace");
        ui.line(format!(
            "   ├─ lines       {}",
            format_coverage_count(&coverage.totals.line)
        ));
        ui.line(format!(
            "   ├─ branches    {}",
            format_coverage_count(&coverage.totals.branch)
        ));
        ui.line(format!(
            "   └─ methods     {}",
            format_coverage_count(&coverage.totals.method)
        ));
        if let Some(html) = &coverage.html {
            ui.line(format!("HTML report: {}", html.display()));
        }
        if let Some(xml) = &coverage.xml {
            ui.line(format!("XML report: {}", xml.display()));
        }
        if let Some(json) = &coverage.json {
            ui.line(format!("JSON report: {}", json.display()));
        }
    }
    if coverage.threshold_failures.is_empty() {
        return Ok(());
    }
    let failures = coverage
        .threshold_failures
        .iter()
        .map(|failure| {
            format!(
                "{} {:.2}% is below {}%",
                failure.metric,
                failure.actual_percentage(),
                failure.required
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    bail!("coverage threshold failed: {failures}")
}

fn format_coverage_count(count: &jman_build::CoverageCount) -> String {
    format!(
        "{:>5.1}%  {}/{}",
        count.percentage(),
        count.covered,
        count.total()
    )
}

fn finish_test_results(
    arguments: &TestCommand,
    ui: &Ui,
    results: &[jman_build::TestModuleResult],
) -> Result<()> {
    let failed_modules = results
        .iter()
        .filter(|result| !result.status.success())
        .map(|result| result.module.as_str())
        .collect::<Vec<_>>();
    let totals = aggregate_test_summaries(results);
    if failed_modules.is_empty() {
        if let Some((passed, failed, skipped, duration, _)) = totals {
            ui.success(format!(
                "Result: {passed} passed / {failed} failed / {skipped} skipped ({})",
                ui::format_duration(std::time::Duration::from_millis(duration))
            ));
        } else {
            ui.success(format!(
                "Tests passed in {} module{}",
                results.len(),
                if results.len() == 1 { "" } else { "s" }
            ));
        }
        Ok(())
    } else {
        if let Some((passed, failures, skipped, duration, found)) = totals {
            ui.warning(format!(
                "Result: {passed} passed / {failures} failed / {skipped} skipped ({})",
                ui::format_duration(std::time::Duration::from_millis(duration))
            ));
            if !arguments.tests.is_empty() && found == 0 {
                bail!(
                    "no tests matched {}",
                    arguments
                        .tests
                        .iter()
                        .map(|pattern| format!("`{pattern}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        bail!("tests failed in {}", failed_modules.join(", "))
    }
}

fn aggregate_test_summaries(
    results: &[jman_build::TestModuleResult],
) -> Option<(usize, usize, usize, u64, usize)> {
    let summaries = results
        .iter()
        .map(|result| result.summary.as_ref())
        .collect::<Option<Vec<_>>>()?;
    Some((
        summaries
            .iter()
            .map(|summary| summary.tests_successful)
            .sum(),
        summaries.iter().map(|summary| summary.tests_failed).sum(),
        summaries.iter().map(|summary| summary.tests_skipped).sum(),
        summaries
            .iter()
            .map(|summary| summary.duration_millis)
            .sum(),
        summaries.iter().map(|summary| summary.tests_found).sum(),
    ))
}

async fn run_project(arguments: &RunCommand, ui: &Ui) -> Result<()> {
    let target = absolute_path(&arguments.path)?;
    if !target.join("jman.toml").is_file() {
        bail!("cannot run {}: jman.toml was not found", target.display());
    }
    let workspace_root = find_workspace_root(&target);
    let jobs = arguments
        .jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from));
    if jobs == 0 {
        bail!("--jobs must be at least 1");
    }
    let activity = ui.activity(format!("Preparing {}", workspace_root.display()));
    jman_build::check_workspace(
        &workspace_root,
        &default_cache_dir(),
        jobs,
        arguments.offline,
    )
    .await
    .context("Java compilation failed")?;
    activity.finish("Application compiled");
    ui.detail("Starting JVM");
    let result = jman_build::run_workspace(
        &workspace_root,
        &default_cache_dir(),
        jobs,
        arguments.offline,
        &arguments.arguments,
    )
    .await
    .context("application execution failed")?;
    if !result.status.success() {
        bail!("application exited with {}", result.status);
    }
    Ok(())
}

async fn build_project(arguments: &BuildCommand, ui: &Ui) -> Result<()> {
    let target = absolute_path(&arguments.path)?;
    if !target.join("jman.toml").is_file() {
        bail!("cannot build {}: jman.toml was not found", target.display());
    }
    let workspace_root = find_workspace_root(&target);
    let jobs = arguments
        .jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from));
    if jobs == 0 {
        bail!("--jobs must be at least 1");
    }
    let activity = ui.activity(format!("Building {}", workspace_root.display()));
    let packages = jman_build::package_workspace(
        &workspace_root,
        &default_cache_dir(),
        jobs,
        arguments.offline,
        jman_build::ArtifactOptions {
            fat: arguments.fat || arguments.all,
            sources: arguments.sources || arguments.all,
            javadoc: arguments.javadoc || arguments.all,
        },
    )
    .await
    .context("JAR packaging failed")?;
    let unchanged = packages.iter().filter(|package| package.unchanged).count();
    activity.finish(format!(
        "Built {} artifact{}{}",
        packages.len(),
        if packages.len() == 1 { "" } else { "s" },
        if unchanged == packages.len() && !packages.is_empty() {
            " (unchanged)"
        } else {
            ""
        }
    ));
    for package in packages {
        if !package.warnings.is_empty() {
            ui.warning(format!(
                "{} {} kept the first occurrence of {} conflicting resource{}; use -v for details",
                package.module,
                package.kind,
                package.warnings.len(),
                if package.warnings.len() == 1 { "" } else { "s" }
            ));
            for warning in &package.warnings {
                ui.detail(warning);
            }
        }
        ui.success(format!(
            "{} {} -> {} ({}, {}, {})",
            package.module,
            package.kind,
            package.artifact.display(),
            indicatif::HumanBytes(package.size),
            package.checksum,
            if package.unchanged {
                "unchanged"
            } else {
                "written"
            }
        ));
    }
    Ok(())
}

async fn publish_project(arguments: &PublishCommand, ui: &Ui) -> Result<()> {
    let target = absolute_path(&arguments.path)?;
    if !target.join("jman.toml").is_file() {
        bail!(
            "cannot publish {}: jman.toml was not found",
            target.display()
        );
    }
    if arguments.timeout_seconds == 0 {
        bail!("--timeout-seconds must be at least 1");
    }
    let workspace_root = find_workspace_root(&target);
    let jobs = arguments
        .jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from));
    if jobs == 0 {
        bail!("--jobs must be at least 1");
    }
    let publish_target = publication_target(arguments)?;
    let activity = ui.activity(format!("Publishing {}", workspace_root.display()));
    let report = jman_publish::publish(&jman_publish::PublishOptions {
        root: workspace_root,
        cache_dir: default_cache_dir(),
        jobs,
        offline: arguments.offline,
        dry_run: arguments.dry_run,
        sign: arguments.sign,
        gpg_key: arguments
            .gpg_key
            .clone()
            .or_else(|| env::var("JMAN_GPG_KEY_ID").ok()),
        gpg_passphrase: env::var("JMAN_GPG_PASSPHRASE").ok(),
        allow_dirty: arguments.allow_dirty,
        timeout: std::time::Duration::from_secs(arguments.timeout_seconds),
        target: publish_target,
    })
    .await
    .context("publication failed")?;
    activity.finish(if report.dry_run {
        "Publication staged and validated"
    } else {
        "Publication completed"
    });
    print_publish_report(&report, arguments.format, ui)
}

fn publication_target(arguments: &PublishCommand) -> Result<jman_publish::PublishTarget> {
    Ok(match arguments.to {
        PublishTargetArgument::Local => {
            if arguments.repository_url.is_some() {
                bail!("--repository-url requires --to repository");
            }
            if arguments.automatic || arguments.allow_insecure {
                bail!("--automatic and --allow-insecure do not apply to local publication");
            }
            let repository = arguments.local_repository.clone().unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".m2/repository")
            });
            jman_publish::PublishTarget::Local { repository }
        }
        PublishTargetArgument::Repository => {
            if arguments.local_repository.is_some() {
                bail!("--local-repository requires --to local");
            }
            if arguments.automatic {
                bail!("--automatic requires --to central");
            }
            let value = arguments
                .repository_url
                .as_deref()
                .context("--to repository requires --repository-url")?;
            let url = value
                .parse()
                .with_context(|| format!("invalid repository URL `{value}`"))?;
            jman_publish::PublishTarget::Repository {
                url,
                credentials: jman_publish::RepositoryCredentials {
                    username: env::var("JMAN_PUBLISH_USERNAME").ok(),
                    password: env::var("JMAN_PUBLISH_PASSWORD").ok(),
                    token: env::var("JMAN_PUBLISH_TOKEN").ok(),
                },
                allow_insecure: arguments.allow_insecure,
            }
        }
        PublishTargetArgument::Central => {
            if arguments.repository_url.is_some() || arguments.local_repository.is_some() {
                bail!("repository URL/path options do not apply to Maven Central");
            }
            if arguments.allow_insecure {
                bail!("--allow-insecure does not apply to Maven Central");
            }
            let token = central_credentials(arguments.dry_run)?;
            jman_publish::PublishTarget::Central {
                token,
                automatic: arguments.automatic,
                api_url: jman_publish::CENTRAL_API_URL
                    .parse()
                    .expect("static Maven Central API URL"),
            }
        }
    })
}

fn print_publish_report(
    report: &jman_publish::PublishReport,
    format: ReportFormat,
    ui: &Ui,
) -> Result<()> {
    match format {
        ReportFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        ReportFormat::Human => {
            for module in &report.modules {
                ui.success(format!(
                    "{}:{}:{} ({})",
                    module.group, module.artifact, module.version, module.packaging
                ));
            }
            ui.success(format!(
                "{} file{} staged at {}",
                report.files.len(),
                if report.files.len() == 1 { "" } else { "s" },
                report.staging_directory.display()
            ));
            if let Some(bundle) = &report.bundle {
                ui.success(format!("Central bundle -> {}", bundle.display()));
            }
            if let Some(id) = &report.deployment_id {
                ui.success(format!(
                    "Central deployment {id}: {}",
                    report.deployment_state.as_deref().unwrap_or("submitted")
                ));
            }
        }
    }
    Ok(())
}

fn central_credentials(optional: bool) -> Result<String> {
    if let Ok(token) = env::var("JMAN_CENTRAL_TOKEN") {
        if !token.trim().is_empty() {
            return Ok(token);
        }
    }
    match (
        env::var("JMAN_CENTRAL_USERNAME").ok(),
        env::var("JMAN_CENTRAL_PASSWORD").ok(),
    ) {
        (Some(username), Some(password)) if !username.is_empty() && !password.is_empty() => {
            Ok(jman_publish::central_token(&username, &password))
        }
        _ if optional => Ok(String::new()),
        _ => bail!(
            "Maven Central credentials are missing; set JMAN_CENTRAL_TOKEN or both JMAN_CENTRAL_USERNAME and JMAN_CENTRAL_PASSWORD"
        ),
    }
}

async fn compile_project(
    arguments: &Check,
    ui: &Ui,
    active_verb: &str,
    completed_verb: &str,
) -> Result<()> {
    let target = absolute_path(&arguments.path)?;
    if !target.join("jman.toml").is_file() {
        bail!("cannot check {}: jman.toml was not found", target.display());
    }
    let workspace_root = find_workspace_root(&target);
    let jobs = arguments
        .jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from));
    if jobs == 0 {
        bail!("--jobs must be at least 1");
    }
    let activity = ui.activity(format!("{active_verb} {}", workspace_root.display()));
    let result = jman_build::check_workspace(
        &workspace_root,
        &default_cache_dir(),
        jobs,
        arguments.offline,
    )
    .await
    .context("Java compilation failed")?;
    let rebuilt = result
        .modules
        .iter()
        .filter(|module| module.rebuilt)
        .count();
    let sources = result
        .modules
        .iter()
        .map(|module| module.sources)
        .sum::<usize>();
    for module in &result.modules {
        ui.detail(format!(
            "{}: {} source files ({})",
            module.name,
            module.sources,
            if module.rebuilt { "compiled" } else { "cached" }
        ));
    }
    activity.finish(if rebuilt == 0 {
        format!(
            "{completed_verb} {} modules and {sources} source files (unchanged)",
            result.modules.len()
        )
    } else {
        format!(
            "{completed_verb} {} modules and {sources} source files ({rebuilt} compiled)",
            result.modules.len()
        )
    });
    Ok(())
}

async fn doctor_project(path: &Path, ui: &Ui) -> Result<()> {
    let target = absolute_path(path)?;
    let manifest = Manifest::read(&target.join("jman.toml"))
        .with_context(|| format!("could not read {}", target.join("jman.toml").display()))?;
    let activity = ui.activity("Inspecting Java toolchain");
    let toolchain = if let Some(request) = manifest_toolchain_request(&manifest) {
        jman_build::toolchain::resolve(&request, &default_cache_dir(), true, false).await?
    } else {
        jman_build::toolchain::resolve_default(manifest.project.java_release, &default_cache_dir())
            .await?
    };
    activity.finish(format!(
        "{} at {} (project release {})",
        toolchain.version,
        toolchain.javac.display(),
        manifest.project.java_release
    ));
    let activity = ui.activity("Inspecting container runtime");
    if let Some(runtime) = detect_container_runtime().await {
        activity.finish(runtime);
    } else {
        activity.finish("No reachable Docker or Podman service (optional for tests)".to_owned());
    }
    Ok(())
}

async fn detect_container_runtime() -> Option<String> {
    let configured = ["DOCKER_HOST", "CONTAINER_HOST"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().map(|value| (name, value)));
    for (name, arguments) in [
        (
            "Docker",
            &["version", "--format", "{{.Server.Version}}"][..],
        ),
        (
            "Podman",
            &["version", "--format", "{{.Server.Version}}"][..],
        ),
    ] {
        let executable = name.to_ascii_lowercase();
        let mut command = tokio::process::Command::new(&executable);
        command.kill_on_drop(true).args(arguments);
        let Ok(Ok(output)) =
            tokio::time::timeout(std::time::Duration::from_secs(2), command.output()).await
        else {
            continue;
        };
        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let endpoint_variable = if name == "Docker" {
                "DOCKER_HOST"
            } else {
                "CONTAINER_HOST"
            };
            let endpoint = std::env::var(endpoint_variable).map_or_else(
                |_| "default endpoint".to_owned(),
                |value| format!("{endpoint_variable}={}", display_container_endpoint(&value)),
            );
            return Some(format!("{name} {version} via {endpoint}"));
        }
    }
    configured.map(|(variable, value)| {
        format!(
            "Configured {variable}={}, but no reachable Docker or Podman service",
            display_container_endpoint(&value)
        )
    })
}

fn display_container_endpoint(endpoint: &str) -> String {
    let Some((scheme, address)) = endpoint.split_once("://") else {
        return endpoint.to_owned();
    };
    let Some((_, host)) = address.rsplit_once('@') else {
        return endpoint.to_owned();
    };
    format!("{scheme}://***@{host}")
}

async fn java_command(arguments: Java, ui: &Ui) -> Result<()> {
    use jman_build::toolchain::{ToolchainManager, ToolchainRequest};

    let manager = ToolchainManager::new(&default_cache_dir())?;
    match arguments.command {
        JavaCommand::Install(arguments) => {
            let vendor = jman_build::toolchain::normalize_vendor(&arguments.vendor);
            let activity = ui.activity(format!("Installing {vendor} JDK {}", arguments.version));
            let report_download = |downloaded, total| {
                activity.set_download_progress(downloaded, total);
            };
            let jdk = manager
                .install_with_progress(
                    &ToolchainRequest {
                        version: arguments.version,
                        vendor: vendor.clone(),
                    },
                    arguments.offline,
                    Some(&report_download),
                )
                .await?;
            activity.finish(format!(
                "Installed {} {} at {}",
                jdk.vendor,
                jdk.version,
                jdk.home.display()
            ));
        }
        JavaCommand::List(arguments) => list_java(&manager, arguments, ui).await?,
        JavaCommand::Remove(arguments) => remove_java(&manager, arguments, ui).await?,
        JavaCommand::Use(arguments) => {
            let target = absolute_path(&arguments.path)?;
            let path = target.join("jman.toml");
            let mut manifest = Manifest::read(&path)
                .with_context(|| format!("could not read {}", path.display()))?;
            let selected_version = arguments.version;
            let vendor = jman_build::toolchain::normalize_vendor(&arguments.vendor);
            manifest.toolchain = Some(ManifestToolchain {
                jdk: selected_version.clone(),
                vendor: vendor.clone(),
            });
            let text = manifest
                .to_toml()
                .context("invalid JDK selection for this project")?;
            write_atomically(&path, &text).await?;
            ui.success(format!(
                "Pinned {} to {} JDK {}",
                manifest.project.name, vendor, selected_version
            ));
        }
        JavaCommand::Which(arguments) => {
            let target = absolute_path(&arguments.path)?;
            let manifest = Manifest::read(&target.join("jman.toml")).with_context(|| {
                format!("could not read {}", target.join("jman.toml").display())
            })?;
            if let Some(request) = manifest_toolchain_request(&manifest) {
                let toolchain =
                    jman_build::toolchain::resolve(&request, &default_cache_dir(), false, true)
                        .await?;
                println!("{}", toolchain.javac.display());
            } else {
                println!(
                    "{}",
                    jman_build::toolchain::resolve_default(
                        manifest.project.java_release,
                        &default_cache_dir(),
                    )
                    .await?
                    .javac
                    .display()
                );
            }
        }
    }
    Ok(())
}

async fn list_java(
    manager: &jman_build::toolchain::ToolchainManager,
    arguments: JavaList,
    ui: &Ui,
) -> Result<()> {
    let vendor = arguments
        .vendor
        .as_deref()
        .map(jman_build::toolchain::normalize_vendor);
    let installed = manager
        .list()
        .await?
        .into_iter()
        .filter(|jdk| arguments.major.is_none_or(|major| jdk.major == major))
        .filter(|jdk| vendor.as_ref().is_none_or(|vendor| jdk.vendor == *vendor))
        .filter(|jdk| !arguments.lts || jman_build::toolchain::is_lts_major(jdk.major))
        .collect::<Vec<_>>();

    if arguments.scope.local {
        if arguments.format == ReportFormat::Json {
            print_java_list_json(&installed, &[], None)?;
            return Ok(());
        }
        if installed.is_empty() {
            println!("No matching JMAN-managed JDKs installed.");
        } else {
            print_java_table(&installed, &[], true);
        }
        return Ok(());
    }

    let installed_versions = installed
        .iter()
        .map(|jdk| (jdk.vendor.as_str(), jdk.version.as_str()))
        .collect::<std::collections::HashSet<_>>();
    let catalog = manager
        .available(arguments.major, arguments.lts, arguments.refresh)
        .await
        .context("could not load remote JDK catalog; use `jman java list --local` to list installed JDKs only")?;
    let remote = catalog
        .jdks
        .into_iter()
        .filter(|jdk| vendor.as_ref().is_none_or(|vendor| jdk.vendor == *vendor))
        .collect::<Vec<_>>();
    let available = remote
        .iter()
        .filter(|jdk| !installed_versions.contains(&(jdk.vendor.as_str(), jdk.version.as_str())))
        .cloned()
        .collect::<Vec<_>>();

    if arguments.format == ReportFormat::Json {
        print_java_list_json(&installed, &available, Some(&catalog.status))?;
        return Ok(());
    }
    if let Some(warning) = &catalog.status.warning {
        ui.warning(warning);
    }

    if installed.is_empty() && remote.is_empty() {
        println!("No matching JDKs found.");
        return Ok(());
    }
    print_java_table(&installed, &remote, arguments.scope.all);
    if !arguments.scope.all {
        println!();
        println!("Showing the latest matching release per vendor; use --all for every release.");
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct JavaTableRow {
    installed: bool,
    vendor: String,
    major: u16,
    version: String,
    support: &'static str,
    platform: String,
}

fn print_java_table(
    installed: &[jman_build::toolchain::ManagedJdk],
    remote: &[jman_build::toolchain::AvailableJdk],
    detailed: bool,
) {
    print!("{}", java_table(installed, remote, detailed));
}

fn java_table(
    installed: &[jman_build::toolchain::ManagedJdk],
    remote: &[jman_build::toolchain::AvailableJdk],
    detailed: bool,
) -> String {
    let rows = java_table_rows(installed, remote, detailed);
    let mut output = String::new();
    writeln!(
        output,
        "{:<9}  {:<21}  {:>4}  {:<22}  {:<7}  PLATFORM",
        "INSTALLED", "VENDOR", "JAVA", "VERSION", "SUPPORT"
    )
    .expect("writing a String cannot fail");
    writeln!(
        output,
        "{:-<9}  {:-<21}  {:-<4}  {:-<22}  {:-<7}  {:-<15}",
        "", "", "", "", "", ""
    )
    .expect("writing a String cannot fail");
    for row in rows {
        writeln!(
            output,
            "{:<9}  {:<21}  {:>4}  {:<22}  {:<7}  {}",
            if row.installed { "yes" } else { "no" },
            row.vendor,
            row.major,
            row.version,
            row.support,
            row.platform
        )
        .expect("writing a String cannot fail");
    }
    output
}

fn java_table_rows(
    installed: &[jman_build::toolchain::ManagedJdk],
    remote: &[jman_build::toolchain::AvailableJdk],
    detailed: bool,
) -> Vec<JavaTableRow> {
    let installed_by_release = installed
        .iter()
        .map(|jdk| ((jdk.vendor.as_str(), jdk.version.as_str()), jdk))
        .collect::<std::collections::HashMap<_, _>>();
    let selected_remote = if detailed {
        remote.iter().collect::<Vec<_>>()
    } else {
        let mut latest_by_vendor = std::collections::HashMap::new();
        for jdk in remote {
            latest_by_vendor
                .entry(jdk.vendor.as_str())
                .and_modify(|current: &mut &jman_build::toolchain::AvailableJdk| {
                    if java_release_key(jdk.major, &jdk.version)
                        > java_release_key(current.major, &current.version)
                    {
                        *current = jdk;
                    }
                })
                .or_insert(jdk);
        }
        latest_by_vendor.into_values().collect::<Vec<_>>()
    };

    let mut represented = std::collections::HashSet::new();
    let mut rows = selected_remote
        .into_iter()
        .map(|jdk| {
            let installed = installed_by_release
                .get(&(jdk.vendor.as_str(), jdk.version.as_str()))
                .copied();
            represented.insert((jdk.vendor.as_str(), jdk.version.as_str()));
            JavaTableRow {
                installed: installed.is_some(),
                vendor: jdk.vendor.clone(),
                major: jdk.major,
                version: jdk.version.clone(),
                support: if jdk.lts { "LTS" } else { "STS" },
                platform: format!("{}-{}", jdk.os, jdk.architecture),
            }
        })
        .collect::<Vec<_>>();
    rows.extend(
        installed
            .iter()
            .filter(|jdk| !represented.contains(&(jdk.vendor.as_str(), jdk.version.as_str())))
            .map(|jdk| JavaTableRow {
                installed: true,
                vendor: jdk.vendor.clone(),
                major: jdk.major,
                version: jdk.version.clone(),
                support: if jman_build::toolchain::is_lts_major(jdk.major) {
                    "LTS"
                } else {
                    "STS"
                },
                platform: format!("{}-{}", jdk.os, jdk.architecture),
            }),
    );
    rows.sort_by(|left, right| {
        right
            .installed
            .cmp(&left.installed)
            .then_with(|| left.vendor.cmp(&right.vendor))
            .then_with(|| {
                java_release_key(right.major, &right.version)
                    .cmp(&java_release_key(left.major, &left.version))
            })
    });
    rows
}

fn java_release_key(major: u16, version: &str) -> (u16, Vec<u32>) {
    let (core, suffix) = version.split_once('+').unwrap_or((version, ""));
    let mut key = core
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect::<Vec<_>>();
    key.resize(4, 0);
    key.extend(
        suffix
            .split(|character: char| !character.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<u32>().ok()),
    );
    (major, key)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstalledJava<'a> {
    vendor: &'a str,
    version: &'a str,
    major: u16,
    lts: bool,
    os: &'a str,
    architecture: &'a str,
    home: &'a Path,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JavaListReport<'a> {
    catalog: Option<&'a jman_build::toolchain::CatalogStatus>,
    installed: Vec<InstalledJava<'a>>,
    available: &'a [jman_build::toolchain::AvailableJdk],
}

fn print_java_list_json(
    installed: &[jman_build::toolchain::ManagedJdk],
    available: &[jman_build::toolchain::AvailableJdk],
    catalog: Option<&jman_build::toolchain::CatalogStatus>,
) -> Result<()> {
    println!("{}", java_list_json(installed, available, catalog)?);
    Ok(())
}

fn java_list_json(
    installed: &[jman_build::toolchain::ManagedJdk],
    available: &[jman_build::toolchain::AvailableJdk],
    catalog: Option<&jman_build::toolchain::CatalogStatus>,
) -> Result<String> {
    let report = JavaListReport {
        catalog,
        installed: installed
            .iter()
            .map(|jdk| InstalledJava {
                vendor: &jdk.vendor,
                version: &jdk.version,
                major: jdk.major,
                lts: jman_build::toolchain::is_lts_major(jdk.major),
                os: &jdk.os,
                architecture: &jdk.architecture,
                home: &jdk.home,
            })
            .collect(),
        available,
    };
    Ok(serde_json::to_string_pretty(&report)?)
}

async fn remove_java(
    manager: &jman_build::toolchain::ToolchainManager,
    arguments: JavaRemove,
    ui: &Ui,
) -> Result<()> {
    let request = jman_build::toolchain::ToolchainRequest {
        version: arguments.version,
        vendor: jman_build::toolchain::normalize_vendor(&arguments.vendor),
    };
    let candidates = manager.removal_candidates(&request, arguments.all).await?;
    if candidates.is_empty() {
        bail!(
            "no JMAN-managed {} JDK {} installation was found",
            request.vendor,
            request.version
        );
    }
    if !arguments.force {
        protect_pinned_toolchain(&arguments.path, &candidates)?;
    }
    if arguments.dry_run {
        for jdk in candidates {
            println!(
                "Would remove {} {} at {}",
                jdk.vendor,
                jdk.version,
                jdk.home.display()
            );
        }
        return Ok(());
    }
    let activity = ui.activity(format!(
        "Removing {} JDK {}",
        request.vendor, request.version
    ));
    let result = manager.remove(&request, arguments.all).await?;
    activity.finish(format!(
        "Removed {} JDK installation{} and reclaimed {}",
        result.removed.len(),
        if result.removed.len() == 1 { "" } else { "s" },
        indicatif::HumanBytes(result.reclaimed_bytes)
    ));
    Ok(())
}

fn protect_pinned_toolchain(
    project: &Path,
    candidates: &[jman_build::toolchain::ManagedJdk],
) -> Result<()> {
    let target = absolute_path(project)?;
    let manifest_path = target.join("jman.toml");
    if !manifest_path.is_file() {
        return Ok(());
    }
    let manifest = Manifest::read(&manifest_path)
        .with_context(|| format!("could not read {}", manifest_path.display()))?;
    let Some(pin) = manifest.toolchain else {
        return Ok(());
    };
    let pin_major = pin.jdk.parse::<u16>().ok();
    if candidates.iter().any(|jdk| {
        jman_build::toolchain::normalize_vendor(&pin.vendor) == jdk.vendor
            && (pin.jdk == jdk.version || pin_major == Some(jdk.major))
    }) {
        bail!(
            "{} pins {} JDK {}; use --force to remove it anyway",
            manifest.project.name,
            pin.vendor,
            pin.jdk
        );
    }
    Ok(())
}

fn manifest_toolchain_request(
    manifest: &Manifest,
) -> Option<jman_build::toolchain::ToolchainRequest> {
    manifest
        .toolchain
        .as_ref()
        .map(|toolchain| jman_build::toolchain::ToolchainRequest {
            version: toolchain.jdk.clone(),
            vendor: jman_build::toolchain::normalize_vendor(&toolchain.vendor),
        })
}

fn default_cache_dir() -> PathBuf {
    std::env::var_os("JMAN_CACHE_DIR").map_or_else(
        || {
            dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from(".jman-cache"))
                .join("jman")
        },
        PathBuf::from,
    )
}

async fn init_project(arguments: &Init, ui: &Ui) -> Result<()> {
    let target = absolute_path(&arguments.path)?;
    let detection = ui.activity(format!("Inspecting {}", target.display()));
    let pom_path = target.join("pom.xml");
    if pom_path.is_file() {
        detection.finish("Maven project detected");
        if arguments.import || confirm_maven_import(&target)? {
            return import_maven(&pom_path, ui).await;
        }
        bail!("initialization cancelled; the Maven project was not modified");
    }
    drop(detection);
    if arguments.import {
        bail!("cannot import {}: no pom.xml was found", target.display());
    }
    scaffold_project(arguments, &target, ui).await
}

#[allow(clippy::too_many_lines)]
async fn scaffold_project(arguments: &Init, target: &Path, ui: &Ui) -> Result<()> {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != ".")
        .context("project directory must have a valid UTF-8 name")?;
    validate_scaffold_name("project", name)?;
    for module in &arguments.modules {
        validate_scaffold_name("module", module)?;
    }
    if target.exists() {
        let mut entries = std::fs::read_dir(target)
            .with_context(|| format!("could not inspect {}", target.display()))?;
        if entries.next().transpose()?.is_some() {
            bail!(
                "refusing to initialize non-empty directory {}",
                target.display()
            );
        }
    }
    let activity = ui.activity(format!("Creating project {}", target.display()));
    let root_manifest = scaffold_manifest(
        &arguments.group,
        name,
        &arguments.version,
        arguments.java,
        if arguments.modules.is_empty() {
            "jar"
        } else {
            "pom"
        },
        arguments.modules.clone(),
        if arguments.lib || !arguments.modules.is_empty() {
            None
        } else {
            Some(arguments.main_class.clone().unwrap_or_else(|| {
                format!(
                    "{}.{}.Application",
                    arguments.group,
                    java_package_segment(name)
                )
            }))
        },
    )?;
    let mut projects = vec![(target.to_owned(), root_manifest)];
    for module in &arguments.modules {
        projects.push((
            target.join(module),
            scaffold_manifest(
                &arguments.group,
                module,
                &arguments.version,
                arguments.java,
                "jar",
                Vec::new(),
                None,
            )?,
        ));
    }
    let mut files = Vec::<(PathBuf, String)>::new();
    for (directory, manifest) in &projects {
        let manifest_text = manifest.to_toml().context("could not generate jman.toml")?;
        files.push((directory.join("jman.toml"), manifest_text.clone()));
        files.push((
            directory.join("jman.lock"),
            empty_lock(&manifest_text, arguments.java)
                .to_toml()
                .context("could not generate jman.lock")?,
        ));
        let package = format!(
            "{}/{}",
            arguments.group.replace('.', "/"),
            java_package_segment(&manifest.project.name)
        );
        let main_root = directory.join("src/main/java").join(&package);
        let test_root = directory.join("src/test/java").join(&package);
        files.push((main_root.join(".gitkeep"), String::new()));
        files.push((test_root.join(".gitkeep"), String::new()));
        files.push((directory.join("src/main/resources/.gitkeep"), String::new()));
        files.push((directory.join("src/test/resources/.gitkeep"), String::new()));
        if let Some(main_class) = &manifest.project.main_class {
            let (class_package, class_name) = main_class.rsplit_once('.').with_context(|| {
                format!("main class `{main_class}` must include a Java package")
            })?;
            let source_root = directory
                .join("src/main/java")
                .join(class_package.replace('.', "/"));
            files.push((
                source_root.join(format!("{class_name}.java")),
                format!(
                    "package {class_package};\n\npublic final class {class_name} {{\n\
                     \x20   private {class_name}() {{}}\n\n\
                     \x20   public static void main(String[] args) {{\n\
                     \x20       System.out.println(\"Hello from {}!\");\n\
                     \x20   }}\n}}\n",
                    manifest.project.name
                ),
            ));
        }
    }
    files.push((
        target.join(".gitignore"),
        ".jman/\n*.class\n*.log\n".to_owned(),
    ));
    for (path, _) in &files {
        ensure_absent(path)?;
    }
    for (path, contents) in files {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        tokio::fs::write(&path, contents)
            .await
            .with_context(|| format!("could not write {}", path.display()))?;
    }
    activity.finish(format!("Created {}", target.display()));
    ui.success(format!(
        "Initialized {} {}",
        if arguments.modules.is_empty() {
            if arguments.lib {
                "library"
            } else {
                "application"
            }
        } else {
            "workspace"
        },
        name
    ));
    Ok(())
}

fn scaffold_manifest(
    group: &str,
    name: &str,
    version: &str,
    java_release: u16,
    packaging: &str,
    modules: Vec<String>,
    main_class: Option<String>,
) -> Result<Manifest> {
    let manifest = Manifest {
        manifest_version: MANIFEST_VERSION,
        project: Project {
            group: group.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            java_release,
            packaging: packaging.to_owned(),
            modules,
            main_class,
        },
        toolchain: Some(ManifestToolchain {
            jdk: java_release.to_string(),
            vendor: "temurin".to_owned(),
        }),
        maven: None,
        build: Some(Build {
            encoding: "UTF-8".to_owned(),
            compiler_args: Vec::new(),
        }),
        test: None,
        publishing: None,
        audit: None,
        repositories: Vec::new(),
        dependencies: Dependencies::default(),
        annotation_processors: BTreeMap::new(),
        path_dependencies: BTreeMap::new(),
    };
    manifest.validate()?;
    Ok(manifest)
}

fn empty_lock(manifest: &str, java_release: u16) -> Lockfile {
    Lockfile {
        lock_version: LOCK_VERSION,
        manifest_hash: sha256(manifest.as_bytes()),
        workspace_hash: sha256(manifest.as_bytes()),
        platform: platform_id(),
        toolchain: LockedToolchain { java_release },
        packages: Vec::new(),
        classpath: LockedClasspaths::default(),
    }
}

fn validate_scaffold_name(kind: &str, name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("invalid {kind} name `{name}`");
    }
    Ok(())
}

fn java_package_segment(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn confirm_maven_import(target: &Path) -> Result<bool> {
    if !io::stdin().is_terminal() {
        bail!(
            "Maven project detected at {}; rerun with `jman init --import {}` to import it non-interactively",
            target.display(),
            target.display()
        );
    }
    print!(
        "Maven project detected at {}. Import it into JMAN? [Y/n] ",
        target.display()
    );
    io::stdout().flush().context("could not display prompt")?;
    let mut response = String::new();
    io::stdin()
        .read_line(&mut response)
        .context("could not read import response")?;
    Ok(matches!(
        response.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum MavenModelSource {
    Wrapper(PathBuf),
    System(PathBuf),
    Native,
}

impl MavenModelSource {
    fn description(&self) -> String {
        match self {
            Self::Wrapper(path) => format!("project wrapper {}", path.display()),
            Self::System(path) => format!("system Maven {}", path.display()),
            Self::Native => "JMAN native Maven importer".to_owned(),
        }
    }

    fn executable(&self) -> Option<&Path> {
        match self {
            Self::Wrapper(path) | Self::System(path) => Some(path),
            Self::Native => None,
        }
    }
}

fn select_maven_model_source(root: &Path, path: Option<&OsStr>) -> MavenModelSource {
    let wrappers: &[&str] = if cfg!(windows) {
        &["mvnw.cmd", "mvnw.bat", "mvnw"]
    } else {
        &["mvnw"]
    };
    if let Some(wrapper) = wrappers
        .iter()
        .map(|name| root.join(name))
        .find(|candidate| candidate.is_file())
    {
        return MavenModelSource::Wrapper(wrapper);
    }

    let executables: &[&str] = if cfg!(windows) {
        &["mvn.cmd", "mvn.bat", "mvn.exe", "mvn"]
    } else {
        &["mvn"]
    };
    if let Some(executable) = path
        .into_iter()
        .flat_map(std::env::split_paths)
        .flat_map(|directory| executables.iter().map(move |name| directory.join(name)))
        .find(|candidate| candidate.is_file())
    {
        MavenModelSource::System(executable)
    } else {
        MavenModelSource::Native
    }
}

async fn export_effective_maven_model(executable: &Path, root: &Path) -> Result<String> {
    static IMPORT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let state = default_cache_dir().join("imports").join(format!(
        "maven-{}-{}",
        std::process::id(),
        IMPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    tokio::fs::create_dir_all(&state)
        .await
        .with_context(|| format!("could not create Maven import state {}", state.display()))?;
    let effective_pom = state.join("effective-pom.xml");
    let mut command = if cfg!(windows)
        && executable
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| matches!(extension, "cmd" | "bat"))
    {
        let mut command = tokio::process::Command::new("cmd");
        command.arg("/c").arg(executable);
        command
    } else {
        tokio::process::Command::new(executable)
    };
    let output = command
        .current_dir(root)
        .kill_on_drop(true)
        .args([
            "--batch-mode",
            "--no-transfer-progress",
            "-q",
            "-Dstyle.color=never",
            "help:effective-pom",
        ])
        .arg(format!("-Doutput={}", effective_pom.display()))
        .output()
        .await
        .with_context(|| format!("could not start Maven importer {}", executable.display()))?;
    if !output.status.success() {
        let _ = tokio::fs::remove_dir_all(&state).await;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let diagnostic = [stdout.trim(), stderr.trim()]
            .into_iter()
            .filter(|message| !message.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        bail!(
            "Maven model export with {} failed with status {}: {}",
            executable.display(),
            output.status,
            diagnostic
        );
    }
    let model = tokio::fs::read_to_string(&effective_pom)
        .await
        .with_context(|| {
            format!(
                "Maven completed without producing effective model {}",
                effective_pom.display()
            )
        });
    let _ = tokio::fs::remove_dir_all(&state).await;
    model
}

#[allow(clippy::too_many_lines)]
async fn import_maven(pom_path: &Path, ui: &Ui) -> Result<()> {
    let root_directory = pom_path
        .parent()
        .context("Maven reactor root POM has no parent directory")?;
    let model_source =
        select_maven_model_source(root_directory, std::env::var_os("PATH").as_deref());
    let model_activity = ui.activity(format!(
        "Constructing effective Maven reactor with {}",
        model_source.description()
    ));
    let bootstrap_repository = RepositoryClient::with_default_cache(Vec::new())
        .context("could not initialize Maven repository client")?;
    let bootstrap_resolver = DependencyResolver::new(bootstrap_repository);
    let maven_authoritative = model_source.executable().is_some();
    let projects = if let Some(executable) = model_source.executable() {
        let model = export_effective_maven_model(executable, root_directory).await?;
        bootstrap_resolver
            .import_effective_workspace(&model, root_directory)
            .await
            .context("could not translate Maven's effective reactor")?
    } else {
        bootstrap_resolver
            .import_workspace(pom_path)
            .await
            .context("could not construct the Maven reactor with JMAN's native importer")?
    };
    model_activity.finish(format!(
        "Loaded Maven reactor through {} ({} modules)",
        model_source.description(),
        projects.len()
    ));
    let mut detected_plugins = BTreeSet::new();
    for project in &projects {
        let xml = tokio::fs::read_to_string(&project.source)
            .await
            .with_context(|| format!("could not read {}", project.source.display()))?;
        for plugin in plugin_coordinates(&xml)
            .with_context(|| format!("could not inspect plugins in {}", project.source.display()))?
        {
            detected_plugins.insert(plugin);
        }
    }
    let root = projects
        .first()
        .context("Maven reactor did not contain a root project")?;
    let root_directory = root
        .source
        .parent()
        .context("Maven reactor root POM has no parent directory")?;
    let manifests = projects
        .iter()
        .map(|project| {
            manifest_from_maven(&project.effective, maven_authoritative).and_then(|mut manifest| {
                let directory = project
                    .source
                    .parent()
                    .context("Maven module POM has no parent directory")?;
                for dependency in &project.effective.dependencies {
                    if let Some(module) = projects.iter().find(|module| {
                        module.effective.coordinate.group == dependency.group
                            && module.effective.coordinate.artifact == dependency.artifact
                            && dependency.version.as_deref()
                                == Some(&module.effective.coordinate.version)
                    }) {
                        let module_directory = module
                            .source
                            .parent()
                            .context("Maven module POM has no parent directory")?;
                        let workspace_path = module_directory
                            .strip_prefix(root_directory)
                            .unwrap_or(module_directory)
                            .to_string_lossy()
                            .into_owned();
                        manifest
                            .path_dependencies
                            .insert(dependency.ga(), workspace_path);
                    }
                }
                Ok((project, directory.to_owned(), manifest))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let workspace_hash = hash_generated_manifests(root_directory, &manifests)?;
    for (_, directory, _) in &manifests {
        ensure_absent(&directory.join("jman.toml"))?;
        ensure_absent(&directory.join("jman.lock"))?;
    }
    let repositories = manifests
        .iter()
        .flat_map(|(_, _, manifest)| &manifest.repositories)
        .map(|repository| repository.url.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let repository = RepositoryClient::with_default_cache(repositories)
        .context("could not initialize Maven repositories")?;
    let resolver = DependencyResolver::new(repository);

    let dependency_activity = ui.activity(format!(
        "Resolving and downloading dependencies for {} modules",
        projects.len()
    ));
    let generation =
        manifests
            .into_iter()
            .enumerate()
            .map(|(index, (project, directory, manifest))| {
                let resolver = &resolver;
                let projects = &projects;
                let workspace_hash = &workspace_hash;
                async move {
                    let manifest_text =
                        manifest.to_toml().context("could not generate jman.toml")?;
                    let graph = resolver
                        .resolve_in_workspace(&project.effective, projects)
                        .await
                        .with_context(|| {
                            format!(
                                "dependency resolution failed for {}",
                                project.effective.coordinate
                            )
                        })?;
                    let lock = lock_from_graph(
                        &manifest_text,
                        workspace_hash,
                        manifest.project.java_release,
                        graph,
                    );
                    let lock_text = lock.to_toml().context("could not generate jman.lock")?;
                    Ok::<_, anyhow::Error>((
                        index,
                        (directory, manifest, manifest_text, lock, lock_text),
                    ))
                }
            });
    let mut generated = stream::iter(generation)
        .buffer_unordered(4)
        .try_collect::<Vec<_>>()
        .await?;
    generated.sort_unstable_by_key(|(index, _)| *index);
    let generated = generated
        .into_iter()
        .map(|(_, generated)| generated)
        .collect::<Vec<_>>();
    let package_count = generated
        .iter()
        .map(|(_, _, _, lock, _)| lock.packages.len())
        .sum::<usize>();
    dependency_activity.finish(format!("Resolved {package_count} module dependencies"));
    let write_activity = ui.activity("Writing JMAN manifests and lockfiles");
    write_workspace_atomically(&generated).await?;
    write_activity.finish("Wrote manifests and lockfiles");
    for (directory, manifest, _, lock, _) in &generated {
        let relative = directory.strip_prefix(root_directory).unwrap_or(directory);
        let module = if relative.as_os_str().is_empty() {
            ".".to_owned()
        } else {
            relative.display().to_string()
        };
        ui.detail(format!(
            "{module}: {}:{}:{} ({} packages)",
            manifest.project.group,
            manifest.project.name,
            manifest.project.version,
            lock.packages.len()
        ));
    }
    ui.success(format!(
        "Imported {} modules with {package_count} resolved dependencies",
        projects.len()
    ));
    if !detected_plugins.is_empty() {
        ui.warning(format!(
            "Maven plugins were detected but were not imported: {}. \
             JMAN does not execute or translate Maven plugins.",
            detected_plugins.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(())
}

async fn write_workspace_atomically(
    generated: &[(PathBuf, Manifest, String, Lockfile, String)],
) -> Result<()> {
    let mut staged = Vec::with_capacity(generated.len() * 2);
    for (directory, _, manifest, _, lock) in generated {
        for (target, contents) in [
            (directory.join("jman.toml"), manifest),
            (directory.join("jman.lock"), lock),
        ] {
            let temporary = staging_path(&target);
            if let Err(error) = tokio::fs::write(&temporary, contents).await {
                for (_, staged_path) in &staged {
                    let _ = tokio::fs::remove_file(staged_path).await;
                }
                return Err(error).with_context(|| format!("could not stage {}", target.display()));
            }
            staged.push((target, temporary));
        }
    }
    let mut committed = Vec::new();
    for (target, temporary) in &staged {
        if let Err(error) = tokio::fs::rename(temporary, target).await {
            for (_, staged_path) in &staged {
                let _ = tokio::fs::remove_file(staged_path).await;
            }
            for committed_path in committed {
                let _ = tokio::fs::remove_file(committed_path).await;
            }
            return Err(error).with_context(|| format!("could not commit {}", target.display()));
        }
        committed.push(target);
    }
    Ok(())
}

fn staging_path(target: &Path) -> PathBuf {
    let file_name = target.file_name().map_or_else(
        || "jman".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    target.with_file_name(format!("{file_name}.tmp-{}", std::process::id()))
}

async fn add_dependency(arguments: &Add, ui: &Ui) -> Result<()> {
    let target = absolute_path(&arguments.path)?;
    let manifest_path = target.join("jman.toml");
    let original = tokio::fs::read_to_string(&manifest_path)
        .await
        .with_context(|| format!("could not read {}", manifest_path.display()))?;
    let mut manifest = Manifest::read(&manifest_path)
        .with_context(|| format!("could not read {}", manifest_path.display()))?;
    let (ga, version) = parse_add_coordinate(&arguments.dependency)?;
    if manifest_contains_dependency(&manifest, ga) {
        bail!("dependency `{ga}` is already declared; remove it before changing its scope");
    }
    manifest_scope_mut(&mut manifest, arguments.scope).insert(ga.to_owned(), version.to_owned());
    let updated = manifest
        .to_toml()
        .context("could not serialize jman.toml")?;
    write_atomically(&manifest_path, &updated).await?;
    let sync = Sync {
        path: target,
        report: ReportFormat::Human,
        refresh: true,
        offline: arguments.offline,
    };
    if let Err(error) = sync_project(&sync, ui).await {
        write_atomically(&manifest_path, &original)
            .await
            .context("dependency resolution failed and jman.toml could not be restored")?;
        return Err(error).context("dependency was not added; jman.toml was restored");
    }
    ui.success(format!(
        "Added {ga}@{version} to {} dependencies",
        arguments.scope.name()
    ));
    Ok(())
}

async fn remove_dependency(arguments: &Remove, ui: &Ui) -> Result<()> {
    let target = absolute_path(&arguments.path)?;
    let manifest_path = target.join("jman.toml");
    let original = tokio::fs::read_to_string(&manifest_path)
        .await
        .with_context(|| format!("could not read {}", manifest_path.display()))?;
    let mut manifest = Manifest::read(&manifest_path)
        .with_context(|| format!("could not read {}", manifest_path.display()))?;
    split_ga(&arguments.dependency)?;
    let mut removed = false;
    for dependencies in [
        &mut manifest.dependencies.compile,
        &mut manifest.dependencies.runtime,
        &mut manifest.dependencies.provided,
        &mut manifest.dependencies.test,
        &mut manifest.annotation_processors,
    ] {
        removed |= dependencies.remove(&arguments.dependency).is_some();
    }
    if !removed {
        bail!("dependency `{}` is not declared", arguments.dependency);
    }
    if let Some(maven) = &mut manifest.maven {
        for scope in ["compile", "runtime", "provided", "test"] {
            maven
                .dependencies
                .remove(&format!("{scope}:{}", arguments.dependency));
        }
    }
    manifest.path_dependencies.remove(&arguments.dependency);
    let updated = manifest
        .to_toml()
        .context("could not serialize jman.toml")?;
    write_atomically(&manifest_path, &updated).await?;
    let sync = Sync {
        path: target,
        report: ReportFormat::Human,
        refresh: true,
        offline: arguments.offline,
    };
    if let Err(error) = sync_project(&sync, ui).await {
        write_atomically(&manifest_path, &original)
            .await
            .context("dependency resolution failed and jman.toml could not be restored")?;
        return Err(error).context("dependency was not removed; jman.toml was restored");
    }
    ui.success(format!("Removed {}", arguments.dependency));
    Ok(())
}

fn dependency_tree(path: &Path) -> Result<()> {
    let target = absolute_path(path)?;
    let manifest = Manifest::read(&target.join("jman.toml"))
        .with_context(|| format!("could not read {}", target.join("jman.toml").display()))?;
    let lock = Lockfile::read(&target.join("jman.lock"))
        .with_context(|| format!("could not read {}", target.join("jman.lock").display()))?;
    println!(
        "{}:{}:{}",
        manifest.project.group, manifest.project.name, manifest.project.version
    );
    let packages = lock
        .packages
        .iter()
        .map(|package| (locked_coordinate(package), package))
        .collect::<BTreeMap<_, _>>();
    let mut seen = std::collections::BTreeSet::new();
    for dependency in locked_root_dependencies(&lock) {
        print_tree_node(&dependency, "", true, &packages, &mut seen);
    }
    Ok(())
}

fn print_tree_node(
    coordinate: &str,
    prefix: &str,
    last: bool,
    packages: &BTreeMap<String, &LockedPackage>,
    seen: &mut std::collections::BTreeSet<String>,
) {
    let branch = if last { "└── " } else { "├── " };
    if !seen.insert(coordinate.to_owned()) {
        println!("{prefix}{branch}{coordinate} (*)");
        return;
    }
    println!("{prefix}{branch}{coordinate}");
    let Some(package) = packages.get(coordinate) else {
        return;
    };
    let child_prefix = format!("{prefix}{}", if last { "    " } else { "│   " });
    for (index, child) in package.dependencies.iter().enumerate() {
        print_tree_node(
            child,
            &child_prefix,
            index + 1 == package.dependencies.len(),
            packages,
            seen,
        );
    }
}

fn dependency_why(arguments: &Why) -> Result<()> {
    split_ga(&arguments.dependency)?;
    let target = absolute_path(&arguments.path)?;
    let manifest = Manifest::read(&target.join("jman.toml"))
        .with_context(|| format!("could not read {}", target.join("jman.toml").display()))?;
    let lock = Lockfile::read(&target.join("jman.lock"))
        .with_context(|| format!("could not read {}", target.join("jman.lock").display()))?;
    let root = format!(
        "{}:{}:{}",
        manifest.project.group, manifest.project.name, manifest.project.version
    );
    let paths = shortest_dependency_paths(&lock)
        .into_iter()
        .filter(|(coordinate, _)| coordinate_ga(coordinate) == arguments.dependency)
        .flat_map(|(_, paths)| paths)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if paths.is_empty() {
        bail!(
            "dependency `{}` is not present in jman.lock",
            arguments.dependency
        );
    }
    println!(
        "Dependency paths for {}:\n{}",
        terminal_text(&arguments.dependency),
        dependency_path_tree(&root, &paths, "")
    );
    Ok(())
}

fn manifest_scope_mut(
    manifest: &mut Manifest,
    scope: DependencyScope,
) -> &mut BTreeMap<String, String> {
    match scope {
        DependencyScope::Compile => &mut manifest.dependencies.compile,
        DependencyScope::Runtime => &mut manifest.dependencies.runtime,
        DependencyScope::Provided => &mut manifest.dependencies.provided,
        DependencyScope::Test => &mut manifest.dependencies.test,
        DependencyScope::Processor => &mut manifest.annotation_processors,
    }
}

fn manifest_contains_dependency(manifest: &Manifest, coordinate: &str) -> bool {
    [
        &manifest.dependencies.compile,
        &manifest.dependencies.runtime,
        &manifest.dependencies.provided,
        &manifest.dependencies.test,
        &manifest.annotation_processors,
    ]
    .iter()
    .any(|dependencies| dependencies.contains_key(coordinate))
}

fn parse_add_coordinate(value: &str) -> Result<(&str, &str)> {
    let (ga, version) = value
        .rsplit_once('@')
        .filter(|(_, version)| !version.is_empty())
        .with_context(|| {
            format!("dependency `{value}` must use group:artifact@version notation")
        })?;
    split_ga(ga)?;
    Ok((ga, version))
}

fn locked_coordinate(package: &LockedPackage) -> String {
    Coordinate {
        group: package.group.clone(),
        artifact: package.artifact.clone(),
        version: package.version.clone(),
        extension: package.extension.clone(),
        classifier: package.classifier.clone(),
    }
    .to_string()
}

fn coordinate_ga(coordinate: &str) -> String {
    let parts = coordinate.split(':').collect::<Vec<_>>();
    format!(
        "{}:{}",
        parts.first().copied().unwrap_or_default(),
        parts.get(1).copied().unwrap_or_default()
    )
}

#[derive(Clone, Debug)]
struct OutdatedRequest {
    dependency: String,
    current: String,
    scopes: BTreeSet<String>,
    modules: BTreeSet<String>,
    repositories: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum OutdatedStatus {
    Outdated,
    UpToDate,
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OutdatedDependencyReport {
    dependency: String,
    current: String,
    status: OutdatedStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    change: Option<jman_resolver::VersionChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    minor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    major: Option<String>,
    scopes: Vec<String>,
    modules: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OutdatedReport {
    checked: usize,
    outdated: usize,
    up_to_date: usize,
    unavailable: usize,
    dependencies: Vec<OutdatedDependencyReport>,
}

async fn outdated_dependencies(arguments: &Outdated, ui: &Ui) -> Result<()> {
    let target = absolute_path(&arguments.path)?;
    if !target.join("jman.toml").is_file() {
        bail!(
            "cannot inspect {}: jman.toml was not found",
            target.display()
        );
    }
    let workspace_root = find_workspace_root(&target);
    let requests = collect_outdated_requests(&workspace_root).await?;
    let checked = requests.len();
    let activity = ui.activity(format!(
        "Checking {} direct dependenc{}",
        checked,
        if checked == 1 { "y" } else { "ies" }
    ));
    let report =
        build_outdated_report(requests, arguments.offline, arguments.include_prerelease).await;
    activity.finish(format!("Checked {} direct dependencies", report.checked));
    match arguments.format {
        ReportFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        ReportFormat::Human => print_outdated_report(&report, ui),
    }
    Ok(())
}

async fn build_outdated_report(
    requests: BTreeMap<(String, String), OutdatedRequest>,
    offline: bool,
    include_prerelease: bool,
) -> OutdatedReport {
    let checks = requests.into_values().map(|request| async move {
        check_outdated_dependency(request, offline, include_prerelease).await
    });
    let mut dependencies = stream::iter(checks)
        .buffer_unordered(8)
        .collect::<Vec<_>>()
        .await;
    dependencies.sort_by(|left, right| {
        (&left.dependency, &left.current).cmp(&(&right.dependency, &right.current))
    });
    let outdated = dependencies
        .iter()
        .filter(|dependency| dependency.status == OutdatedStatus::Outdated)
        .count();
    let up_to_date = dependencies
        .iter()
        .filter(|dependency| dependency.status == OutdatedStatus::UpToDate)
        .count();
    let unavailable = dependencies
        .iter()
        .filter(|dependency| dependency.status == OutdatedStatus::Unavailable)
        .count();
    OutdatedReport {
        checked: dependencies.len(),
        outdated,
        up_to_date,
        unavailable,
        dependencies,
    }
}

async fn collect_outdated_requests(
    root: &Path,
) -> Result<BTreeMap<(String, String), OutdatedRequest>> {
    let directories = workspace_directories(root).await?;
    let mut requests = BTreeMap::new();
    for directory in directories {
        let manifest_path = directory.join("jman.toml");
        let manifest = Manifest::read(&manifest_path)
            .with_context(|| format!("could not read {}", manifest_path.display()))?;
        let workspace_dependencies = manifest
            .path_dependencies
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for (scope, dependencies) in [
            ("compile", &manifest.dependencies.compile),
            ("runtime", &manifest.dependencies.runtime),
            ("provided", &manifest.dependencies.provided),
            ("test", &manifest.dependencies.test),
            ("processor", &manifest.annotation_processors),
        ] {
            for (dependency, current) in dependencies {
                if workspace_dependencies.contains(dependency.as_str()) {
                    continue;
                }
                let key = (dependency.clone(), current.clone());
                let request = requests.entry(key).or_insert_with(|| OutdatedRequest {
                    dependency: dependency.clone(),
                    current: current.clone(),
                    scopes: BTreeSet::new(),
                    modules: BTreeSet::new(),
                    repositories: Vec::new(),
                });
                request.scopes.insert(scope.to_owned());
                request.modules.insert(manifest.project.name.clone());
                if manifest.repositories.is_empty() {
                    let central = jman_resolver::MAVEN_CENTRAL_URL.to_owned();
                    if !request.repositories.contains(&central) {
                        request.repositories.push(central);
                    }
                } else {
                    for repository in &manifest.repositories {
                        if !request.repositories.contains(&repository.url) {
                            request.repositories.push(repository.url.clone());
                        }
                    }
                }
            }
        }
    }
    Ok(requests)
}

async fn check_outdated_dependency(
    request: OutdatedRequest,
    offline: bool,
    include_prerelease: bool,
) -> OutdatedDependencyReport {
    let result = async {
        let (group, artifact) = split_ga(&request.dependency)?;
        validate_outdated_version(&request.current)?;
        let repository =
            RepositoryClient::with_default_cache_mode(request.repositories.clone(), offline)
                .context("could not initialize Maven repositories")?;
        let catalog = repository
            .available_versions(group, artifact)
            .await
            .context("could not load Maven version metadata")?;
        let mut versions = catalog.versions;
        for advertised in [catalog.release, catalog.latest].into_iter().flatten() {
            if !versions.contains(&advertised) {
                versions.push(advertised);
            }
        }
        let updates =
            jman_resolver::analyze_versions(&request.current, &versions, include_prerelease);
        Ok::<_, anyhow::Error>((updates, catalog.source))
    }
    .await;
    let scopes = request.scopes.into_iter().collect();
    let modules = request.modules.into_iter().collect();
    match result {
        Ok((updates, source)) => OutdatedDependencyReport {
            dependency: request.dependency,
            current: request.current,
            status: if updates.latest.is_some() {
                OutdatedStatus::Outdated
            } else {
                OutdatedStatus::UpToDate
            },
            latest: updates.latest,
            change: updates.change,
            patch: updates.patch,
            minor: updates.minor,
            major: updates.major,
            scopes,
            modules,
            source: Some(redact_repository_source(&source)),
            error: None,
        },
        Err(error) => OutdatedDependencyReport {
            dependency: request.dependency,
            current: request.current,
            status: OutdatedStatus::Unavailable,
            latest: None,
            change: None,
            patch: None,
            minor: None,
            major: None,
            scopes,
            modules,
            source: None,
            error: Some(format!("{error:#}")),
        },
    }
}

fn validate_outdated_version(version: &str) -> Result<()> {
    let normalized = version.trim().to_ascii_uppercase();
    if normalized.is_empty()
        || normalized.contains(['[', ']', '(', ')', ',', '*'])
        || normalized.ends_with(".+")
        || normalized.contains("${")
        || matches!(normalized.as_str(), "LATEST" | "RELEASE")
    {
        bail!("dependency version `{version}` is not an exact version");
    }
    Ok(())
}

fn redact_repository_source(source: &str) -> String {
    let Some((scheme, remainder)) = source.split_once("://") else {
        return source.to_owned();
    };
    let Some((_, host_and_path)) = remainder.rsplit_once('@') else {
        return source.to_owned();
    };
    format!("{scheme}://***@{host_and_path}")
}

fn print_outdated_report(report: &OutdatedReport, ui: &Ui) {
    if report.dependencies.is_empty() {
        println!("No external direct dependencies found.");
        return;
    }
    print!("{}", outdated_table(&report.dependencies));
    println!();
    println!(
        "{} outdated, {} up to date, {} unavailable",
        report.outdated, report.up_to_date, report.unavailable
    );
    for dependency in &report.dependencies {
        if dependency.source.as_deref() == Some("stale-cache") {
            ui.warning(format!(
                "{}: repository metadata was unavailable; using the last cached catalog",
                dependency.dependency
            ));
        }
        if let Some(error) = &dependency.error {
            ui.warning(format!("{}: {error}", dependency.dependency));
        }
    }
}

fn outdated_table(dependencies: &[OutdatedDependencyReport]) -> String {
    let rows = dependencies
        .iter()
        .map(|dependency| {
            let state = match dependency.status {
                OutdatedStatus::Outdated => {
                    dependency.change.map_or("other", |change| match change {
                        jman_resolver::VersionChange::Patch => "patch",
                        jman_resolver::VersionChange::Minor => "minor",
                        jman_resolver::VersionChange::Major => "major",
                        jman_resolver::VersionChange::Other => "other",
                    })
                }
                OutdatedStatus::UpToDate => "current",
                OutdatedStatus::Unavailable => "unavailable",
            };
            [
                dependency.dependency.clone(),
                dependency.current.clone(),
                dependency.latest.clone().unwrap_or_else(|| "-".to_owned()),
                state.to_owned(),
                dependency.scopes.join(","),
                dependency.modules.join(","),
            ]
        })
        .collect::<Vec<_>>();
    let headers = [
        "DEPENDENCY",
        "CURRENT",
        "LATEST",
        "CHANGE",
        "SCOPES",
        "MODULES",
    ];
    let widths = std::array::from_fn::<_, 6, _>(|index| {
        rows.iter()
            .map(|row| row[index].chars().count())
            .max()
            .unwrap_or_default()
            .max(headers[index].len())
    });
    let mut output = String::new();
    writeln!(
        output,
        "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}  {}",
        headers[0],
        headers[1],
        headers[2],
        headers[3],
        headers[4],
        headers[5],
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
        w3 = widths[3],
        w4 = widths[4],
    )
    .expect("writing a String cannot fail");
    writeln!(
        output,
        "{:-<w0$}  {:-<w1$}  {:-<w2$}  {:-<w3$}  {:-<w4$}  {:-<w5$}",
        "",
        "",
        "",
        "",
        "",
        "",
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
        w3 = widths[3],
        w4 = widths[4],
        w5 = widths[5],
    )
    .expect("writing a String cannot fail");
    for row in rows {
        writeln!(
            output,
            "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}  {}",
            row[0],
            row[1],
            row[2],
            row[3],
            row[4],
            row[5],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
            w3 = widths[3],
            w4 = widths[4],
        )
        .expect("writing a String cannot fail");
    }
    output
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DependencyUpdate {
    dependency: String,
    from: String,
    to: String,
    change: jman_resolver::VersionChange,
    scopes: Vec<String>,
    modules: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DependencyUpdateReport {
    level: UpdateLevel,
    dry_run: bool,
    applied: bool,
    updated: usize,
    changes: Vec<DependencyUpdate>,
}

#[derive(Debug)]
struct WorkspaceFileSnapshot {
    path: PathBuf,
    contents: Option<String>,
}

async fn update_dependencies(arguments: &Update, ui: &Ui) -> Result<()> {
    let target = absolute_path(&arguments.path)?;
    if !target.join("jman.toml").is_file() {
        bail!(
            "cannot update {}: jman.toml was not found",
            target.display()
        );
    }
    if let Some(dependency) = &arguments.dependency {
        split_ga(dependency)?;
    }
    let workspace_root = find_workspace_root(&target);
    let requests = collect_outdated_requests(&workspace_root).await?;
    let activity = ui.activity("Planning dependency updates");
    let mut outdated =
        build_outdated_report(requests, arguments.offline, arguments.include_prerelease)
            .await
            .dependencies;
    if let Some(dependency) = &arguments.dependency {
        outdated.retain(|candidate| candidate.dependency == *dependency);
        if outdated.is_empty() {
            bail!("external dependency `{dependency}` is not declared in this workspace");
        }
    }
    let unavailable = outdated
        .iter()
        .filter(|dependency| dependency.status == OutdatedStatus::Unavailable)
        .map(|dependency| {
            format!(
                "{}@{} ({})",
                dependency.dependency,
                dependency.current,
                dependency
                    .error
                    .as_deref()
                    .unwrap_or("metadata unavailable")
            )
        })
        .collect::<Vec<_>>();
    if !unavailable.is_empty() {
        bail!(
            "cannot create a complete update plan: {}",
            unavailable.join("; ")
        );
    }
    for dependency in &outdated {
        if dependency.source.as_deref() == Some("stale-cache") {
            ui.warning(format!(
                "{}: repository metadata was unavailable; planning from the last cached catalog",
                dependency.dependency
            ));
        }
    }
    let changes = outdated
        .iter()
        .filter_map(|dependency| select_dependency_update(dependency, arguments.level))
        .collect::<Vec<_>>();
    activity.finish(format!("Planned {} dependency updates", changes.len()));
    let mut report = DependencyUpdateReport {
        level: arguments.level,
        dry_run: arguments.dry_run,
        applied: false,
        updated: changes.len(),
        changes,
    };
    if arguments.dry_run || report.changes.is_empty() {
        print_dependency_update_report(&report, arguments.format, ui)?;
        return Ok(());
    }

    let directories = workspace_directories(&workspace_root).await?;
    let snapshots = snapshot_workspace_files(&directories).await?;
    let result = async {
        apply_dependency_update_plan(&directories, &report.changes).await?;
        sync_project(
            &Sync {
                path: workspace_root,
                report: ReportFormat::Human,
                refresh: true,
                offline: arguments.offline,
            },
            ui,
        )
        .await
    }
    .await;
    if let Err(error) = result {
        if let Err(restoration_error) = restore_workspace_files(&snapshots).await {
            bail!(
                "dependency update failed: {error:#}; workspace restoration also failed: \
                 {restoration_error:#}"
            );
        }
        bail!("dependency update failed and workspace files were restored: {error:#}");
    }
    report.applied = true;
    print_dependency_update_report(&report, arguments.format, ui)
}

fn select_dependency_update(
    dependency: &OutdatedDependencyReport,
    level: UpdateLevel,
) -> Option<DependencyUpdate> {
    let selected = match level {
        UpdateLevel::Patch => dependency
            .patch
            .as_ref()
            .map(|version| (version, jman_resolver::VersionChange::Patch)),
        UpdateLevel::Minor => dependency
            .minor
            .as_ref()
            .map(|version| (version, jman_resolver::VersionChange::Minor))
            .or_else(|| {
                dependency
                    .patch
                    .as_ref()
                    .map(|version| (version, jman_resolver::VersionChange::Patch))
            }),
        UpdateLevel::Major => dependency
            .major
            .as_ref()
            .map(|version| (version, jman_resolver::VersionChange::Major))
            .or_else(|| {
                dependency
                    .minor
                    .as_ref()
                    .map(|version| (version, jman_resolver::VersionChange::Minor))
            })
            .or_else(|| {
                dependency
                    .patch
                    .as_ref()
                    .map(|version| (version, jman_resolver::VersionChange::Patch))
            }),
        UpdateLevel::Latest => dependency.latest.as_ref().zip(dependency.change),
    }?;
    Some(DependencyUpdate {
        dependency: dependency.dependency.clone(),
        from: dependency.current.clone(),
        to: selected.0.clone(),
        change: selected.1,
        scopes: dependency.scopes.clone(),
        modules: dependency.modules.clone(),
        source: dependency.source.clone(),
    })
}

async fn apply_dependency_update_plan(
    directories: &[PathBuf],
    changes: &[DependencyUpdate],
) -> Result<()> {
    let versions = changes
        .iter()
        .map(|change| {
            (
                (change.dependency.as_str(), change.from.as_str()),
                change.to.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for directory in directories {
        let manifest_path = directory.join("jman.toml");
        let mut manifest = Manifest::read(&manifest_path)
            .with_context(|| format!("could not read {}", manifest_path.display()))?;
        let mut manifest_changed = false;
        for dependencies in [
            &mut manifest.dependencies.compile,
            &mut manifest.dependencies.runtime,
            &mut manifest.dependencies.provided,
            &mut manifest.dependencies.test,
            &mut manifest.annotation_processors,
        ] {
            for (dependency, current) in dependencies {
                if let Some(version) = versions.get(&(dependency.as_str(), current.as_str())) {
                    *current = (*version).to_owned();
                    manifest_changed = true;
                }
            }
        }
        if manifest_changed {
            let updated = manifest
                .to_toml()
                .context("could not serialize jman.toml")?;
            write_atomically(&manifest_path, &updated).await?;
        }
    }
    Ok(())
}

async fn snapshot_workspace_files(directories: &[PathBuf]) -> Result<Vec<WorkspaceFileSnapshot>> {
    let mut snapshots = Vec::with_capacity(directories.len() * 2);
    for directory in directories {
        for file in ["jman.toml", "jman.lock"] {
            let path = directory.join(file);
            let contents = match tokio::fs::read_to_string(&path).await {
                Ok(contents) => Some(contents),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("could not snapshot {}", path.display()));
                }
            };
            snapshots.push(WorkspaceFileSnapshot { path, contents });
        }
    }
    Ok(snapshots)
}

async fn restore_workspace_files(snapshots: &[WorkspaceFileSnapshot]) -> Result<()> {
    for snapshot in snapshots {
        if let Some(contents) = &snapshot.contents {
            write_atomically(&snapshot.path, contents).await?;
        } else if let Err(error) = tokio::fs::remove_file(&snapshot.path).await {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error).with_context(|| {
                    format!("could not remove generated {}", snapshot.path.display())
                });
            }
        }
    }
    Ok(())
}

fn print_dependency_update_report(
    report: &DependencyUpdateReport,
    format: ReportFormat,
    ui: &Ui,
) -> Result<()> {
    if format == ReportFormat::Json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }
    if report.changes.is_empty() {
        println!("No dependency updates match the selected level.");
        return Ok(());
    }
    println!("{}", dependency_update_table(&report.changes));
    if report.dry_run {
        println!("Dry run: no manifests or lockfiles were changed.");
    } else if report.applied {
        ui.success(format!(
            "Applied {} dependency updates and synchronized the workspace",
            report.updated
        ));
    }
    Ok(())
}

fn dependency_update_table(changes: &[DependencyUpdate]) -> String {
    let rows = changes
        .iter()
        .map(|change| {
            [
                change.dependency.clone(),
                change.from.clone(),
                change.to.clone(),
                format!("{:?}", change.change).to_ascii_lowercase(),
                change.modules.join(","),
            ]
        })
        .collect::<Vec<_>>();
    let headers = ["DEPENDENCY", "FROM", "TO", "CHANGE", "MODULES"];
    let widths = std::array::from_fn::<_, 5, _>(|index| {
        rows.iter()
            .map(|row| row[index].chars().count())
            .max()
            .unwrap_or_default()
            .max(headers[index].len())
    });
    let mut output = String::new();
    writeln!(
        output,
        "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {}",
        headers[0],
        headers[1],
        headers[2],
        headers[3],
        headers[4],
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
        w3 = widths[3],
    )
    .expect("writing a String cannot fail");
    for row in rows {
        writeln!(
            output,
            "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {}",
            row[0],
            row[1],
            row[2],
            row[3],
            row[4],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
            w3 = widths[3],
        )
        .expect("writing a String cannot fail");
    }
    output.trim_end().to_owned()
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditDependencyPath {
    module: String,
    coordinates: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppliedAuditSuppression {
    reason: String,
    expires: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditFindingReport {
    package: jman_audit::AuditPackage,
    advisory: jman_audit::Advisory,
    paths: Vec<AuditDependencyPath>,
    active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    suppression: Option<AppliedAuditSuppression>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DependencyAuditReport {
    provider: String,
    source: jman_audit::AuditSource,
    packages_checked: usize,
    total_findings: usize,
    reported_findings: usize,
    active: usize,
    suppressed: usize,
    denied: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    suppression_warnings: Vec<String>,
    findings: Vec<AuditFindingReport>,
}

#[derive(Debug)]
struct AuditWorkspace {
    packages: Vec<jman_audit::AuditPackage>,
    paths: BTreeMap<jman_audit::AuditPackage, Vec<AuditDependencyPath>>,
    suppressions: Vec<jman_config::AuditSuppression>,
}

async fn audit_dependencies(arguments: &AuditCommand, ui: &Ui) -> Result<()> {
    validate_audit_thresholds(arguments.severity, arguments.deny)?;
    let target = absolute_path(&arguments.path)?;
    if !target.join("jman.toml").is_file() {
        bail!("cannot audit {}: jman.toml was not found", target.display());
    }
    let workspace_root = find_workspace_root(&target);
    let activity = ui.activity("Loading resolved dependency graph");
    let workspace = load_audit_workspace(&workspace_root).await?;
    activity.finish(format!(
        "Loaded {} resolved packages",
        workspace.packages.len()
    ));
    let activity = ui.activity("Querying vulnerability intelligence");
    let provider = std::env::var("JMAN_AUDIT_OSV_URL")
        .map_or_else(
            |_| jman_audit::OsvProvider::new(),
            jman_audit::OsvProvider::with_base_url,
        )
        .context("could not initialize OSV provider")?;
    let auditor = jman_audit::Auditor::new(
        provider,
        jman_audit::AuditOptions::platform_default(arguments.offline, arguments.refresh),
    );
    let result = auditor
        .audit(&workspace.packages)
        .await
        .context("dependency vulnerability audit failed")?;
    activity.finish(format!(
        "Received {} vulnerability findings",
        result.findings.len()
    ));
    if result.source == jman_audit::AuditSource::StaleCache {
        ui.warning("the audit provider was unavailable; using the last cached result");
    }
    let report = evaluate_audit(
        result,
        &workspace.paths,
        &workspace.suppressions,
        arguments.severity.into(),
        arguments.deny.map(Into::into),
        &utc_date(),
    );
    for warning in &report.suppression_warnings {
        ui.warning(terminal_text(warning));
    }
    match arguments.format {
        ReportFormat::Human => print_dependency_audit(&report),
        ReportFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    if report.denied > 0 {
        let threshold = arguments
            .deny
            .expect("denied findings require a configured threshold");
        bail!(
            "audit policy denied {} active finding{} at {:?} severity or higher",
            report.denied,
            if report.denied == 1 { "" } else { "s" },
            threshold
        );
    }
    Ok(())
}

fn validate_audit_thresholds(minimum: AuditSeverity, deny: Option<AuditSeverity>) -> Result<()> {
    if deny.is_some_and(|deny| minimum > deny) {
        bail!("--severity cannot be higher than --deny because denied findings would be hidden");
    }
    Ok(())
}

async fn load_audit_workspace(root: &Path) -> Result<AuditWorkspace> {
    let directories = workspace_directories(root).await?;
    let workspace_hash = hash_workspace_manifests(root).await?;
    let root_manifest = Manifest::read(&root.join("jman.toml"))
        .with_context(|| format!("could not read {}", root.join("jman.toml").display()))?;
    let suppressions = root_manifest
        .audit
        .map_or_else(Vec::new, |audit| audit.suppressions);
    let manifests = directories
        .iter()
        .map(|directory| {
            Manifest::read(&directory.join("jman.toml")).with_context(|| {
                format!("could not read {}", directory.join("jman.toml").display())
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let workspace_packages = manifests
        .iter()
        .map(|manifest| {
            (
                manifest.project.group.clone(),
                manifest.project.name.clone(),
                manifest.project.version.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut packages = BTreeSet::new();
    let mut paths = BTreeMap::<_, BTreeSet<_>>::new();
    for (directory, manifest) in directories.iter().zip(&manifests) {
        let lock_path = directory.join("jman.lock");
        let lock = Lockfile::read(&lock_path).with_context(|| {
            format!(
                "could not read {}; run jman sync before auditing",
                lock_path.display()
            )
        })?;
        let manifest_text = manifest
            .to_toml()
            .context("could not serialize jman.toml for lock validation")?;
        if lock.manifest_hash != sha256(manifest_text.as_bytes())
            || lock.workspace_hash != workspace_hash
            || lock.platform != platform_id()
            || lock.toolchain.java_release != manifest.project.java_release
        {
            bail!(
                "{} is stale for the current workspace; run jman sync before auditing",
                lock_path.display()
            );
        }
        let module_paths = audit_paths_for_lock(&lock, &manifest.project.name);
        for package in &lock.packages {
            if workspace_packages.contains(&(
                package.group.clone(),
                package.artifact.clone(),
                package.version.clone(),
            )) {
                continue;
            }
            let audit_package = jman_audit::AuditPackage {
                group: package.group.clone(),
                artifact: package.artifact.clone(),
                version: package.version.clone(),
            };
            packages.insert(audit_package.clone());
            let coordinate = locked_coordinate(package);
            let package_paths = module_paths.get(&coordinate).cloned().unwrap_or_else(|| {
                BTreeSet::from([AuditDependencyPath {
                    module: manifest.project.name.clone(),
                    coordinates: vec![coordinate],
                }])
            });
            paths
                .entry(audit_package)
                .or_default()
                .extend(package_paths);
        }
    }
    Ok(AuditWorkspace {
        packages: packages.into_iter().collect(),
        paths: paths
            .into_iter()
            .map(|(package, paths)| (package, paths.into_iter().collect()))
            .collect(),
        suppressions,
    })
}

fn audit_paths_for_lock(
    lock: &Lockfile,
    module: &str,
) -> BTreeMap<String, BTreeSet<AuditDependencyPath>> {
    shortest_dependency_paths(lock)
        .into_iter()
        .map(|(coordinate, paths)| {
            (
                coordinate,
                paths
                    .into_iter()
                    .map(|coordinates| AuditDependencyPath {
                        module: module.to_owned(),
                        coordinates,
                    })
                    .collect(),
            )
        })
        .collect()
}

fn shortest_dependency_paths(lock: &Lockfile) -> BTreeMap<String, BTreeSet<Vec<String>>> {
    let packages = lock
        .packages
        .iter()
        .map(|package| (locked_coordinate(package), package))
        .collect::<BTreeMap<_, _>>();
    let mut paths = BTreeMap::<String, BTreeSet<Vec<String>>>::new();
    let mut queue = std::collections::VecDeque::new();
    for root in locked_root_dependencies(lock) {
        queue.push_back((root.clone(), vec![root]));
    }
    let mut shortest = BTreeMap::<String, usize>::new();
    while let Some((coordinate, path)) = queue.pop_front() {
        let length = path.len();
        if shortest
            .get(&coordinate)
            .is_some_and(|existing| *existing < length)
        {
            continue;
        }
        shortest.insert(coordinate.clone(), length);
        paths
            .entry(coordinate.clone())
            .or_default()
            .insert(path.clone());
        let Some(package) = packages.get(&coordinate) else {
            continue;
        };
        for dependency in &package.dependencies {
            if path.contains(dependency) {
                continue;
            }
            let mut child_path = path.clone();
            child_path.push(dependency.clone());
            queue.push_back((dependency.clone(), child_path));
        }
    }
    paths
}

fn evaluate_audit(
    result: jman_audit::AuditResult,
    paths: &BTreeMap<jman_audit::AuditPackage, Vec<AuditDependencyPath>>,
    suppressions: &[jman_config::AuditSuppression],
    minimum: jman_audit::Severity,
    deny: Option<jman_audit::Severity>,
    today: &str,
) -> DependencyAuditReport {
    let total_findings = result.findings.len();
    let mut used_suppressions = BTreeSet::new();
    let mut suppression_warnings = suppressions
        .iter()
        .filter(|suppression| suppression.expires.as_str() < today)
        .map(|suppression| {
            format!(
                "audit suppression '{}' expired on {} and no longer applies",
                suppression.id, suppression.expires
            )
        })
        .collect::<BTreeSet<_>>();
    let mut all = Vec::new();
    for finding in result.findings {
        let suppression = suppressions.iter().find(|suppression| {
            suppression.id == finding.advisory.id
                || finding.advisory.aliases.contains(&suppression.id)
        });
        let applied = suppression.and_then(|suppression| {
            if suppression.expires.as_str() < today {
                None
            } else {
                used_suppressions.insert(suppression.id.clone());
                Some(AppliedAuditSuppression {
                    reason: suppression.reason.clone(),
                    expires: suppression.expires.clone(),
                })
            }
        });
        all.push(AuditFindingReport {
            paths: paths.get(&finding.package).cloned().unwrap_or_default(),
            package: finding.package,
            advisory: finding.advisory,
            active: applied.is_none(),
            suppression: applied,
        });
    }
    for suppression in suppressions {
        if suppression.expires.as_str() >= today && !used_suppressions.contains(&suppression.id) {
            suppression_warnings.insert(format!(
                "audit suppression '{}' did not match any finding",
                suppression.id
            ));
        }
    }
    let denied = deny.map_or(0, |threshold| {
        all.iter()
            .filter(|finding| finding.active && finding.advisory.severity >= threshold)
            .count()
    });
    all.retain(|finding| finding.advisory.severity >= minimum);
    all.sort_by(|left, right| {
        right
            .advisory
            .severity
            .cmp(&left.advisory.severity)
            .then_with(|| left.package.cmp(&right.package))
            .then_with(|| left.advisory.id.cmp(&right.advisory.id))
    });
    let active = all.iter().filter(|finding| finding.active).count();
    let suppressed = all.len() - active;
    DependencyAuditReport {
        provider: result.provider,
        source: result.source,
        packages_checked: paths.len(),
        total_findings,
        reported_findings: all.len(),
        active,
        suppressed,
        denied,
        suppression_warnings: suppression_warnings.into_iter().collect(),
        findings: all,
    }
}

fn print_dependency_audit(report: &DependencyAuditReport) {
    if report.findings.is_empty() {
        println!(
            "No known vulnerabilities found in {} resolved packages.",
            report.packages_checked
        );
        return;
    }
    for finding in &report.findings {
        let status = if finding.active {
            "ACTIVE"
        } else {
            "SUPPRESSED"
        };
        println!(
            "{}  {}  {}  {}",
            finding.advisory.severity.name().to_ascii_uppercase(),
            status,
            terminal_text(&finding.advisory.id),
            terminal_text(&finding.package.coordinate())
        );
        println!("  {}", terminal_text(&finding.advisory.summary));
        if !finding.advisory.aliases.is_empty() {
            println!(
                "  aliases: {}",
                finding
                    .advisory
                    .aliases
                    .iter()
                    .map(|alias| terminal_text(alias))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !finding.advisory.fixed_versions.is_empty() {
            println!(
                "  fixed: {}",
                finding
                    .advisory
                    .fixed_versions
                    .iter()
                    .map(|version| terminal_text(version))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !finding.paths.is_empty() {
            let mut module_paths = BTreeMap::<&str, BTreeSet<Vec<String>>>::new();
            for path in &finding.paths {
                module_paths
                    .entry(&path.module)
                    .or_default()
                    .insert(path.coordinates.clone());
            }
            println!("  dependency paths:");
            for (module, paths) in module_paths {
                println!(
                    "{}",
                    dependency_path_tree(
                        &format!("module {module}"),
                        &paths.into_iter().collect::<Vec<_>>(),
                        "    "
                    )
                );
            }
        }
        if let Some(suppression) = &finding.suppression {
            println!(
                "  suppressed until {}: {}",
                terminal_text(&suppression.expires),
                terminal_text(&suppression.reason)
            );
        }
        if let Some(reference) = finding.advisory.references.first() {
            println!("  {}", terminal_text(reference));
        }
        println!();
    }
    println!(
        "{} active, {} suppressed, {} reported ({} total)",
        report.active, report.suppressed, report.reported_findings, report.total_findings
    );
}

fn utc_date() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400;
    let (year, month, day) = civil_date_from_unix_days(days.cast_signed());
    format!("{year:04}-{month:02}-{day:02}")
}

fn civil_date_from_unix_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

async fn sync_project(arguments: &Sync, ui: &Ui) -> Result<()> {
    let target = absolute_path(&arguments.path)?;
    if !target.join("jman.toml").is_file() {
        bail!(
            "cannot synchronize {}: jman.toml was not found",
            target.display()
        );
    }
    let workspace_root = find_workspace_root(&target);
    if target == workspace_root {
        let root_manifest = Manifest::read(&target.join("jman.toml"))
            .with_context(|| format!("could not read {}", target.join("jman.toml").display()))?;
        if !root_manifest.project.modules.is_empty() {
            let directories = workspace_directories(&workspace_root).await?;
            let mut synchronized = Vec::with_capacity(directories.len());
            let module_count = directories.len();
            for (index, directory) in directories.into_iter().enumerate() {
                let name = display_module(&workspace_root, &directory);
                let activity = ui.activity(format!(
                    "Synchronizing {name} ({}/{module_count})",
                    index + 1
                ));
                let result = sync_one(
                    &directory,
                    arguments.refresh,
                    arguments.offline,
                    Some(&activity),
                )
                .await?;
                activity.finish(format!("Synchronized {name}"));
                synchronized.push((directory, result));
            }
            match arguments.report {
                ReportFormat::Human => {
                    for (_, (manifest, lock, unchanged)) in synchronized {
                        print_sync_report(ReportFormat::Human, &manifest, &lock, unchanged, ui)?;
                    }
                }
                ReportFormat::Json => {
                    let modules = synchronized
                        .into_iter()
                        .map(|(directory, (manifest, lock, _))| {
                            let name = directory
                                .strip_prefix(&workspace_root)
                                .ok()
                                .filter(|path| !path.as_os_str().is_empty())
                                .map_or_else(
                                    || ".".to_owned(),
                                    |path| path.to_string_lossy().into_owned(),
                                );
                            (name, SyncReport::from_lock(&manifest, &lock))
                        })
                        .collect::<BTreeMap<_, _>>();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&WorkspaceSyncReport { modules })?
                    );
                }
            }
            return Ok(());
        }
    }
    let activity = ui.activity(format!("Synchronizing {}", target.display()));
    let (manifest, lock, unchanged) = sync_one(
        &target,
        arguments.refresh,
        arguments.offline,
        Some(&activity),
    )
    .await?;
    activity.finish(if unchanged {
        "Workspace already synchronized"
    } else {
        "Dependency lock updated"
    });
    print_sync_report(arguments.report, &manifest, &lock, unchanged, ui)
}

async fn sync_one(
    target: &Path,
    refresh: bool,
    offline: bool,
    activity: Option<&ui::Activity>,
) -> Result<(Manifest, Lockfile, bool)> {
    let profile = std::env::var_os("JMAN_PROFILE_RESOLVER").is_some();
    let sync_started = std::time::Instant::now();
    let workspace_root = find_workspace_root(target);
    update_activity(activity, "Checking manifests and dependency cache");
    let manifest_path = target.join("jman.toml");
    let lock_path = target.join("jman.lock");
    let manifest = Manifest::read(&manifest_path)
        .with_context(|| format!("could not read {}", manifest_path.display()))?;
    let manifest_text = manifest
        .to_toml()
        .context("could not serialize jman.toml")?;
    let manifest_hash = sha256(manifest_text.as_bytes());
    let workspace_hash = hash_workspace_manifests(&workspace_root).await?;
    let repository_urls = manifest
        .repositories
        .iter()
        .map(|repository| repository.url.clone())
        .collect::<Vec<_>>();
    let repository = RepositoryClient::with_default_cache_mode(repository_urls, offline)
        .context("could not initialize Maven repositories")?;
    if !refresh {
        if let Ok(lock) = Lockfile::read(&lock_path) {
            if lock_is_current(
                &lock,
                &manifest_hash,
                &workspace_hash,
                manifest.project.java_release,
                repository.cache_dir(),
                offline,
            ) {
                return Ok((manifest, lock, true));
            }
        }
    }
    let resolver = DependencyResolver::new(repository);
    update_activity(activity, "Constructing native workspace models");
    let model_started = std::time::Instant::now();
    let projects = manifest_projects(&workspace_root).await?;
    let canonical_target = tokio::fs::canonicalize(&target)
        .await
        .with_context(|| format!("could not canonicalize {}", target.display()))?;
    let effective = projects
        .iter()
        .find(|project| project.source.parent() == Some(canonical_target.as_path()))
        .map(|project| project.effective.clone())
        .with_context(|| {
            format!(
                "{} is not a module in the JMAN workspace rooted at {}",
                target.display(),
                workspace_root.display()
            )
        })?;
    if profile {
        eprintln!("profile: effective model {:?}", model_started.elapsed());
    }
    let resolution_started = std::time::Instant::now();
    update_activity(activity, "Resolving and downloading dependencies");
    let graph = resolver
        .resolve_in_workspace(&effective, &projects)
        .await
        .context("dependency resolution failed")?;
    if profile {
        eprintln!(
            "profile: dependency graph {:?}",
            resolution_started.elapsed()
        );
        eprintln!("profile: sync total {:?}", sync_started.elapsed());
    }
    let lock = lock_from_graph(
        &manifest_text,
        &workspace_hash,
        manifest.project.java_release,
        graph,
    );
    let lock_text = lock.to_toml().context("could not generate jman.lock")?;
    update_activity(activity, "Writing deterministic lockfile");
    write_atomically(&lock_path, &lock_text).await?;
    Ok((manifest, lock, false))
}

fn update_activity(activity: Option<&ui::Activity>, message: &str) {
    if let Some(activity) = activity {
        activity.set_message(message);
    }
}

fn print_sync_report(
    format: ReportFormat,
    manifest: &Manifest,
    lock: &Lockfile,
    unchanged: bool,
    ui: &Ui,
) -> Result<()> {
    match format {
        ReportFormat::Human => {
            let status = if unchanged { ", unchanged" } else { "" };
            ui.success(format!(
                "Synchronized {}:{}:{} ({} packages{status})",
                manifest.project.group,
                manifest.project.name,
                manifest.project.version,
                lock.packages.len()
            ));
        }
        ReportFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&SyncReport::from_lock(manifest, lock))?
            );
        }
    }
    Ok(())
}

fn display_module(workspace_root: &Path, directory: &Path) -> String {
    directory
        .strip_prefix(workspace_root)
        .ok()
        .filter(|path| !path.as_os_str().is_empty())
        .map_or_else(
            || ".".to_owned(),
            |path| path.to_string_lossy().into_owned(),
        )
}

#[derive(Debug, Serialize)]
struct SyncReport {
    root: ReportCoordinate,
    root_dependencies: Vec<String>,
    packages: Vec<ReportPackage>,
}

#[derive(Debug, Serialize)]
struct WorkspaceSyncReport {
    modules: BTreeMap<String, SyncReport>,
}

#[derive(Debug, Serialize)]
struct ReportPackage {
    coordinate: ReportCoordinate,
    scopes: Vec<String>,
    parent: String,
    dependencies: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ReportCoordinate {
    group: String,
    artifact: String,
    version: String,
    extension: String,
    classifier: Option<String>,
}

impl SyncReport {
    fn from_lock(manifest: &Manifest, lock: &Lockfile) -> Self {
        let root_parent = format!(
            "{}:{}:{}:{}",
            manifest.project.group,
            manifest.project.name,
            manifest.project.packaging,
            manifest.project.version
        );
        let mut packages = lock
            .packages
            .iter()
            .map(|package| ReportPackage {
                coordinate: ReportCoordinate {
                    group: package.group.clone(),
                    artifact: package.artifact.clone(),
                    version: package.version.clone(),
                    extension: package.extension.clone(),
                    classifier: package.classifier.clone(),
                },
                scopes: package.scopes.clone(),
                parent: package
                    .selected_parent
                    .clone()
                    .unwrap_or_else(|| root_parent.clone()),
                dependencies: package.dependencies.clone(),
            })
            .collect::<Vec<_>>();
        packages.sort_by(|left, right| {
            (
                &left.coordinate.group,
                &left.coordinate.artifact,
                &left.coordinate.extension,
                &left.coordinate.classifier,
                &left.coordinate.version,
            )
                .cmp(&(
                    &right.coordinate.group,
                    &right.coordinate.artifact,
                    &right.coordinate.extension,
                    &right.coordinate.classifier,
                    &right.coordinate.version,
                ))
        });
        Self {
            root: ReportCoordinate {
                group: manifest.project.group.clone(),
                artifact: manifest.project.name.clone(),
                version: manifest.project.version.clone(),
                extension: manifest.project.packaging.clone(),
                classifier: None,
            },
            root_dependencies: locked_root_dependencies(lock),
            packages,
        }
    }
}

#[allow(clippy::too_many_lines)]
fn manifest_from_maven(effective: &EffectivePom, maven_authoritative: bool) -> Result<Manifest> {
    let java_release = [
        "maven.compiler.release",
        "maven.compiler.source",
        "java.version",
        "release.version",
        "jdk.version",
    ]
    .iter()
    .find_map(|key| effective.properties.get(*key))
    .map(|value| {
        value
            .parse::<u16>()
            .with_context(|| format!("Maven property for Java release is `{value}`"))
    })
    .transpose()?
    .unwrap_or(17);
    let mut dependencies = Dependencies::default();
    let mut dependency_metadata = BTreeMap::new();
    for dependency in &effective.dependencies {
        let version = dependency
            .version
            .clone()
            .with_context(|| format!("dependency {} has no effective version", dependency.ga()))?;
        let (normalized_scope, target) = match dependency.scope.as_str() {
            "compile" => ("compile", &mut dependencies.compile),
            "runtime" => ("runtime", &mut dependencies.runtime),
            "provided" | "system" => ("provided", &mut dependencies.provided),
            "test" => ("test", &mut dependencies.test),
            scope => bail!(
                "dependency {} uses unsupported Maven scope `{scope}`",
                dependency.ga()
            ),
        };
        target.insert(dependency.ga(), version);
        if dependency.dependency_type != "jar"
            || dependency.classifier.is_some()
            || dependency.optional == Some(true)
            || !dependency.exclusions.is_empty()
        {
            dependency_metadata.insert(
                format!("{normalized_scope}:{}", dependency.ga()),
                MavenDependencyMetadata {
                    dependency_type: dependency.dependency_type.clone(),
                    classifier: dependency.classifier.clone(),
                    optional: dependency.optional == Some(true),
                    exclusions: dependency
                        .exclusions
                        .iter()
                        .map(|exclusion| format!("{}:{}", exclusion.group, exclusion.artifact))
                        .collect(),
                },
            );
        }
    }
    let manifest = Manifest {
        manifest_version: MANIFEST_VERSION,
        project: Project {
            group: effective.coordinate.group.clone(),
            name: effective.coordinate.artifact.clone(),
            version: effective.coordinate.version.clone(),
            java_release,
            packaging: effective.packaging.clone(),
            modules: effective.modules.clone(),
            main_class: if maven_authoritative {
                ["exec.mainClass", "start-class", "main.class"]
                    .iter()
                    .find_map(|key| effective.properties.get(*key))
                    .cloned()
            } else {
                None
            },
        },
        toolchain: Some(ManifestToolchain {
            jdk: java_release.to_string(),
            vendor: "temurin".to_owned(),
        }),
        maven: Some(MavenCompatibility {
            parent: effective
                .parent
                .as_ref()
                .map(|parent| format!("{}:{}:{}", parent.group, parent.artifact, parent.version)),
            dependencies: dependency_metadata,
            dependency_management: effective
                .dependency_management
                .iter()
                .map(|dependency| MavenManagedDependency {
                    group: dependency.group.clone(),
                    artifact: dependency.artifact.clone(),
                    version: dependency.version.clone(),
                    scope: dependency.scope.clone(),
                    dependency_type: dependency.dependency_type.clone(),
                    classifier: dependency.classifier.clone(),
                    optional: dependency.optional,
                    exclusions: dependency
                        .exclusions
                        .iter()
                        .map(|exclusion| format!("{}:{}", exclusion.group, exclusion.artifact))
                        .collect(),
                })
                .collect(),
            bom_imports: effective
                .bom_imports
                .iter()
                .map(ToString::to_string)
                .collect(),
        }),
        test: None,
        build: Some(Build {
            encoding: effective
                .properties
                .get("project.build.sourceEncoding")
                .cloned()
                .unwrap_or_else(|| "UTF-8".to_owned()),
            compiler_args: effective.compiler_args.clone(),
        }),
        publishing: None,
        audit: None,
        repositories: effective
            .repositories
            .iter()
            .filter(|repository| repository.releases_enabled)
            .map(|repository| Repository {
                id: repository.id.clone(),
                url: repository.url.clone(),
            })
            .collect(),
        dependencies,
        annotation_processors: effective
            .annotation_processors
            .iter()
            .map(|processor| {
                let dependency = &processor.dependency;
                let version = dependency.version.clone().with_context(|| {
                    format!(
                        "annotation processor {} has no effective version",
                        dependency.ga()
                    )
                })?;
                Ok((dependency.ga(), version))
            })
            .collect::<Result<_>>()?,
        path_dependencies: BTreeMap::new(),
    };
    manifest.validate()?;
    Ok(manifest)
}

fn lock_from_graph(
    manifest: &str,
    workspace_hash: &str,
    java_release: u16,
    graph: ResolvedGraph,
) -> Lockfile {
    let coordinate_by_index = graph
        .packages
        .iter()
        .map(|package| package.coordinate.to_string())
        .collect::<Vec<_>>();
    let mut packages = graph
        .packages
        .into_iter()
        .map(|package| LockedPackage {
            group: package.coordinate.group,
            artifact: package.coordinate.artifact,
            version: package.coordinate.version,
            extension: package.coordinate.extension,
            classifier: package.coordinate.classifier,
            source: package.source,
            pom_checksum: package.pom_checksum,
            artifact_checksum: package.artifact_checksum,
            artifact_size: package.artifact_size,
            dependencies: package.dependencies,
            scopes: package.effective_scopes.into_iter().collect(),
            selected_parent: package
                .selected_parent
                .map(|index| coordinate_by_index[index].clone()),
        })
        .collect::<Vec<_>>();
    packages.sort_by(|left, right| {
        (&left.group, &left.artifact, &left.version, &left.extension).cmp(&(
            &right.group,
            &right.artifact,
            &right.version,
            &right.extension,
        ))
    });
    Lockfile {
        lock_version: LOCK_VERSION,
        manifest_hash: sha256(manifest.as_bytes()),
        workspace_hash: workspace_hash.to_owned(),
        platform: platform_id(),
        toolchain: LockedToolchain { java_release },
        packages,
        classpath: LockedClasspaths {
            compile: graph.compile_classpath,
            runtime: graph.runtime_classpath,
            test: graph.test_classpath,
            processors: graph.processor_classpath,
        },
    }
}

fn lock_is_current(
    lock: &Lockfile,
    manifest_hash: &str,
    workspace_hash: &str,
    java_release: u16,
    cache_dir: &Path,
    verify_integrity: bool,
) -> bool {
    lock.manifest_hash == manifest_hash
        && lock.workspace_hash == workspace_hash
        && lock.platform == platform_id()
        && lock.toolchain.java_release == java_release
        && lock.packages.iter().all(|package| {
            match (&package.artifact_checksum, package.artifact_size) {
                (None, None) => true,
                (Some(checksum), Some(expected_size)) => {
                    checksum.strip_prefix("sha256:").is_some_and(|digest| {
                        cache_dir
                            .join("artifacts")
                            .join(format!("{digest}.{}", package.extension))
                            .metadata()
                            .is_ok_and(|metadata| {
                                metadata.is_file() && metadata.len() == expected_size
                            })
                            && (!verify_integrity
                                || std::fs::read(
                                    cache_dir
                                        .join("artifacts")
                                        .join(format!("{digest}.{}", package.extension)),
                                )
                                .is_ok_and(|bytes| sha256(&bytes) == *checksum))
                    })
                }
                _ => false,
            }
        })
}

fn locked_root_dependencies(lock: &Lockfile) -> Vec<String> {
    lock.packages
        .iter()
        .filter(|package| package.selected_parent.is_none())
        .map(locked_coordinate)
        .collect()
}

fn find_workspace_root(target: &Path) -> PathBuf {
    let mut workspace_root = target.to_owned();
    for ancestor in target.ancestors() {
        let manifest_path = ancestor.join("jman.toml");
        let Ok(manifest) = Manifest::read(&manifest_path) else {
            continue;
        };
        if !manifest.project.modules.is_empty() {
            ancestor.clone_into(&mut workspace_root);
        }
    }
    workspace_root
}

async fn workspace_directories(root: &Path) -> Result<Vec<PathBuf>> {
    let canonical_root = tokio::fs::canonicalize(root)
        .await
        .with_context(|| format!("could not canonicalize {}", root.display()))?;
    let mut pending = vec![canonical_root.clone()];
    let mut seen = std::collections::BTreeSet::new();
    let mut directories = Vec::new();
    while let Some(directory) = pending.pop() {
        if !seen.insert(directory.clone()) {
            bail!("duplicate or cyclic JMAN module {}", directory.display());
        }
        let manifest = Manifest::read(&directory.join("jman.toml"))
            .with_context(|| format!("could not read module {}", directory.display()))?;
        for module in manifest.project.modules.iter().rev() {
            let module_path = tokio::fs::canonicalize(directory.join(module))
                .await
                .with_context(|| format!("could not resolve module `{module}`"))?;
            if !module_path.starts_with(&canonical_root) {
                bail!(
                    "module {} escapes workspace {}",
                    module_path.display(),
                    canonical_root.display()
                );
            }
            pending.push(module_path);
        }
        directories.push(directory);
    }
    Ok(directories)
}

async fn manifest_projects(root: &Path) -> Result<Vec<MavenProject>> {
    let directories = workspace_directories(root).await?;
    directories
        .into_iter()
        .map(|directory| {
            let source = directory.join("jman.toml");
            let manifest = Manifest::read(&source)
                .with_context(|| format!("could not read {}", source.display()))?;
            Ok(MavenProject {
                source,
                effective: effective_from_manifest(&manifest)?,
            })
        })
        .collect()
}

#[allow(clippy::too_many_lines)]
fn effective_from_manifest(manifest: &Manifest) -> Result<EffectivePom> {
    let mut dependencies = Vec::new();
    for (scope, entries) in [
        ("compile", &manifest.dependencies.compile),
        ("runtime", &manifest.dependencies.runtime),
        ("provided", &manifest.dependencies.provided),
        ("test", &manifest.dependencies.test),
    ] {
        for (ga, version) in entries {
            let (group, artifact) = split_ga(ga)?;
            let metadata = manifest
                .maven
                .as_ref()
                .and_then(|maven| maven.dependencies.get(&format!("{scope}:{ga}")));
            dependencies.push(Dependency {
                group: group.to_owned(),
                artifact: artifact.to_owned(),
                version: Some(version.clone()),
                scope: scope.to_owned(),
                scope_explicit: scope != "compile",
                dependency_type: metadata
                    .map_or_else(|| "jar".to_owned(), |item| item.dependency_type.clone()),
                type_explicit: metadata.is_some_and(|item| item.dependency_type != "jar"),
                classifier: metadata.and_then(|item| item.classifier.clone()),
                optional: metadata.filter(|item| item.optional).map(|_| true),
                exclusions: metadata
                    .map(|item| parse_exclusions(&item.exclusions))
                    .transpose()?
                    .unwrap_or_default(),
            });
        }
    }
    let dependency_management = manifest
        .maven
        .as_ref()
        .map(|maven| {
            maven
                .dependency_management
                .iter()
                .map(|managed| {
                    Ok(Dependency {
                        group: managed.group.clone(),
                        artifact: managed.artifact.clone(),
                        version: managed.version.clone(),
                        scope: managed.scope.clone(),
                        scope_explicit: managed.scope != "compile",
                        dependency_type: managed.dependency_type.clone(),
                        type_explicit: managed.dependency_type != "jar",
                        classifier: managed.classifier.clone(),
                        optional: managed.optional,
                        exclusions: parse_exclusions(&managed.exclusions)?,
                    })
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let annotation_processors = manifest
        .annotation_processors
        .iter()
        .map(|(ga, version)| {
            let (group, artifact) = split_ga(ga)?;
            Ok(AnnotationProcessor {
                dependency: Dependency {
                    group: group.to_owned(),
                    artifact: artifact.to_owned(),
                    version: Some(version.clone()),
                    scope: "processor".to_owned(),
                    scope_explicit: true,
                    dependency_type: "jar".to_owned(),
                    type_explicit: false,
                    classifier: None,
                    optional: None,
                    exclusions: Vec::new(),
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let parent = manifest
        .maven
        .as_ref()
        .and_then(|maven| maven.parent.as_deref())
        .map(Coordinate::parse)
        .transpose()?;
    let bom_imports = manifest
        .maven
        .as_ref()
        .map(|maven| {
            maven
                .bom_imports
                .iter()
                .map(|coordinate| Coordinate::parse(coordinate))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(EffectivePom {
        coordinate: Coordinate::pom(
            manifest.project.group.clone(),
            manifest.project.name.clone(),
            manifest.project.version.clone(),
        ),
        parent,
        packaging: manifest.project.packaging.clone(),
        modules: manifest.project.modules.clone(),
        property_templates: BTreeMap::new(),
        properties: BTreeMap::new(),
        dependencies,
        dependency_management,
        bom_imports,
        repositories: manifest
            .repositories
            .iter()
            .map(|repository| jman_resolver::Repository {
                id: repository.id.clone(),
                url: repository.url.clone(),
                releases_enabled: true,
                snapshots_enabled: true,
            })
            .collect(),
        annotation_processors,
        compiler_args: manifest
            .build
            .as_ref()
            .map_or_else(Vec::new, |build| build.compiler_args.clone()),
        relocation: None,
    })
}

fn split_ga(coordinate: &str) -> Result<(&str, &str)> {
    coordinate
        .split_once(':')
        .filter(|(group, artifact)| {
            !group.is_empty() && !artifact.is_empty() && !artifact.contains(':')
        })
        .with_context(|| format!("invalid dependency coordinate `{coordinate}`"))
}

fn parse_exclusions(exclusions: &[String]) -> Result<Vec<Exclusion>> {
    exclusions
        .iter()
        .map(|coordinate| {
            let (group, artifact) = split_ga(coordinate)?;
            Ok(Exclusion {
                group: group.to_owned(),
                artifact: artifact.to_owned(),
            })
        })
        .collect()
}

async fn hash_workspace_manifests(root: &Path) -> Result<String> {
    let canonical_root = tokio::fs::canonicalize(root)
        .await
        .with_context(|| format!("could not canonicalize {}", root.display()))?;
    let directories = workspace_directories(&canonical_root).await?;
    let mut inputs = Vec::new();
    for directory in directories {
        let manifest = directory.join("jman.toml");
        let bytes = tokio::fs::read(&manifest)
            .await
            .with_context(|| format!("could not read {}", manifest.display()))?;
        let relative = manifest.strip_prefix(&canonical_root).unwrap_or(&manifest);
        inputs.push((relative.to_path_buf(), bytes));
    }
    inputs.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (path, bytes) in inputs {
        digest.update(path.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(bytes);
        digest.update([0]);
    }
    Ok(format!("sha256:{}", hex::encode(digest.finalize())))
}

fn hash_generated_manifests(
    root: &Path,
    manifests: &[(&MavenProject, PathBuf, Manifest)],
) -> Result<String> {
    let mut inputs = Vec::with_capacity(manifests.len());
    for (_, directory, manifest) in manifests {
        let path = directory.join("jman.toml");
        let relative = path.strip_prefix(root).unwrap_or(&path);
        inputs.push((
            relative.to_path_buf(),
            manifest
                .to_toml()
                .context("could not serialize imported manifest")?
                .into_bytes(),
        ));
    }
    inputs.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (path, bytes) in inputs {
        digest.update(path.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(bytes);
        digest.update([0]);
    }
    Ok(format!("sha256:{}", hex::encode(digest.finalize())))
}

fn platform_id() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn ensure_absent(path: &Path) -> Result<()> {
    if path.exists() {
        bail!("refusing to overwrite existing {}", path.display());
    }
    Ok(())
}

async fn write_atomically(path: &Path, contents: &str) -> Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    tokio::fs::write(&temporary, contents)
        .await
        .with_context(|| format!("could not write {}", temporary.display()))?;
    tokio::fs::rename(&temporary, path)
        .await
        .with_context(|| format!("could not install {}", path.display()))
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()
            .context("could not determine current directory")?
            .join(path))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn java_list_accepts_catalog_filters_refresh_and_json() {
        let cli = Cli::try_parse_from([
            "jman",
            "java",
            "list",
            "--major",
            "21",
            "--lts",
            "--vendor",
            "open",
            "--refresh",
            "--format",
            "json",
        ])
        .expect("java list arguments");

        let Command::Java(Java {
            command: JavaCommand::List(arguments),
        }) = cli.command
        else {
            panic!("expected java list command");
        };
        assert!(!arguments.scope.local);
        assert!(!arguments.scope.all);
        assert_eq!(arguments.major, Some(21));
        assert!(arguments.lts);
        assert_eq!(arguments.vendor.as_deref(), Some("open"));
        assert!(arguments.refresh);
        assert_eq!(arguments.format, ReportFormat::Json);

        assert!(Cli::try_parse_from(["jman", "java", "list", "--local", "--refresh"]).is_err());
        assert!(Cli::try_parse_from(["jman", "java", "list", "--local", "--all"]).is_err());

        let all = Cli::try_parse_from(["jman", "java", "list", "--all"])
            .expect("complete catalog arguments");
        let Command::Java(Java {
            command: JavaCommand::List(all),
        }) = all.command
        else {
            panic!("expected java list command");
        };
        assert!(all.scope.all);
    }

    #[test]
    fn maven_model_source_prefers_wrapper_then_system_then_native() {
        let project = tempfile::tempdir().expect("project");
        let commands = tempfile::tempdir().expect("commands");
        let system_maven = commands
            .path()
            .join(if cfg!(windows) { "mvn.cmd" } else { "mvn" });
        fs::write(&system_maven, "fixture").expect("system Maven");

        assert_eq!(
            select_maven_model_source(project.path(), Some(commands.path().as_os_str())),
            MavenModelSource::System(system_maven.clone())
        );

        let wrapper = project
            .path()
            .join(if cfg!(windows) { "mvnw.cmd" } else { "mvnw" });
        fs::write(&wrapper, "fixture").expect("Maven wrapper");
        assert_eq!(
            select_maven_model_source(project.path(), Some(commands.path().as_os_str())),
            MavenModelSource::Wrapper(wrapper)
        );

        fs::remove_file(
            project
                .path()
                .join(if cfg!(windows) { "mvnw.cmd" } else { "mvnw" }),
        )
        .expect("remove wrapper");
        assert_eq!(
            select_maven_model_source(project.path(), Some(OsStr::new(""))),
            MavenModelSource::Native
        );
    }

    #[test]
    fn java_distribution_defaults_to_temurin_and_accepts_vendor_selection() {
        let default =
            Cli::try_parse_from(["jman", "java", "install", "21"]).expect("default distribution");
        let Command::Java(Java {
            command: JavaCommand::Install(default),
        }) = default.command
        else {
            panic!("expected java install command");
        };
        assert_eq!(default.vendor, "temurin");

        let selected =
            Cli::try_parse_from(["jman", "java", "install", "21", "--vendor", "corretto"])
                .expect("selected distribution");
        let Command::Java(Java {
            command: JavaCommand::Install(selected),
        }) = selected.command
        else {
            panic!("expected java install command");
        };
        assert_eq!(selected.vendor, "corretto");
    }

    #[test]
    fn outdated_accepts_offline_prerelease_and_json_controls() {
        let cli = Cli::try_parse_from([
            "jman",
            "outdated",
            "workspace",
            "--offline",
            "--include-prerelease",
            "--format",
            "json",
        ])
        .expect("outdated command");
        let Command::Outdated(arguments) = cli.command else {
            panic!("expected outdated command");
        };
        assert_eq!(arguments.path, PathBuf::from("workspace"));
        assert!(arguments.offline);
        assert!(arguments.include_prerelease);
        assert_eq!(arguments.format, ReportFormat::Json);
    }

    #[test]
    fn outdated_rejects_dynamic_versions_without_guessing() {
        for version in ["[1,2)", "1.+", "LATEST", "${revision}"] {
            assert!(validate_outdated_version(version).is_err(), "{version}");
        }
        for version in ["1", "1.2.3", "21.0.8+9", "1.0.0.Final"] {
            assert!(validate_outdated_version(version).is_ok(), "{version}");
        }
    }

    #[test]
    fn outdated_reports_do_not_expose_repository_credentials() {
        assert_eq!(
            redact_repository_source("https://user:secret@repo.example/releases"),
            "https://***@repo.example/releases"
        );
        assert_eq!(redact_repository_source("cache"), "cache");
    }

    #[test]
    fn outdated_table_distinguishes_updates_current_and_unavailable_metadata() {
        let dependency = |name: &str,
                          status: OutdatedStatus,
                          latest: Option<&str>,
                          change: Option<jman_resolver::VersionChange>| {
            OutdatedDependencyReport {
                dependency: name.to_owned(),
                current: "1.0.0".to_owned(),
                status,
                latest: latest.map(ToOwned::to_owned),
                change,
                patch: None,
                minor: None,
                major: None,
                scopes: vec!["compile".to_owned()],
                modules: vec!["app".to_owned()],
                source: None,
                error: None,
            }
        };
        let table = outdated_table(&[
            dependency(
                "org.example:old",
                OutdatedStatus::Outdated,
                Some("2.0.0"),
                Some(jman_resolver::VersionChange::Major),
            ),
            dependency("org.example:current", OutdatedStatus::UpToDate, None, None),
            dependency(
                "org.example:unknown",
                OutdatedStatus::Unavailable,
                None,
                None,
            ),
        ]);
        assert!(table.starts_with("DEPENDENCY"));
        assert!(table.contains("org.example:old      1.0.0    2.0.0   major"));
        assert!(table.contains("org.example:current  1.0.0    -       current"));
        assert!(table.contains("org.example:unknown  1.0.0    -       unavailable"));
    }

    #[test]
    fn update_defaults_to_patch_and_accepts_safe_automation_controls() {
        let default = Cli::try_parse_from(["jman", "update", "org.example:library"])
            .expect("default update command");
        let Command::Update(default) = default.command else {
            panic!("expected update command");
        };
        assert_eq!(default.dependency.as_deref(), Some("org.example:library"));
        assert_eq!(default.level, UpdateLevel::Patch);
        assert!(!default.dry_run);

        let controlled = Cli::try_parse_from([
            "jman",
            "update",
            "--path",
            "workspace",
            "--level",
            "major",
            "--offline",
            "--include-prerelease",
            "--dry-run",
            "--format",
            "json",
        ])
        .expect("controlled update command");
        let Command::Update(controlled) = controlled.command else {
            panic!("expected update command");
        };
        assert_eq!(controlled.dependency, None);
        assert_eq!(controlled.path, PathBuf::from("workspace"));
        assert_eq!(controlled.level, UpdateLevel::Major);
        assert!(controlled.offline);
        assert!(controlled.include_prerelease);
        assert!(controlled.dry_run);
        assert_eq!(controlled.format, ReportFormat::Json);
    }

    #[test]
    fn update_levels_never_cross_their_selected_boundary() {
        let dependency = OutdatedDependencyReport {
            dependency: "org.example:library".to_owned(),
            current: "1.2.3".to_owned(),
            status: OutdatedStatus::Outdated,
            latest: Some("4.0.0-special".to_owned()),
            change: Some(jman_resolver::VersionChange::Other),
            patch: Some("1.2.4".to_owned()),
            minor: Some("1.3.0".to_owned()),
            major: Some("2.0.0".to_owned()),
            scopes: vec!["compile".to_owned()],
            modules: vec!["app".to_owned()],
            source: Some("fixture".to_owned()),
            error: None,
        };
        for (level, expected, change) in [
            (
                UpdateLevel::Patch,
                "1.2.4",
                jman_resolver::VersionChange::Patch,
            ),
            (
                UpdateLevel::Minor,
                "1.3.0",
                jman_resolver::VersionChange::Minor,
            ),
            (
                UpdateLevel::Major,
                "2.0.0",
                jman_resolver::VersionChange::Major,
            ),
            (
                UpdateLevel::Latest,
                "4.0.0-special",
                jman_resolver::VersionChange::Other,
            ),
        ] {
            let selected = select_dependency_update(&dependency, level).expect("selected update");
            assert_eq!(selected.to, expected);
            assert_eq!(selected.change, change);
        }
    }

    #[test]
    fn audit_accepts_offline_filter_policy_and_json_controls() {
        let cli = Cli::try_parse_from([
            "jman",
            "audit",
            "workspace",
            "--offline",
            "--severity",
            "medium",
            "--deny",
            "critical",
            "--format",
            "json",
        ])
        .expect("audit command");
        let Command::Audit(arguments) = cli.command else {
            panic!("expected audit command");
        };
        assert_eq!(arguments.path, PathBuf::from("workspace"));
        assert!(arguments.offline);
        assert_eq!(arguments.severity, AuditSeverity::Medium);
        assert_eq!(arguments.deny, Some(AuditSeverity::Critical));
        assert_eq!(arguments.format, ReportFormat::Json);
        assert!(
            validate_audit_thresholds(AuditSeverity::High, Some(AuditSeverity::Critical)).is_ok()
        );
        assert!(
            validate_audit_thresholds(AuditSeverity::Critical, Some(AuditSeverity::High)).is_err()
        );
    }

    #[test]
    fn dependency_reports_keep_every_shortest_transitive_path() {
        let package = |artifact: &str, dependencies: Vec<String>, selected_parent: Option<&str>| {
            LockedPackage {
                group: "org.example".to_owned(),
                artifact: artifact.to_owned(),
                version: "1".to_owned(),
                extension: "jar".to_owned(),
                classifier: None,
                source: "fixture".to_owned(),
                pom_checksum: format!("sha256:{}", "0".repeat(64)),
                artifact_checksum: None,
                artifact_size: None,
                dependencies,
                scopes: vec!["compile".to_owned()],
                selected_parent: selected_parent.map(ToOwned::to_owned),
            }
        };
        let child = "org.example:child:1".to_owned();
        let lock = Lockfile {
            lock_version: LOCK_VERSION,
            manifest_hash: format!("sha256:{}", "0".repeat(64)),
            workspace_hash: format!("sha256:{}", "0".repeat(64)),
            platform: "fixture".to_owned(),
            toolchain: LockedToolchain { java_release: 21 },
            packages: vec![
                package("root-a", vec![child.clone()], None),
                package("root-b", vec![child.clone()], None),
                package("child", Vec::new(), Some("org.example:root-a:1")),
            ],
            classpath: LockedClasspaths::default(),
        };
        let paths = shortest_dependency_paths(&lock);
        let child_paths = paths.get(&child).expect("child paths");
        assert_eq!(
            child_paths,
            &BTreeSet::from([
                vec!["org.example:root-a:1".to_owned(), child.clone()],
                vec!["org.example:root-b:1".to_owned(), child],
            ])
        );
    }

    #[test]
    fn audit_applies_alias_suppressions_and_denies_only_active_findings() {
        let package = jman_audit::AuditPackage {
            group: "org.example".to_owned(),
            artifact: "library".to_owned(),
            version: "1".to_owned(),
        };
        let finding = |id: &str, alias: &str, severity| jman_audit::Finding {
            package: package.clone(),
            advisory: jman_audit::Advisory {
                id: id.to_owned(),
                aliases: vec![alias.to_owned()],
                summary: "fixture".to_owned(),
                severity,
                fixed_versions: vec!["2".to_owned()],
                references: Vec::new(),
                modified: None,
            },
        };
        let report = evaluate_audit(
            jman_audit::AuditResult {
                provider: "fixture".to_owned(),
                source: jman_audit::AuditSource::Network,
                findings: vec![
                    finding("GHSA-active", "CVE-active", jman_audit::Severity::Critical),
                    finding("GHSA-muted", "CVE-muted", jman_audit::Severity::High),
                ],
            },
            &BTreeMap::from([(package, Vec::new())]),
            &[
                jman_config::AuditSuppression {
                    id: "CVE-muted".to_owned(),
                    reason: "not reachable".to_owned(),
                    expires: "2027-01-01".to_owned(),
                },
                jman_config::AuditSuppression {
                    id: "GHSA-active".to_owned(),
                    reason: "expired exception".to_owned(),
                    expires: "2025-01-01".to_owned(),
                },
            ],
            jman_audit::Severity::Medium,
            Some(jman_audit::Severity::High),
            "2026-09-19",
        );
        assert_eq!(report.active, 1);
        assert_eq!(report.suppressed, 1);
        assert_eq!(report.denied, 1);
        assert!(report.suppression_warnings[0].contains("expired"));
    }

    #[test]
    fn audit_calendar_conversion_is_stable() {
        assert_eq!(civil_date_from_unix_days(0), (1970, 1, 1));
        assert_eq!(civil_date_from_unix_days(20_350), (2025, 9, 19));
    }

    #[test]
    fn java_list_json_has_stable_sections_and_catalog_status() {
        let installed = jman_build::toolchain::ManagedJdk {
            vendor: "temurin".to_owned(),
            version: "21.0.2+13".to_owned(),
            major: 21,
            os: "linux".to_owned(),
            architecture: "x64".to_owned(),
            checksum: "sha256:test".to_owned(),
            home: PathBuf::from("/cache/jdks/21"),
        };
        let available = jman_build::toolchain::AvailableJdk {
            vendor: "temurin".to_owned(),
            version: "25.0.1+8".to_owned(),
            major: 25,
            lts: true,
            os: "linux".to_owned(),
            architecture: "x64".to_owned(),
        };
        let status = jman_build::toolchain::CatalogStatus {
            provider: "foojay".to_owned(),
            source: jman_build::toolchain::CatalogSource::StaleCache,
            fetched_at: 123,
            warning: Some("offline".to_owned()),
        };

        let json = java_list_json(&[installed], &[available], Some(&status)).expect("JSON");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        assert_eq!(value["catalog"]["source"], "stale-cache");
        assert_eq!(value["catalog"]["fetchedAt"], 123);
        assert_eq!(value["installed"][0]["home"], "/cache/jdks/21");
        assert_eq!(value["available"][0]["version"], "25.0.1+8");
    }

    #[test]
    fn java_table_merges_installed_releases_and_summarizes_each_vendor() {
        let installed = vec![managed_jdk("temurin", 21, "21.0.12+7", "/jdks/temurin-21")];
        let remote = vec![
            available_jdk("temurin", 17, "17.0.16+8"),
            available_jdk("zulu", 21, "21.0.9+10"),
            available_jdk("temurin", 21, "21.0.12+7"),
            available_jdk("zulu", 21, "21.0.10+7"),
        ];

        let rows = java_table_rows(&installed, &remote, false);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].vendor, "temurin");
        assert!(rows[0].installed);
        assert_eq!(rows[1].vendor, "zulu");
        assert_eq!(rows[1].version, "21.0.10+7");
        assert!(!rows[1].installed);

        let table = java_table(&installed, &remote, false);
        assert!(table.starts_with("INSTALLED  VENDOR"));
        assert!(table.contains("yes        temurin"));
        assert!(table.contains("no         zulu"));
        assert!(!table.contains("LOCATION"));
        assert!(!table.contains("/jdks/temurin-21"));
    }

    #[test]
    fn detailed_java_table_keeps_every_remote_release_and_local_only_install() {
        let installed = vec![managed_jdk(
            "corretto",
            11,
            "11.0.28+6",
            "/jdks/corretto-11",
        )];
        let remote = vec![
            available_jdk("temurin", 21, "21.0.12+7"),
            available_jdk("temurin", 17, "17.0.16+8"),
        ];

        let rows = java_table_rows(&installed, &remote, true);

        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().filter(|row| row.installed).count(), 1);
        assert!(rows.iter().any(|row| row.vendor == "corretto"));
        assert!(rows.iter().any(|row| row.version == "21.0.12+7"));
        assert!(rows.iter().any(|row| row.version == "17.0.16+8"));
    }

    fn managed_jdk(
        vendor: &str,
        major: u16,
        version: &str,
        home: &str,
    ) -> jman_build::toolchain::ManagedJdk {
        jman_build::toolchain::ManagedJdk {
            vendor: vendor.to_owned(),
            version: version.to_owned(),
            major,
            os: "linux".to_owned(),
            architecture: "x64".to_owned(),
            checksum: "sha256:test".to_owned(),
            home: PathBuf::from(home),
        }
    }

    fn available_jdk(
        vendor: &str,
        major: u16,
        version: &str,
    ) -> jman_build::toolchain::AvailableJdk {
        jman_build::toolchain::AvailableJdk {
            vendor: vendor.to_owned(),
            version: version.to_owned(),
            major,
            lts: jman_build::toolchain::is_lts_major(major),
            os: "linux".to_owned(),
            architecture: "x64".to_owned(),
        }
    }

    #[test]
    fn workspace_manifest_and_lock_use_distinct_staging_paths() {
        let directory = Path::new("/workspace/module");

        let manifest = staging_path(&directory.join("jman.toml"));
        let lock = staging_path(&directory.join("jman.lock"));

        assert_ne!(manifest, lock);
        assert_eq!(manifest.parent(), Some(directory));
        assert_eq!(lock.parent(), Some(directory));
    }

    #[test]
    fn container_diagnostics_redact_endpoint_credentials() {
        assert_eq!(
            display_container_endpoint("tcp://user:secret@example.test:2376"),
            "tcp://***@example.test:2376"
        );
        assert_eq!(
            display_container_endpoint("unix:///run/user/1000/podman.sock"),
            "unix:///run/user/1000/podman.sock"
        );
    }

    #[test]
    fn warm_lock_requires_matching_inputs_and_present_artifacts() {
        let cache = tempfile::tempdir().expect("cache");
        let artifacts = cache.path().join("artifacts");
        fs::create_dir(&artifacts).expect("artifacts directory");
        fs::write(artifacts.join("abc.jar"), b"artifact").expect("artifact");
        let lock = Lockfile {
            lock_version: LOCK_VERSION,
            manifest_hash: "sha256:manifest".to_owned(),
            workspace_hash: "sha256:pom".to_owned(),
            platform: platform_id(),
            toolchain: LockedToolchain { java_release: 25 },
            packages: vec![LockedPackage {
                group: "org.example".to_owned(),
                artifact: "library".to_owned(),
                version: "1".to_owned(),
                extension: "jar".to_owned(),
                classifier: None,
                source: "fixture".to_owned(),
                pom_checksum: "sha256:pom".to_owned(),
                artifact_checksum: Some("sha256:abc".to_owned()),
                artifact_size: Some(8),
                dependencies: Vec::new(),
                scopes: vec!["compile".to_owned()],
                selected_parent: None,
            }],
            classpath: LockedClasspaths::default(),
        };

        assert!(lock_is_current(
            &lock,
            "sha256:manifest",
            "sha256:pom",
            25,
            cache.path(),
            false,
        ));
        assert!(!lock_is_current(
            &lock,
            "sha256:changed",
            "sha256:pom",
            25,
            cache.path(),
            false,
        ));
        fs::remove_file(artifacts.join("abc.jar")).expect("remove fixture artifact");
        assert!(!lock_is_current(
            &lock,
            "sha256:manifest",
            "sha256:pom",
            25,
            cache.path(),
            false,
        ));
    }
}
