use std::path::{Path, PathBuf};
use std::process::Command;

use jman_config::{Lockfile, Manifest};
use serde::Deserialize;
use serde_json::Value;

use crate::build_runtime::{BuildRuntime, select_gradle_runtime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildTool {
    Jman { root: PathBuf },
    Gradle { root: PathBuf, executable: PathBuf },
    Maven { root: PathBuf, executable: PathBuf },
}

#[derive(Debug)]
pub struct LoadedProject {
    pub tool: BuildTool,
    pub models: Vec<CompileModel>,
    pub build_runtime: Option<BuildRuntime>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileModel {
    pub schema_version: u32,
    pub build_system: String,
    pub project_path: String,
    pub project_directory: PathBuf,
    pub task_path: String,
    #[serde(default)]
    pub source_files: Vec<PathBuf>,
    #[serde(default)]
    pub source_roots: Vec<PathBuf>,
    #[serde(default)]
    pub classpath: Vec<PathBuf>,
    #[serde(default)]
    pub module_path: Vec<PathBuf>,
    #[serde(default)]
    pub project_dependencies: Vec<String>,
    #[serde(default)]
    pub annotation_processor_path: Vec<PathBuf>,
    #[serde(default)]
    pub annotation_processor_options: Vec<String>,
    #[serde(default)]
    pub compiler_args: Vec<String>,
    pub release: Option<Value>,
    pub encoding: Option<String>,
    pub generated_sources_directory: Option<PathBuf>,
    #[serde(default)]
    pub generated_source_directories: Vec<PathBuf>,
    pub destination_directory: Option<PathBuf>,
    pub java_compiler_executable: Option<PathBuf>,
    pub java_language_version: Option<Value>,
    #[serde(default)]
    pub resolution_errors: Vec<ModelResolutionError>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelResolutionError {
    pub component: String,
    pub message: String,
}

impl CompileModel {
    pub fn java_release(&self) -> Option<u8> {
        numeric_value(self.release.as_ref())
            .or_else(|| numeric_value(self.java_language_version.as_ref()))
    }

    pub fn source_path(&self) -> Vec<PathBuf> {
        let mut roots = self.source_roots.clone();
        for source in &self.source_files {
            if let Some(root) = conventional_source_root(source)
                && !roots.contains(&root)
            {
                roots.push(root);
            }
        }
        if let Some(generated) = &self.generated_sources_directory
            && !roots.contains(generated)
        {
            roots.push(generated.clone());
        }
        for generated in &self.generated_source_directories {
            if !roots.contains(generated) {
                roots.push(generated.clone());
            }
        }
        roots
    }
}

pub fn detect_build_tools(root: &Path) -> Vec<BuildTool> {
    let root = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    let mut tools = Vec::new();
    if root.join("jman.toml").is_file() {
        tools.push(BuildTool::Jman { root: root.clone() });
    }
    let gradlew = root.join("gradlew");
    if gradlew.is_file()
        || root.join("settings.gradle").is_file()
        || root.join("settings.gradle.kts").is_file()
        || root.join("build.gradle").is_file()
        || root.join("build.gradle.kts").is_file()
    {
        tools.push(BuildTool::Gradle {
            root: root.clone(),
            executable: if gradlew.is_file() {
                gradlew
            } else {
                PathBuf::from("gradle")
            },
        });
    }
    if root.join("pom.xml").is_file() {
        let wrapper = root.join("mvnw");
        tools.push(BuildTool::Maven {
            root,
            executable: if wrapper.is_file() {
                wrapper
            } else {
                PathBuf::from("mvn")
            },
        });
    }
    tools
}

pub fn parse_models(ndjson: &str) -> Result<Vec<CompileModel>, serde_json::Error> {
    ndjson
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect()
}

pub fn select_compile_model<'a>(
    models: &'a [CompileModel],
    source: &Path,
) -> Option<&'a CompileModel> {
    models
        .iter()
        .filter(|model| {
            model
                .source_files
                .iter()
                .any(|candidate| candidate == source)
                || model
                    .source_path()
                    .iter()
                    .any(|root| source.starts_with(root))
                || (is_main_compile_task(&model.task_path)
                    && source.starts_with(&model.project_directory))
        })
        .min_by_key(|model| {
            if model
                .source_files
                .iter()
                .any(|candidate| candidate == source)
            {
                (0, 0)
            } else if let Some(depth) = model
                .source_path()
                .iter()
                .filter(|root| source.starts_with(root))
                .map(|root| root.components().count())
                .max()
            {
                (1, usize::MAX - depth)
            } else {
                (2, usize::MAX - model.project_directory.components().count())
            }
        })
}

pub fn load_project(
    root: &Path,
    preferred_build_system: Option<&str>,
) -> Result<LoadedProject, String> {
    let started = std::time::Instant::now();
    let tools = detect_build_tools(root);
    let tool = tools
        .iter()
        .find(|tool| {
            matches!(
                (preferred_build_system, tool),
                (Some("jman"), BuildTool::Jman { .. })
                    | (Some("gradle"), BuildTool::Gradle { .. })
                    | (Some("maven"), BuildTool::Maven { .. })
            )
        })
        .or_else(|| tools.first())
        .cloned()
        .ok_or_else(|| "no JMAN, Maven, or Gradle build was detected".to_owned())?;
    if matches!(tool, BuildTool::Jman { .. }) {
        let models = load_jman_models(root)?;
        return Ok(LoadedProject {
            tool,
            models,
            build_runtime: None,
        });
    }
    let cache = model_cache_path(root, &tool);
    if let Some(parent) = cache.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create model cache: {error}"))?;
    }
    let _lock = crate::cache::FileLock::acquire(&cache)?;
    let build_runtime = import_project(&tool, root, &cache)?;
    let encoded = std::fs::read_to_string(&cache)
        .map_err(|error| format!("cannot read {}: {error}", cache.display()))?;
    let mut models = parse_models(&encoded)
        .map_err(|error| format!("invalid compile model {}: {error}", cache.display()))?;
    retain_usable_model_paths(&mut models);
    if models.is_empty() {
        return Err("build importer returned no compile units".to_owned());
    }
    crate::metrics::emit(
        "build-import",
        serde_json::json!({
            "milliseconds": started.elapsed().as_millis(),
            "compileUnits": models.len(),
            "sourceFiles": models.iter().map(|model| model.source_files.len()).sum::<usize>(),
            "partialUnits": models.iter().filter(|model| !model.resolution_errors.is_empty()).count()
        }),
    );
    Ok(LoadedProject {
        tool,
        models,
        build_runtime,
    })
}

