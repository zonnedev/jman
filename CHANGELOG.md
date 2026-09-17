# Changelog

All notable changes to JMAN are documented here. The project follows Semantic
Versioning while it approaches a stable 0.3.0 release.

## [Unreleased]

### Fixed

- Gradle model import now separates the build-tool JVM from the Java 25 native
  frontend runtime, honors project and editor overrides, and automatically
  selects an installed JDK compatible with the wrapper.
- Gradle project-dependency discovery now supports both the pre-8.11 and
  current `ProjectDependency` APIs.
- Annotation processors now run with each compile unit's configured JDK, or
  the selected build JDK when the build model has no compiler executable, so
  processors such as Lombok are not forced onto JMAN's Java 25 runtime.
- Annotation processing now runs dependency modules first and retains useful
  partial generated classes when unrelated project errors prevent a complete
  compile; false missing-`log` diagnostics are suppressed for Lombok logging
  annotations while a workspace is temporarily broken.
- Go to Definition and Go to Type Definition now prefer matching workspace
  source files over decompiled classpath copies while background indexing is
  still pending.

## [0.3.0-rc.1] - 2026-09-17

### Added

- Native Maven metadata resolution, deterministic lockfiles, managed JDKs, and
  bounded incremental multi-module compilation.
- Deterministic thin, fat, source, and Javadoc archives plus native application
  execution.
- Isolated JUnit Platform execution with protocol-v2 structured events,
  parameterized-test identities, reruns, debug descriptors, unit/integration
  source sets, and best-effort cancellation.
- Rust/GraalVM Java language server with Maven, Gradle, and native JMAN project
  models; semantic navigation and refactoring; diagnostics; semantic tokens;
  multi-root workspaces; generated-source metadata; and VS Code/Neovim clients.
- Compatibility fixtures for Maven 3.9.9, Gradle 8.14.1/9.1.0, JDK
  17/21/25, JPMS, annotation processing, and real framework projects.
- Docker/Podman diagnostics and documented Testcontainers cleanup behavior.

### Changed

- Adopted **JMAN** and `jman` as the canonical product and executable identity.
- Migrated crate and package names, Java namespaces, project files, caches,
  environment variables, editor integrations, protocol identifiers, tests, and
  release artifacts to the JMAN namespace.
- `jman java list` now prints installed JDKs first and the remote Temurin catalog
  afterward; `--local`, `--major`, and `--lts` control catalog selection.
- Remote Java catalogs use a platform-scoped TTL cache, HTTP ETag revalidation,
  stale-cache fallback, explicit `--refresh`, and structured `--format json`
  output for scripts and editor integrations.
- GitHub Actions now validates portable release gates, builds and attests the
  Linux x86-64 CLI archive and Linux x64 VSIX from version-matched tags, creates
  checksum and manifest assets, publishes GitHub Releases, and supports guarded
  OIDC-based Visual Studio Marketplace publication.

### Known limitations

- Maven plugins and arbitrary lifecycle behavior are not imported or executed.
- Coverage collection, watch mode, and native OCI images are not included in
  this release candidate.
- Fresh GraalVM Native Image builds are checksummed but not byte-identical;
  reproducible-archive claims remain limited to JMAN's Java JAR outputs.
- Maven compatibility claims are limited to the metadata contract in
  `docs/product-contract.md`.
