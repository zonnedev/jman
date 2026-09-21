# JMAN Java language server

JMAN includes a Rust Java language server backed by an Oracle GraalVM Native
Image library containing the OpenJDK javac frontend. Java bridge code lives
under the `io.github.zonnedev.jman.javac` namespace.

The language server is embedded in the `jman` executable and is started over
stdio with:

```bash
jman lsp
```

The server detects JMAN, Gradle, and Maven workspaces from the LSP `rootUri`.
Automatic selection always prefers `jman.toml`; an explicit
`initializationOptions.buildSystem` value of `jman`, `gradle`, or `maven`
overrides automatic selection. Changes to `jman.toml`, `jman.lock`, POM, Gradle
settings/build files, properties, or wrapper properties mark the last
successfully imported model as stale. Clients choose
`manual`, `prompt`, or `automatic` synchronization with
`initializationOptions.buildSync`; the default VS Code behavior prompts before
executing build logic. Failed synchronization preserves the last working
model, sessions, indexes, and diagnostics.

The exploration currently targets Java 25. The scripts select the configured
GraalVM directly, so a fresh shell can run the complete gate:

```bash
make gates
```

The native ABI is deliberately small and versioned. Java compiler objects
never cross the FFI boundary. Rust owns isolate lifecycle and copies each
length-prefixed result before asking the native library to free it.

## Verified exploration surface

- OpenJDK `javac` parsing and diagnostics inside a GraalVM Java 25 shared
  library, called through a versioned C ABI from Rust.
- Project-aware javac attribution through a typed Rust API. Requests carry the
  edited source, file name, Java release, dependency classpath, and source
  roots; results contain declarations, references, qualified symbol names,
  source ranges, and diagnostics.
- Lifetime-bound project sessions reuse javac file-manager state and keep a
  bounded cache of the latest attributed document versions. Repeated LSP
  features on the same buffer reuse one result; changing the in-memory source
  invalidates that document without exposing native handles unsafely.
- The Rust dependency graph indexes qualified declarations and references,
  computes transitive affected-document sets for updates and removals, and
  evicts only those native session entries. Unrelated cached documents remain
  available.
- An incremental workspace symbol index supports definition lookup, reference
  lookup, ranked workspace-symbol search, and document replacement/removal.
- Project caches use a canonical workspace/build identity under
  `$XDG_CACHE_HOME` (or the server cache fallback). Structural, semantic,
  external-symbol, build-model, and annotation-processor state survives
  restarts; per-document fingerprints invalidate source or dependency changes.
  Process-safe lock files and atomic publication allow multiple editor
  instances to share the same project cache.
- `jman.java.status` reports cache paths, sizes, entries, hit/miss counts, and
  indexing duration. `jman.java.syncWorkspace` explicitly refreshes Maven/Gradle
  state, while `jman.java.rebuildIndex` and
  `jman.java.clearWorkspaceCache` are project-scoped commands exposed by the VS
  Code extension. Build reloads retain sessions and editor-query entries for
  compile units whose normalized model did not change.
- Rename preparation uses javac-resolved identities and exact identifier
  ranges, validates the replacement as a non-keyword Java identifier, requires
  one indexed definition, deduplicates edits, rejects same-kind declaration
  conflicts, and indexes package declarations for package-segment renames.
- Javac-derived override families connect interface/super methods, concrete
  overrides, calls, and method references. Rename follows the complete family;
  the conservative change-signature command rewrites every declaration and
  call only when parameter types prove that a rename/reorder is safe.
- Source actions generate implement-method stubs, constructors, accessors,
  `equals`, `hashCode`, and `toString`; generated member suites are compiled
  with javac in tests. Unsupported declarations are excluded instead of
  receiving guessed edits.
- Exception diagnostics offer `throws` and statement-level `try/catch`
  rewrites. Import organization sorts and deduplicates imports and removes
  only imports whose simple name is provably unused.
- Editor attribution uses javac access checks and `Types.asMemberOf`, preserving
  inherited generic substitutions, covariant returns, protected/package/private
  visibility, erased overload identities, boxing, varargs, lambdas, and method
  references. Editor queries also receive the compile unit's real compiler and
  JPMS override options.
- Repeated isolate creation and teardown, malformed-source recovery,
  independent concurrent isolates, and a warm parse latency budget.
