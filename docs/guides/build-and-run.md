# Build and run

JMAN separates fast compilation checks, packaging, and application execution so
each command does only the work its caller needs.

## Synchronize after model changes

Run `jman sync` after manually editing dependencies, repositories, modules, or
toolchain settings:

```bash
jman sync
jman sync --report json
jman sync --refresh
jman sync --offline
```

Synchronization validates all manifests, resolves the complete graph, verifies
artifacts, and atomically updates lockfiles. `--refresh` rechecks repository
metadata. `--offline` rejects any operation that needs an uncached dependency
or managed JDK.

`jman add`, `remove`, and `update` synchronize automatically.

## Compile without packaging

```bash
jman check
jman check --jobs 4
jman check --offline
```

`check` compiles production sources for every module in dependency order. It is
the quickest command for validating the main codebase and is suitable for an
early CI gate. Annotation processors declared in `jman.toml` run as ordinary
processors; generated types participate in compilation.

## Build artifacts

```bash
jman build             # thin JARs
jman build --fat       # add executable fat JARs for applications
jman build --sources   # add source JARs
jman build --javadoc   # add Javadoc JARs
jman build --all       # all optional artifacts
jman build --rebuild   # recompile and repackage this time
```

Artifacts are deterministic and appear in each module's
`.jman/artifacts/` directory. A thin JAR contains that module's own classes and
resources. An executable fat JAR also contains runtime dependencies and writes
the configured main class into its manifest.

After the first build, JMAN reuses each artifact when its inputs are unchanged.
It checks the JAR checksum before reuse and rebuilds missing or modified JARs.
The cache tracks generated sources, Javadoc toolchain and classpath inputs, and
the ordered runtime dependencies used by fat JARs. Cache records live beside
the artifacts and do not need to be committed.
`--rebuild` bypasses compilation and packaging reuse for one build, but keeps
downloaded dependencies and installed JDKs. A later ordinary build can reuse
the freshly written outputs.

The fat-JAR merger applies generic Java archive rules, not framework-specific
patches: duplicate classes are rejected, signatures and unsafe input manifests
are removed, service descriptors and supported registry resources are merged,
and output order/timestamps are normalized.

## Declare the entry point

Set `main-class` in the application's `[project]` section:

```toml
[project]
group = "dev.example"
name = "application"
version = "1.0.0"
java-release = 21
packaging = "jar"
main-class = "dev.example.Application"
```

## Run the application

```bash
jman run
jman run -- --server.port=8081
```

Everything after `--` is passed unchanged to `main(String[] args)`. JMAN uses
compiled main output plus the resolved runtime classpath, which is why `run`
can execute a framework application even though its thin JAR alone cannot.
Use `jman build --fat` when the desired output is a standalone `java -jar`
artifact.

## Control output and concurrency

All commands accept `--quiet`, repeatable `-v`/`--verbose`, and
`--no-progress`. Build-oriented commands accept `--jobs <count>` to bound
module concurrency. Disable animated output in logs while retaining status:

```bash
jman --no-progress build --all
```

Global options may appear before or after the subcommand as shown by
`jman <command> --help`.
