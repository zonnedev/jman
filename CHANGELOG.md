# Changelog

All notable changes to JMAN are documented here. The project follows Semantic
Versioning while it approaches a stable 0.3.0 release.

## [Unreleased]

## [0.8.0] - 2026-09-26

### Added

- JJFS 1 defines JMAN's deterministic Java formatting contract and applies it
  consistently through `jman fmt`, VS Code, and Neovim, including semantic
  import normalization, API-first member ordering, mandatory control-flow
  braces, AST-aware wrapping, conservative prose-comment formatting,
  formatter directives, and idempotent safety validation.
- VS Code and Neovim now expose JJFS through dedicated format commands,
  editor-native format-on-save configuration, and JMAN-only provider
  selection that avoids competing language-server edits.

### Fixed

- Compatibility-tool provisioning now retries connection-level failures and
  falls back to alternate official Gradle or Maven download endpoints.
- JJFS import conflict resolution now preserves types that were already bound
  through a simple name, preventing a fully qualified type with the same name
  from changing annotations or other existing references.
- JJFS removes blank lines immediately after an opening brace while preserving
  intentional blank lines between statements inside the block.
- JJFS keeps declaration Javadocs directly attached to the annotations and
  modifiers they document instead of preserving an authored blank separator.
- JJFS uses one indentation level for fluent-chain continuations and keeps
  compact lists of simple zero-argument calls inline instead of expanding them
  as structurally complex arguments.
- JJFS inserts the required separator between a parenthesized inline annotation
  and its annotated parameter or type.
- JJFS preserves statement boundaries and continuation indentation inside block
  lambdas passed as method arguments instead of collapsing their bodies.

## [0.7.1] - 2026-09-22

### Added

- `jman build --rebuild` forces recompilation and repackaging for one run
  without clearing dependency downloads or installed JDKs.

### Changed

- `jman doctor` now finishes successful JDK and container checks with a visible
  check mark and marks unavailable optional container runtimes as warnings.
- Build output groups artifacts by module in a compact tree, shows their output
  directory once, and reserves full checksums and conflict details for `-v`.
- Repository cache entries now retain their repository identity, so identical
  Maven coordinates from different repositories cannot cross-contaminate
  workspaces. Existing unscoped cache entries require a one-time online
  `jman sync --refresh` before offline resolution.
- Dependency fetches share immutable artifact bytes in memory, and build
  classpaths index locked packages once instead of rescanning them per entry.
- Warm builds reuse verified thin, sources, Javadoc, and fat JARs when their
  effective inputs are unchanged, avoiding repeated archive generation.

## [0.7.0] - 2026-09-22

### Added

- JMAN can now own the current user's Java selection: `java install --global`
  installs and selects, `java use --global` switches among installed JDKs,
  project selections override the global default, and `java which` explains
  the effective source. Durable JDK storage, atomic configuration/current-link
  updates, project-aware JDK command shims, Bash/Zsh/Fish initialization,
  scoped `java exec`, doctor diagnostics, and editor build-runtime integration
  keep CLI, shell, build, and language-server behavior aligned.
- `jman shell init` now generates command and option completion for Bash, Zsh,
  and Fish from JMAN's Clap command model alongside the Java shim and
  `JAVA_HOME` integration.
- GitHub Releases now include a checksum-verified remote installer that keeps
  versioned user-local installations, safely updates the `jman` symlink,
  offers Java 25 and shim setup, and prints shell activation instructions.

### Changed

- `jman java list --installed` replaces the ambiguous `--local` filter. It
  lists only JMAN-installed JDKs, performs no catalog network request, and
  composes with the existing release, LTS, vendor, and output-format filters.
- Installed-JDK commands now infer an omitted vendor from a matching project
  selection, matching global selection, or unique installed vendor. Ambiguous
  matches require `--vendor`; downloads continue to default to Temurin.

## [0.6.0] - 2026-09-20

### Added

