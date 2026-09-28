# Command-line reference

The synopsis is `jman [global options] <command> [command options]`. Paths
default to the current directory unless stated otherwise.

## Global options

| Option | Effect |
| --- | --- |
| `--quiet` | Suppress status and progress output. |
| `-v`, `--verbose` | Show additional detail; repeat for greater verbosity. |
| `--no-progress` | Disable animation while retaining plain status messages. |
| `-h`, `--help` | Show contextual help. |
| `-V`, `--version` | Show the JMAN version. |

## jman init

Create a project or import Maven metadata.

```text
jman init [PATH] [--import] [--lib] [--group GROUP] [--java RELEASE]
          [--version VERSION] [--modules A,B] [--main-class CLASS]
```

- `--import` imports a detected Maven project without prompting.
- `--lib` creates a reusable library rather than an application.
- `--group` defaults to `com.example`; `--java` defaults to `21`;
  `--version` defaults to `0.1.0-SNAPSHOT`.
- `--modules` creates a workspace root and comma-separated child modules.
- `--main-class` overrides the generated `<group>.<name>.Application`.

```bash
jman init inventory --group dev.example --java 21 \
  --main-class dev.example.inventory.Application
jman init domain-model --lib --group dev.example --version 1.0.0
jman init platform --modules api,application
```

## jman sync

Resolve declarations, prepare classpaths, and atomically regenerate lockfiles.

```text
jman sync [PATH] [--report human|json] [--refresh] [--offline]
```

`--refresh` reconstructs effective models and lockfiles even when current.
`--offline` uses only the local dependency cache.

```bash
jman sync --refresh
jman sync services/orders --report json
```

## jman add

Add a direct dependency and synchronize.

```text
jman add GROUP:ARTIFACT@VERSION
         [--path PATH] [--scope compile|runtime|provided|test|processor]
         [--offline]
```

The default scope is `compile`. Processor scope writes to
`[annotation-processors]` rather than a runtime dependency section.

```bash
jman add com.fasterxml.jackson.core:jackson-databind@2.17.2
jman add org.projectlombok:lombok@1.18.34 --scope processor
jman add org.junit.jupiter:junit-jupiter@5.10.2 --scope test --path modules/api
```

## jman remove

Remove a direct declaration and synchronize.

```text
jman remove GROUP:ARTIFACT [--path PATH] [--offline]
```

```bash
jman remove org.junit.jupiter:junit-jupiter --path modules/api
```

## jman outdated

Report newer versions for direct repository dependencies without changing
files.

```text
jman outdated [PATH] [--offline] [--include-prerelease]
              [--format human|json]
```

Stable versions are preferred unless `--include-prerelease` is present.

```bash
jman outdated
jman outdated --include-prerelease --format json
```

## jman update

Update one or every direct repository dependency and synchronize the workspace
transactionally.

```text
jman update [GROUP:ARTIFACT] [--path PATH]
            [--level patch|minor|major|latest] [--offline]
            [--include-prerelease] [--dry-run] [--format human|json]
```

The default level is `patch`. `--dry-run` prints the plan without editing
manifests or lockfiles.

```bash
jman update --dry-run
jman update com.fasterxml.jackson.core:jackson-databind --level minor
```

## jman audit

Audit the exact locked graph against known vulnerabilities.

```text
jman audit [PATH] [--offline] [--refresh]
           [--severity unknown|low|medium|high|critical]
           [--deny unknown|low|medium|high|critical]
           [--format human|json]
```

`--severity` filters displayed findings. `--deny` adds a failure threshold for
active findings. `--offline` requires a cached result for the exact graph;
`--refresh` bypasses a still-fresh audit cache entry.

```bash
jman audit --severity medium
jman audit --offline --deny high
```

## jman tree

Render the complete resolved dependency hierarchy.

```text
jman tree [PATH]
```

```bash
jman tree
jman tree services/orders
```

## jman why

Render every shortest path that explains why an artifact is present.

```text
jman why GROUP:ARTIFACT [--path PATH]
```

```bash
jman why com.fasterxml.jackson.core:jackson-core
```

## jman check

Type-check and compile production Java sources.

```text
jman check [PATH] [--jobs COUNT] [--offline]
```

