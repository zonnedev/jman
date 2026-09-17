# Change Log

All notable changes to the JMAN Java extension are documented here.

## Unreleased

### Fixed

- Select a Gradle-compatible installed JDK independently from the Java 25
  native frontend, with an explicit `jman.java.buildJavaHome` override and
  actionable diagnostics when no compatible runtime is installed.
- Run annotation processors with the compile unit or selected build JDK,
  preventing false missing-symbol diagnostics from older Lombok versions.

## 0.3.0 - 2026-09-17

### Added

- First Visual Studio Marketplace pre-release for Linux x64.
- Native Java language features backed by JMAN's Rust language server and
  GraalVM compiler frontend.
- JMAN, Maven, and Gradle workspace discovery and synchronization.
- VS Code Test Explorer integration for Java unit and integration tests.
- Commands for project checks, builds, runs, tests, synchronization, status,
  index rebuilds, workspace-cache cleanup, and server restarts.
- Marketplace branding, support documentation, workspace-trust declarations,
  and platform-specific release packaging.

### Security

- Refreshed runtime and release dependencies before the initial public build.
