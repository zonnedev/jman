# JMAN MVP Plan

This file is the shared execution checklist. Work proceeds in milestone order. Each
milestone must end with a compiling workspace, passing tests, and manually
verifiable CLI behavior.

Status: `[ ]` pending, `[/]` in progress, `[x]` completed.

Current focus: ship JMAN 0.3.0-rc.3, then expand the real-world resolver corpus
to satisfy the stable-release Maven metadata threshold.

## Milestone 0 — Architecture Agreement

- [x] Review the specification, identify risks, and define decision gates.
- [x] Re-evaluate every product and technical decision from first principles.
- [x] Resolve the `jman` product-name collision by releasing the product and
      executable as JMAN/`jman` while retaining stable data/protocol identifiers.
- [x] Define the self-contained, `uv`-inspired Java product contract and target
      users without relying on Maven or Gradle executables/libraries while
      preserving Java ecosystem conventions.
- [x] Commit to a native Rust implementation of Maven repository, effective-POM,
      BOM, scope, exclusion, and dependency-mediation semantics; do not invoke or
      embed Maven/Gradle.
- [x] Specify the exact supported Maven 3 metadata contract, validate it with
      differential graph fixtures, and explicitly exclude a Maven 4 compatibility
      claim until it has its own pinned reference matrix.
- [ ] Build a resolver conformance suite with generated graph matrices,
      adversarial fixtures, real Maven Central corpora, and differential results
      against pinned Maven reference versions.
- [x] Add a Maven-vs-JMAN differential harness that runs `jman sync`, captures
      normalized versioned dependency trees from both tools, and reports a
      deterministic unified diff.
- [x] Extend the resolver conformance harness with comparable Maven/JMAN full and
      no-change wall-clock measurements, median sampling, and speedup reporting.
- [x] Correct the performance harness to isolate Maven and JMAN caches and report
      equivalent cold-cache, warm re-resolution, and no-change scenarios.
- [x] Profile native resolution on the Micronaut fixture and record the dominant
      cold/warm costs before optimizing.
- [x] Parallelize safe repository downloads and effective-model preparation
      while preserving deterministic Maven nearest-wins traversal semantics.
- [x] Eliminate redundant POM/model/cache work identified by profiling.
- [x] Add concurrency, determinism, cache, and resolver-regression tests.
- [x] Re-run resolver conformance and comparative benchmarks against the
      Micronaut fixture; document results and remaining bottlenecks honestly.
- [x] Add a persistent, input-addressed Maven conformance cache containing the
      resolved tree and best observed timings, with `--refresh-maven` support.
- [x] Test Maven cache keys, hits, refreshes, corruption recovery, and reporting.
- [x] Move default Maven baselines under
      `.local/benchmark/<project-name>/maven-cache/` and ignore local artifacts.
- [x] Diagnose the zero-package Spring Boot microservices import and identify
      whether aggregation, profiles, inheritance, or manifest reuse is at fault.
- [ ] Require 100% agreement for the declared supported metadata contract and at
      least 99% successful, graph-identical resolution across the curated
      real-world corpus before calling the resolver Maven-compatible.
- [x] Replace the original feature bundle with a validated vertical-slice MVP.
- [x] Agree on measurable MVP success criteria and compatibility boundaries.
- [x] Record and approve the implemented Milestone 1 architecture baseline.

Decision gates:

- [x] Define Maven compatibility subset: scopes, exclusions, optional
      dependencies, parent inheritance, properties, dependency management, BOM
      imports, repositories, classifiers/types, snapshots, relocation, and
      conflict tie-breaking.
- [x] Use Tokio for HTTP and child-process orchestration; use bounded
      dependency-aware scheduling for module builds. Add Rayon only when
      CPU-bound work is demonstrated, rather than mixing two schedulers by
      default.
- [x] Keep durable metadata in a versioned, inspectable format. Reserve
      `bincode` for disposable caches, never `jman.lock`.
- [x] Define the Java toolchain contract (`JAVA_HOME`, `PATH`, version
      validation, cross-platform behavior, and reproducible selection).
- [x] Treat `jman-runner` as a small Java artifact built or embedded by the Rust
      workspace, not as a Rust crate that somehow produces a JAR.
- [x] Replace absolute container-cleanup claims with best-effort signal cleanup
      plus Testcontainers' resource reaper; document SIGKILL/crash limitations.