- Gradle `JavaCompile` models with source files, classpaths, compiler
  arguments, toolchains, output directories, and annotation-processor paths.
- Build Model v2 normalizes source roots, multiple generated-source roots,
  module paths, reactor/project dependency edges, and annotation-processor
  options while the Rust reader remains compatible with v1 model output.
- JPMS-aware sessions keep class paths and module paths separate, preserve
  module compiler flags, enforce named-module readability and exports, and use
  isolated overlays for unsaved modular sources and module descriptors.
- `module-info.java` editing supports workspace-module navigation and
  completion for `requires`, qualified `exports`/`opens`, `uses`, and
  `provides`. Javac readability/export diagnostics offer edits for `requires`,
  `requires transitive`, `exports`, and `opens`; unsafe classpath-to-module-path
  build changes are surfaced as explicit guidance rather than guessed edits.
- JPMS correctness tests cover automatic and explicit modules, transitive
  readability, qualified exports/opens, services and providers,
  `--patch-module`, `--add-reads`, `--add-exports`, split packages, and
  duplicate module-path entries.
- Multi-release JARs honor `Multi-Release: true` and select the highest
  `META-INF/versions/N` class, module descriptor, attached source, and
  decompiler input not newer than the owning compile unit's Java release.
  Version-selected class bytes also make decompiler caches release-safe.
- The executable [compatibility matrix](compatibility-matrix.md) imports
  modular Maven and Gradle projects across JDK 17, 21, and 25.
- Gradle dependency resolution is isolated per compile unit. Broken optional
  source sets produce structured `resolutionErrors` and partial models instead
  of aborting the entire workspace.
- Real Gradle JSR 269 generation: a processor is discovered, generates Java,
  and the generated source, class, model metadata, and javac-resolved generated
  symbol are verified.
- Processor compilation carries the transitive reactor compile/compile-only
  closure when missing project artifacts are substituted with sources. This is
  exercised against more than 100 processor-enabled Micronaut Core compile
  units, including Lombok and Micronaut-generated main/test classes.
- A versioned persistent JVM processor worker keeps arbitrary processors
  outside the native image. Its Rust controller fingerprints source and
  processor inputs, reuses unchanged output, reruns after source changes, and
  promotes generated output transactionally so a failed run preserves the
  last good tree. Processor failures do not abort LSP startup. Gates cover the
  custom processor, Lombok 1.18.46, and MapStruct 1.6.3.
- Processor-enabled compile units also use a persistent semantic worker on the
  compile unit's own JDK. The worker runs the build's exact processor path and
  options, then extracts diagnostics, symbols, completion, hover, and definition
  data from that same post-processing javac task. This is a generic JSR 269
  path: JMAN contains no Lombok annotation or generated-member emulation.
- Lombok regression coverage includes generated accessors, builders, and
  `@Slf4j` fields; completion and hover for generated members; navigation back
  to the local source; unrelated real errors; and unsaved document overlays.
- Native LSP initialization now runs configured processors before workspace
  indexing. Saving an owned Java source triggers fingerprinted regeneration
  and generated-symbol reindexing. The VS Code package ships the worker JAR
  beside the Rust server and discovers it without development-only paths.
- Processor fingerprints cover source contents, dependency and processor
  artifacts, processor selection/options, release, and output location. Source
  roots are rescanned for additions and removals. Successful runs replace
  stale generated output and produce isolated class output for Lombok-style
  AST augmentation; failed runs retain the last good source/class tree.
- Maven and Gradle preserve explicit `-processor` selection as well as `-A`
  options. Worker startup and execution failures remain warnings and cannot
  abort language-server startup.
- Maven effective compile models for main and test source sets, including
  reactor modules, release levels, compiler arguments, dependency classpaths,
  and annotation-processor paths.
- Real imports for Spring Petclinic and Gson. Gson is evaluated with JDK 21 so
  its Error Prone profile is active; the bridge itself remains built on Java
  25.
- Real semantic probes resolve Spring Boot's `SpringApplication` from the
  Petclinic classpath and Error Prone's `CanIgnoreReturnValue` from Gson's
  classpath. Project sibling types are resolved through source paths, including
  Gson's build-generated Java template source root, without running a full
  compile.
