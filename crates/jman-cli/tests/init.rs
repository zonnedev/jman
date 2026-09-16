use std::{fs, process::Command};

use sha2::{Digest, Sha256};

#[cfg(target_os = "linux")]
#[test]
fn executable_finds_native_frontend_outside_the_build_directory() {
    let directory = tempfile::tempdir().expect("working directory");
    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .current_dir(directory.path())
        .env_remove("LD_LIBRARY_PATH")
        .arg("--help")
        .output()
        .expect("run jman without a loader-path override");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_command_exposes_native_module_and_test_selectors() {
    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .args(["test", "--help"])
        .output()
        .expect("read test help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    assert!(help.contains("--module <MODULES>"));
    assert!(help.contains("--tests <PATTERN>"));
    assert!(help.contains("--source-set <SOURCE_SET>"));
    assert!(help.contains("--debug"));
}

#[test]
fn local_java_list_json_is_machine_readable_without_network() {
    let cache = tempfile::tempdir().expect("temporary cache");
    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .args(["java", "list", "--local", "--format", "json"])
        .output()
        .expect("list local JDKs as JSON");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["catalog"].is_null());
    assert_eq!(report["installed"], serde_json::json!([]));
    assert_eq!(report["available"], serde_json::json!([]));
    assert!(!cache.path().join("catalog").exists());
}

#[test]
fn lsp_command_runs_the_embedded_server_to_clean_eof() {
    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .arg("lsp")
        .output()
        .expect("run embedded language server");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn lsp_command_accepts_the_stdio_flag_appended_by_editor_clients() {
    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .args(["lsp", "--stdio"])
        .output()
        .expect("run embedded language server over stdio");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn initializes_native_app_library_and_workspace() {
    let root = tempfile::tempdir().expect("temporary root");
    let binary = env!("CARGO_BIN_EXE_jman");
    let app = root.path().join("hello-app");
    let output = Command::new(binary)
        .args([
            "--no-progress",
            "init",
            app.to_str().expect("UTF-8 path"),
            "--group",
            "dev.example",
            "--java",
            "17",
        ])
        .output()
        .expect("initialize app");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = fs::read_to_string(app.join("jman.toml")).expect("app manifest");
    assert!(manifest.contains("main-class = \"dev.example.hello_app.Application\""));
    assert!(app
        .join("src/main/java/dev/example/hello_app/Application.java")
        .is_file());
    assert!(app.join("jman.lock").is_file());
    assert!(app.join("src/main/resources").is_dir());

    let library = root.path().join("utilities");
    assert!(Command::new(binary)
        .args([
            "--quiet",
            "init",
            library.to_str().expect("UTF-8 path"),
            "--lib",
        ])
        .status()
        .expect("initialize library")
        .success());
    assert!(!fs::read_to_string(library.join("jman.toml"))
        .expect("library manifest")
        .contains("main-class"));

    let workspace = root.path().join("platform");
    assert!(Command::new(binary)
        .args([
            "--quiet",
            "init",
            workspace.to_str().expect("UTF-8 path"),
            "--modules",
            "core,service",
        ])
        .status()
        .expect("initialize workspace")
        .success());
    let root_manifest = fs::read_to_string(workspace.join("jman.toml")).expect("root manifest");
    assert!(root_manifest.contains("packaging = \"pom\""));
    assert!(root_manifest.contains("modules = ["));
    for module in ["core", "service"] {
        assert!(workspace.join(module).join("jman.toml").is_file());
        assert!(workspace
            .join(module)
            .join("src/main/java/com/example")
            .join(module)
            .is_dir());
    }

    let repeated = Command::new(binary)
        .args(["init", app.to_str().expect("UTF-8 path")])
        .output()
        .expect("repeat initialization");
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("non-empty directory"));
}

#[test]
fn runs_native_application_and_passes_arguments() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    fs::create_dir_all(project.path().join("src/main/java/com/example")).expect("sources");
    fs::write(
        project
            .path()
            .join("src/main/java/com/example/Application.java"),
        "package com.example; public final class Application { \
         public static void main(String[] args) { System.out.print(\"hello \" + args[0]); } }",
    )
    .expect("source");
    fs::write(
        project.path().join("jman.toml"),
        r#"manifest-version = 1
[project]
group = "com.example"
name = "app"
version = "1"
java-release = 17
packaging = "jar"
main-class = "com.example.Application"
"#,
    )
    .expect("manifest");
    fs::write(
        project.path().join("jman.lock"),
        r#"lock-version = 3
manifest-hash = "sha256:test"
workspace-hash = "sha256:test"
platform = "test"
[toolchain]
java-release = 17
[classpath]
"#,
    )
    .expect("lock");

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--no-progress",
            "run",
            project.path().to_str().expect("UTF-8 path"),
            "--",
            "world",
        ])
        .output()
        .expect("run application");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "hello world"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn compiles_tests_and_launches_junit_platform_console() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    let runner = tempfile::tempdir().expect("temporary runner");
    let runner_source = runner
        .path()
        .join("src/org/junit/platform/console/ConsoleLauncher.java");
    let processor_source = runner.path().join("src/fixture/TestGenerator.java");
    fs::create_dir_all(runner_source.parent().expect("runner source parent"))
        .expect("runner sources");
    fs::create_dir_all(processor_source.parent().expect("processor source parent"))
        .expect("processor sources");
    fs::write(
        &runner_source,
        r#"package org.junit.platform.console;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
public final class ConsoleLauncher {
  public static void main(String[] args) throws Exception {
    if (!args[0].equals("execute")) System.exit(2);
    if (!"0".equals(System.getProperty("jman.test.port"))) System.exit(4);
    if (!"false".equals(System.getenv("TESTCONTAINERS_RYUK_DISABLED"))) System.exit(5);
    var arguments = List.of(args);
    if (arguments.stream().anyMatch(value -> value.contains("MissingTest"))) {
      System.exit(arguments.contains("--fail-if-no-tests") ? 1 : 3);
    }
    var report = arguments.stream()
        .filter(value -> value.startsWith("--reports-dir="))
        .findFirst().orElseThrow().substring("--reports-dir=".length());
    Files.writeString(Path.of(report).resolve("TEST-fixture.xml"),
        "<testsuite><testcase name=\"greets(String)[1]\" classname=\"com.example.AppTest\" time=\"0.012\">"
        + "<failure message=\"expected Ada\">exact stack trace</failure></testcase></testsuite>");
    System.out.print("JUnit Platform fixture passed");
  }
}"#,
    )
    .expect("runner source");
    fs::write(
        &processor_source,
        r#"package fixture;
import java.io.Writer;
import java.util.Set;
import javax.annotation.processing.*;
import javax.lang.model.SourceVersion;
import javax.lang.model.element.TypeElement;
@SupportedAnnotationTypes("*")
@SupportedSourceVersion(SourceVersion.RELEASE_17)
public final class TestGenerator extends AbstractProcessor {
  private boolean generated;
  public boolean process(Set<? extends TypeElement> annotations, RoundEnvironment round) {
    boolean testRound = round.getRootElements().stream()
        .anyMatch(element -> element.getSimpleName().toString().endsWith("Test"));
    if (!generated && testRound) {
      generated = true;
      try {
        var file = processingEnv.getFiler().createSourceFile("com.example.GeneratedTestSupport");
        try (Writer writer = file.openWriter()) {
          writer.write("package com.example; public final class GeneratedTestSupport {}");
        }
      } catch (Exception error) {
        throw new RuntimeException(error);
      }
    }
    return false;
  }
}"#,
    )
    .expect("processor source");
    let runner_classes = runner.path().join("classes");
    fs::create_dir_all(&runner_classes).expect("runner classes");
    assert!(Command::new("javac")
        .args(["--release", "17", "-d"])
        .arg(&runner_classes)
        .arg(&runner_source)
        .arg(&processor_source)
        .status()
        .expect("compile runner")
        .success());
    let services = runner_classes.join("META-INF/services");
    fs::create_dir_all(&services).expect("processor services");
    fs::write(
        services.join("javax.annotation.processing.Processor"),
        "fixture.TestGenerator\n",
    )
    .expect("processor registration");
    let runner_jar = runner.path().join("junit-platform-console-standalone.jar");
    assert!(Command::new("jar")
        .args(["--create", "--file"])
        .arg(&runner_jar)
        .arg("-C")
        .arg(&runner_classes)
        .arg(".")
        .status()
        .expect("package runner")
        .success());
    let runner_bytes = fs::read(&runner_jar).expect("runner JAR");
    let digest = hex::encode(Sha256::digest(&runner_bytes));
    fs::create_dir_all(cache.path().join("artifacts")).expect("artifact cache");
    fs::write(
        cache.path().join(format!("artifacts/{digest}.jar")),
        &runner_bytes,
    )
    .expect("cache runner");

    fs::create_dir_all(project.path().join("src/main/java/com/example")).expect("main sources");
    fs::create_dir_all(project.path().join("src/test/java/com/example")).expect("test sources");
    fs::create_dir_all(project.path().join("src/integrationTest/java/com/example"))
        .expect("integration test sources");
    fs::write(
        project.path().join("src/main/java/com/example/App.java"),
        "package com.example; public final class App {}",
    )
    .expect("main source");
    fs::write(
        project
            .path()
            .join("src/test/java/com/example/AppTest.java"),
        "package com.example; public final class AppTest { \
         App value = new App(); GeneratedTestSupport generated = new GeneratedTestSupport(); }",
    )
    .expect("test source");
    fs::write(
        project
            .path()
            .join("src/integrationTest/java/com/example/AppIntegrationTest.java"),
        "package com.example; public final class AppIntegrationTest { App value = new App(); }",
    )
    .expect("integration test source");
    fs::write(
        project.path().join("jman.toml"),
        r#"manifest-version = 1
[project]
group = "com.example"
name = "app"
version = "1"
java-release = 17
packaging = "jar"
[annotation-processors]
"fixture:test-generator" = "1"
"#,
    )
    .expect("manifest");
    fs::write(
        project.path().join("jman.lock"),
        format!(
            r#"lock-version = 3
manifest-hash = "sha256:test"
workspace-hash = "sha256:test"
platform = "test"
[toolchain]
java-release = 17
[[package]]
group = "org.junit.platform"
artifact = "junit-platform-console-standalone"
version = "1"
extension = "jar"
source = "fixture"
pom_checksum = "sha256:pom"
artifact_checksum = "sha256:{digest}"
artifact_size = {}
scopes = ["test"]
[[package]]
group = "fixture"
artifact = "test-generator"
version = "1"
extension = "jar"
source = "fixture"
pom_checksum = "sha256:pom"
artifact_checksum = "sha256:{digest}"
artifact_size = {}
scopes = ["processor"]
[classpath]
test = ["sha256:{digest}"]
processors = ["sha256:{digest}"]
"#,
            runner_bytes.len(),
            runner_bytes.len()
        ),
    )
    .expect("lock");

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .env("TESTCONTAINERS_RYUK_DISABLED", "false")
        .args([
            "--no-progress",
            "test",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("test project");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "JUnit Platform fixture passed"
    );
    assert!(project
        .path()
        .join(".jman/output/test-classes/com/example/AppTest.class")
        .is_file());
    assert!(project
        .path()
        .join(".jman/output/test-classes/com/example/AppIntegrationTest.class")
        .is_file());
    assert!(project
        .path()
        .join(
            ".jman/output/generated/test-sources/annotations/com/example/GeneratedTestSupport.java"
        )
        .is_file());

    let structured = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .env("TESTCONTAINERS_RYUK_DISABLED", "false")
        .args([
            "--no-progress",
            "test",
            project.path().to_str().expect("UTF-8 path"),
            "--report",
            "json",
            "--source-set",
            "unit",
            "--tests",
            "com.example.AppTest#greets",
        ])
        .output()
        .expect("structured test project");
    assert!(structured.status.success());
    let events = String::from_utf8(structured.stdout)
        .expect("UTF-8 events")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("test event"))
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["protocolVersion"], 2);
    assert_eq!(events[0]["reason"], "test-module-started");
    assert_eq!(events[1]["reason"], "test-case");
    assert_eq!(events[1]["test"]["selector"], "com.example.AppTest#greets");
    assert_eq!(events[1]["test"]["displayName"], "greets(String)[1]");
    assert_eq!(events[1]["test"]["message"], "expected Ada");
    assert_eq!(events[1]["test"]["details"], "exact stack trace");
    assert_eq!(events[2]["reason"], "test-module-finished");
    assert!(!project
        .path()
        .join(".jman/output/test-classes/com/example/AppIntegrationTest.class")
        .exists());

    let missing = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .env("TESTCONTAINERS_RYUK_DISABLED", "false")
        .args([
            "--no-progress",
            "test",
            project.path().to_str().expect("UTF-8 path"),
            "--tests",
            "com.example.MissingTest",
        ])
        .output()
        .expect("run missing test selector");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("tests failed in app"));
}

