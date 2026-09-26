<p align="center">
  <img src="resources/icons/jman.svg" alt="JMAN logo" width="180">
</p>

# JMAN

[![CI](https://github.com/zonnedev/jman/actions/workflows/ci.yml/badge.svg)](https://github.com/zonnedev/jman/actions/workflows/ci.yml)
[![GitHub Release](https://img.shields.io/github/v/release/zonnedev/jman?include_prereleases)](https://github.com/zonnedev/jman/releases)

JMAN is an opinionated, native Java toolchain written in Rust. One executable
creates projects, resolves dependencies, selects JDKs, builds, tests, runs,
formats source, audits, publishes, and powers Java support in VS Code and
Neovim.

```bash
curl -fsSL https://github.com/zonnedev/jman/releases/latest/download/install.sh | sh
```

The installer verifies the release archive, installs JMAN under the current
user's data directory, links it into `~/.local/bin`, offers to install Java 25
and create project-aware shims, then prints the exact shell initialization to
add. See the [installation guide](docs/getting-started/installation.md) for
manual installation, non-interactive options, and verification details.

```console
$ jman init hello --main-class dev.example.Application
$ cd hello
$ jman add org.junit.jupiter:junit-jupiter@5.10.2 --scope test
$ jman test
$ jman run
```

JMAN uses a readable `jman.toml` manifest and a deterministic `jman.lock`. Its
normal build path does not invoke Maven or Gradle, but existing Maven projects
can be imported and the language server understands JMAN, Maven, and Gradle
workspaces.

## Manage Java globally and per project

JMAN can be the source of truth for Java across your shell, projects, builds,
and editors. It discovers multiple vendors, verifies downloads, keeps installed
JDKs outside disposable caches, and uses project-aware command shims so `java`,
`javac`, `jar`, and the remaining JDK commands follow the effective selection.

```console
$ jman java list --lts
$ jman java install 21 --global
$ jman java setup --shell zsh
$ eval "$(jman shell init zsh)"
$ jman java which
temurin 21.0.8+9
/home/me/.local/share/jman/jdks/temurin-21.0.8_9-linux-x64
Selected by global configuration: /home/me/.config/jman/config.toml
$ java --version
```

Add the printed `eval` line near the end of your shell startup file.
`jman java setup` creates the shims; `jman shell init` only prints the shell
code that activates them, keeps `JAVA_HOME` synchronized, and enables JMAN
command and option completion.

A project can override the global default without changing the rest of the
machine:

```console
$ jman java install 27 --vendor zulu
$ jman java use 27 --vendor zulu
$ jman java which
```

Selection precedence is the nearest project's `[toolchain]` configuration,
then the exact global selection. Use `jman java list --installed` for a
network-free view of installed JDKs. Commands that operate on an installed JDK
infer its vendor from the project selection, global selection, or a unique
installed match; `--vendor` is only required when the choice is ambiguous. The
[Java toolchain guide](docs/guides/java-toolchains.md) covers vendors, shell
setup, project overrides, scoped execution, storage, and troubleshooting.

## Start here

- [Install JMAN](docs/getting-started/installation.md)
- [Build your first project](docs/getting-started/first-project.md)
- [Understand the project layout](docs/getting-started/project-layout.md)
- [Browse every feature](docs/reference/features.md)
- [Look up a command](docs/reference/cli.md)
- [Configure `jman.toml`](docs/reference/manifest.md)
- [Solve common problems](docs/troubleshooting.md)

## What JMAN covers

| Area | Capabilities |
| --- | --- |
| Projects | New applications, libraries, multi-module workspaces, Maven import |
| Dependencies | Maven repositories, BOMs, scopes, exclusions, paths, lockfiles, offline builds |
| Build | Checks, thin and executable fat JARs, source JARs, Javadocs, annotation processors |
| Format | [JJFS 1](docs/reference/jjfs-v1.md), native javac-aware formatting, canonical imports, API-first member ordering, CI checks |
| Test | Live JUnit tree, selectors, unit/integration source sets, parallel modules, coverage |
| Supply chain | Upgrade discovery, transactional updates, OSV audits, signed publishing |
| Java | Multi-vendor JDK catalog, verified installation, global/project selection, project-aware shims |
| Editors | Native Java LSP, VS Code Test Explorer, Neovim commands and CodeLens |

The [documentation home](docs/index.md) contains task-oriented tutorials,
how-to guides, and detailed reference material. The exact compatibility
boundary is recorded in the [product contract](docs/product-contract.md) and
[compatibility matrix](docs/compatibility-matrix.md).

## Platform status

The current release targets x86-64 glibc-based Linux and Apple Silicon macOS.
The Java language server bundles its native compiler frontend and matching Java
platform symbols; builds target and run on independently selected project JDKs.
Windows, Intel macOS, Linux ARM64, and Alpine Linux are not supported yet.

## Contributing

Use `make ci` for the normal push suite and `make release-gates` for the full
tag acceptance suite, including pinned real-world projects and the
Maven/Gradle/JDK compatibility matrix. The test setup bootstraps the
checksum-pinned JMAN release configured in `scripts/setup-test-jdks.sh`, then
uses JMAN itself to provision the required GraalVM Community 25 and Temurin
17/21/25 installations under `target/`; no system-wide JDK manager is required.
Maintainer workflows live in the [release guide](docs/releasing.md); user
documentation intentionally keeps release engineering separate from everyday
JMAN usage.

To preview documentation locally, install
[uv](https://docs.astral.sh/uv/getting-started/installation/), then run
`make docs` for a strict, pinned production build or `make serve-docs` for live
review. The Make targets provision the core MkDocs version in
`docs/requirements.txt` without relying on globally installed themes.

JMAN is licensed under the [Apache License 2.0](LICENSE).
