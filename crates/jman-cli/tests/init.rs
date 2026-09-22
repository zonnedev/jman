use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

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
    for option in [
        "--coverage",
        "--coverage-format <COVERAGE_FORMATS>",
        "--coverage-output <COVERAGE_OUTPUT>",
        "--coverage-min-line <COVERAGE_MIN_LINE>",
        "--coverage-min-branch <COVERAGE_MIN_BRANCH>",
        "--coverage-include <COVERAGE_INCLUDE>",
        "--coverage-exclude <COVERAGE_EXCLUDE>",
    ] {
        assert!(help.contains(option), "missing {option} in:\n{help}");
    }
}

#[test]
fn publish_command_exposes_safe_repository_controls() {
    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .args(["publish", "--help"])
        .output()
        .expect("read publish help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    for option in [
        "--to <TO>",
        "--repository-url <REPOSITORY_URL>",
        "--local-repository <LOCAL_REPOSITORY>",
        "--dry-run",
        "--sign",
        "--allow-dirty",
        "--automatic",
        "--format <FORMAT>",
    ] {
        assert!(help.contains(option), "missing {option} in:\n{help}");
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn outdated_aggregates_workspace_dependencies_and_uses_cached_maven_metadata() {
    let project = tempfile::tempdir().expect("temporary workspace");
    let cache = tempfile::tempdir().expect("temporary cache");
    let repository = tempfile::tempdir().expect("temporary repository");
    for module in ["alpha", "beta"] {
        fs::create_dir_all(project.path().join(module)).expect("module directory");
    }
    fs::write(
        project.path().join("jman.toml"),
        r#"manifest-version = 1
[project]
group = "com.example"
name = "workspace"
version = "1.0.0"
java-release = 17
packaging = "pom"
modules = ["alpha", "beta"]
"#,
    )
    .expect("root manifest");
    let repository_url = format!("file://{}", repository.path().display());
    fs::write(
        project.path().join("alpha/jman.toml"),
        format!(
            r#"manifest-version = 1
[project]
group = "com.example"
name = "alpha"
version = "1.0.0"
java-release = 17
packaging = "jar"

[[repositories]]
id = "fixture"
url = "{repository_url}"

[dependencies.compile]
"org.example:library" = "1.2.3"
"#
        ),
    )
    .expect("alpha manifest");
    fs::write(
        project.path().join("beta/jman.toml"),
        format!(
            r#"manifest-version = 1
[project]
group = "com.example"
name = "beta"
version = "1.0.0"
java-release = 17
packaging = "jar"

[[repositories]]
id = "fixture"
url = "{repository_url}"

[dependencies.compile]
"com.example:alpha" = "1.0.0"

[dependencies.runtime]
"org.example:library" = "1.2.3"

[path-dependencies]
"com.example:alpha" = "../alpha"
"#
        ),
    )
    .expect("beta manifest");
    let metadata = repository
        .path()
        .join("org/example/library/maven-metadata.xml");
    fs::create_dir_all(metadata.parent().expect("metadata parent")).expect("metadata directory");
    fs::write(
        &metadata,
        r"<metadata>
  <groupId>org.example</groupId>
  <artifactId>library</artifactId>
  <versioning>
    <latest>2.0.0-rc1</latest>
    <release>1.3.0</release>
    <versions>
      <version>1.2.3</version>
      <version>1.2.4</version>
      <version>1.3.0</version>
      <version>2.0.0-rc1</version>
    </versions>
  </versioning>
</metadata>
",
    )
    .expect("Maven metadata");

    let run = |additional: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_jman"));
        command
            .env("JMAN_CACHE_DIR", cache.path())
            .args(["outdated", project.path().to_str().expect("UTF-8 path")])
            .args(additional)
            .args(["--format", "json"])
            .output()
            .expect("inspect dependencies")
    };
    let stable = run(&[]);
    assert!(
        stable.status.success(),
        "{}",
        String::from_utf8_lossy(&stable.stderr)
    );
    assert!(stable.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&stable.stdout).expect("stable report");
    assert_eq!(report["checked"], 1);
    assert_eq!(report["outdated"], 1);
    assert_eq!(
        report["dependencies"][0]["dependency"],
        "org.example:library"
    );
    assert_eq!(report["dependencies"][0]["current"], "1.2.3");
    assert_eq!(report["dependencies"][0]["patch"], "1.2.4");
    assert_eq!(report["dependencies"][0]["minor"], "1.3.0");
    assert!(report["dependencies"][0]["major"].is_null());
    assert_eq!(report["dependencies"][0]["latest"], "1.3.0");
    assert_eq!(
        report["dependencies"][0]["scopes"],
        serde_json::json!(["compile", "runtime"])
    );
    assert_eq!(
        report["dependencies"][0]["modules"],
        serde_json::json!(["alpha", "beta"])
    );

    let prerelease = run(&["--include-prerelease"]);
    let report: serde_json::Value =
        serde_json::from_slice(&prerelease.stdout).expect("prerelease report");
    assert_eq!(report["dependencies"][0]["major"], "2.0.0-rc1");
    assert_eq!(report["dependencies"][0]["latest"], "2.0.0-rc1");
    assert_eq!(report["dependencies"][0]["change"], "major");

    fs::remove_file(metadata).expect("remove repository metadata");
    let offline = run(&["--offline"]);
    assert!(offline.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&offline.stdout).expect("offline report");
    assert_eq!(report["dependencies"][0]["source"], "cache");
    assert_eq!(report["dependencies"][0]["latest"], "1.3.0");
}

#[test]
#[allow(clippy::too_many_lines)]
fn update_is_patch_safe_dry_runnable_and_transactional_across_a_workspace() {
    let project = tempfile::tempdir().expect("temporary workspace");
    let cache = tempfile::tempdir().expect("temporary cache");
    let repository = tempfile::tempdir().expect("temporary repository");
    for module in ["alpha", "beta"] {
        fs::create_dir_all(project.path().join(module)).expect("module directory");
    }
    fs::write(
        project.path().join("jman.toml"),
        r#"manifest-version = 1
[project]
group = "com.example"
name = "workspace"
version = "1.0.0"
java-release = 17
packaging = "pom"
modules = ["alpha", "beta"]
"#,
    )
    .expect("root manifest");
    let repository_url = format!("file://{}", repository.path().display());
    fs::write(
        project.path().join("alpha/jman.toml"),
        format!(
            r#"manifest-version = 1
[project]
group = "com.example"
name = "alpha"
version = "1.0.0"
java-release = 17
packaging = "jar"

[[repositories]]
id = "fixture"
url = "{repository_url}"

[dependencies.compile]
"org.example:library" = "1.2.3"
"#
        ),
    )
    .expect("alpha manifest");
    fs::write(
        project.path().join("beta/jman.toml"),
        format!(
            r#"manifest-version = 1
[project]
group = "com.example"
name = "beta"
version = "1.0.0"
java-release = 17
packaging = "jar"

[[repositories]]
id = "fixture"
url = "{repository_url}"

[dependencies.runtime]
"org.example:library" = "1.2.3"
"#
        ),
    )
    .expect("beta manifest");
    let metadata = repository
        .path()
        .join("org/example/library/maven-metadata.xml");
    fs::create_dir_all(metadata.parent().expect("metadata parent")).expect("metadata directory");
    let write_metadata = |patch: &str| {
        fs::write(
            &metadata,
            format!(
                "<metadata><versioning><release>2.0.0</release><versions>\
                 <version>1.2.3</version><version>1.2.4</version>\
                 <version>{patch}</version><version>1.3.0</version>\
                 <version>2.0.0</version></versions></versioning></metadata>"
            ),
        )
        .expect("Maven metadata");
    };
    write_metadata("1.2.4");
    let artifact = repository.path().join("org/example/library/1.2.4");
    fs::create_dir_all(&artifact).expect("artifact directory");
    fs::write(
        artifact.join("library-1.2.4.pom"),
        r"<project><modelVersion>4.0.0</modelVersion><groupId>org.example</groupId><artifactId>library</artifactId><version>1.2.4</version></project>",
    )
    .expect("artifact POM");
    fs::write(artifact.join("library-1.2.4.jar"), b"fixture").expect("artifact JAR");

    let run = |additional: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_jman"))
            .env("JMAN_CACHE_DIR", cache.path())
            .args(["update", "org.example:library"])
            .args(["--path", project.path().to_str().expect("UTF-8 path")])
            .args(additional)
            .args(["--format", "json"])
            .output()
            .expect("update dependencies")
    };
    let before = fs::read_to_string(project.path().join("alpha/jman.toml")).expect("manifest");
    let dry_run = run(&["--dry-run"]);
    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&dry_run.stdout).expect("dry-run report");
    assert_eq!(report["level"], "patch");
    assert_eq!(report["dryRun"], true);
    assert_eq!(report["applied"], false);
    assert_eq!(report["changes"][0]["from"], "1.2.3");
    assert_eq!(report["changes"][0]["to"], "1.2.4");
    assert_eq!(
        fs::read_to_string(project.path().join("alpha/jman.toml")).expect("manifest"),
        before
    );

    let minor = run(&["--dry-run", "--level", "minor"]);
    assert!(minor.status.success());
    let report: serde_json::Value = serde_json::from_slice(&minor.stdout).expect("minor report");
    assert_eq!(report["changes"][0]["to"], "1.3.0");

    let applied = run(&[]);
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&applied.stdout).expect("applied report");
    assert_eq!(report["applied"], true);
    for module in ["alpha", "beta"] {
        let manifest = fs::read_to_string(project.path().join(module).join("jman.toml"))
            .expect("updated manifest");
        assert!(manifest.contains("\"org.example:library\" = \"1.2.4\""));
        let lock = fs::read_to_string(project.path().join(module).join("jman.lock"))
            .expect("module lockfile");
        assert!(lock.contains("version = \"1.2.4\""));
    }

    let preserved = [
        "jman.toml",
        "jman.lock",
        "alpha/jman.toml",
        "alpha/jman.lock",
        "beta/jman.toml",
        "beta/jman.lock",
    ]
    .map(|path| {
        (
            path,
            fs::read(project.path().join(path)).expect("transaction snapshot"),
        )
    });
    write_metadata("1.2.5");
    let failed = run(&[]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("restored"));
    for (path, expected) in preserved {
        assert_eq!(
            fs::read(project.path().join(path)).expect("restored workspace file"),
            expected,
            "{path} was not restored"
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn audit_reports_transitive_policy_results_and_reuses_them_offline() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    let repository = tempfile::tempdir().expect("temporary repository");
    let artifact = repository.path().join("org/example/library/1.0.0");
    fs::create_dir_all(&artifact).expect("artifact directory");
    fs::write(
        artifact.join("library-1.0.0.pom"),
        r"<project><modelVersion>4.0.0</modelVersion><groupId>org.example</groupId><artifactId>library</artifactId><version>1.0.0</version></project>",
    )
    .expect("artifact POM");
    fs::write(artifact.join("library-1.0.0.jar"), b"fixture").expect("artifact JAR");
    fs::write(
        project.path().join("jman.toml"),
        format!(
            r#"manifest-version = 1
[project]
group = "com.example"
name = "audit-demo"
version = "1.0.0"
java-release = 17
packaging = "jar"

[[repositories]]
id = "fixture"
url = "file://{}"

[dependencies.compile]
"org.example:library" = "1.0.0"

[audit]
[[audit.suppressions]]
id = "CVE-muted"
reason = "The vulnerable entry point is not reachable"
expires = "2999-12-31"
"#,
            repository.path().display()
        ),
    )
    .expect("manifest");
    let sync = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "sync",
            project.path().to_str().expect("UTF-8 path"),
            "--report",
            "json",
        ])
        .output()
        .expect("synchronize fixture");
    assert!(
        sync.status.success(),
        "{}",
        String::from_utf8_lossy(&sync.stderr)
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("OSV listener");
    let address = listener.local_addr().expect("OSV address");
    let server = std::thread::spawn(move || serve_osv_audit_fixture(&listener));
    let audit = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .env("JMAN_AUDIT_OSV_URL", format!("http://{address}"))
        .args([
            "audit",
            project.path().to_str().expect("UTF-8 path"),
            "--deny",
            "high",
            "--format",
            "json",
        ])
        .output()
        .expect("audit fixture");
    server.join().expect("OSV server");
    assert!(!audit.status.success());
    assert!(String::from_utf8_lossy(&audit.stderr).contains("audit policy denied"));
    let report: serde_json::Value = serde_json::from_slice(&audit.stdout).expect("audit report");
    assert_eq!(report["provider"], "osv");
    assert_eq!(report["source"], "network");
    assert_eq!(report["packagesChecked"], 1);
    assert_eq!(report["totalFindings"], 2);
    assert_eq!(report["active"], 1);
    assert_eq!(report["suppressed"], 1);
    assert_eq!(report["denied"], 1);
    assert_eq!(report["findings"][0]["advisory"]["severity"], "critical");
    assert_eq!(report["findings"][0]["active"], false);
    assert_eq!(report["findings"][1]["paths"][0]["module"], "audit-demo");

    let human = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .env("JMAN_AUDIT_OSV_URL", format!("http://{address}"))
        .args([
            "audit",
            project.path().to_str().expect("UTF-8 path"),
            "--offline",
        ])
        .output()
        .expect("human offline audit");
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    assert!(String::from_utf8_lossy(&human.stdout)
        .contains("  dependency paths:\n    module audit-demo\n    └── org.example:library:1.0.0"));

    let offline = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .env("JMAN_AUDIT_OSV_URL", format!("http://{address}"))
        .args([
            "audit",
            project.path().to_str().expect("UTF-8 path"),
            "--offline",
            "--format",
            "json",
        ])
        .output()
        .expect("offline audit");
    assert!(
        offline.status.success(),
        "{}",
        String::from_utf8_lossy(&offline.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&offline.stdout).expect("offline audit report");
    assert_eq!(report["source"], "cache");
    assert_eq!(report["totalFindings"], 2);
}

#[test]
fn publishes_complete_artifact_set_to_local_repository_and_reports_json() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    let repository = tempfile::tempdir().expect("local Maven repository");
    fs::create_dir_all(project.path().join("src/main/java/com/example")).expect("sources");
    fs::write(
        project
            .path()
            .join("src/main/java/com/example/Library.java"),
        "package com.example; /** Example API. */ public final class Library { \
         private Library() {} /** Returns a value. */ public static int value() { return 1; } }",
    )
    .expect("source");
    fs::write(
        project.path().join("jman.toml"),
        r#"manifest-version = 1
[project]
group = "com.example"
name = "library"
version = "1.2.3"
java-release = 17
packaging = "jar"

[publishing]
name = "Library"
description = "Example library"
url = "https://example.test/library"
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
    .expect("lockfile");

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "publish",
            project.path().to_str().expect("UTF-8 path"),
            "--local-repository",
            repository.path().to_str().expect("UTF-8 repository"),
            "--format",
            "json",
        ])
        .output()
        .expect("publish project");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["target"], "local");
    assert_eq!(report["dryRun"], false);
    assert_eq!(report["modules"][0]["artifact"], "library");
    let published = repository.path().join("com/example/library/1.2.3");
    for file in [
        "library-1.2.3.jar",
        "library-1.2.3-sources.jar",
        "library-1.2.3-javadoc.jar",
        "library-1.2.3.pom",
        "library-1.2.3.jar.sha256",
        "library-1.2.3.pom.sha512",
    ] {
        assert!(published.join(file).is_file(), "missing {file}");
    }
    let pom = fs::read_to_string(published.join("library-1.2.3.pom")).expect("published POM");
    assert!(pom.contains("<groupId>com.example</groupId>"));
    assert!(pom.contains("<description>Example library</description>"));

    let dry_repository = project.path().join("must-not-exist");
    let dry_run = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .args([
            "--quiet",
            "publish",
            project.path().to_str().expect("UTF-8 path"),
            "--local-repository",
            dry_repository.to_str().expect("UTF-8 repository"),
            "--dry-run",
            "--format",
            "json",
        ])
        .output()
        .expect("dry-run publication");
    assert!(dry_run.status.success());
    assert!(!dry_repository.exists());
    let report: serde_json::Value = serde_json::from_slice(&dry_run.stdout).expect("JSON report");
    assert_eq!(report["dryRun"], true);
}