#[test]
fn detects_maven_project_and_explains_non_interactive_import() {
    let project = tempfile::tempdir().expect("temporary project");
    fs::write(
        project.path().join("pom.xml"),
        "<project><modelVersion>4.0.0</modelVersion></project>",
    )
    .expect("POM");

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .args(["init", project.path().to_str().expect("UTF-8 path")])
        .output()
        .expect("run jman");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("Maven project detected"));
    assert!(stderr.contains("jman init --import"));
    assert!(!project.path().join("jman.toml").exists());
    assert!(!project.path().join("jman.lock").exists());
}

#[test]
fn import_requires_a_pom() {
    let project = tempfile::tempdir().expect("temporary project");

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .args([
            "init",
            "--import",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("run jman");

    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .expect("UTF-8 stderr")
        .contains("no pom.xml was found"));
}

#[test]
fn import_warns_about_plugins_without_translating_them() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    fs::write(
        project.path().join("pom.xml"),
        r"<project><modelVersion>4.0.0</modelVersion>
          <groupId>com.example</groupId><artifactId>demo</artifactId><version>1</version>
          <properties><exec.mainClass>com.example.Main</exec.mainClass></properties>
          <build><plugins><plugin><artifactId>maven-compiler-plugin</artifactId>
            <configuration><compilerArgs><arg>-Aignored=true</arg></compilerArgs></configuration>
          </plugin></plugins></build>
        </project>",
    )
    .expect("POM");

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--no-progress",
            "init",
            "--import",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("import");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("Maven plugins were detected but were not imported"));
    assert!(stderr.contains("org.apache.maven.plugins:maven-compiler-plugin"));
    let manifest = fs::read_to_string(project.path().join("jman.toml")).expect("manifest");
    assert!(!manifest.contains("main-class"));
    assert!(!manifest.contains("compiler-args"));
    assert!(!manifest.contains("annotation-processors"));
}

