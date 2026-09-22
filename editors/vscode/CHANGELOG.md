# Change Log

All notable changes to the JMAN Java extension are documented here.

## Unreleased

## 0.7.0 - 2026-09-22

## 0.6.0 - 2026-09-20

### Added

- Added a Test Explorer coverage profile with workspace/file counters and
  line-level coverage details for native JMAN projects.

## 0.5.2 - 2026-09-20

## 0.5.1 - 2026-09-20

### Added

- Bundled JMAN 0.5.1 with repository-backed dependency maintenance,
  vulnerability auditing, Maven-compatible publication, live hierarchical test
  reporting, generic annotation-processor semantics, and expanded Java
  navigation.

### Changed

- Promoted JMAN Java from the Marketplace pre-release channel to the stable
  channel.

## 0.3.2 - 2026-09-18

### Fixed

- Run Gradle and Maven Test Explorer sessions and project commands with the
  compatible build-tool JDK selected by JMAN instead of GraalVM Java 25.

## 0.3.1 - 2026-09-18

### Fixed

- Select a Gradle-compatible installed JDK independently from the Java 25
  native frontend, with an explicit `jman.java.buildJavaHome` override and
  actionable diagnostics when no compatible runtime is installed.
- Run annotation processors with the compile unit or selected build JDK,
  preventing false missing-symbol diagnostics from older Lombok versions.
- Preserve usable annotation-processor output across partial compilation
  failures and avoid false Lombok logging-field diagnostics during refactors.
- Keep definition navigation on local workspace sources instead of opening
  decompiled copies while the background index is still being built.

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