fn retain_usable_model_paths(models: &mut [CompileModel]) {
    let project_directories: Vec<_> = models
        .iter()
        .map(|model| model.project_directory.clone())
        .collect();
    let usable = |path: &Path| {
        path.is_file()
            || path.is_dir()
            || project_directories
                .iter()
                .any(|directory| path.starts_with(directory))
    };
    for model in models {
        model.classpath.retain(|path| usable(path));
        model.module_path.retain(|path| usable(path));
        model.annotation_processor_path.retain(|path| usable(path));
    }
}

fn import_project(
    tool: &BuildTool,
    root: &Path,
    cache: &Path,
) -> Result<Option<BuildRuntime>, String> {
    // The language server owns stdout: every byte written there must be an LSP
    // frame. Capture build-tool output so Maven/Gradle diagnostics can never
    // corrupt the JSON-RPC transport.
    let temporary = cache.with_extension(format!("ndjson.{}.tmp", std::process::id()));
    let build_runtime = match tool {
        BuildTool::Gradle { .. } => select_gradle_runtime(root),
        BuildTool::Maven { .. } => Ok(None),
        BuildTool::Jman { .. } => unreachable!("JMAN models do not use an external importer"),
    };
    let result = match (tool, &build_runtime) {
        (BuildTool::Jman { .. }, _) => {
            unreachable!("JMAN models do not use an external importer")
        }
        (BuildTool::Gradle { executable, .. }, Ok(runtime)) => {
            import_gradle(executable, root, &temporary, runtime.as_ref())
        }
        (BuildTool::Gradle { .. }, Err(error)) => Err(error.clone()),
        (BuildTool::Maven { executable, .. }, _) => import_maven(executable, root, &temporary),
    };
    if result.is_ok() {
        let encoded = std::fs::read_to_string(&temporary)
            .map_err(|error| format!("importer produced no model: {error}"))?;
        let models = parse_models(&encoded)
            .map_err(|error| format!("importer produced an invalid model: {error}"))?;
        if models.is_empty() {
            return Err("build importer returned no compile units".to_owned());
        }
        std::fs::rename(&temporary, cache)
            .map_err(|error| format!("cannot publish build model: {error}"))?;
        return Ok(build_runtime.ok().flatten());
    }
    let _ = std::fs::remove_file(&temporary);
    let diagnostic = result.unwrap_err();
    if std::fs::read_to_string(cache)
        .ok()
        .and_then(|encoded| parse_models(&encoded).ok())
        .is_some_and(|models| !models.is_empty())
    {
        crate::metrics::emit(
            "build-import-fallback",
            serde_json::json!({
                "buildSystem": match tool {
                    BuildTool::Jman { .. } => "jman",
                    BuildTool::Gradle { .. } => "gradle",
                    BuildTool::Maven { .. } => "maven",
                },
                "diagnostic": diagnostic.trim()
            }),
        );
        return Ok(build_runtime.ok().flatten());
    }
    Err(diagnostic)
}