#[cfg(target_os = "linux")]
#[test]
fn installed_java_list_json_is_machine_readable_without_network() {
    let cache = tempfile::tempdir().expect("temporary cache");
    let data = tempfile::tempdir().expect("temporary data");
    fake_managed_jdk(data.path(), "21.0.8+9", 21, "lts-21");
    fake_managed_jdk(data.path(), "22.0.2+9", 22, "feature-22");
    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .env("JMAN_DATA_DIR", data.path())
        .args(["java", "list", "--lts", "--installed", "--format", "json"])
        .output()
        .expect("list installed JDKs as JSON");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["catalog"].is_null());
    assert_eq!(report["installed"].as_array().map(Vec::len), Some(1));
    assert_eq!(report["installed"][0]["major"], 21);
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
    let listener_source = runner
        .path()
        .join("src/org/junit/platform/launcher/TestExecutionListener.java");
    let identifier_source = runner
        .path()
        .join("src/org/junit/platform/launcher/TestIdentifier.java");
    let result_source = runner
        .path()
        .join("src/org/junit/platform/engine/TestExecutionResult.java");
    let test_source = runner
        .path()
        .join("src/org/junit/platform/engine/TestSource.java");
    let method_source = runner
        .path()
        .join("src/org/junit/platform/engine/support/descriptor/MethodSource.java");
    let class_source = runner
        .path()
        .join("src/org/junit/platform/engine/support/descriptor/ClassSource.java");
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
import java.util.ServiceLoader;
import org.junit.platform.engine.TestExecutionResult;
import org.junit.platform.engine.support.descriptor.MethodSource;
import org.junit.platform.launcher.TestExecutionListener;
import org.junit.platform.launcher.TestIdentifier;
public final class ConsoleLauncher {
  public static void main(String[] args) throws Exception {
    if (!args[0].equals("execute")) System.exit(2);
    if (!"0".equals(System.getProperty("jman.test.port"))) System.exit(4);
    if (!"false".equals(System.getenv("TESTCONTAINERS_RYUK_DISABLED"))) System.exit(5);
    var arguments = List.of(args);
    if (arguments.stream().anyMatch(value -> value.contains("MissingTest"))) {
      System.exit(arguments.contains("--fail-if-no-tests") ? 1 : 3);
    }
    Class.forName("com.example.App")
        .getMethod("greeting", boolean.class)
        .invoke(null, true);
    var report = arguments.stream()
        .filter(value -> value.startsWith("--reports-dir="))
        .findFirst().orElseThrow().substring("--reports-dir=".length());
    var container = new TestIdentifier(
        "[engine:fixture]/[class:com.example.AppTest]",
        "AppTest",
        new org.junit.platform.engine.support.descriptor.ClassSource("com.example.AppTest"),
        null,
        false);
    var identifier = new TestIdentifier(
        "[engine:fixture]/[class:com.example.AppTest]/[method:greets]",
        "greets(String)[1]",
        new MethodSource("com.example.AppTest", "greets"),
        container.getUniqueId(),
        true);
    for (var listener : ServiceLoader.load(TestExecutionListener.class)) {
      listener.executionStarted(container);
      Thread.sleep(150);
      listener.executionStarted(identifier);
      Thread.sleep(120);
      listener.executionFinished(identifier, TestExecutionResult.failed(
          new AssertionError("expected Ada")));
      listener.executionFinished(container, TestExecutionResult.successful());
    }
    Thread.sleep(350);
    Files.writeString(Path.of(report).resolve("TEST-fixture.xml"),
        "<testsuite><testcase name=\"greets(String)[1]\" classname=\"com.example.AppTest\" time=\"0.012\">"
        + "<failure message=\"expected Ada\">exact stack trace</failure></testcase></testsuite>");
    System.out.print("JUnit Platform fixture passed");
  }
}"#,
    )
    .expect("runner source");
    fs::create_dir_all(listener_source.parent().expect("listener source parent"))
        .expect("listener sources");
    fs::create_dir_all(result_source.parent().expect("result source parent"))
        .expect("engine sources");
    fs::create_dir_all(method_source.parent().expect("descriptor source parent"))
        .expect("descriptor sources");
    fs::write(
        &listener_source,
        r"package org.junit.platform.launcher;
import org.junit.platform.engine.TestExecutionResult;
public interface TestExecutionListener {
  default void executionStarted(TestIdentifier identifier) {}
  default void executionSkipped(TestIdentifier identifier, String reason) {}
  default void executionFinished(TestIdentifier identifier, TestExecutionResult result) {}
}",
    )
    .expect("listener API");
    fs::write(
        &identifier_source,
        r"package org.junit.platform.launcher;
import java.util.Optional;
import org.junit.platform.engine.TestSource;
public final class TestIdentifier {
  private final String id;
  private final String displayName;
  private final TestSource source;
  private final String parentId;
  private final boolean test;
  public TestIdentifier(
      String id, String displayName, TestSource source, String parentId, boolean test) {
    this.id = id; this.displayName = displayName; this.source = source;
    this.parentId = parentId; this.test = test;
  }
  public boolean isTest() { return test; }
  public boolean isContainer() { return !test; }
  public String getUniqueId() { return id; }
  public Optional<String> getParentId() { return Optional.ofNullable(parentId); }
  public Optional<TestSource> getSource() { return Optional.ofNullable(source); }
  public String getDisplayName() { return displayName; }
}",
    )
    .expect("identifier API");
    fs::write(
        &test_source,
        "package org.junit.platform.engine; public interface TestSource {}",
    )
    .expect("test source API");
    fs::write(
        &result_source,
        r"package org.junit.platform.engine;
import java.util.Optional;
public final class TestExecutionResult {
  public enum Status { SUCCESSFUL, FAILED, ABORTED }
  private final Status status;
  private final Throwable throwable;
  private TestExecutionResult(Status status, Throwable throwable) {
    this.status = status; this.throwable = throwable;
  }
  public static TestExecutionResult failed(Throwable throwable) {
    return new TestExecutionResult(Status.FAILED, throwable);
  }
  public static TestExecutionResult successful() {
    return new TestExecutionResult(Status.SUCCESSFUL, null);
  }
  public Status getStatus() { return status; }
  public Optional<Throwable> getThrowable() { return Optional.ofNullable(throwable); }
}",
    )
    .expect("result API");
    fs::write(
        &method_source,
        r"package org.junit.platform.engine.support.descriptor;
import org.junit.platform.engine.TestSource;
public final class MethodSource implements TestSource {
  private final String className;
  private final String methodName;
  public MethodSource(String className, String methodName) {
    this.className = className; this.methodName = methodName;
  }
  public String getClassName() { return className; }
  public String getMethodName() { return methodName; }
}",
    )
    .expect("method source API");
    fs::write(
        &class_source,
        r"package org.junit.platform.engine.support.descriptor;
import org.junit.platform.engine.TestSource;
public final class ClassSource implements TestSource {
  private final String className;
  public ClassSource(String className) { this.className = className; }
  public String getClassName() { return className; }
}",
    )
    .expect("class source API");
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
        .arg(&listener_source)
        .arg(&identifier_source)
        .arg(&result_source)
        .arg(&test_source)
        .arg(&method_source)
        .arg(&class_source)
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
        r#"package com.example;