`--offline` prevents download of a missing managed JDK. Dependencies must
already match the lockfile and cache.

```bash
jman check --jobs 4
jman check modules/api --offline
```

## jman fmt

Format Java source with the project-aware JMAN Java Formatting Style (JJFS 1)
formatter.

```text
jman fmt [PATH] [--check]
```

`PATH` may be a JMAN, Maven, or Gradle project directory, a source subtree, or
one `.java` file. The default is the current directory. `--check` reports files
that differ and exits unsuccessfully without writing. See the
[formatting guide](../guides/formatting.md) for usage and the
[JJFS 1 specification](jjfs-v1.md) for the canonical style and safety contract.

```bash
jman fmt
jman fmt --check src/main/java
jman fmt src/main/java/dev/example/Application.java
```

## jman build

Compile and package modules.

```text
jman build [PATH] [--jobs COUNT] [--offline] [--rebuild]
           [--fat] [--sources] [--javadoc] [--all]
```

The default produces thin JARs. Optional flags add executable fat, source, and
Javadoc JARs; `--all` enables all three.
`--rebuild` forces compilation and repackaging for this invocation without
deleting caches, redownloading dependencies, or reinstalling the JDK.

```bash
jman build --fat
jman build --rebuild --all
```

## jman publish

Stage Maven-compatible publications and deliver them locally or remotely.

```text
jman publish [PATH] [--to local|repository|central]
             [--repository-url URL] [--local-repository PATH]
             [--jobs COUNT] [--offline] [--dry-run] [--sign]
             [--gpg-key ID] [--allow-dirty] [--allow-insecure]
             [--automatic] [--timeout-seconds SECONDS]
             [--format human|json]
```

- `local` (default) installs into `~/.m2/repository` or `--local-repository`.
- `repository` requires `--repository-url` and accepts generic repository
  credentials from the environment.
- `central` validates required metadata, signing, and Central credentials.
- `--dry-run` performs build, validation, staging, signing, and bundle creation
  without delivery.
- `--sign` signs generic/local publications; Central always signs.
- Remote publishing requires a clean worktree unless `--allow-dirty` is set.
- HTTPS is required unless `--allow-insecure` explicitly permits HTTP for a
  development repository.
- `--automatic` asks Central to publish after validation. The wait defaults to
  300 seconds and is controlled by `--timeout-seconds`.

```bash
jman publish --dry-run
jman publish --local-repository /tmp/jman-publication-test
jman publish --to central --automatic --timeout-seconds 900
```

## jman run

Compile and run the configured application.

```text
jman run [PATH] [--jobs COUNT] [--offline] [-- APPLICATION_ARGUMENTS...]
```

Arguments after `--` are passed unchanged to the Java main method.

```bash
jman run
jman run services/web -- --server.port=8081 --spring.profiles.active=local
```

## jman test

Compile and run JUnit Platform tests.

```text
jman test [PATH] [--jobs COUNT] [--offline]
          [--module NAME]... [--tests SELECTOR]...
          [--source-set all|unit|integration] [--debug]
          [--coverage] [--coverage-format summary|json|xml|html]...
          [--coverage-output PATH]
          [--coverage-min-line PERCENT] [--coverage-min-branch PERCENT]
          [--coverage-include PATTERN]... [--coverage-exclude PATTERN]...
          [--report human|json] [-- RUNNER_ARGUMENTS...]
```

`--debug` starts each test JVM suspended on an ephemeral JDWP port. Module,
test, format, include, and exclude options may be repeated. The default source
set and report are `all` and `human`.

```bash
jman test --source-set unit --module application
jman test --tests 'dev.example.OrderServiceTest#createsOrder'
jman test --coverage --coverage-format html --coverage-min-line 80
```

## jman doctor

Diagnose the selected JDK, project model, and optional container runtime.

```text
jman doctor [PATH]
```

Run this first when a build behaves differently between machines.

```bash
jman doctor
jman doctor services/orders
```

## jman java install

```text
jman java install VERSION [--vendor VENDOR] [--offline]
                          [--path PATH | --global]
```