fn import_gradle(
    executable: &Path,
    root: &Path,
    output: &Path,
    runtime: Option<&BuildRuntime>,
) -> Result<(), String> {
    let support = support_directory();
    let init_script = support.join("tools/gradle-importer/javac-frontend-model.init.gradle");
    if !init_script.is_file() {
        return Err(format!(
            "Gradle model importer is missing: {}",
            init_script.display()
        ));
    }
    let gradle_home = std::env::var_os("JAVA_LSP_GRADLE_USER_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join("io.github.zonnedev.jman.lsp/gradle-user-home")
        });
    std::fs::create_dir_all(&gradle_home)
        .map_err(|error| format!("cannot create {}: {error}", gradle_home.display()))?;
    let mut command = Command::new(executable);
    command
        .current_dir(root)
        .env("GRADLE_USER_HOME", gradle_home)
        .args(["--console=plain", "--no-configuration-cache", "-I"])
        .arg(&init_script)
        .arg("javaFrontendModel");
    if let Some(runtime) = runtime {
        configure_java_home(&mut command, &runtime.java_home);
        eprintln!("jman-java-lsp build import: {}", runtime.summary());
    }
    let process = captured_output(&mut command, "Gradle model import")?;
    let model = process
        .lines()
        .filter_map(|line| line.strip_prefix("JAVAC_FRONTEND_MODEL "))
        .collect::<Vec<_>>()
        .join("\n");
    if model.is_empty() {
        return Err("Gradle model import produced no compile units".to_owned());
    }
    std::fs::write(output, format!("{model}\n"))
        .map_err(|error| format!("cannot write {}: {error}", output.display()))
}

fn import_maven(executable: &Path, root: &Path, output: &Path) -> Result<(), String> {
    let support = support_directory();
    let importer = std::env::var_os("JAVAC_FRONTEND_IMPORTER_CLASSES")
        .map(PathBuf::from)
        .or_else(|| {
            let packaged = support.join("maven-importer.jar");
            packaged.is_file().then_some(packaged)
        })
        .unwrap_or_else(|| support.join("target/java-test-classes"));
    if !importer.exists() {
        return Err(format!(
            "Maven model importer is missing: {}",
            importer.display()
        ));
    }
    let state = output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("maven-import");
    let local_repository = std::env::var_os("JAVA_LSP_MAVEN_REPOSITORY")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".m2/repository"))
                .unwrap_or_else(|| {
                    std::env::temp_dir().join("io.github.zonnedev.jman.lsp/maven-repository")
                })
        });
    std::fs::create_dir_all(&state)
        .and_then(|()| std::fs::create_dir_all(&local_repository))
        .map_err(|error| format!("cannot create Maven importer state: {error}"))?;
    let effective_pom = state.join("effective-pom.xml");
    let classpath = state.join("classpath.txt");
    let common = [
        "--batch-mode",
        "--no-transfer-progress",
        "-q",
        "-Denforcer.skip=true",
        "-DskipTests",
    ];
    let run_maven = |goal: &str, extra: Option<String>| {
        let mut command = Command::new(executable);
        command
            .current_dir(root)
            .args(common)
            .arg(format!("-Dmaven.repo.local={}", local_repository.display()));
        command.arg(goal);
        if let Some(extra) = extra {
            command.arg(extra);
        }
        configure_build_java(&mut command, "17.0.20-tem");
        captured_output(&mut command, &format!("Maven {goal}"))
    };
    let _ = run_maven("generate-sources", None);
    run_maven(
        "help:effective-pom",
        Some(format!("-Doutput={}", effective_pom.display())),
    )?;
    let _ = run_maven(
        "dependency:build-classpath",
        Some(format!("-Dmdep.outputFile={}", classpath.display())),
    );
    if !classpath.exists() {
        std::fs::write(&classpath, "")
            .map_err(|error| format!("cannot create {}: {error}", classpath.display()))?;
    }
    let java = build_java_executable("17.0.20-tem");
    let mut command = Command::new(&java);
    command
        .current_dir(root)
        .arg("-cp")
        .arg(&importer)
        .arg("io.github.zonnedev.jman.maven.importer.MavenModelImporter")
        .arg(&effective_pom)
        .arg(&classpath)
        .arg(&local_repository);
    let model = captured_output(&mut command, "Maven model conversion")?;
    if model.trim().is_empty() {
        return Err("Maven model conversion produced no compile units".to_owned());
    }
    std::fs::write(output, model)
        .map_err(|error| format!("cannot write {}: {error}", output.display()))
}