public final class App {
  public static String greeting(boolean formal) {
    if (formal) {
      return "Hello";
    }
    return "Hi";
}
}"#,
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
    let human_report = String::from_utf8(output.stderr).expect("UTF-8 human report");
    assert!(human_report.contains("\napp\n├─ AppTest\n"));
    assert!(human_report.contains("│  ├─ ✗ greets(String)[1]"));
    assert!(human_report.contains("│  └─ "));
    assert!(human_report.contains(" total · "));
    assert!(human_report.contains(" lifecycle"));
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

    let mut structured = Command::new(env!("CARGO_BIN_EXE_jman"))
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("structured test project");
    let stdout = structured.stdout.take().expect("structured stdout");
    let mut events = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let event =
            serde_json::from_str::<serde_json::Value>(&line.expect("read structured test event"))
                .expect("test event");
        if event["reason"] == "test-case" {
            assert!(
                structured
                    .try_wait()
                    .expect("poll structured test")
                    .is_none(),
                "test result must be reported before the worker exits"
            );
        }
        events.push(event);
    }
    let structured = structured
        .wait_with_output()
        .expect("finish structured test project");
    assert!(
        structured.status.success(),
        "{}",
        String::from_utf8_lossy(&structured.stderr)
    );
    assert_eq!(events.len(), 6);
    assert_eq!(events[0]["protocolVersion"], 3);
    assert_eq!(events[0]["reason"], "test-module-started");
    assert_eq!(events[1]["reason"], "test-suite-started");
    assert_eq!(events[2]["reason"], "test-case-started");
    assert_eq!(events[3]["reason"], "test-case");
    assert_eq!(events[3]["test"]["selector"], "com.example.AppTest#greets");
    assert_eq!(events[3]["test"]["displayName"], "greets(String)[1]");
    assert!(
        events[3]["test"]["durationMillis"]
            .as_u64()
            .expect("duration")
            >= 100
    );
    assert!(
        events[3]["test"]["durationNanos"]
            .as_u64()
            .expect("precise duration")
            >= 100_000_000
    );
    assert_eq!(events[3]["test"]["message"], "expected Ada");
    assert!(events[3]["test"]["details"]
        .as_str()
        .expect("failure details")
        .contains("AssertionError: expected Ada"));
    assert_eq!(events[4]["reason"], "test-suite");
    assert!(
        events[4]["suite"]["durationMillis"]
            .as_u64()
            .expect("suite duration")
            >= 250
    );
    assert!(
        events[4]["suite"]["lifecycleMillis"]
            .as_u64()
            .expect("lifecycle duration")
            >= 140
    );
    assert!(
        events[4]["suite"]["lifecycleNanos"]
            .as_u64()
            .expect("precise lifecycle duration")
            >= 140_000_000
    );
    assert_eq!(events[5]["reason"], "test-module-finished");
    assert!(!project
        .path()
        .join(".jman/output/test-classes/com/example/AppIntegrationTest.class")
        .exists());

    if coverage_tools_available() {
        let coverage_directory = project.path().join("coverage-report");
        fs::create_dir_all(&coverage_directory).expect("coverage report directory");
        fs::write(
            coverage_directory.join("keep-me.txt"),
            "unrelated user content\n",
        )
        .expect("coverage output sentinel");
        let coverage = Command::new(env!("CARGO_BIN_EXE_jman"))
            .env("JMAN_CACHE_DIR", cache.path())
            .env("TESTCONTAINERS_RYUK_DISABLED", "false")
            .args([
                "--no-progress",
                "test",
                project.path().to_str().expect("UTF-8 path"),
                "--report",
                "json",
                "--coverage",
                "--coverage-output",
                coverage_directory.to_str().expect("UTF-8 coverage path"),
                "--coverage-min-line",
                "1",
            ])
            .output()
            .expect("run tests with coverage");
        assert!(
            coverage.status.success(),
            "{}",
            String::from_utf8_lossy(&coverage.stderr)
        );
        let coverage_events = String::from_utf8(coverage.stdout)
            .expect("coverage events")
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("coverage event"))
            .collect::<Vec<_>>();
        assert!(coverage_events
            .iter()
            .any(|event| event["reason"] == "coverage-file"));
        assert!(coverage_events
            .iter()
            .any(|event| event["reason"] == "coverage-summary"));
        for report in ["index.html", "coverage.xml", "coverage.json"] {
            assert!(
                coverage_directory.join(report).is_file(),
                "missing {report}"
            );
        }
        assert_eq!(
            fs::read_to_string(coverage_directory.join("keep-me.txt"))
                .expect("preserved coverage output sentinel"),
            "unrelated user content\n"
        );
        let report: serde_json::Value = serde_json::from_slice(
            &fs::read(coverage_directory.join("coverage.json")).expect("coverage JSON"),
        )
        .expect("parse coverage JSON");
        assert_eq!(report["engine"], "jacoco");
        assert_eq!(report["modules"][0]["module"], "app");
        assert!(
            report["totals"]["line"]["covered"]
                .as_u64()
                .unwrap_or_default()
                > 0
        );
        assert_eq!(
            report["modules"][0]["files"][0]["path"],
            project
                .path()
                .join("src/main/java/com/example/App.java")
                .to_string_lossy()
                .as_ref()
        );

        let threshold = Command::new(env!("CARGO_BIN_EXE_jman"))
            .env("JMAN_CACHE_DIR", cache.path())
            .env("TESTCONTAINERS_RYUK_DISABLED", "false")
            .args([
                "--no-progress",
                "test",
                project.path().to_str().expect("UTF-8 path"),
                "--coverage",
                "--coverage-format",
                "summary",
                "--coverage-min-line",
                "100",
            ])
            .output()
            .expect("enforce coverage threshold");
        assert!(!threshold.status.success());
        let threshold_output = String::from_utf8_lossy(&threshold.stderr);
        assert!(threshold_output.contains("Coverage\n"));
        assert!(threshold_output.contains("├─ app\n"));
        assert!(threshold_output.contains("└─ workspace\n"));
        assert!(threshold_output.contains("coverage threshold failed: line"));
    }

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