`VERSION` can be a feature release such as `21` or an exact version such as
`25.0.1+8`. Temurin is the default download vendor. Offline mode selects only
an already installed match from that vendor. By default, the command also
selects the JDK in the nearest `jman.toml`, creating a toolchain-only manifest
in the current directory when needed. `--path` changes that directory context.
`--global` instead selects the exact installed release as the current user's
default. JMAN automatically selects the current operating system and processor
architecture; cross-platform JDK installation is intentionally not exposed.

```bash
jman java install 21 --global
jman java install 25 --vendor zulu
```

## jman java list

```text
jman java list [--installed] [--all] [--major RELEASE] [--lts]
               [--vendor VENDOR] [--refresh] [--format human|json]
```

The default merges installed and available releases into a compact table.
`--installed` filters the result to JDKs installed by JMAN and never contacts
the remote catalog. `--all` disables latest-per-vendor
compaction. `--refresh` revalidates even a fresh cache entry. Local and remote
results are limited to the current operating system and architecture.

```bash
jman java list --lts
jman java list --major 25 --vendor zulu --all
jman java list --installed --format json
```

## jman java remove

```text
jman java remove VERSION [--vendor VENDOR] [--all] [--dry-run]
                         [--force] [--path PATH]
```

`--all` removes every installed match for the requested major. Pin-safety is
checked against the nearest `jman.toml` at `--path`; `--force` overrides a
directory or project pin. A globally selected JDK must be replaced before it
can be removed. Without `--vendor`, JMAN uses a matching local selection, then
a matching global selection, then a unique installed vendor; ambiguous matches
require an explicit vendor.

```bash
jman java remove 21 --dry-run
jman java remove 21 --vendor corretto
```

## jman java use

```text
jman java use VERSION [--vendor VENDOR] [--path PATH | --global]
```

Select a JDK that is already installed. The default writes the version/vendor
into the nearest `jman.toml`, creating a toolchain-only manifest at `--path`
when needed. `--global` instead records the exact installed release as the
current user's default. When `--vendor` is omitted, JMAN uses a matching
directory or project selection, then a matching global selection, then a
unique installed vendor. Ambiguous matches require an explicit vendor. This
command never downloads a JDK.

```bash
jman java use 25 --vendor zulu
jman java use 21 --global
```

## jman java which

```text
jman java which [PATH] [--format human|json|home|shell]
```

Print the effective managed JDK and its selection source. The nearest
directory or project selection wins over the global selection. `home` prints
only the JDK directory;
`shell` prints a POSIX `JAVA_HOME` export.

```bash
jman java which
jman java which --format home
```

## jman java exec

```text
jman java exec [VERSION] [--vendor VENDOR] [--path PATH] -- COMMAND [ARGUMENTS...]
```

Execute a command with the effective JDK's `JAVA_HOME` and `bin` directory.
Supplying `VERSION` selects a matching installed JDK for this invocation only.
Without `--vendor`, JMAN uses a matching directory or project selection, then
a matching global selection, then a unique installed vendor; ambiguous matches
require an explicit vendor.

```bash
jman java exec -- java --version
jman java exec 25 --vendor zulu -- javac --version
```

## jman java setup

```text
jman java setup [--shell bash|zsh|fish]
```

Create project-aware shims for every command supplied by the globally selected
JDK, refresh the `current` link, and print the shell activation instruction.
A global selection must already exist. Run this before enabling shell
integration; `shell init` does not create shims.

```bash
jman java setup --shell zsh
```

## jman shell init

```text
jman shell init bash|zsh|fish
```

Print shell code that prepends JMAN's shims and keeps `JAVA_HOME` synchronized
with the effective project-or-global selection. The generated Bash, Zsh, or
Fish code also registers completion for JMAN commands, options, and fixed-value
arguments directly from the current CLI definition. Evaluate it from the
matching shell startup file; the command does not edit that file itself and
does not create the shims. In Zsh, run `rehash` after the first `java setup` in
an already-running session.

```bash
eval "$(jman shell init zsh)"
```

## jman lsp

```text
jman lsp [--stdio]
```

Start the bundled Java language server over standard input/output. `--stdio` is
the default and is accepted explicitly for editor clients. The protocol stream
owns stdout; clients should display server diagnostics from stderr.

```bash
jman lsp --stdio
```