- `jman test --coverage` now provides pinned JaCoCo instrumentation, isolated
  multi-module collection, provider-neutral line/branch/method reports,
  HTML/XML/JSON output, include/exclude filters, CI thresholds, structured
  editor events, and native VS Code and Neovim coverage workflows.

## [0.5.2] - 2026-09-20

## [0.5.1] - 2026-09-20

### Added

- `jman audit` now scans the complete locked Maven graph through a
  provider-neutral audit core backed by OSV, reports shortest transitive paths
  and fixed versions, normalizes advisory severity, caches exact-graph results
  for offline and resilient operation, supports human and JSON output plus CI
  deny thresholds, and validates reasoned, expiring suppressions.
- `jman update` now applies repository-backed direct-dependency upgrades across
  a workspace, defaults to patch-only changes, supports bounded minor and major
  upgrades, coordinate selection, dry-run and JSON plans, cached offline use,
  and restores every manifest and lockfile if synchronization fails.
- `jman outdated` now aggregates direct dependencies across workspaces,
  discovers Maven repository versions, classifies patch, minor, and major
  upgrades, excludes prereleases by default, supports cached offline operation,
  and provides human-readable and JSON reports without modifying project state.
- `jman publish` now builds and stages Maven-compatible multi-module
  publications for the local Maven repository, generic HTTPS repositories, and
  the Maven Central Portal. It generates standalone POM metadata, source and
  Javadoc JARs, checksums, optional or Central-required GPG signatures,
  deterministic Central bundles, dry-run validation, and JSON reports while
  keeping credentials in environment variables. A cross-tool acceptance test
  verifies the resulting multi-module repository with independent JMAN, Maven,
  and Gradle consumers.
- `jman test` now reports each JUnit test as it starts and finishes through a
  versioned listener protocol, with a live module/suite/test tree, nested
  failure details, accurate method and suite-lifecycle timings, JSON and editor
  updates, and XML-backed reconciliation when live events are unavailable.
- Maven imports now prefer the project wrapper, then system Maven, before
  falling back to JMAN's native project importer. Maven-provided effective
  models retain active model semantics, compiler arguments, processor paths,
  Java release settings, and conventional application entry-point properties.
- `jman java list` now discovers installable JDKs across audited Foojay
  distributions, supports `--vendor`, and translates provider metadata into a
  versioned JMAN catalog model so another provider can be added without changing
  CLI, cache, manifest, or installer contracts.
- Java install, use, and remove commands now accept `--vendor`, while preserving
  Temurin as the default distribution.
- Human-readable Java listings now merge local and remote releases into a table
  with an installed marker. The default shows the latest release per vendor,
  while `--all` exposes the complete matching catalog.

### Changed

- Human dependency paths in `jman why` and `jman audit` now render as
  deterministic trees with merged common prefixes. `jman why` includes every
  shortest route, and audit trees are grouped by workspace module; structured
  JSON path data is unchanged.
- The Neovim client now provides VS Code-equivalent JMAN, Maven, and Gradle
  project operations, wrapper and compatible build-JDK selection, LSP-prepared
  class and method tests, richer status and health reporting, synchronization
  events, safer maintenance actions, complete help, and direct installation
  from the main repository.

### Security

- Managed JDK downloads now require valid SHA-256 metadata, HTTPS, and a
  distribution-scoped allowlist for the initial vendor URL and every redirect.

### Fixed

- The Neovim client now treats LSP JSON `null` values as absent data instead of
  indexing Neovim's `vim.NIL` sentinel in status and command callbacks.
- Neovim Gradle workspace discovery now prefers the enclosing settings or
  wrapper root over nested module build files, preserving the project wrapper
  and compatible build JDK in multi-module builds.
- Fat JAR packaging now merges line registries, property registries, and
  list-valued property registries deterministically. This preserves service and
  framework discovery metadata when dependencies contribute the same resource,
  including Spring Boot auto-configuration and factory registrations.
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
