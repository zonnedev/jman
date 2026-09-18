# Changelog

All notable changes to JMAN are documented here. The project follows Semantic
Versioning while it approaches a stable 0.3.0 release.

## [Unreleased]

### Added

- `jman java list` now discovers installable JDKs across audited Foojay
  distributions, supports `--vendor`, and translates provider metadata into a
  versioned JMAN catalog model so another provider can be added without changing
  CLI, cache, manifest, or installer contracts.
- Java install, use, and remove commands now accept `--vendor`, while preserving
  Temurin as the default distribution.

### Security

- Managed JDK downloads now require valid SHA-256 metadata, HTTPS, and a
  distribution-scoped allowlist for the initial vendor URL and every redirect.

### Fixed

- Go to Declaration on an overriding method now follows its compiler-resolved
  override family to the workspace interface or superclass declaration, while
  Go to Definition continues to select the concrete method.
- Find References on an interface or superclass method now includes calls
  resolved to every compiler-linked overriding implementation, including when
  the selected declaration is available only in the structural workspace index.

## [0.3.0-rc.3] - 2026-09-18

### Added

- Processor-enabled compile units now use a persistent, project-JDK javac lane
  for diagnostics, completion, hover, and navigation. It runs the exact
  annotation-processor path and options exported by Maven or Gradle, so Lombok,
  MapStruct, Micronaut, and custom JSR 269 processors share one generic path.
- End-to-end Lombok coverage now exercises generated getters, setters, builders,
  logging fields, completion, hover, local-source navigation, honest unrelated
  diagnostics, and unsaved editor overlays.

### Changed

- The processor worker artifact now also carries the Java 17-compatible semantic
  bridge. Native `-proc:none` attribution remains the fast path only for compile
  units without processors; processor-specific diagnostic heuristics were
  removed.

### Fixed

- Gradle and Maven tests and project operations launched from editor clients now
  inherit the compatible build-tool JDK selected during project import instead
  of the Java 25 native-frontend runtime.

## [0.3.0-rc.2] - 2026-09-18

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
  compile.
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