#[test]
fn sync_requires_an_imported_project() {
    let project = tempfile::tempdir().expect("temporary project");

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .args(["sync", project.path().to_str().expect("UTF-8 path")])
        .output()
        .expect("run jman");

    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .expect("UTF-8 stderr")
        .contains("jman.toml was not found"));
}

#[test]
fn no_progress_uses_stable_plain_status_lines() {
    let project = tempfile::tempdir().expect("temporary project");

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .args([
            "--no-progress",
            "init",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("run jman");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("→ Inspecting"));
    assert!(!stderr.contains('⠋'));
}

#[test]
fn quiet_suppresses_status_but_not_errors() {
    let project = tempfile::tempdir().expect("temporary project");

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .args([
            "--quiet",
            "init",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("run jman");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(!stderr.contains("→ Inspecting"));
    assert!(stderr.contains("error:"));
}

#[test]
fn json_sync_keeps_stderr_clean() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    fs::write(
        project.path().join("pom.xml"),
        "<project><modelVersion>4.0.0</modelVersion><groupId>com.example</groupId>\
         <artifactId>demo</artifactId><version>1</version></project>",
    )
    .expect("POM");
    let import = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "init",
            "--import",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("import project");
    assert!(
        import.status.success(),
        "{}",
        String::from_utf8_lossy(&import.stderr)
    );
    fs::remove_file(project.path().join("pom.xml")).expect("remove imported POM");

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "sync",
            project.path().to_str().expect("UTF-8 path"),
            "--report",
            "json",
            "--refresh",
            "--offline",
        ])
        .output()
        .expect("synchronize project");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON report");
    assert_eq!(report["root"]["artifact"], "demo");
}

