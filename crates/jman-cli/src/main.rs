use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use futures::{stream, StreamExt, TryStreamExt};
use jman_config::{
    Build, Dependencies, LockedClasspaths, LockedPackage, LockedToolchain, Lockfile, Manifest,
    MavenCompatibility, MavenDependencyMetadata, MavenManagedDependency, Project, Repository,
    Toolchain as ManifestToolchain, LOCK_VERSION, MANIFEST_VERSION,
};
use jman_resolver::{
    plugin_coordinates, AnnotationProcessor, Coordinate, Dependency, DependencyResolver,
    EffectivePom, Exclusion, MavenProject, RepositoryClient, ResolvedGraph,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

mod ui;

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
    /// Display the resolved dependency tree.
    Tree(ProjectPath),
    /// Explain why a dependency is present.
    Why(Why),
    /// Type-check and compile main Java sources.
    Check(Check),
    /// Build standard and optional module artifacts.
    Build(BuildCommand),
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
    /// Download and install a verified Temurin JDK.
    Install(JavaVersion),
    /// List JDKs installed by JMAN.
    List,
    /// Remove a JDK installed by JMAN.
    Remove(JavaRemove),
    /// Pin this project to a JDK version.
    Use(JavaUse),
    /// Print the managed JDK selected for this project.
    Which(ProjectPath),
}

#[derive(Debug, Args)]
struct JavaVersion {
    /// JDK major or exact version, such as 17 or 17.0.20+8.
    version: String,
    /// Resolve only from installed JDKs.
    #[arg(long)]
    offline: bool,
}