- [x] Define safe fat-JAR merge rules for signatures, service descriptors,
      multi-release JARs, duplicate entries, licenses, and module metadata.
- [x] Define benchmark baselines against Maven and Gradle: cold startup, warm
      no-op build, clean multi-module build, dependency resolution, and
      correctness fixtures.

## Milestone 1 — Foundations & Project Scaffolding

- [x] Make the default Rust workspace test workflow build and locate the
      embedded javac native frontend automatically.
- [x] Make development and packaged `jman` executables locate the embedded native
      frontend independently of the caller's working directory and environment.
- [x] Inspect the Micronaut Maven acceptance project and specify lossless import
      mappings for its POM, modules, dependencies, and repositories.
- [x] Implement `jman init .` Maven detection with an interactive import prompt and
      `jman init --import .` non-interactive import.
- [x] Generate a complete `jman.toml`, resolve and download imported dependencies
      natively, and atomically create `jman.lock`.
- [x] Prove import against `/home/jfsanchez/Downloads/tmp/demo/` without invoking
      Maven or Gradle and add a workspace-owned regression fixture.
- [x] Create a Rust 2021 Cargo workspace with `jman-cli`, `jman-config`,
      `jman-resolver`, and `jman-build`; reserve the Java runner for Milestone 4.
- [x] Establish shared error/reporting conventions and minimal dependencies.
- [x] Define typed manifest models for root, child, external, and path
      dependencies with validation and useful diagnostics.
- [x] Implement workspace discovery and normalized, containment-checked module
      and path-dependency resolution.
- [x] Implement the CLI command tree and functional `jman init` for single-module
      applications, libraries, and multi-module projects.
- [x] Generate deterministic `jman.toml`, Java source/resource directories,
      package paths, and `.gitignore` without overwriting existing files.
- [x] Add unit, integration, snapshot/golden, and CLI tests for parsing,
      validation, discovery, and scaffolding.
- [x] Add basic project documentation and development commands.
- [x] Verify `cargo fmt --check`, `cargo clippy --workspace --all-targets`, and
      `cargo test --workspace`.
- [x] Manually verify representative `jman init` commands in temporary
      directories and record the expected file trees.

Milestone 1 acceptance:

- `jman --help` exposes the planned command surface; unimplemented commands fail
  clearly rather than pretending to work.
- `jman init` creates valid single- and multi-module projects deterministically.
- Generated projects are rediscoverable and all manifests/path dependencies
  validate.
- Existing files are never silently overwritten.
- The complete workspace compiles cleanly and all checks pass.

## Milestone 1.5 — Native Maven Reactor & Multi-Module Import

- [x] Extend the raw/effective POM model to parse ordered `<modules>` declarations
      and preserve nested reactor structure.
- [x] Discover reactor modules from the root POM using normalized,
      containment-checked paths; reject missing POMs, duplicates, cycles, and
      paths escaping the workspace.
- [x] Resolve Maven's default and explicit `<relativePath>` parent rules from
      reactor POMs before consulting remote repositories.
- [x] Construct child effective models with local-parent inheritance,
      properties, dependency management, BOM imports, and repositories intact;
      Maven plugin configuration is detected but deliberately not translated.
- [x] Represent workspace membership explicitly in the root `jman.toml` and
      generate a deterministic child `jman.toml` for every imported module.
- [x] Model inter-module dependencies as path dependencies while retaining the
      module's Maven coordinates for publication and mediation.
- [x] Generate one deterministic `jman.lock` per module so compile, runtime, test,
      provided, and processor classpaths remain isolated rather than being
      merged into a reactor-wide mega-classpath.
- [x] Make `jman init --import` preflight every target and commit all generated
      manifests/locks atomically, leaving no partial workspace after failure.
- [x] Make `jman sync` discover the reactor root from either the root or a child
      directory and synchronize one selected module or the complete workspace.
- [x] Resolve independent module graphs and artifact downloads concurrently
      with bounded scheduling while keeping output and lockfiles deterministic.
- [x] Add module-aware structured reports containing reactor totals and
      separate dependency graphs/timings for each module.
- [x] Update the Maven comparator to capture one reference tree per reactor
      module, cache them as a single input-addressed baseline, and compare each
      module independently.
- [x] Suppress all performance ratios when any compared module fails dependency
      conformance; never present an empty or partial graph as a speed win.
- [x] Add parser, reactor-discovery, local-parent, nested-module,
      inter-module-dependency, atomicity, sync-selection, and report tests.
