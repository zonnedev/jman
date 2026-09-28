# Common workflows

These recipes combine commands into complete, copyable tasks. Run them from
the workspace root unless a recipe says otherwise. Use `jman <command> --help`
when adapting an option to automation.

## Create and run an application

```bash
jman init orders \
  --group dev.example \
  --version 1.0.0 \
  --java 21 \
  --main-class dev.example.orders.Application
cd orders
jman fmt --check
jman test
jman run -- --server.port=8081
```

Arguments after `--` belong to the application, not JMAN.

## Create a reusable library

```bash
jman init customer-api --lib --group dev.example --version 1.0.0 --java 21
cd customer-api
jman check
jman build --sources --javadoc
jman publish --dry-run
```

The dry run stages a Maven-shaped repository under `.jman/publications/`
without installing or uploading it.

## Add a runtime driver or replace the generated test engine

```bash
jman add org.postgresql:postgresql@42.7.4 --scope runtime
jman add org.junit.platform:junit-platform-console-standalone@1.12.2 --scope test
jman tree
jman why org.postgresql:postgresql
```

Use `runtime` for code needed when the application starts but not while its
sources compile. `jman init` already declares JUnit Platform Console
Standalone; the second command is useful for an older project or after
removing/replacing its test engine. Both declarations are written to
`jman.toml`, resolved, and locked transactionally.

## Add Lombok as an annotation processor

```bash
jman add org.projectlombok:lombok@1.18.34 --scope processor
jman check
```

Processor scope creates an `[annotation-processors]` declaration. It does not
place Lombok on the runtime classpath or package it in the application.

## Investigate and update dependencies

```bash
jman outdated
jman update --dry-run
jman update com.fasterxml.jackson.core:jackson-databind --level minor
jman tree
```

Use the dry run for review. A real update edits matching declarations and
regenerates every affected lockfile as one transaction.

## Audit dependencies in CI

```bash
jman sync --offline
jman audit --offline --deny high
```

Prime the dependency and audit caches in an earlier online job when CI must be
strictly offline. Without `--deny`, findings are reported but do not fail the
command.

## Run one test while iterating

```bash
jman test --tests 'dev.example.orders.OrderServiceTest#createsOrder'
```

Then run the complete unit set and coverage policy before pushing:

```bash
jman test --source-set unit
jman test --coverage --coverage-min-line 80 --coverage-min-branch 70
```

## Build a standalone application

```bash
jman build --fat
java -jar .jman/artifacts/orders-1.0.0-fat.jar
```

Use the artifact path printed by JMAN instead of assuming the filename when
the project name or version differs. A thin JAR intentionally excludes runtime
dependencies.

## Force one clean build without deleting downloads

```bash
jman build --rebuild --all
```

`--rebuild` bypasses compilation and packaging reuse for this invocation. It
does not remove repository artifacts, catalog entries, or installed JDKs.

## Reproduce a build without the network

Resolve once while online:

```bash
jman sync --refresh
jman test
```

Then verify the cached path:

```bash
jman check --offline
jman test --offline
jman build --offline --all
```

An offline cache miss is an error; JMAN never silently contacts a repository.

## Select Java globally and override one project

```bash
jman java install 21 --global
jman java setup --shell zsh
eval "$(jman shell init zsh)"
jman java which
```

Inside a project that needs Java 25:

```bash
jman java install 25 --vendor zulu
jman java which
java --version
```

Installation creates or updates the nearest `jman.toml`; no separate selection
command is needed. The directory pin wins only inside that tree. Outside it,
the exact global selection remains effective.

## Run one command with another installed JDK

```bash
jman java exec 21 -- java --version
jman java exec 25 --vendor zulu -- javac --version
```

This changes neither the project manifest nor the global selection.

## Format locally and enforce the same result in CI

```bash
jman fmt
git diff --check
jman fmt --check
```

Editors call the same JJFS formatter, so command-line, VS Code, Neovim, and CI
produce one canonical result.

## A practical CI sequence

```bash
jman sync --offline
jman fmt --check
jman check --offline
jman test --offline --coverage
jman audit --offline --deny high
jman build --offline --all
```

This example assumes dependencies, the selected JDK, and the exact audit graph
were restored from trusted caches. Remove `--offline` when CI is responsible
for populating them.

## Diagnose a machine-specific failure

```bash
jman --version
jman doctor
jman java which
jman java list --installed
jman --no-progress -vv check
```

The first four commands establish build identity and toolchain selection. The
last command exposes the failing operation without animated terminal output.

Continue with the focused guides for [dependencies](dependencies.md),
[building](build-and-run.md), [testing](testing.md),
[Java toolchains](java-toolchains.md), and [publishing](../publishing.md).