#[test]
#[allow(clippy::too_many_lines)]
fn native_dependency_commands_work_without_maven_poms() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    let repository = tempfile::tempdir().expect("temporary repository");
    let artifact_directory = repository.path().join("org/example/library/1");
    fs::create_dir_all(&artifact_directory).expect("artifact directory");
    fs::write(
        artifact_directory.join("library-1.pom"),
        "<project><modelVersion>4.0.0</modelVersion><groupId>org.example</groupId>\
         <artifactId>library</artifactId><version>1</version></project>",
    )
    .expect("dependency POM");
    fs::write(artifact_directory.join("library-1.jar"), b"library").expect("dependency JAR");
    fs::write(
        project.path().join("pom.xml"),
        format!(
            "<project><modelVersion>4.0.0</modelVersion><groupId>com.example</groupId>\
             <artifactId>demo</artifactId><version>1</version>\
             <repositories><repository><id>fixture</id><url>file://{}</url></repository></repositories>\
             <dependencies><dependency><groupId>org.example</groupId>\
             <artifactId>library</artifactId><version>1</version></dependency></dependencies></project>",
            repository.path().display()
        ),
    )
    .expect("project POM");
    let binary = env!("CARGO_BIN_EXE_jman");
    let import = Command::new(binary)
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--quiet",
            "init",
            "--import",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("import");
    assert!(
        import.status.success(),
        "{}",
        String::from_utf8_lossy(&import.stderr)
    );
    fs::remove_file(project.path().join("pom.xml")).expect("remove POM");

    let tree = Command::new(binary)
        .args(["tree", project.path().to_str().expect("UTF-8 path")])
        .output()
        .expect("tree");
    assert!(tree.status.success());
    assert!(String::from_utf8_lossy(&tree.stdout).contains("org.example:library:1"));

    let why = Command::new(binary)
        .args([
            "why",
            "org.example:library",
            "--path",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("why");
    assert!(why.status.success());
    assert!(String::from_utf8_lossy(&why.stdout)
        .contains("com.example:demo:1 -> org.example:library:1"));

    let remove = Command::new(binary)
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--quiet",
            "remove",
            "org.example:library",
            "--offline",
            "--path",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("remove");
    assert!(
        remove.status.success(),
        "{}",
        String::from_utf8_lossy(&remove.stderr)
    );

    let add = Command::new(binary)
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--quiet",
            "add",
            "org.example:library@1",
            "--offline",
            "--path",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("add");
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let cached_artifact = fs::read_dir(cache.path().join("artifacts"))
        .expect("artifact cache")
        .next()
        .expect("cached artifact")
        .expect("artifact entry")
        .path();
    fs::write(&cached_artifact, b"corrupt").expect("same-size cache corruption");
    let repair = Command::new(binary)
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--quiet",
            "sync",
            "--offline",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("repair sync");
    assert!(
        repair.status.success(),
        "{}",
        String::from_utf8_lossy(&repair.stderr)
    );
    assert_eq!(
        fs::read(&cached_artifact).expect("repaired artifact"),
        b"library"
    );
    assert!(Command::new(binary)
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--quiet",
            "remove",
            "org.example:library",
            "--offline",
            "--path",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .status()
        .expect("remove compile dependency")
        .success());
    assert!(Command::new(binary)
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--quiet",
            "add",
            "org.example:library@1",
            "--scope",
            "processor",
            "--offline",
            "--path",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .status()
        .expect("add processor dependency")
        .success());
    let processor_manifest =
        fs::read_to_string(project.path().join("jman.toml")).expect("processor manifest");
    assert!(processor_manifest.contains("[annotation-processors]"));
    assert!(processor_manifest.contains("\"org.example:library\" = \"1\""));

    let manifest_before_failure =
        fs::read(project.path().join("jman.toml")).expect("manifest before failed add");
    let failed_add = Command::new(binary)
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--quiet",
            "add",
            "org.example:missing@1",
            "--offline",
            "--path",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("failed add");
    assert!(!failed_add.status.success());
    assert!(
        String::from_utf8_lossy(&failed_add.stderr).contains("jman.toml was restored"),
        "{}",
        String::from_utf8_lossy(&failed_add.stderr)
    );
    assert_eq!(
        fs::read(project.path().join("jman.toml")).expect("restored manifest"),
        manifest_before_failure
    );
}

#[test]
fn check_compiles_caches_and_preserves_last_good_output() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    fs::create_dir_all(project.path().join("src/main/java/com/example")).expect("source tree");
    fs::write(
        project.path().join("jman.toml"),
        r#"manifest-version = 1

[project]
group = "com.example"
name = "demo"
version = "1"
java-release = 17
packaging = "jar"
"#,
    )
    .expect("manifest");
    fs::write(
        project.path().join("jman.lock"),
        r#"lock-version = 3
manifest-hash = "sha256:manifest"
workspace-hash = "sha256:workspace"
platform = "test"

[toolchain]
java-release = 17

[classpath]
"#,
    )
    .expect("lock");
    let source = project
        .path()
        .join("src/main/java/com/example/Greeting.java");
    fs::write(
        &source,
        "package com.example; public final class Greeting { public static String text() { return \"hello\"; } }",
    )
    .expect("Java source");
    let binary = env!("CARGO_BIN_EXE_jman");
    let run = |project: &std::path::Path| {
        Command::new(binary)
            .env("JMAN_CACHE_DIR", cache.path())
            .args([
                "--no-progress",
                "check",
                project.to_str().expect("UTF-8 path"),
            ])
            .output()
            .expect("run check")
    };

    let first = run(project.path());
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let class = project
        .path()
        .join(".jman/output/classes/com/example/Greeting.class");
    assert!(class.is_file());

    let second = run(project.path());
    assert!(second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("unchanged"));

    fs::create_dir_all(project.path().join("src/main/resources")).expect("resource tree");
    fs::write(
        project
            .path()
            .join("src/main/resources/application.properties"),
        "message=hello",
    )
    .expect("resource");
    let resource_change = run(project.path());
    assert!(resource_change.status.success());
    assert!(String::from_utf8_lossy(&resource_change.stderr).contains("1 compiled"));

    fs::write(&source, "this is not Java").expect("invalid Java source");
    let failed = run(project.path());
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("Java compilation failed"));
    assert!(
        class.is_file(),
        "last successful classes must remain intact"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn check_rebuilds_downstream_modules_after_upstream_change() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    let lock = r#"lock-version = 3
manifest-hash = "sha256:manifest"
workspace-hash = "sha256:workspace"
platform = "test"

[toolchain]
java-release = 17

[classpath]
"#;
    let root_manifest = r#"manifest-version = 1
[project]
group = "com.example"
name = "workspace"
version = "1"
java-release = 17
packaging = "pom"
modules = ["core", "app", "empty"]
"#;
    fs::write(project.path().join("jman.toml"), root_manifest).expect("root manifest");
    fs::write(project.path().join("jman.lock"), lock).expect("root lock");
    for module in ["core", "app", "empty"] {
        fs::create_dir_all(
            project
                .path()
                .join(module)
                .join("src/main/java/com/example"),
        )
        .expect("source tree");
        fs::write(project.path().join(module).join("jman.lock"), lock).expect("module lock");
    }
    fs::write(
        project.path().join("core/jman.toml"),
        r#"manifest-version = 1
[project]
group = "com.example"
name = "core"
version = "1"
java-release = 17
packaging = "jar"
"#,
    )
    .expect("core manifest");
    fs::write(
        project.path().join("app/jman.toml"),
        r#"manifest-version = 1
[project]
group = "com.example"
name = "app"
version = "1"
java-release = 17
packaging = "jar"

[path-dependencies]
"com.example:core" = "../core"
"#,
    )
    .expect("app manifest");
    fs::write(
        project.path().join("empty/jman.toml"),
        r#"manifest-version = 1
[project]
group = "com.example"
name = "empty"
version = "1"
java-release = 17
packaging = "jar"
"#,
    )
    .expect("empty manifest");
    let core_source = project
        .path()
        .join("core/src/main/java/com/example/Core.java");
    fs::write(
        &core_source,
        "package com.example; public final class Core { public static int value() { return 1; } }",
    )
    .expect("core source");
    fs::write(
        project
            .path()
            .join("app/src/main/java/com/example/App.java"),
        "package com.example; public final class App { int value() { return Core.value(); } }",
    )
    .expect("app source");
    let check = || {
        Command::new(env!("CARGO_BIN_EXE_jman"))
            .env("JMAN_CACHE_DIR", cache.path())
            .args([
                "--no-progress",
                "-v",
                "check",
                project.path().to_str().expect("UTF-8 path"),
            ])
            .output()
            .expect("check workspace")
    };

    let first = check();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(project
        .path()
        .join("app/.jman/output/classes/com/example/App.class")
        .is_file());

    fs::write(
        &core_source,
        "package com.example; public final class Core { public static int value() { return 2; } }",
    )
    .expect("change core");
    let second = check();
    assert!(second.status.success());
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(stderr.contains("core: 1 source files (compiled)"));
    assert!(stderr.contains("app: 1 source files (compiled)"));
    assert!(stderr.contains("workspace: 0 source files (cached)"));

    let package = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--no-progress",
            "build",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("build workspace");
    assert!(
        package.status.success(),
        "{}",
        String::from_utf8_lossy(&package.stderr)
    );
    assert!(project
        .path()
        .join("core/.jman/artifacts/core-1.jar")
        .is_file());
    assert!(project
        .path()
        .join("app/.jman/artifacts/app-1.jar")
        .is_file());
    assert!(project
        .path()
        .join("empty/.jman/artifacts/empty-1.jar")
        .is_file());
    assert!(!project
        .path()
        .join(".jman/artifacts/workspace-1.jar")
        .exists());
}