- [x] Add a workspace-owned Spring Boot reactor fixture modeled after
      `springboot-microservices`.
- [x] Import and synchronize
      `/home/jfsanchez/Downloads/tmp/benchmarks/springboot-microservices`
      without invoking Maven or Gradle from jman.
- [x] Require exact Maven agreement for every module in that fixture, then
      refresh and record cold, warm-resolution, and no-change benchmark results.
- [x] Verify formatting, Clippy, all workspace/Python tests, and manual root and
      child CLI workflows.

Milestone 1.5 acceptance:

- `jman init --import` converts a Maven reactor into a complete JMAN workspace with
  root and child manifests plus isolated module lockfiles.
- Local reactor parents and inter-module dependencies resolve without being
  published to or fetched from an external repository.
- `jman sync` works from the reactor root and from any child module.
- Every module's selected dependency coordinates and versions exactly match
  Maven's reference tree.
- Benchmark output reports per-module and aggregate timings only after complete
  graph conformance.
- The codebase compiles cleanly and every test passes at milestone completion.

## Milestone 1.6 — Gson Reactor Compatibility Regression

- [x] Diagnose the inherited-version failure in the Gson reactor and capture the
      exact Maven model pattern in a focused regression test.
- [x] Correct effective-model/local-parent behavior without special-casing Gson.
- [x] Require exact Maven agreement for the Gson reactor.
- [x] Re-run Micronaut and Spring Boot reactor dependency regressions.
- [x] Verify formatting, Clippy, Rust/Python tests, release build, and diff checks.

## Milestone 1.7 — Consistent CLI Progress & Output UX

- [x] Add a shared, TTY-aware presentation layer with common spinner, progress,
      success, quiet, verbose, and no-progress behavior.
- [x] Integrate phase-based progress and concise completion summaries into
      `jman init` and `jman sync`, including multi-module workspaces.
- [x] Preserve clean JSON, redirected, CI, and non-interactive output.
- [x] Add CLI/presentation tests and verify formatting, Clippy, workspace tests,
      release build, and manual terminal behavior.

## Milestone 1.8 — Narrow Maven Import Contract

- [x] Remove Maven-plugin-derived compiler, annotation-processor, and main-class
      translation from native import.
- [x] Detect Maven build/plugin-management declarations in imported POMs and emit one
      concise warning listing plugin coordinates.
- [x] Preserve dependency, BOM, parent, repository, profile, and reactor import
      behavior and update focused regression expectations.
- [x] Re-run resolver conformance fixtures and all quality gates.

## Milestone 2 — Maven Resolution & Content-Addressed Storage

- [x] Implement a correctness-preserving warm `jman sync` fast path with complete
      input fingerprints, CAS presence validation, lock-backed reports, and an
      explicit `--refresh` escape hatch.
- [x] Make `jman.toml` the authoritative post-import project model and remove the
      runtime requirement for Maven POM files.
- [x] Preserve dependency types, classifiers, optional flags, exclusions,
      effective dependency management, and BOM provenance in the manifest.
- [x] Implement native workspace synchronization and prove it works after all
      imported POM files are removed.
- [x] Implement `jman add`, `jman remove`, `jman tree`, and `jman why` with atomic
      manifest/lock updates and consistent shared CLI UX.
- [x] Implement `jman sync --offline`, explicit cache-miss diagnostics, lockfile
      integrity validation, and recoverable CAS repair.
- [x] Add relocation, snapshots, profile, BOM-precedence, exclusion, mediation,
      native-sync, offline, mutation, tree, and why regression coverage.
- [x] Re-run POM-free Micronaut, Gson, and Spring Boot acceptance workflows and
      complete all formatting, lint, test, release, and diff checks.

## Milestone 2.5 — Managed JDK Toolchains

- [x] Define separate Java release and compiler-JDK requirements in `jman.toml`.
- [x] Implement deterministic selection across project pins, jman-managed JDKs,
      `JAVA_HOME`, and `PATH`.
- [x] Implement verified, atomic Temurin JDK downloads and safe archive
      extraction for supported operating systems and architectures.
- [x] Add `jman java install`, `list`, `use`, and `which`.
- [x] List installed and remotely available Temurin releases with major/LTS
      filtering and stable human/JSON output.
- [x] Cache the platform catalog with TTL freshness, per-resource HTTP ETags,
      explicit refresh, atomic replacement, and stale-network fallback.