- A pinned Micronaut Core correctness corpus exercises 153 Gradle compile
  units, more than 3,500 Java sources, Java 25 sealed types, type-safe project
  accessors, cross-module navigation, and over 100 processor-enabled units.
  Run it explicitly with `make test-micronaut-correctness`; the network-heavy
  corpus is intentionally separate from the fast default gate.
- Cross-file probes verify Petclinic `Owner` and Gson `Gson` definitions,
  references, workspace search, and exact rename edits.
- The stdio LSP supports initialize/shutdown, incremental UTF-16 document
  synchronization, javac diagnostics, definition, references, workspace
  symbols, prepare-rename, rename, completion, hover, signature help, semantic
  tokens, and initial javac-diagnostic quick fixes. Completion and code actions
  negotiate lazy resolve properties with each client; deferred edits are
  withheld if the originating open-document version has changed. Semantic
  tokens support full, range, and delta requests with Java-specific record,
  parameter, enum-member, annotation, default-library, and write
  classifications. Completion combines
  project declarations, persistent javac-resolved external references, and
  Java 25 keywords. A lightweight catalog adds top-level JDK and dependency
  types without loading their class bodies. Selecting an unimported type adds
  the import through `additionalTextEdits`; matching missing-type diagnostics
  expose `Ctrl+.` import fixes while retaining ambiguous alternatives. A
  versioned editor-query ABI uses javac's attributed
  receiver type for `receiver.member` completion, filters inaccessible
  members, substitutes generic type arguments, and returns overload parameter
  and return types for signature help. Javadoc is extracted from project
  sources with javac and from JDK `src.zip`, Maven sibling `-sources.jar`
  files, and Gradle's content-addressed source artifacts through a cached
  `DocTrees` source index. Inline code/links and `@param`, `@return`, `@throws`,
  `@deprecated`, `@since`, and `@see` sections are rendered as LSP Markdown in
  completion, hover, and signature help. A native subprocess gate exercises
  the editor features against Petclinic through real `Content-Length` framing.
- Go-to-definition follows attributed javac symbols into attached project,
  generated, Maven, Gradle, and JDK sources. When a dependency contains only
  bytecode, the server lazily decompiles the exact owning class with pinned
  Vineflower 1.12.0, selects overloads by parameter type, and exposes a stable
  file-backed source under the external-source cache. Decompiled output is
  content-addressed by class bytes, so it is generated once and reused across
  sessions. Source archives always take precedence over decompilation.
  External files retain their originating compile-unit context in persistent
  sidecar metadata, allowing definition chains, hover, workspace references,
  and semantic highlighting to continue after the language server restarts.
  Binary-only dependencies may be loaded from both JARs and class directories.
- Attributed symbols also carry a canonical JVM identity independent of their
  human-readable qualified name. The identity includes the JPMS module,
  binary owner name, member name, erased JVM parameter/return descriptor, and
  array/primitive encoding. Overloads, constructors, nested `$` classes, and
  erased generic signatures therefore occupy distinct navigation, reference,
  rename, dependency, and decompiler-index entries.
- Environment-free subprocess gates verify automatic Petclinic initialization
  and live model reload through both its Gradle and Maven builds.
- Workspace discovery, processor execution, and indexing run behind a
  non-blocking LSP initialization handshake. The Graal isolate stays owned by
  one background thread; open-document attribution queues behind project
  readiness without moving native contexts across threads.
- A Java 25 `JavacTask.parse()` batch ABI supplies release/preview-correct
  structural facts without attribution. Rust splits cold work into bounded
  file/byte batches, uses up to four independent Graal isolates, preserves
  deterministic order, cancels stale generations, and publishes only complete
  index generations.
- A second, versioned workspace index refines the fast structural snapshot with
  javac-attributed canonical symbols for every owned source file. Entries are
  fingerprinted by source, ABI, compile model, compiler options, source paths,
  and immutable dependency artifacts; unchanged results load atomically from
  a per-workspace persistent cache while removed files disappear on the next
  published revision. Open buffers override cached locations, and definition,
  references, workspace search, dependency invalidation, and rename merge
  exact closed-file results without duplicates. On the Petclinic gate, cold
  attribution of 30 files takes roughly 0.7 seconds and a full warm restore
  roughly 0.04 seconds on the development machine.
