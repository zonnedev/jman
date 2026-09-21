<p align="center">
  <img src="resources/icons/jman.svg" alt="JMAN logo" width="180">
</p>

# JMAN

[![CI](https://github.com/zonnedev/jman/actions/workflows/ci.yml/badge.svg)](https://github.com/zonnedev/jman/actions/workflows/ci.yml)
[![GitHub Release](https://img.shields.io/github/v/release/zonnedev/jman?include_prereleases)](https://github.com/zonnedev/jman/releases)

JMAN is an opinionated, native Java toolchain written in Rust. One executable
creates projects, resolves dependencies, selects JDKs, builds, tests, runs,
audits, publishes, and powers Java support in VS Code and Neovim.

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
| Test | Live JUnit tree, selectors, unit/integration source sets, parallel modules, coverage |
| Supply chain | Upgrade discovery, transactional updates, OSV audits, signed publishing |
| Java | Multi-vendor JDK catalog, verified installation, global/project selection, project-aware shims |
| Editors | Native Java LSP, VS Code Test Explorer, Neovim commands and CodeLens |

The [documentation home](docs/index.md) contains task-oriented tutorials,
how-to guides, and detailed reference material. The exact compatibility
boundary is recorded in the [product contract](docs/product-contract.md) and
[compatibility matrix](docs/compatibility-matrix.md).

## Platform status

The current release targets 64-bit glibc-based Linux systems. The Java language
server requires Java 25 GraalVM for its compiler frontend; builds may target
and run on supported project JDKs independently. Windows, macOS, ARM, and
Alpine Linux are not supported yet.

## Contributing

Use `make gates` for the complete local acceptance suite and `make ci` for the
portable CI suite. Maintainer workflows live in the
[release guide](docs/releasing.md); user documentation intentionally keeps
release engineering separate from everyday JMAN usage.

To preview documentation locally, install
[uv](https://docs.astral.sh/uv/getting-started/installation/), then run
`make docs` for a strict, pinned production build or `make serve-docs` for live
review. The Make targets provision the core MkDocs version in
`docs/requirements.txt` without relying on globally installed themes.

JMAN is licensed under the [Apache License 2.0](LICENSE).