- [x] Make `jman doctor` and all compilation use the shared toolchain selector.
- [x] Add offline behavior, concurrent-install locking, corruption recovery, and
      actionable incompatibility diagnostics.
- [x] Add unit/integration tests and verify real Micronaut and Spring Boot
      workflows with managed toolchains.
- [x] Verify formatting, Clippy, workspace/Python tests, release build, and
      dependency-resolution regressions.
- [x] Integrate byte-accurate JDK download progress, throughput, and ETA with
      the shared TTY-aware CLI presentation layer.
- [x] Add safe managed-JDK removal with exact/major selection, `--all`,
      `--dry-run`, project-pin protection, locking, and reclaimed-space
      reporting.
- [x] Keep JDK reproducibility manifest-driven (`jdk = "17"` or an exact
      `jdk = "17.0.20+8"`) and reject a separate platform-specific toolchain
      lockfile.

## Milestone 3 — Module DAG & Incremental Java Compilation

- [x] Build and validate the module DAG, including cycle diagnostics.
- [x] Discover and validate the requested JDK toolchain.
- [x] Construct deterministic compile/runtime/test classpaths.
- [x] Invoke `javac` asynchronously with bounded parallel module scheduling.
- [x] Design a versioned incremental fingerprint containing sources, compiler
      options, toolchain identity, dependency ABI/content inputs, and resources.
- [x] Implement atomic output directories and correct no-op rebuild behavior.
- [x] Implement `jman check` and `jman build` with concise and diagnostic output.
- [x] Add multi-module, invalid-source, cache-hit, invalidation, and
      cross-platform command-construction tests.
- [x] Verify all workspace checks and manual clean/incremental builds.

Completed `jman check` vertical slice:

- [x] Add shared `jman doctor` toolchain diagnostics using `JAVA_HOME` with PATH
      fallback and requested-release validation.
- [x] Materialize compile and processor classpaths from content-addressed
      lockfile artifacts, with actionable missing-cache diagnostics.
- [x] Preserve pre-JDK-23 annotation-processing behavior for projects targeting
      older Java releases and diagnose incompatible legacy processors.
- [x] Prove whole-module cache hits, resource invalidation, atomic failure
      recovery, module-cycle rejection, and downstream invalidation.
- [x] Re-run exact Maven dependency conformance for Micronaut (65 dependencies)
      and Spring Boot microservices (776 dependencies).

`jman build` vertical slice:

- [x] Materialize annotation-generated Java sources in a dedicated atomic output
      directory and isolate explicitly declared processor classpaths.
- [x] Support native `jman add --scope processor` and removal for annotation
      processor dependencies required by frameworks such as Micronaut.
- [x] Copy main resources deterministically with defined class/resource
      precedence.
- [x] Implement `jman build` over the bounded module DAG without duplicating work
      already completed by `jman check`.
- [x] Add a real annotation-processor fixture plus generated-source, resource,
      no-op, failure-recovery, and multi-module build regressions.
- [x] Re-run Micronaut and Spring Boot compilation and dependency conformance,
      then complete all quality gates.

## Milestone 3.5 — Deterministic Thin JAR Packaging

- [x] Implement `jman package` over module-local build outputs.
- [x] Generate specification-compliant deterministic manifests with optional
      `Main-Class`.
- [x] Produce byte-identical JARs using stable paths, ordering, timestamps,
      permissions, and compression.
- [x] Write artifacts atomically under each module's `.jman/artifacts/` and
      preserve the last successful artifact on failure.
- [x] Print absolute artifact paths, sizes, and SHA-256 values in consistent CLI
      output.
- [x] Add library, application, resources, generated classes, multi-module,
      reproducibility, unsafe-path, and failure-recovery tests.
- [x] Compare representative JAR inventories with Maven and complete all quality
      gates.

## Milestone 3.6 — Unified Build Artifacts

- [x] Make `jman build` always produce the standard thin JAR and remove the
      standalone `jman package` command.
- [x] Add `jman build --sources` with deterministic original and generated source
      archives.
- [x] Add `jman build --javadoc` using the selected managed JDK and deterministic
      Javadoc archives.
- [x] Add `jman build --fat` with application validation, dependency/workspace
      merging, signature/module metadata handling, service aggregation, and
      deterministic output.