- Semantic refinement schedules cache misses across up to four independent
  Graal isolates, each reusing one javac session per compile unit. Workspace
  publications are generation-checked and deterministic, so a newer build or
  source generation cancels stale work. Dependency invalidation compares
  canonical declaration sets: method-body/reference-only edits retain
  dependent caches, while type/member/signature changes invalidate transitive
  consumers. Four-way attribution lowers the Petclinic cold refinement gate
  from roughly 0.67 seconds to 0.37 seconds on the development machine.
- The structural index is persisted per workspace with bridge/schema,
  release, preview, path, and source-content fingerprints. Cache publication
  uses a temporary file, `fsync`, and atomic rename. Processor successes and
  stable failures are fingerprinted separately so unchanged broken optional
  processors do not impose a JVM startup cost on every editor launch.
- `JAVA_LSP_TELEMETRY=1` emits build-import, processor, parse, merge, and
  process metrics. `scripts/benchmark-indexing.sh PROJECT [gradle|maven]`
  measures elapsed time and peak RSS; `JAVA_LSP_PARSE_WORKERS` overrides the
  bounded parser-worker count for target-specific benchmarking.
- Build-model publication is validated and atomic. A failed Maven or Gradle
  refresh keeps the last-good model instead of destroying a working session.
  The VS Code status item opens **JMAN Java: Show Status**, while
  **JMAN Java: Restart Server** provides explicit recovery. Protocol
  framing rejects oversized, truncated, and conflicting messages.

## Architecture boundary

The native library is the latency-sensitive compiler frontend. It must not
load arbitrary build plugins or annotation processors: Native Image has a
closed world, while real project extensions are open-ended. Maven and Gradle
therefore remain isolated build-model workers. Annotation processing has two
isolated JVM roles: a transactional compile worker materializes generated
sources/classes for indexing, and a semantic worker obtains editor answers from
the processor-mutated javac model. Both consume the same normalized compile
unit and run on its selected compiler JDK; third-party processor code never
enters the Rust process or GraalVM native image.

Compile units without processors still use the low-latency native session.
Processor-enabled documents create fresh attributed tasks inside a persistent
per-JDK JVM when an open document version changes, preserving unsaved overlays
while ensuring processors and editor semantics share a task. The persistent
Rust structural index handles workspace-scale search and closed-file
navigation. Closed processor-enabled sources stay on the processor-neutral
structural index instead of rerunning an aggregating processor once per file;
javac attribution remains the correctness path for diagnostics and precise
open-document semantics.
Further latency work should concentrate on dependency classification beyond
direct symbol edges and batched background re-attribution.

Rename is currently intentionally conservative. It covers symbols whose
definition and references have been attributed and indexed, but does not yet
expand through method override hierarchies, record-component families,
reflection/string usages, or files that have not entered the project index.

## First VS Code trial

Build and install the development extension:

```bash
make package-vscode
code --install-extension target/vscode/jman-java-0.6.0-linux-x64.vsix
```

Set `jman.java.javaHome` to an Oracle GraalVM 25 installation. Leave
`jman.java.buildSystem` on `auto`, or select `maven`/`gradle` when the workspace
contains both. Disable other Java language servers for the workspace during
the first comparison. The **JMAN Java** output channel contains server
startup and protocol failures.

Gradle imports select a compatible installed JDK from the wrapper version,
project daemon criteria, JMAN-managed installations, and the environment. Set
`jman.java.buildJavaHome` only when an explicit build-tool
runtime is required. Test Explorer and external Gradle/Maven project commands
reuse the selected build runtime. JMAN does not download a JDK during project
import.
Annotation processors run in isolated workers using the compile unit's Gradle
toolchain when available, otherwise the selected build JDK. This keeps
compiler-coupled processors such as Lombok aligned with the project's javac
instead of JMAN's Java 25 native-frontend runtime.

Processor workers execute dependency modules before their consumers. If javac
reports unrelated source errors after processors have emitted usable classes,
JMAN retains those partial outputs in a separate cache generation without
overwriting the last complete result. Open documents are attributed by a
separate post-processing javac task, so generated Lombok members resolve without
hiding unrelated compiler errors.

Definition navigation always gives workspace source precedence over classpath
and decompiled copies. During initial background indexing, JMAN matches javac's
resolved owner and source name against the active workspace roots so local
navigation remains stable before the structural index is available.

Real-project import tests default to the read-only fixtures under
`/home/jfsanchez/zonnedev/tmp/test`. Override them with environment variables
when needed.