fn captured_output(command: &mut Command, description: &str) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|error| format!("cannot start {description}: {error}"))?;
    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map_err(|error| format!("{description} emitted invalid UTF-8: {error}"));
    }
    Err(format!(
        "{description} failed with status {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

fn support_directory() -> PathBuf {
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
        && directory.join("tools/gradle-importer").is_dir()
    {
        return directory.to_path_buf();
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn build_java_home(default_sdkman_candidate: &str) -> Option<PathBuf> {
    std::env::var_os("JAVA_LSP_BUILD_JAVA_HOME")
        .or_else(|| std::env::var_os("JAVA_HOME"))
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| {
                PathBuf::from(home)
                    .join(".sdkman/candidates/java")
                    .join(default_sdkman_candidate)
            })
        })
        .filter(|home| home.join("bin/java").is_file())
}

fn build_java_executable(default_sdkman_candidate: &str) -> PathBuf {
    build_java_home(default_sdkman_candidate)
        .map(|home| home.join("bin/java"))
        .unwrap_or_else(|| PathBuf::from("java"))
}

fn configure_build_java(command: &mut Command, default_sdkman_candidate: &str) {
    let Some(java_home) = build_java_home(default_sdkman_candidate) else {
        return;
    };
    configure_java_home(command, &java_home);
}

fn configure_java_home(command: &mut Command, java_home: &Path) {
    command.env("JAVA_HOME", java_home);
    let mut paths = vec![java_home.join("bin")];
    if let Some(current) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&current));
    }
    if let Ok(path) = std::env::join_paths(paths) {
        command.env("PATH", path);
    }
}

pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    Some(PathBuf::from(percent_decode(path)?))
}

fn model_cache_path(root: &Path, tool: &BuildTool) -> PathBuf {
    let name = match tool {
        BuildTool::Jman { .. } => "jman.ndjson",
        BuildTool::Gradle { .. } => "gradle.ndjson",
        BuildTool::Maven { .. } => "maven.ndjson",
    };
    crate::cache::project_cache_directory(root)
        .join("build-model")
        .join(name)
}