- [x] Add `jman build --all` as `--fat --sources --javadoc`.
- [x] Print every artifact's absolute path, size, SHA-256, and unchanged status.
- [x] Add single/multi-module, application/library, duplicate-entry, service,
      reproducibility, and atomic-failure regression coverage.
- [x] Implement deterministic Maven-Shade-like fat-JAR collisions: merge known
      runtime indexes, keep the first unknown resource with contributor-aware
      warnings, and reject conflicting classes.
- [x] Re-run benchmark builds and all quality gates.

## Milestone 3.7 — Native Application Execution

- [x] Treat `project.main-class` as the native application declaration.
- [x] Implement `jman run` with incremental compilation, the resolved runtime
      classpath, argument forwarding, inherited terminal I/O, and exit status.
- [x] Add application execution and CLI regression coverage.

## Milestone 4 — Isolated JUnit Testing

- [x] Add native test selection by module, class, method, and Gradle-style
      wildcard pattern without requiring raw JUnit Console arguments.
- [x] Define a versioned structured test-discovery and execution event protocol
      shared by JMAN, the LSP, and editor clients.
- [x] Integrate VS Code's Testing API with JMAN test discovery, run, rerun,
      cancellation, source locations, and streamed results.
- [x] Add VS Code commands and tasks for JMAN sync, check, build, run, and test
      operations with consistent output and cancellation; cache clearing is a
      separate explicit LSP command rather than a destructive clean task.
- [x] Expose native JMAN workspace capabilities and operation status through the
      `jman.java.*` protocol without making the LSP own child build processes.
- [x] Add Rust, Node, protocol-contract, packaging, and end-to-end regressions
      for the complete editor workflow.
- [x] Expand the external Micronaut JMAN acceptance application with three
      additional passing tests and two intentional failures for report UX.
- [x] Fail explicit `jman test --tests` selections when they match no tests while
      preserving successful unfiltered empty-project behavior.

## Milestone 5 — Neovim Integration

- [x] Build a repository-owned Neovim 0.11+ plugin using the native LSP client
      and the versioned `jman.java.*` protocol.
- [x] Support JMAN-first root detection, automatic Java attachment, build-sync
      notifications, status, restart, index rebuild, and cache clearing.
- [x] Add terminal workflows for sync, check, build, run, full tests, filtered
      tests, and nearest-test execution.
- [x] Support change-signature refactoring and expose practical default
      keymaps, commands, configuration, health checks, and statusline state.
- [x] Add headless Neovim contract tests and plugin documentation.
- [x] Install the local plugin in the user's LazyVim configuration and validate
      a clean headless startup against a real JMAN workspace.
- [x] Add local JMAN-only code-action and organize-import keybindings and verify
      the installed LazyVim configuration.
- [x] Implement and advertise `textDocument/documentSymbol`, with protocol
      regressions for Java outline/navigation clients.
- [x] Implement and advertise `textDocument/declaration` using Java declaration
      navigation semantics, including external source locations.
- [x] Add a versioned `jman.java/tests/*` discovery and execution contract,
      standard test CodeLens actions, and structured test identities.
- [x] Drive VS Code Test Explorer from LSP discovery instead of editor-side
      Java regex parsing, preserving filtered execution and cancellation.
- [x] Expose LSP-backed test discovery and selection in `jman.nvim` and document
      the editor-neutral testing workflow.

## Milestone 6 — Complete Java LSP Surface

- [x] Implement `textDocument/implementation` with override-family navigation.
- [x] Implement `textDocument/typeDefinition`.
- [x] Implement call hierarchy prepare, incoming calls, and outgoing calls.
- [x] Implement type hierarchy prepare, supertypes, and subtypes.
- [x] Implement read/write-aware `textDocument/documentHighlight`.
- [x] Implement deterministic document, range, and on-type formatting.
- [x] Implement semantic `textDocument/inlayHint`.
- [x] Implement `textDocument/selectionRange`.
- [x] Implement `textDocument/foldingRange`.
- [x] Return hierarchical document symbols.
- [x] Implement negotiated lazy completion resolve with bounded server-side
      state and stale-document edit protection.
- [x] Fix missing-import quick fixes for annotations supplied by dependency
      JARs, using the Micronaut `@Singleton` acceptance case.
- [x] Fix LSP test discovery so package-qualified methods retain their owning
      test class in VS Code and editor run selectors.
- [x] Make VS Code test-class nodes directly runnable with one class selector,
      while preserving method runs, Run All, and exclusions.
