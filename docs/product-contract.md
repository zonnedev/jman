# JMAN product and compatibility contract

This document is the release contract for JMAN 0.1.0. It distinguishes behavior
that is supported and tested from behavior that merely happens to work.

## Identity and audience

The public product is **JMAN** and the command is `jman`. JMAN is the canonical
identity across the repository, distributions, editor clients, Java packages,
configuration, caches, environment variables, and protocol extensions.

JMAN targets Java application and library teams that want a fast, reproducible,
locked build without a resident daemon. Its primary workflow is importing an
existing Maven reactor once, then using a small native CLI for dependency
resolution, compilation, packaging, execution, testing, and Java editor
services. It also serves Maven and Gradle workspaces through the language
server, without coupling the native JMAN build path to either tool.

The `jman.toml`, `jman.lock`, `.jman/`, `JMAN_*`, and `jman.java.*` names are the
stable data-format, environment, and protocol identifiers for 0.1.x.

## Supported Maven metadata

JMAN supports this declared Maven model subset:

- Maven POM model 4.0.0 coordinates, packaging, properties, and repositories;
- local-reactor and repository parents, including Maven `relativePath` rules;
- dependency management and ordered BOM imports;
- compile, runtime, provided, test, and import scope propagation;
- optional dependencies, exclusions, classifiers, and artifact types;
- nearest-definition mediation with declaration-order tie breaking;
- release and timestamped snapshot artifacts with repository policies;
- repository version catalogs for read-only direct-dependency update reports;
- artifact relocation;
- active-by-default profiles plus environment property and OS activation;
- nested reactors, local parents, and inter-module dependencies.

Import asks the project Maven wrapper for its effective reactor, falling back to
system Maven and then JMAN's native importer only when the preceding executable
is absent. A selected Maven executable is authoritative: failures are reported
instead of silently changing import semantics. JMAN translates the exported
model, independently resolves its dependencies, and preserves the result in
deterministic `jman.toml` and `jman.lock` files. Normal JMAN operations do not
invoke Maven or Gradle.

The following are explicitly outside the 0.1.x contract:

- Maven build plugins, extensions, lifecycle bindings, and arbitrary goals;
- settings.xml mirrors, servers, credentials, proxies, and encrypted settings;
- every Maven profile activation form or arbitrary model-builder quirk;
- a claim of complete Maven 3.x or Maven 4.x compatibility.

JMAN publishing supports local Maven installation, generic HTTPS repositories,
and Maven Central Portal bundles for `jar` and `pom` modules. Generated
standalone POMs, source and Javadoc JARs, checksums, signatures, and transitive
workspace coordinates are covered by the cross-tool publishing acceptance
suite. Arbitrary Maven repository staging protocols and non-JAR packaging remain
outside the contract.

Unsupported build-plugin declarations are diagnosed during import instead of
being silently translated. Compiler arguments, processors, main classes, and
toolchains must be expressed in `jman.toml` after import.

## Measurable 0.1 release criteria

Every release candidate must satisfy all of the following from a clean tree:

1. `make gates` passes, including Rust, Java, native frontend, importer, real
   semantics, LSP subprocess, VS Code, and Neovim tests.
2. `cargo fmt --all -- --check` and strict workspace Clippy pass.
3. `make test-compatibility-matrix` passes against Maven 3.9.9, Gradle 8.7,
   8.14.1 and 9.1.0, and their supported JDK 17, 21, and 25 cells.
4. `cargo build --release --workspace` and `make release` succeed.
5. Declared resolver fixtures have exact normalized dependency-graph agreement;
   deterministic Java build and packaging tests are byte-for-byte stable.
6. Offline cache misses, corrupted artifacts, failed compilation, cancellation,
   and absent optional container runtimes produce actionable failures without
   destroying the last successful output.
7. `make test-publishing` proves that independent JMAN, Maven, and Gradle
   consumers can compile and run against the generated multi-module repository.

Stable-release Maven compatibility remains gated on 100% agreement for the
declared subset and at least 99% successful, graph-identical resolution across
the curated real-world corpus. Until that corpus gate is met, documentation
must say “supported Maven metadata subset,” never “Maven-compatible.”

## Versioning and compatibility

The CLI, Rust workspace, runner protocol, lockfile, and LSP extensions are
versioned independently where their compatibility needs differ. Patch releases
may add diagnostics and compatible metadata. Any incompatible manifest,
lockfile, test protocol, or LSP extension change requires an explicit version
increment and migration note.