fn load_jman_models(root: &Path) -> Result<Vec<CompileModel>, String> {
    let root = std::fs::canonicalize(root).map_err(|error| {
        format!(
            "cannot canonicalize JMAN workspace {}: {error}",
            root.display()
        )
    })?;
    let cache = std::env::var_os("JMAN_CACHE_DIR").map_or_else(
        || {
            dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from(".jman-cache"))
                .join("jman")
        },
        PathBuf::from,
    );
    let mut pending = vec![root.clone()];
    let mut seen = std::collections::BTreeSet::new();
    let mut modules = Vec::new();
    while let Some(directory) = pending.pop() {
        if !seen.insert(directory.clone()) {
            return Err(format!(
                "duplicate or cyclic JMAN module declaration at {}",
                directory.display()
            ));
        }
        let manifest = Manifest::read(&directory.join("jman.toml"))
            .map_err(|error| format!("cannot read JMAN module {}: {error}", directory.display()))?;
        for child in manifest.project.modules.iter().rev() {
            let child = std::fs::canonicalize(directory.join(child)).map_err(|error| {
                format!(
                    "cannot resolve JMAN module `{child}` from {}: {error}",
                    directory.display()
                )
            })?;
            if !child.starts_with(&root) {
                return Err(format!(
                    "JMAN module {} escapes workspace {}",
                    child.display(),
                    root.display()
                ));
            }
            pending.push(child);
        }
        modules.push((directory, manifest));
    }

    let outputs = modules
        .iter()
        .map(|(directory, manifest)| {
            (
                format!("{}:{}", manifest.project.group, manifest.project.name),
                directory.join(".jman/output/classes"),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut models = Vec::new();
    for (directory, manifest) in modules {
        let lock = Lockfile::read(&directory.join("jman.lock")).map_err(|error| {
            format!(
                "cannot read JMAN lockfile for {}: {error}; run `jman sync`",
                directory.display()
            )
        })?;
        let compile_dependencies = jman_artifact_paths(&lock.classpath.compile, &lock, &cache)?;
        let test_dependencies = jman_artifact_paths(&lock.classpath.test, &lock, &cache)?;
        let processors = jman_artifact_paths(&lock.classpath.processors, &lock, &cache)?;
        let project_outputs = manifest
            .path_dependencies
            .keys()
            .filter_map(|coordinate| outputs.get(coordinate))
            .cloned()
            .collect::<Vec<_>>();
        let build = manifest.build.as_ref();
        let encoding = build
            .map(|build| build.encoding.clone())
            .unwrap_or_else(|| "UTF-8".to_owned());
        let compiler_args = build
            .map(|build| build.compiler_args.clone())
            .unwrap_or_default();
        let main_output = directory.join(".jman/output/classes");
        let main_generated = directory.join(".jman/output/generated/sources/annotations");
        let mut main_classpath = project_outputs.clone();
        main_classpath.extend(compile_dependencies.clone());
        models.push(CompileModel {
            schema_version: 2,
            build_system: "jman".to_owned(),
            project_path: manifest.project.name.clone(),
            project_directory: directory.clone(),
            task_path: "compile".to_owned(),
            source_files: Vec::new(),
            source_roots: vec![directory.join("src/main/java")],
            classpath: main_classpath.clone(),
            module_path: Vec::new(),
            project_dependencies: manifest.path_dependencies.keys().cloned().collect(),
            annotation_processor_path: processors.clone(),
            annotation_processor_options: Vec::new(),
            compiler_args: compiler_args.clone(),
            release: Some(Value::from(manifest.project.java_release)),
            encoding: Some(encoding.clone()),
            generated_sources_directory: Some(main_generated.clone()),
            generated_source_directories: vec![main_generated],
            destination_directory: Some(main_output.clone()),
            java_compiler_executable: None,
            java_language_version: Some(Value::from(manifest.project.java_release)),
            resolution_errors: Vec::new(),
        });
        let mut test_classpath = vec![main_output];
        test_classpath.extend(main_classpath);
        test_classpath.extend(test_dependencies);
        models.push(CompileModel {
            schema_version: 2,
            build_system: "jman".to_owned(),
            project_path: manifest.project.name,
            project_directory: directory.clone(),
            task_path: "testCompile".to_owned(),
            source_files: Vec::new(),
            source_roots: vec![directory.join("src/test/java")],
            classpath: test_classpath,
            module_path: Vec::new(),
            project_dependencies: manifest.path_dependencies.keys().cloned().collect(),
            annotation_processor_path: processors,
            annotation_processor_options: Vec::new(),
            compiler_args,
            release: Some(Value::from(manifest.project.java_release)),
            encoding: Some(encoding),
            generated_sources_directory: Some(
                directory.join(".jman/output/generated/test-sources/annotations"),
            ),
            generated_source_directories: vec![
                directory.join(".jman/output/generated/test-sources/annotations"),
            ],
            destination_directory: Some(directory.join(".jman/output/test-classes")),
            java_compiler_executable: None,
            java_language_version: Some(Value::from(manifest.project.java_release)),
            resolution_errors: Vec::new(),
        });
    }
    Ok(models)
}

fn jman_artifact_paths(
    checksums: &[String],
    lock: &Lockfile,
    cache: &Path,
) -> Result<Vec<PathBuf>, String> {
    checksums
        .iter()
        .map(|checksum| {
            let package = lock
                .packages
                .iter()
                .find(|package| package.artifact_checksum.as_deref() == Some(checksum))
                .ok_or_else(|| {
                    format!(
                        "JMAN classpath checksum `{checksum}` has no package in jman.lock; run `jman sync`"
                    )
                })?;
            let digest = checksum.strip_prefix("sha256:").ok_or_else(|| {
                format!("invalid JMAN classpath checksum `{checksum}`")
            })?;
            Ok(cache
                .join("artifacts")
                .join(format!("{digest}.{}", package.extension)))
        })
        .collect()
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let encoded = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            decoded.push(u8::from_str_radix(encoded, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn is_main_compile_task(task: &str) -> bool {
    task == "compile" || task.ends_with(":compileJava")
}

fn numeric_value(value: Option<&Value>) -> Option<u8> {
    value.and_then(|value| {
        value
            .as_u64()
            .and_then(|number| u8::try_from(number).ok())
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
    })
}

fn conventional_source_root(source: &Path) -> Option<PathBuf> {
    let components: Vec<_> = source.components().collect();
    for index in 0..components.len().saturating_sub(2) {
        if components[index].as_os_str() == "src"
            && matches!(
                components[index + 1].as_os_str().to_str(),
                Some("main" | "test")
            )
            && components[index + 2].as_os_str() == "java"
        {
            return Some(components[..=index + 2].iter().collect());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn detects_both_builds_and_prefers_the_project_wrappers() {
        let root = std::env::temp_dir().join(format!("jman-java-detect-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("pom.xml"), "<project/>").unwrap();
        std::fs::write(root.join("mvnw"), "").unwrap();
        std::fs::write(root.join("settings.gradle"), "").unwrap();
        std::fs::write(root.join("gradlew"), "").unwrap();

        let tools = detect_build_tools(&root);
        assert!(
            matches!(&tools[0], BuildTool::Gradle { executable, .. } if executable.ends_with("gradlew"))
        );
        assert!(
            matches!(&tools[1], BuildTool::Maven { executable, .. } if executable.ends_with("mvnw"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_detection_prioritizes_jman_over_gradle_and_maven() {
        let root =
            std::env::temp_dir().join(format!("jman-java-jman-first-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("jman.toml"), "manifest-version = 1").unwrap();
        std::fs::write(root.join("pom.xml"), "<project/>").unwrap();
        std::fs::write(root.join("settings.gradle"), "").unwrap();

        let tools = detect_build_tools(&root);
        assert!(matches!(&tools[0], BuildTool::Jman { .. }));
        assert!(matches!(&tools[1], BuildTool::Gradle { .. }));
        assert!(matches!(&tools[2], BuildTool::Maven { .. }));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loads_native_jman_main_and_test_compile_units() {
        let root =
            std::env::temp_dir().join(format!("jman-java-jman-model-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src/main/java/demo")).unwrap();
        std::fs::create_dir_all(root.join("src/test/java/demo")).unwrap();
        std::fs::write(
            root.join("jman.toml"),
            r#"manifest-version = 1

[project]
group = "com.example"
name = "demo"
version = "0.1.0"
java-release = 25
packaging = "jar"

[build]
encoding = "UTF-16"
compiler-args = ["-parameters"]
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("jman.lock"),
            r#"lock-version = 3
manifest-hash = "sha256:00"
workspace-hash = "sha256:00"
platform = "test"

[toolchain]
java-release = 25

[classpath]
compile = []
runtime = []
test = []
processors = []
"#,
        )
        .unwrap();

        let loaded = load_project(&root, None).expect("load JMAN workspace");
        assert!(matches!(loaded.tool, BuildTool::Jman { .. }));
        assert_eq!(loaded.models.len(), 2);
        assert!(
            loaded
                .models
                .iter()
                .all(|model| model.build_system == "jman")
        );
        assert_eq!(loaded.models[0].java_release(), Some(25));
        assert_eq!(loaded.models[0].encoding.as_deref(), Some("UTF-16"));
        assert_eq!(loaded.models[0].compiler_args, ["-parameters"]);
        assert_eq!(
            select_compile_model(
                &loaded.models,
                &root.join("src/test/java/demo/DemoTest.java")
            )
            .expect("test compile unit")
            .task_path,
            "testCompile"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_and_selects_the_owning_main_compile_unit() {
        let models = parse_models(
            r#"{"schemaVersion":1,"buildSystem":"maven","projectPath":"app","projectDirectory":"/work/app","taskPath":"compile","sourceFiles":["/work/app/src/main/java/demo/App.java"],"classpath":["/repo/a.jar"],"annotationProcessorPath":[],"compilerArgs":[],"release":"17","encoding":"UTF-8","generatedSourcesDirectory":"/work/app/target/generated-sources/annotations","destinationDirectory":"/work/app/target/classes","javaCompilerExecutable":null,"javaLanguageVersion":"17"}"#,
        )
        .unwrap();
        let model =
            select_compile_model(&models, Path::new("/work/app/src/main/java/demo/App.java"))
                .unwrap();
        assert_eq!(model.java_release(), Some(17));
        assert_eq!(
            model.source_path(),
            [
                PathBuf::from("/work/app/src/main/java"),
                PathBuf::from("/work/app/target/generated-sources/annotations")
            ]
        );
    }

    #[test]
    fn selects_test_compile_unit_by_source_root_before_main_fallback() {
        let models = parse_models(
            r#"{"schemaVersion":2,"buildSystem":"gradle","projectPath":":app","projectDirectory":"/work/app","taskPath":":app:compileJava","sourceFiles":[],"sourceRoots":["/work/app/src/main/java"],"classpath":["/repo/main.jar"],"annotationProcessorPath":[],"compilerArgs":[],"release":25,"encoding":"UTF-8","generatedSourcesDirectory":null,"destinationDirectory":"/work/app/build/classes/java/main","javaCompilerExecutable":null,"javaLanguageVersion":25}
{"schemaVersion":2,"buildSystem":"gradle","projectPath":":app","projectDirectory":"/work/app","taskPath":":app:compileTestJava","sourceFiles":[],"sourceRoots":["/work/app/src/test/java"],"classpath":["/repo/test.jar"],"annotationProcessorPath":[],"compilerArgs":[],"release":25,"encoding":"UTF-8","generatedSourcesDirectory":null,"destinationDirectory":"/work/app/build/classes/java/test","javaCompilerExecutable":null,"javaLanguageVersion":25}"#,
        )
        .unwrap();

        let test = select_compile_model(
            &models,
            Path::new("/work/app/src/test/java/demo/AppTest.java"),
        )
        .unwrap();
        assert_eq!(test.task_path, ":app:compileTestJava");
        assert_eq!(test.classpath, [PathBuf::from("/repo/test.jar")]);

        let main =
            select_compile_model(&models, Path::new("/work/app/src/main/java/demo/App.java"))
                .unwrap();
        assert_eq!(main.task_path, ":app:compileJava");
    }

    #[test]
    fn preserves_missing_project_outputs_for_test_to_main_source_substitution() {
        let mut models = parse_models(
            r#"{"schemaVersion":2,"buildSystem":"gradle","projectPath":":","projectDirectory":"/work/petclinic","taskPath":":compileJava","sourceFiles":[],"sourceRoots":["/work/petclinic/src/main/java"],"classpath":[],"annotationProcessorPath":[],"compilerArgs":[],"release":25,"encoding":"UTF-8","generatedSourcesDirectory":null,"destinationDirectory":"/work/petclinic/build/classes/java/main","javaCompilerExecutable":null,"javaLanguageVersion":25}
{"schemaVersion":2,"buildSystem":"gradle","projectPath":":","projectDirectory":"/work/petclinic","taskPath":":compileTestJava","sourceFiles":[],"sourceRoots":["/work/petclinic/src/test/java"],"classpath":["/work/petclinic/build/classes/java/main","/missing/repository/dependency.jar"],"annotationProcessorPath":[],"compilerArgs":[],"release":25,"encoding":"UTF-8","generatedSourcesDirectory":null,"destinationDirectory":"/work/petclinic/build/classes/java/test","javaCompilerExecutable":null,"javaLanguageVersion":25}"#,
        )
        .expect("models");

        retain_usable_model_paths(&mut models);

        assert_eq!(
            models[1].classpath,
            [PathBuf::from("/work/petclinic/build/classes/java/main")]
        );
    }

    #[test]
    fn parses_v2_roots_module_path_dependencies_and_processor_options() {
        let models = parse_models(
            r#"{"schemaVersion":2,"buildSystem":"gradle","projectPath":":app","projectDirectory":"/work/app","taskPath":":app:compileJava","sourceFiles":[],"sourceRoots":["/work/app/src/main/java"],"classpath":[],"modulePath":["/repo/module.jar"],"projectDependencies":[":model"],"annotationProcessorPath":["/repo/processor.jar"],"annotationProcessorOptions":["-Ademo=strict"],"compilerArgs":[],"release":25,"encoding":"UTF-8","generatedSourcesDirectory":null,"generatedSourceDirectories":["/work/app/build/generated/sources/annotationProcessor/java/main"],"destinationDirectory":"/work/app/build/classes/java/main","javaCompilerExecutable":null,"javaLanguageVersion":25}"#,
        )
        .unwrap();
        let model = &models[0];
        assert_eq!(model.schema_version, 2);
        assert_eq!(model.module_path, [PathBuf::from("/repo/module.jar")]);
        assert_eq!(model.project_dependencies, [":model"]);
        assert_eq!(model.annotation_processor_options, ["-Ademo=strict"]);
        assert_eq!(
            model.source_path(),
            [
                PathBuf::from("/work/app/src/main/java"),
                PathBuf::from("/work/app/build/generated/sources/annotationProcessor/java/main")
            ]
        );
    }

    #[test]
    fn captures_gradle_output_and_extracts_only_model_frames() {
        let root = std::env::temp_dir().join(format!("jman-java-importer-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let script = root.join("gradle");
        let cache = root.join("model.ndjson");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf 'ordinary Gradle output\\n'\nprintf 'build warning' >&2\nprintf '%s\\n' 'JAVAC_FRONTEND_MODEL {\"schemaVersion\":2,\"buildSystem\":\"gradle\",\"projectPath\":\":app\",\"projectDirectory\":\"/work/app\",\"taskPath\":\":app:compileJava\",\"sourceFiles\":[],\"sourceRoots\":[],\"classpath\":[],\"annotationProcessorPath\":[],\"compilerArgs\":[],\"release\":25,\"encoding\":\"UTF-8\",\"generatedSourcesDirectory\":null,\"destinationDirectory\":null,\"javaCompilerExecutable\":null,\"javaLanguageVersion\":25}'\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        let tool = BuildTool::Gradle {
            root: root.clone(),
            executable: script,
        };
        assert!(import_project(&tool, &root, &cache).is_ok());
        assert_eq!(
            parse_models(&std::fs::read_to_string(&cache).unwrap())
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gradle_import_receives_the_selected_build_java_home() {
        let root =
            std::env::temp_dir().join(format!("jman-java-importer-runtime-{}", std::process::id()));
        let java_home = root.join("jdk-21");
        std::fs::create_dir_all(java_home.join("bin")).unwrap();
        std::fs::write(java_home.join("bin/java"), "runtime").unwrap();
        std::fs::write(java_home.join("release"), "JAVA_VERSION=\"21.0.2\"\n").unwrap();
        std::fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        std::fs::write(
            root.join("gradle/wrapper/gradle-wrapper.properties"),
            "distributionUrl=https\\://services.gradle.org/distributions/gradle-8.7-bin.zip\n",
        )
        .unwrap();
        std::fs::write(
            root.join("gradle.properties"),
            format!("org.gradle.java.home={}\n", java_home.display()),
        )
        .unwrap();
        let script = root.join("gradlew");
        let cache = root.join("model.ndjson");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf '%s' \"$JAVA_HOME\" > \"$PWD/selected-java-home\"\nprintf '%s\\n' 'JAVAC_FRONTEND_MODEL {\"schemaVersion\":2,\"buildSystem\":\"gradle\",\"projectPath\":\":app\",\"projectDirectory\":\"/work/app\",\"taskPath\":\":app:compileJava\",\"sourceFiles\":[],\"sourceRoots\":[],\"classpath\":[],\"annotationProcessorPath\":[],\"compilerArgs\":[],\"release\":21,\"encoding\":\"UTF-8\",\"generatedSourcesDirectory\":null,\"destinationDirectory\":null,\"javaCompilerExecutable\":null,\"javaLanguageVersion\":21}'\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        let tool = BuildTool::Gradle {
            root: root.clone(),
            executable: script,
        };
        let runtime = import_project(&tool, &root, &cache)
            .expect("Gradle import")
            .expect("selected runtime");

        assert_eq!(runtime.java_major, 21);
        assert_eq!(runtime.build_tool_version.as_deref(), Some("8.7"));
        assert_eq!(
            std::fs::read_to_string(root.join("selected-java-home")).unwrap(),
            java_home.to_string_lossy()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_import_preserves_and_uses_the_last_good_atomic_model() {
        let root =
            std::env::temp_dir().join(format!("jman-java-import-fallback-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let script = root.join("gradle");
        let cache = root.join("model.ndjson");
        let model = r#"{"schemaVersion":2,"buildSystem":"gradle","projectPath":":","projectDirectory":"/work","taskPath":":compileJava","sourceFiles":[],"sourceRoots":[],"classpath":[],"annotationProcessorPath":[],"compilerArgs":[],"release":25,"encoding":"UTF-8","generatedSourcesDirectory":null,"destinationDirectory":null,"javaCompilerExecutable":null,"javaLanguageVersion":25}"#;
        std::fs::write(&cache, model).unwrap();
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf 'offline repository' >&2\nexit 1\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        let tool = BuildTool::Gradle {
            root: root.clone(),
            executable: script,
        };
        assert!(import_project(&tool, &root, &cache).is_ok());
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), model);
        assert_eq!(parse_models(model).unwrap().len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