#[derive(Debug, Args)]
struct JavaUse {
    /// JDK major or exact version, such as 17 or 17.0.20+8.
    version: String,
    /// Project directory.
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

#[derive(Debug, Args)]
struct JavaRemove {
    /// JDK major or exact installed version.
    version: String,
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
        Command::Tree(arguments) => dependency_tree(&arguments.path),
        Command::Why(arguments) => dependency_why(&arguments),
        Command::Check(arguments) => compile_project(&arguments, ui, "Checking", "Checked").await,
        Command::Build(arguments) => build_project(&arguments, ui).await,
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
    };
    let tests = jman_build::test_workspace(
        &workspace_root,
        &cache_dir,
        jobs,
        arguments.offline,
        &test_options,
    );
    tokio::pin!(tests);
    let results = tokio::select! {
        result = &mut tests => result.context("JUnit test execution failed")?,
        signal = test_cancellation_signal() => {
            signal.context("could not install the test cancellation handler")?;
            bail!("test run cancelled; isolated JVM workers were terminated")
        }
    };
    if results.is_empty() {
        ui.success("No test sources found");
        return Ok(());
    }
    for result in &results {
        report_test_module(result, arguments.report, ui)?;
    }
    finish_test_results(arguments, ui, &results)
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
) -> Result<()> {
    if format == ReportFormat::Json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "protocolVersion": 2,
                "reason": "test-module-started",
                "module": result.module,
                "debug": {
                    "supported": true,
                    "transport": "java-debug-adapter",
                    "suspend": true
                },
                "coverage": {
                    "supported": false,
                    "reason": "Coverage collection is not bundled in the MVP runner"
                }
            }))?
        );
        for test_case in &result.test_cases {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "protocolVersion": 2,
                    "reason": "test-case",
                    "module": result.module,
                    "test": test_case,
                }))?
            );
        }
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "protocolVersion": 2,
                "reason": "test-module-finished",
                "module": result.module,
                "successful": result.status.success(),
                "summary": result.summary
            }))?
        );
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
        let report = format!(
            "{}: {} passed / {} failed / {} skipped ({})",
            result.module,
            summary.tests_successful,
            summary.tests_failed,
            summary.tests_skipped,
            ui::format_duration(std::time::Duration::from_millis(summary.duration_millis))
        );
        if result.status.success() {
            ui.success(report);
        } else {
            ui.warning(report);
        }
    }
    Ok(())
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
            let activity = ui.activity(format!("Installing Temurin JDK {}", arguments.version));
            let report_download = |downloaded, total| {
                activity.set_download_progress(downloaded, total);
            };
            let jdk = manager
                .install_with_progress(
                    &ToolchainRequest {
                        version: arguments.version,
                        vendor: "temurin".to_owned(),
                    },
                    arguments.offline,
                    Some(&report_download),
                )
                .await?;
            activity.finish(format!(
                "Installed Temurin {} at {}",
                jdk.version,
                jdk.home.display()
            ));
        }
        JavaCommand::List => list_java(&manager).await?,
        JavaCommand::Remove(arguments) => remove_java(&manager, arguments, ui).await?,
        JavaCommand::Use(arguments) => {
            let target = absolute_path(&arguments.path)?;
            let path = target.join("jman.toml");
            let mut manifest = Manifest::read(&path)
                .with_context(|| format!("could not read {}", path.display()))?;
            let selected_version = arguments.version;
            manifest.toolchain = Some(ManifestToolchain {
                jdk: selected_version.clone(),
                vendor: "temurin".to_owned(),
            });
            let text = manifest
                .to_toml()
                .context("invalid JDK selection for this project")?;
            write_atomically(&path, &text).await?;
            ui.success(format!(
                "Pinned {} to Temurin JDK {}",
                manifest.project.name, selected_version
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

async fn list_java(manager: &jman_build::toolchain::ToolchainManager) -> Result<()> {
    let jdks = manager.list().await?;
    if jdks.is_empty() {
        println!("No JMAN-managed JDKs installed.");
    } else {
        for jdk in jdks {
            println!(
                "{} {} {}-{} {}",
                jdk.vendor,
                jdk.version,
                jdk.os,
                jdk.architecture,
                jdk.home.display()
            );
        }
    }
    Ok(())
}

async fn remove_java(
    manager: &jman_build::toolchain::ToolchainManager,
    arguments: JavaRemove,
    ui: &Ui,
) -> Result<()> {
    let request = jman_build::toolchain::ToolchainRequest {
        version: arguments.version,
        vendor: "temurin".to_owned(),
    };
    let candidates = manager.removal_candidates(&request, arguments.all).await?;
    if candidates.is_empty() {
        bail!(
            "no JMAN-managed Temurin JDK {} installation was found",
            request.version
        );
    }
    if !arguments.force {
        protect_pinned_toolchain(&arguments.path, &candidates)?;
    }
    if arguments.dry_run {
        for jdk in candidates {
            println!(
                "Would remove Temurin {} at {}",
                jdk.version,
                jdk.home.display()
            );
        }
        return Ok(());
    }
    let activity = ui.activity(format!("Removing Temurin JDK {}", request.version));
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
    if candidates
        .iter()
        .any(|jdk| pin.jdk == jdk.version || pin_major == Some(jdk.major))
    {
        bail!(
            "{} pins Temurin JDK {}; use --force to remove it anyway",
            manifest.project.name,
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
            vendor: toolchain.vendor.clone(),
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

#[allow(clippy::too_many_lines)]
async fn import_maven(pom_path: &Path, ui: &Ui) -> Result<()> {
    let model_activity = ui.activity("Constructing effective Maven reactor");
    let bootstrap_repository = RepositoryClient::with_default_cache(Vec::new())
        .context("could not initialize Maven repository client")?;
    let bootstrap_resolver = DependencyResolver::new(bootstrap_repository);
    let projects = bootstrap_resolver
        .import_workspace(pom_path)
        .await
        .context("could not construct the Maven reactor")?;
    model_activity.finish(format!("Loaded Maven reactor ({} modules)", projects.len()));
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
            manifest_from_maven(&project.effective).and_then(|mut manifest| {
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
    let packages = lock
        .packages
        .iter()
        .map(|package| (locked_coordinate(package), package))
        .collect::<BTreeMap<_, _>>();
    let mut queue = std::collections::VecDeque::new();
    let mut previous = BTreeMap::<String, String>::new();
    for dependency in locked_root_dependencies(&lock) {
        previous.insert(dependency.clone(), root.clone());
        queue.push_back(dependency);
    }
    let found = loop {
        let Some(coordinate) = queue.pop_front() else {
            break None;
        };
        if coordinate_ga(&coordinate) == arguments.dependency {
            break Some(coordinate);
        }
        if let Some(package) = packages.get(&coordinate) {
            for child in &package.dependencies {
                if !previous.contains_key(child) {
                    previous.insert(child.clone(), coordinate.clone());
                    queue.push_back(child.clone());
                }
            }
        }
    };
    let Some(mut current) = found else {
        bail!(
            "dependency `{}` is not present in jman.lock",
            arguments.dependency
        );
    };
    let mut path = vec![current.clone()];
    while let Some(parent) = previous.get(&current) {
        path.push(parent.clone());
        if parent == &root {
            break;
        }
        parent.clone_into(&mut current);
    }
    path.reverse();
    println!("{}", path.join(" -> "));
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
fn manifest_from_maven(effective: &EffectivePom) -> Result<Manifest> {
    let java_release = ["maven.compiler.release", "release.version", "jdk.version"]
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
            main_class: None,
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
        build: Some(Build {
            encoding: effective
                .properties
                .get("project.build.sourceEncoding")
                .cloned()
                .unwrap_or_else(|| "UTF-8".to_owned()),
            compiler_args: Vec::new(),
        }),
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
        annotation_processors: BTreeMap::new(),
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