#[test]
fn java_use_pins_toolchain_without_installing_it() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    fs::write(
        project.path().join("jman.toml"),
        r#"manifest-version = 1
[project]
group = "com.example"
name = "demo"
version = "1"
java-release = 17
packaging = "jar"
"#,
    )
    .expect("manifest");
    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--quiet",
            "java",
            "use",
            "17",
            "--path",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("pin Java");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = fs::read_to_string(project.path().join("jman.toml")).expect("manifest");
    assert!(manifest.contains("[toolchain]"));
    assert!(manifest.contains("jdk = \"17\""));

    let which = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "java",
            "which",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("find Java");
    assert!(!which.status.success());
    assert!(String::from_utf8_lossy(&which.stderr).contains("jman java install 17"));
}

#[test]
#[allow(clippy::too_many_lines)]
fn build_runs_isolated_annotation_processor_and_materializes_resources() {
    let project = tempfile::tempdir().expect("project");
    let cache = tempfile::tempdir().expect("cache");
    let processor = tempfile::tempdir().expect("processor");
    let processor_source = processor.path().join("src/fixture/Generator.java");
    fs::create_dir_all(processor_source.parent().expect("source parent")).expect("source tree");
    fs::write(
        &processor_source,
        r#"package fixture;
import java.io.Writer;
import java.util.Set;
import javax.annotation.processing.*;
import javax.lang.model.SourceVersion;
import javax.lang.model.element.TypeElement;
@SupportedAnnotationTypes("*")
@SupportedSourceVersion(SourceVersion.RELEASE_17)
public final class Generator extends AbstractProcessor {
  private boolean generated;
  public boolean process(Set<? extends TypeElement> annotations, RoundEnvironment round) {
    if (!generated && !round.processingOver()) {
      generated = true;
      try {
        var file = processingEnv.getFiler().createSourceFile("com.example.GeneratedGreeting");
        try (Writer writer = file.openWriter()) {
          writer.write("package com.example; public final class GeneratedGreeting { " +
                       "public static String text() { return \"generated\"; } }");
        }
      } catch (Exception error) {
        throw new RuntimeException(error);
      }
    }
    return false;
  }
}"#,
    )
    .expect("processor source");
    let processor_classes = processor.path().join("classes");
    fs::create_dir_all(&processor_classes).expect("processor classes");
    let compilation = Command::new("javac")
        .args(["--release", "17", "-d"])
        .arg(&processor_classes)
        .arg(&processor_source)
        .output()
        .expect("compile processor");
    assert!(
        compilation.status.success(),
        "{}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    let services = processor_classes.join("META-INF/services");
    fs::create_dir_all(&services).expect("services");
    fs::write(
        services.join("javax.annotation.processing.Processor"),
        "fixture.Generator\n",
    )
    .expect("processor service");
    let processor_jar = processor.path().join("generator.jar");
    let jar = Command::new("jar")
        .args(["--create", "--file"])
        .arg(&processor_jar)
        .arg("-C")
        .arg(&processor_classes)
        .arg(".")
        .output()
        .expect("package processor");
    assert!(
        jar.status.success(),
        "{}",
        String::from_utf8_lossy(&jar.stderr)
    );
    let jar_bytes = fs::read(&processor_jar).expect("processor JAR");
    let digest = hex::encode(Sha256::digest(&jar_bytes));
    let artifacts = cache.path().join("artifacts");
    fs::create_dir_all(&artifacts).expect("artifact cache");
    fs::write(artifacts.join(format!("{digest}.jar")), &jar_bytes).expect("cached processor");

    fs::create_dir_all(project.path().join("src/main/java/com/example")).expect("sources");
    fs::create_dir_all(project.path().join("src/main/resources")).expect("resources");
    fs::write(
        project.path().join("src/main/java/com/example/App.java"),
        "package com.example; public final class App { String text() { return GeneratedGreeting.text(); } }",
    )
    .expect("application source");
    fs::write(
        project
            .path()
            .join("src/main/resources/application.properties"),
        "message=hello\n",
    )
    .expect("application resource");
    fs::write(
        project.path().join("jman.toml"),
        r#"manifest-version = 1
[project]
group = "com.example"
name = "generated-app"
version = "1"
java-release = 17
packaging = "jar"
main-class = "com.example.App"
[annotation-processors]
"fixture:generator" = "1"
"#,
    )
    .expect("manifest");
    fs::write(
        project.path().join("jman.lock"),
        format!(
            r#"lock-version = 3
manifest-hash = "sha256:manifest"
workspace-hash = "sha256:workspace"
platform = "test"
[toolchain]
java-release = 17
[[package]]
group = "fixture"
artifact = "generator"
version = "1"
extension = "jar"
source = "fixture"
pom_checksum = "sha256:pom"
artifact_checksum = "sha256:{digest}"
artifact_size = {}
scopes = ["processor"]
[classpath]
processors = ["sha256:{digest}"]
"#,
            jar_bytes.len()
        ),
    )
    .expect("lock");
    let compile = |command| {
        Command::new(env!("CARGO_BIN_EXE_jman"))
            .env("JMAN_CACHE_DIR", cache.path())
            .args([
                "--no-progress",
                command,
                project.path().to_str().expect("UTF-8 path"),
            ])
            .output()
            .expect("build project")
    };

    let first = compile("check");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let output = project.path().join(".jman/output");
    assert!(output
        .join("generated/sources/annotations/com/example/GeneratedGreeting.java")
        .is_file());
    assert!(output
        .join("classes/com/example/GeneratedGreeting.class")
        .is_file());
    assert_eq!(
        fs::read_to_string(output.join("classes/application.properties")).expect("resource"),
        "message=hello\n"
    );
    let second = compile("build");
    assert!(second.status.success());

    let first_package = compile("build");
    assert!(
        first_package.status.success(),
        "{}",
        String::from_utf8_lossy(&first_package.stderr)
    );
    let artifact = project.path().join(".jman/artifacts/generated-app-1.jar");
    assert!(artifact.is_file());
    assert!(String::from_utf8_lossy(&first_package.stderr)
        .contains(artifact.to_str().expect("UTF-8 artifact path")));
    let first_checksum = Sha256::digest(fs::read(&artifact).expect("first JAR"));
    let second_package = compile("build");
    assert!(second_package.status.success());
    assert!(String::from_utf8_lossy(&second_package.stderr).contains("unchanged"));
    let second_checksum = Sha256::digest(fs::read(&artifact).expect("second JAR"));
    assert_eq!(first_checksum, second_checksum);

    let all = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--no-progress",
            "build",
            "--all",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("build all artifacts");
    assert!(
        all.status.success(),
        "{}",
        String::from_utf8_lossy(&all.stderr)
    );
    for suffix in ["sources", "javadoc", "fat"] {
        let path = project
            .path()
            .join(format!(".jman/artifacts/generated-app-1-{suffix}.jar"));
        assert!(path.is_file(), "missing {}", path.display());
        assert!(String::from_utf8_lossy(&all.stderr)
            .contains(path.to_str().expect("UTF-8 artifact path")));
    }
    let sources_inventory = Command::new("jar")
        .args(["--list", "--file"])
        .arg(
            project
                .path()
                .join(".jman/artifacts/generated-app-1-sources.jar"),
        )
        .output()
        .expect("list sources JAR");
    let sources_inventory = String::from_utf8(sources_inventory.stdout).expect("UTF-8 inventory");
    assert!(sources_inventory.contains("com/example/App.java"));
    assert!(sources_inventory.contains("com/example/GeneratedGreeting.java"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let external = project.path().join("external.txt");
        fs::write(&external, "external").expect("external resource");
        symlink(
            &external,
            project.path().join("src/main/resources/unsafe-link"),
        )
        .expect("unsafe resource link");
        let failed_package = compile("build");
        assert!(!failed_package.status.success());
        assert_eq!(
            Sha256::digest(fs::read(&artifact).expect("preserved JAR")),
            second_checksum
        );
    }

    let inventory = Command::new("jar")
        .args(["--list", "--file"])
        .arg(&artifact)
        .output()
        .expect("list JAR");
    assert!(inventory.status.success());
    let inventory = String::from_utf8(inventory.stdout).expect("UTF-8 inventory");
    assert!(inventory.contains("com/example/GeneratedGreeting.class"));
    assert!(inventory.contains("application.properties"));
    let extracted = tempfile::tempdir().expect("extracted JAR");
    let extraction = Command::new("jar")
        .current_dir(extracted.path())
        .args(["--extract", "--file"])
        .arg(&artifact)
        .output()
        .expect("extract JAR");
    assert!(extraction.status.success());
    let jar_manifest =
        fs::read_to_string(extracted.path().join("META-INF/MANIFEST.MF")).expect("JAR manifest");
    assert!(jar_manifest.contains("Manifest-Version: 1.0\r\n"));
    assert!(jar_manifest.contains("Main-Class: com.example.App\r\n"));
}