- [x] Add structured `jman test` events for editor integrations and map class-run
      results to individual VS Code test items with failure details.
- [x] Restore reliable VS Code test discovery and Run buttons after asynchronous
      workspace indexing, covering both class and individual method nodes.
- [x] Unify VS Code Test Explorer, gutter, and CodeLens execution through the
      TestController so every path runs tests and updates pass/fail state.
- [x] Introduce LSP execution providers for JMAN, Maven Wrapper, and Gradle
      Wrapper without coupling the native JMAN build system to external tools.
  - [x] Define a normalized build/test execution contract and provider selection.
  - [x] Implement JMAN, Maven, and Gradle build commands through that contract.
  - [x] Implement Maven/Gradle Test Explorer execution and normalized reports.
  - [x] Add provider regressions and package the VS Code integration.
- [x] Version VS Code provider releases distinctly so a running extension host
      cannot be mistaken for the newly installed Maven/Gradle-capable build action.
- [x] Preserve missing project-local Gradle/Maven output paths so test semantic
      sessions can substitute main sources before the project has been built.
- [x] Prefer workspace Java declarations over compiled or decompiled definitions
      when navigation sees the same symbol through test classpaths.
- [x] Resolve constructor-shaped (`<init>`) classpath definitions to their
      workspace owner type before invoking Vineflower.
- [x] Reproduce `HelloResponse` navigation through the packaged LSP and fix the
      remaining workspace-source precedence failure end to end.
- [x] Implement negotiated lazy code-action resolve with stale-document edit
      protection.
- [x] Implement semantic-token ranges, deltas, and richer Java classifications.
- [x] Support multiple LSP workspace folders and folder changes.
- [x] Apply `workspace/didChangeConfiguration` without restarting.
- [x] Implement safe workspace create, rename, and delete file operations.
- [x] Implement document and workspace pull diagnostics.
- [x] Implement request cancellation and work-done progress consistently.
- [x] Distinguish Java declaration, definition, and implementation semantics.
- [x] Resolve Javadoc links and `@see` targets through document links.
- [x] Identify generated sources and expose their origin/read-only status.

## Milestone 7 — Test Runner Hardening

- [x] Add testing protocol v2 events, exact failures, parameterized tests,
      reruns, debug descriptors, and coverage contracts.
- [x] Validate `jman test` against a real Micronaut Test application and HTTP
      endpoint, adding regressions for any runner or classpath defect discovered.
- [x] Implement the initial native `jman test` slice: compile test sources,
      construct locked test classpaths, include test resources, and execute
      JUnit Platform Console in an isolated JVM per module.
- [x] Require an explicit JUnit Platform Console test dependency rather than
      injecting an untracked runner dependency.
- [x] Forward JUnit selectors/options after `--`, preserve console output, and
      propagate module test failures.
- [x] Add a reproducible Java `jman-runner` artifact using the JUnit Platform
      Launcher protocol.
- [x] Define a versioned Rust/runner protocol and structured test events.
- [x] Implement bounded isolated JVM workers and deterministic result
      aggregation.
- [x] Allocate ephemeral ports without claiming collision-free reservation;
      document the application property contract.
- [x] Detect supported Docker/Podman endpoints without overriding explicit user
      configuration.
- [x] Implement best-effort signal cleanup and validate Testcontainers resource
      reaper behavior.
- [x] Implement `jman test` and unit/integration filtering.
- [x] Add runner protocol, isolation, cancellation, failure, and optional
      Testcontainers integration tests.
- [x] Verify all workspace/runner checks and manual test workflows.

## Milestone 8 — MVP Hardening & Release

- [x] Extend the existing `jman doctor` JDK diagnostics with container-runtime
      diagnostics.
- [x] Add actionable diagnostics, cancellation, corrupted-cache recovery, and
      offline-mode coverage.
- [x] Run compatibility fixtures and benchmark against pinned Maven/Gradle
      versions.
- [x] Document supported Maven semantics and known MVP limitations.
- [x] Produce release builds and complete the end-to-end acceptance suite.

## Deferred Post-MVP

- [ ] `jman run --watch` with fast JVM restart.
- [ ] Local Maven repository installation.
- [ ] Kernel network namespaces.
- [ ] In-memory bytecode hot swapping.
- [ ] Native OCI image building.
- [ ] Maven Central signing and publishing.
