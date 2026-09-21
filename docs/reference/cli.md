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

## jman sync

Resolve declarations, prepare classpaths, and atomically regenerate lockfiles.

```text
jman sync [PATH] [--report human|json] [--refresh] [--offline]
```

`--refresh` reconstructs effective models and lockfiles even when current.
`--offline` uses only the local dependency cache.

## jman add

Add a direct dependency and synchronize.

```text
jman add GROUP:ARTIFACT@VERSION
         [--path PATH] [--scope compile|runtime|provided|test|processor]
         [--offline]
```

The default scope is `compile`. Processor scope writes to
`[annotation-processors]` rather than a runtime dependency section.

## jman remove

Remove a direct declaration and synchronize.

```text
jman remove GROUP:ARTIFACT [--path PATH] [--offline]
```

## jman outdated

Report newer versions for direct repository dependencies without changing
files.

```text
jman outdated [PATH] [--offline] [--include-prerelease]
              [--format human|json]
```

Stable versions are preferred unless `--include-prerelease` is present.

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

## jman tree

Render the complete resolved dependency hierarchy.

```text
jman tree [PATH]
```

## jman why

Render every shortest path that explains why an artifact is present.

```text
jman why GROUP:ARTIFACT [--path PATH]
```

## jman check

Type-check and compile production Java sources.

```text
jman check [PATH] [--jobs COUNT] [--offline]
```

`--offline` prevents download of a missing managed JDK. Dependencies must
already match the lockfile and cache.

## jman build

Compile and package modules.

```text
jman build [PATH] [--jobs COUNT] [--offline]
           [--fat] [--sources] [--javadoc] [--all]
```

The default produces thin JARs. Optional flags add executable fat, source, and
Javadoc JARs; `--all` enables all three.

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

## jman run

Compile and run the configured application.

```text
jman run [PATH] [--jobs COUNT] [--offline] [-- APPLICATION_ARGUMENTS...]
```

Arguments after `--` are passed unchanged to the Java main method.

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

## jman doctor

Diagnose the selected JDK, project model, and optional container runtime.

```text
jman doctor [PATH]
```

Run this first when a build behaves differently between machines.

## jman java install

```text
jman java install VERSION [--vendor VENDOR] [--offline]
```

`VERSION` can be a feature release such as `21` or an exact version such as
`17.0.20+8`. Vendor defaults to `temurin`. Offline mode selects only an already
installed match.

## jman java list

```text
jman java list [--local] [--all] [--major RELEASE] [--lts]
               [--vendor VENDOR] [--refresh] [--format human|json]
```

The default merges installed and available releases into a compact table.
`--local` never contacts the catalog. `--all` disables latest-per-vendor
compaction. `--refresh` revalidates even a fresh cache entry.

## jman java remove

```text
jman java remove VERSION [--vendor VENDOR] [--all] [--dry-run]
                         [--force] [--path PATH]
```

`--all` removes every installed match for the requested major. Pin-safety is
checked against `--path`; `--force` overrides it.

## jman java use

```text
jman java use VERSION [--vendor VENDOR] [--path PATH]
```

Write the selected version/vendor into the project's `[toolchain]` section.

## jman java which

```text
jman java which [PATH]
```

Print the managed JDK selected for the project.

## jman lsp

```text
jman lsp [--stdio]
```

Start the bundled Java language server over standard input/output. `--stdio` is
the default and is accepted explicitly for editor clients. The protocol stream
owns stdout; clients should display server diagnostics from stderr.
