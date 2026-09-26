# Feature catalog

This page is the user-facing inventory of the current JMAN release.
“Supported” means the behavior is implemented and covered by repository tests;
links lead to usage or exact contracts.

## Project model

| Feature | Status | Where to learn it |
| --- | --- | --- |
| Application and library scaffolding | Supported | [First project](../getting-started/first-project.md) |
| Multi-module workspaces and local path dependencies | Supported | [Multi-module tutorial](../tutorials/multi-module.md) |
| Maven reactor import | Supported | [Maven import](../tutorials/import-maven.md) |
| Native Gradle-to-JMAN conversion | Not provided | Gradle is imported by the language server, not converted into `jman.toml`. |
| Deterministic manifest/lockfile schemas | Supported | [Manifest](manifest.md), [storage](storage.md) |
| Java source levels 8, 11, 17, 21, and 25 | Tested | [Compatibility matrix](../compatibility-matrix.md) |
| Preview language features | Supported when enabled by compiler arguments | [Manifest](manifest.md#build) |
| JPMS class/module paths and compiler overrides | Supported in editor/build models | [Compatibility matrix](../compatibility-matrix.md) |

Maven import prefers `mvnw`, then system Maven, then JMAN's native importer.
Normal JMAN build commands use the native project model and do not invoke Maven
or Gradle.

## Dependencies

| Feature | Status |
| --- | --- |
| Maven repositories, releases, snapshots, relocation | Supported |
| Compile, runtime, provided, test, and processor scopes | Supported |
| Parent inheritance and properties | Supported within the product contract |
| Dependency management and imported BOMs | Supported |
| Optional dependencies, exclusions, types, classifiers | Supported |
| Nearest-wins version mediation | Supported |
| Content hashes and offline resolution | Supported |
| Hierarchical `tree` and shortest-path `why` reports | Supported |
| Outdated discovery and transactional updates | Supported |
| Full arbitrary Maven 3/4 model/plugin compatibility | Not claimed |

See [dependency management](../guides/dependencies.md) and the exact
[product contract](../product-contract.md).

## Compilation and packaging

| Feature | Status |
| --- | --- |
| Incremental workspace compilation | Supported |
| Module-parallel compilation | Supported |
| Generic JSR 269 annotation processors | Supported |
| Lombok as an ordinary processor | Supported, including generated members in the LSP |
| Thin JARs | Supported |
| Executable dependency-inclusive fat JARs | Supported |
| Source and Javadoc JARs | Supported |
| Deterministic archive output | Supported |
| Main-class execution with argument passthrough | Supported |
| Native javac-aware opinionated source formatting | Supported in CLI, VS Code, and Neovim |
| Arbitrary Maven/Gradle plugin execution | Not supported in the native build |

See [Build and run](../guides/build-and-run.md) and
[format Java source](../guides/formatting.md).

## Testing and coverage

| Feature | Status |
| --- | --- |
| JUnit Platform execution | Supported |
| Unit and integration-test source sets | Supported |
| Module/class/method selectors | Supported |
| Live human-readable test tree | Supported |
| Streaming newline-delimited JSON events | Supported |
| Isolated and parallel module JVMs | Supported |
| Suspended JDWP debug workers | Supported |
| JaCoCo line, branch, and method coverage | Supported |
| Summary, JSON, XML, and HTML coverage reports | Supported |
| Include/exclude filters and failure thresholds | Supported |
| Testcontainers environment passthrough | Supported |

See [Test Java projects](../guides/testing.md) and [coverage](../coverage.md).

## Supply chain and publication

| Feature | Status |
| --- | --- |
| OSV audit of the complete locked graph | Supported |
| Cached/offline audit and explicit refresh | Supported |
| Severity display and CI denial thresholds | Supported |
| Reasoned suppressions with mandatory expiry | Supported |
| Local Maven repository publication | Supported |
| Generic HTTPS Maven repository publication | Supported |
| Maven Central Portal validation/publication | Supported |
| Deterministic POM/JAR/checksum staging | Supported |
| GPG signing and clean-worktree policy | Supported |

See [security auditing](../security-auditing.md) and
[publishing](../publishing.md).

## Java distributions

| Feature | Status |
| --- | --- |
| Local plus remote multi-vendor catalog | Supported through a provider-neutral model |
| Major, LTS, vendor, and all-release filters | Supported |
| Stable JSON catalog output | Supported |
| TTL cache, ETag/Last-Modified revalidation, stale fallback | Supported |
| Verified HTTPS downloads and redirect-host allowlists | Supported |
| Exact or major-version install/select/remove | Supported |
| User-wide exact selection and project version/vendor override | Supported |
| Project-aware `java`/`javac`/JDK command shims | Bash, Zsh, and Fish |
| JMAN command and option completion | Bash, Zsh, and Fish |
| Scoped command execution with selected `JAVA_HOME` | Supported |
| Automatic background JDK installation | Intentionally not performed |

The current catalog provider is Foojay Disco. Temurin is the default vendor,
not the only available vendor. See [Java toolchains](../guides/java-toolchains.md).

## Java language server

| Area | Supported behavior |
| --- | --- |
| Workspaces | Native JMAN, Maven, and Gradle; wrapper-aware imports and compatible build JDK selection |
| Editing | Incremental UTF-16 synchronization, diagnostics, completion, hover, signature help, semantic tokens, inlay hints, canonical full-document formatting |
| Navigation | Local/generated/dependency/JDK definitions, declarations, implementations, references, workspace/document symbols |
| Refactoring | Prepare rename, conservative rename, safe change signature, organize imports, quick fixes and generated-member actions |
| Dependencies | Attached-source preference, release-aware multi-release JARs, lazy Vineflower fallback for bytecode-only classes |
| Java modules | `module-info.java` completion/navigation and readability/export quick fixes |
| Processors | Isolated persistent JVM workers, transactional generated output, exact processor path/options, unsaved overlays |
| Tests | CodeLens, discovery, exact selectors, editor runs, reruns, and native JMAN coverage |
| Reliability | Persistent structural/semantic caches, atomic model refresh, last-good model retention, bounded parallel indexing |

VS Code and Neovim expose the same server features through their native UX.
See [editor integration](../guides/editors.md). Refactorings are deliberately
conservative: reflection, arbitrary strings, and framework-only generated
relationships may not be discoverable.

## Platform and current limits

- Release artifacts currently target x86-64 glibc Linux and Apple Silicon
  macOS.
- Windows, Intel macOS, Linux ARM64, Alpine/musl, virtual workspaces, and
  untrusted VS Code workspaces are not supported yet.
- Process cancellation is best effort across JVM/container boundaries.
- Maven/Gradle editor import requires a usable wrapper or matching system tool.
- JMAN does not claim arbitrary build-plugin emulation or perfect compatibility
  with every public Maven/Gradle project.

When an edge is important to adoption, verify it against the executable
[compatibility matrix](../compatibility-matrix.md) rather than assuming support.