fn coverage_tools_available() -> bool {
    let configured = ["JMAN_JACOCO_AGENT", "JMAN_JACOCO_CLI"].iter().all(|name| {
        std::env::var_os(name).is_some_and(|path| std::path::Path::new(&path).is_file())
    });
    let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    configured
        || (project.join("target/jacoco-0.8.15-agent.jar").is_file()
            && project.join("target/jacoco-0.8.15-cli.jar").is_file())
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

#[cfg(unix)]
#[test]
fn maven_import_prefers_the_project_wrapper_and_translates_its_effective_model() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    let commands = tempfile::tempdir().expect("commands");
    write_minimal_pom(project.path(), "native.example");
    write_effective_model(project.path(), "wrapper.example");
    write_fake_maven(&project.path().join("mvnw"), "wrapper-called", true);
    write_fake_maven(&commands.path().join("mvn"), "system-called", true);

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .env("PATH", commands.path())
        .args([
            "--no-progress",
            "init",
            "--import",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("import through Maven wrapper");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(project.path().join("wrapper-called").is_file());
    assert!(!project.path().join("system-called").exists());
    let manifest = fs::read_to_string(project.path().join("jman.toml")).expect("manifest");
    assert!(manifest.contains("group = \"wrapper.example\""));
    assert!(manifest.contains("main-class = \"wrapper.example.Application\""));
    assert!(manifest.contains("-Aexternal=true"));
}

#[cfg(unix)]
#[test]
fn maven_import_uses_system_maven_when_the_wrapper_is_absent() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    let commands = tempfile::tempdir().expect("commands");
    write_minimal_pom(project.path(), "native.example");
    write_effective_model(project.path(), "system.example");
    write_fake_maven(&commands.path().join("mvn"), "system-called", true);

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .env("PATH", commands.path())
        .args([
            "--no-progress",
            "init",
            "--import",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("import through system Maven");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(project.path().join("system-called").is_file());
    let manifest = fs::read_to_string(project.path().join("jman.toml")).expect("manifest");
    assert!(manifest.contains("group = \"system.example\""));
}

#[cfg(unix)]
#[test]
fn a_failing_maven_wrapper_does_not_silently_change_to_another_importer() {
    let project = tempfile::tempdir().expect("temporary project");
    let cache = tempfile::tempdir().expect("temporary cache");
    let commands = tempfile::tempdir().expect("commands");
    write_minimal_pom(project.path(), "native.example");
    write_effective_model(project.path(), "system.example");
    write_fake_maven(&project.path().join("mvnw"), "wrapper-called", false);
    write_fake_maven(&commands.path().join("mvn"), "system-called", true);

    let output = Command::new(env!("CARGO_BIN_EXE_jman"))
        .env("JMAN_CACHE_DIR", cache.path())
        .env("PATH", commands.path())
        .args([
            "--no-progress",
            "init",
            "--import",
            project.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("failed wrapper import");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Maven model export"));
    assert!(stderr.contains("fixture model failure"));
    assert!(project.path().join("wrapper-called").is_file());
    assert!(!project.path().join("system-called").exists());
    assert!(!project.path().join("jman.toml").exists());
    assert!(!project.path().join("jman.lock").exists());
}

#[cfg(unix)]
fn write_minimal_pom(project: &std::path::Path, group: &str) {
    fs::write(
        project.join("pom.xml"),
        format!(
            "<project><modelVersion>4.0.0</modelVersion><groupId>{group}</groupId>\
             <artifactId>demo</artifactId><version>1</version></project>"
        ),
    )
    .expect("POM");
}

#[cfg(unix)]
fn write_effective_model(project: &std::path::Path, group: &str) {
    fs::write(
        project.join("effective.xml"),
        format!(
            r"<project><modelVersion>4.0.0</modelVersion>
              <groupId>{group}</groupId><artifactId>demo</artifactId><version>1</version>
              <properties><exec.mainClass>{group}.Application</exec.mainClass></properties>
              <build><directory>{}/target</directory><plugins><plugin>
                <artifactId>maven-compiler-plugin</artifactId><configuration>
                  <release>21</release><parameters>true</parameters>
                  <compilerArgs><arg>-Aexternal=true</arg></compilerArgs>
                </configuration>
              </plugin></plugins></build>
            </project>",
            project.display()
        ),
    )
    .expect("effective model");
}

#[cfg(unix)]
fn write_fake_maven(path: &std::path::Path, marker: &str, succeeds: bool) {
    use std::os::unix::fs::PermissionsExt;

    let behavior = if succeeds {
        "/bin/cp \"$PWD/effective.xml\" \"$output\"\nexit 0"
    } else {
        "printf 'fixture model failure\\n'\nexit 19"
    };
    fs::write(
        path,
        format!(
            "#!/bin/sh\noutput=''\nfor argument in \"$@\"; do\n\
             \x20 case \"$argument\" in -Doutput=*) output=${{argument#-Doutput=}} ;; esac\n\
             done\n: > \"$PWD/{marker}\"\n{behavior}\n"
        ),
    )
    .expect("fake Maven");
    let mut permissions = fs::metadata(path)
        .expect("fake Maven metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("executable fake Maven");
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
        .env("PATH", project.path())
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
        .env("PATH", project.path())
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
        .env("PATH", project.path())
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
    assert!(String::from_utf8_lossy(&why.stdout).contains(
        "Dependency paths for org.example:library:\ncom.example:demo:1\n└── org.example:library:1"
    ));

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
fn java_use_selects_an_installed_project_toolchain() {
    let project = tempfile::tempdir().expect("temporary project");
    let root = tempfile::tempdir().expect("isolated Java home");
    let cache = root.path().join("cache");
    let data = root.path().join("data");
    let config = root.path().join("config");
    let java_17 = fake_managed_jdk(&data, "17.0.12+7", 17, "project-17");
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
    let nested = project.path().join("src/main/java");
    fs::create_dir_all(&nested).expect("nested project directory");
    let output = isolated_jman(&cache, &data, &config)
        .args([
            "--quiet",
            "java",
            "use",
            "17",
            "--vendor",
            "temurin",
            "--path",
            nested.to_str().expect("UTF-8 path"),
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
    assert!(manifest.contains("vendor = \"temurin\""));

    let which = isolated_jman(&cache, &data, &config)
        .args([
            "java",
            "which",
            project.path().to_str().expect("UTF-8 path"),
            "--format",
            "json",
        ])
        .output()
        .expect("find Java");
    assert!(which.status.success());
    let selected: serde_json::Value =
        serde_json::from_slice(&which.stdout).expect("project selection JSON");
    assert_eq!(selected["version"], "17.0.12+7");
    assert_eq!(selected["source"], "project");
    assert_eq!(selected["home"], java_17.to_string_lossy().as_ref());
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

fn serve_osv_audit_fixture(listener: &TcpListener) {
    for _ in 0..3 {
        let (mut stream, _) = listener.accept().expect("OSV connection");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).expect("OSV request bytes");
            request.extend_from_slice(&buffer[..read]);
            let headers = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|index| index + 4);
            let Some(headers) = headers else {
                continue;
            };
            let header_text = String::from_utf8_lossy(&request[..headers]);
            let content_length = header_text
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or_default();
            if request.len() >= headers + content_length {
                break;
            }
        }
        let request = String::from_utf8_lossy(&request);
        let body = if request.starts_with("POST /v1/querybatch") {
            assert!(request.contains("org.example:library"));
            r#"{"results":[{"vulns":[{"id":"GHSA-active"},{"id":"GHSA-muted"}]}]}"#
        } else if request.starts_with("GET /v1/vulns/GHSA-active") {
            r#"{"id":"GHSA-active","summary":"Active vulnerability","database_specific":{"severity":"HIGH"},"affected":[{"package":{"ecosystem":"Maven","name":"org.example:library"},"ranges":[{"events":[{"fixed":"1.0.1"}]}]}]}"#
        } else {
            assert!(request.starts_with("GET /v1/vulns/GHSA-muted"));
            r#"{"id":"GHSA-muted","aliases":["CVE-muted"],"summary":"Suppressed vulnerability","database_specific":{"severity":"CRITICAL"},"affected":[{"package":{"ecosystem":"Maven","name":"org.example:library"},"ranges":[{"events":[{"fixed":"2.0.0"}]}]}]}"#
        };
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("OSV response");
    }
}

#[cfg(target_os = "linux")]
#[test]
#[allow(clippy::too_many_lines)]
fn java_global_project_exec_and_shims_share_one_selection_model() {
    let root = tempfile::tempdir().expect("isolated Java home");
    let cache = root.path().join("cache");
    let data = root.path().join("data");
    let config = root.path().join("config");
    let outside = root.path().join("outside");
    let project = root.path().join("project");
    fs::create_dir_all(&outside).expect("outside directory");
    fs::create_dir_all(&project).expect("project directory");
    let java_21 = fake_managed_jdk(&data, "21.0.8+9", 21, "global-21");
    let java_17 = fake_managed_jdk(&data, "17.0.12+7", 17, "project-17");

    let installed = isolated_jman(&cache, &data, &config)
        .current_dir(&outside)
        .args([
            "java",
            "install",
            "21",
            "--offline",
            "--global",
            "--no-progress",
        ])
        .output()
        .expect("install and select global Java");
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    assert_eq!(
        fs::read_link(data.join("current")).expect("current symlink"),
        java_21
    );
    let user_config = fs::read_to_string(config.join("config.toml")).expect("user config");
    assert!(user_config.contains("jdk = \"21.0.8+9\""));
    assert!(user_config.contains("vendor = \"temurin\""));

    let global = isolated_jman(&cache, &data, &config)
        .args([
            "java",
            "which",
            outside.to_str().expect("outside path"),
            "--format",
            "json",
        ])
        .output()
        .expect("inspect global Java");
    assert!(global.status.success());
    let global: serde_json::Value =
        serde_json::from_slice(&global.stdout).expect("global selection JSON");
    assert_eq!(global["version"], "21.0.8+9");
    assert_eq!(global["source"], "global");
    assert_eq!(global["home"], java_21.to_string_lossy().as_ref());

    fs::write(
        project.join("jman.toml"),
        r#"manifest-version = 1
[project]
group = "com.example"
name = "project"
version = "1"
java-release = 17
packaging = "jar"

[toolchain]
jdk = "17"
vendor = "temurin"
"#,
    )
    .expect("project manifest");
    let selected = isolated_jman(&cache, &data, &config)
        .args([
            "java",
            "which",
            project.to_str().expect("project path"),
            "--format",
            "json",
        ])
        .output()
        .expect("inspect project Java");
    assert!(selected.status.success());
    let selected: serde_json::Value =
        serde_json::from_slice(&selected.stdout).expect("project selection JSON");
    assert_eq!(selected["version"], "17.0.12+7");
    assert_eq!(selected["source"], "project");
    assert_eq!(selected["home"], java_17.to_string_lossy().as_ref());

    let executed = isolated_jman(&cache, &data, &config)
        .current_dir(&outside)
        .args(["java", "exec", "17", "--", "java"])
        .output()
        .expect("execute explicit Java");
    assert!(executed.status.success());
    let executed = String::from_utf8(executed.stdout).expect("exec output");
    assert!(executed.contains("project-17"));
    assert!(executed.contains(java_17.to_string_lossy().as_ref()));

    let global_exec = isolated_jman(&cache, &data, &config)
        .current_dir(&outside)
        .args(["java", "exec", "--", "java"])
        .output()
        .expect("execute effective global Java");
    assert!(global_exec.status.success());
    let global_exec = String::from_utf8(global_exec.stdout).expect("global exec output");
    assert!(global_exec.contains("global-21"));
    assert!(global_exec.contains(java_21.to_string_lossy().as_ref()));

    let setup = isolated_jman(&cache, &data, &config)
        .current_dir(&outside)
        .args(["java", "setup", "--shell", "zsh", "--no-progress"])
        .output()
        .expect("install Java shims");
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    assert!(data.join("shims/java").is_symlink());
    let setup_output = String::from_utf8_lossy(&setup.stdout);
    assert!(setup_output.contains("jman shell init zsh"));
    assert!(setup_output.contains("~/.zshrc"));
    assert!(setup_output.contains("JMAN completion"));

    let shim = Command::new(data.join("shims/java"))
        .current_dir(&project)
        .env("JMAN_CACHE_DIR", &cache)
        .env("JMAN_DATA_DIR", &data)
        .env("JMAN_CONFIG_DIR", &config)
        .output()
        .expect("run project-aware Java shim");
    assert!(
        shim.status.success(),
        "{}",
        String::from_utf8_lossy(&shim.stderr)
    );
    let shim = String::from_utf8(shim.stdout).expect("shim output");
    assert!(shim.contains("project-17"));
    assert!(shim.contains(java_17.to_string_lossy().as_ref()));

    let protected = isolated_jman(&cache, &data, &config)
        .current_dir(&outside)
        .args(["java", "remove", "21", "--force", "--no-progress"])
        .output()
        .expect("protect global Java");
    assert!(!protected.status.success());
    assert!(String::from_utf8_lossy(&protected.stderr).contains("selected globally"));

    let switched = isolated_jman(&cache, &data, &config)
        .current_dir(&outside)
        .args(["java", "use", "17", "--global", "--no-progress"])
        .output()
        .expect("switch global Java");
    assert!(
        switched.status.success(),
        "{}",
        String::from_utf8_lossy(&switched.stderr)
    );
    assert_eq!(
        fs::read_link(data.join("current")).expect("switched current symlink"),
        java_17
    );
    let switched_config = fs::read_to_string(config.join("config.toml")).expect("user config");
    assert!(switched_config.contains("jdk = \"17.0.12+7\""));
}

#[cfg(target_os = "linux")]
#[test]
#[allow(clippy::too_many_lines)]
fn installed_java_commands_resolve_vendor_from_context_or_unique_installation() {
    let root = tempfile::tempdir().expect("isolated Java home");
    let cache = root.path().join("cache");
    let data = root.path().join("data");
    let config = root.path().join("config");
    let outside = root.path().join("outside");
    let project = root.path().join("project");
    fs::create_dir_all(&outside).expect("outside directory");
    fs::create_dir_all(&project).expect("project directory");
    let zulu = fake_managed_jdk_for_vendor(&data, "zulu", "27+35", 27, "zulu-27");

    let unique = isolated_jman(&cache, &data, &config)
        .current_dir(&outside)
        .args(["java", "exec", "27", "--", "java"])
        .output()
        .expect("execute unique installed vendor");
    assert!(
        unique.status.success(),
        "{}",
        String::from_utf8_lossy(&unique.stderr)
    );
    assert!(String::from_utf8_lossy(&unique.stdout).contains("zulu-27"));

    let selected = isolated_jman(&cache, &data, &config)
        .current_dir(&outside)
        .args(["java", "use", "27", "--global", "--no-progress"])
        .output()
        .expect("select unique installed vendor globally");
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert_eq!(
        fs::read_link(data.join("current")).expect("global Java symlink"),
        zulu
    );
    let global_config = fs::read_to_string(config.join("config.toml")).expect("global config");
    assert!(global_config.contains("vendor = \"zulu\""));

    let temurin = fake_managed_jdk_for_vendor(&data, "temurin", "27+35", 27, "temurin-27");
    fs::remove_file(config.join("config.toml")).expect("clear global preference");
    let ambiguous = isolated_jman(&cache, &data, &config)
        .current_dir(&outside)
        .args(["java", "exec", "27", "--", "java"])
        .output()
        .expect("reject ambiguous installed vendors");
    assert!(!ambiguous.status.success());
    let error = String::from_utf8_lossy(&ambiguous.stderr);
    assert!(error.contains("multiple vendors"));
    assert!(error.contains("temurin"));
    assert!(error.contains("zulu"));
    assert!(error.contains("--vendor <vendor>"));

    let selected = isolated_jman(&cache, &data, &config)
        .current_dir(&outside)
        .args([
            "java",
            "use",
            "27",
            "--vendor",
            "zulu",
            "--global",
            "--no-progress",
        ])
        .output()
        .expect("select explicit global vendor");
    assert!(selected.status.success());
    let global = isolated_jman(&cache, &data, &config)
        .current_dir(&outside)
        .args(["java", "exec", "27", "--", "java"])
        .output()
        .expect("reuse matching global vendor");
    assert!(global.status.success());
    assert!(String::from_utf8_lossy(&global.stdout).contains("zulu-27"));

    fs::write(
        project.join("jman.toml"),
        r#"manifest-version = 1
[project]
group = "com.example"
name = "vendor-precedence"
version = "1"
java-release = 27
packaging = "jar"

[toolchain]
jdk = "27"
vendor = "temurin"
"#,
    )
    .expect("project manifest");
    let local = isolated_jman(&cache, &data, &config)
        .current_dir(&project)
        .args(["java", "exec", "27", "--", "java"])
        .output()
        .expect("prefer matching project vendor");
    assert!(local.status.success());
    let local = String::from_utf8_lossy(&local.stdout);
    assert!(local.contains("temurin-27"));
    assert!(local.contains(temurin.to_string_lossy().as_ref()));

    let removal = isolated_jman(&cache, &data, &config)
        .current_dir(&project)
        .args([
            "java",
            "remove",
            "27",
            "--dry-run",
            "--force",
            "--no-progress",
        ])
        .output()
        .expect("resolve removal vendor from project");
    assert!(removal.status.success());
    let removal = String::from_utf8_lossy(&removal.stdout);
    assert!(removal.contains("Would remove temurin 27+35"));
    assert!(!removal.contains("zulu"));
}

#[cfg(target_os = "linux")]
#[test]
fn java_use_requires_an_install_and_shell_initializers_include_completions() {
    let root = tempfile::tempdir().expect("isolated Java home");
    let cache = root.path().join("cache");
    let data = root.path().join("data");
    let config = root.path().join("config");

    let missing = isolated_jman(&cache, &data, &config)
        .args(["java", "use", "25", "--global", "--no-progress"])
        .output()
        .expect("reject missing Java");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("run `jman java install 25`"));

    for shell in ["bash", "zsh", "fish"] {
        let output = isolated_jman(&cache, &data, &config)
            .args(["shell", "init", shell])
            .output()
            .expect("render shell initialization");
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).expect("shell initialization");
        assert!(text.contains("jman java which"));
        assert!(text.contains("shims"));
        if shell == "bash" {
            assert!(text.contains("--installed"));
            assert!(text.contains("complete"));
            assert!(text.contains("_jman"));
            validate_shell_syntax_if_available("bash", &text);
        }
        if shell == "zsh" {
            assert!(text.contains("--installed"));
            assert!(text.contains("#compdef jman"));
            assert!(text.contains("compinit"));
            validate_shell_syntax_if_available("zsh", &text);
        }
        if shell == "fish" {
            assert!(text.contains("complete -c jman"));
            assert!(text.contains("-l installed"));
        }
    }
}

fn validate_shell_syntax_if_available(shell: &str, script: &str) {
    let mut child = match Command::new(shell).arg("-n").stdin(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!("could not start {shell} syntax validation: {error}"),
    };
    child
        .stdin
        .take()
        .expect("shell stdin")
        .write_all(script.as_bytes())
        .expect("write shell initialization");
    let status = child.wait().expect("validate shell initialization");
    assert!(
        status.success(),
        "{shell} rejected generated initialization"
    );
}

#[cfg(target_os = "linux")]
fn isolated_jman(cache: &Path, data: &Path, config: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jman"));
    command
        .env("JMAN_CACHE_DIR", cache)
        .env("JMAN_DATA_DIR", data)
        .env("JMAN_CONFIG_DIR", config)
        .env_remove("JAVA_HOME");
    command
}

#[cfg(target_os = "linux")]
fn fake_managed_jdk(data: &Path, version: &str, major: u16, label: &str) -> PathBuf {
    fake_managed_jdk_for_vendor(data, "temurin", version, major, label)
}

#[cfg(target_os = "linux")]
fn fake_managed_jdk_for_vendor(
    data: &Path,
    vendor: &str,
    version: &str,
    major: u16,
    label: &str,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let home = data
        .join("jdks")
        .join(format!("{vendor}-{}-linux-x64", version.replace('+', "_")));
    fs::create_dir_all(home.join("bin")).expect("managed JDK bin");
    for executable in ["java", "javac", "jar"] {
        let path = home.join("bin").join(executable);
        fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s|%s\\n' '{label}' \"$JAVA_HOME\"\n"),
        )
        .expect("fake Java executable");
        let mut permissions = fs::metadata(&path)
            .expect("executable metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("executable permissions");
    }
    let metadata = jman_build::toolchain::ManagedJdk {
        vendor: vendor.to_owned(),
        version: version.to_owned(),
        major,
        os: "linux".to_owned(),
        architecture: "x64".to_owned(),
        checksum: "sha256:test".to_owned(),
        home: home.clone(),
    };
    fs::write(
        home.join(".jman-toolchain.json"),
        serde_json::to_vec_pretty(&metadata).expect("managed JDK metadata"),
    )
    .expect("write managed JDK metadata");
    home
}
