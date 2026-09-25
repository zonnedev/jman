# One Java toolchain, from first file to release

JMAN gives a Java project one fast, consistent interface for dependencies,
builds, tests, JDKs, security checks, publishing, and editor intelligence. The
native Rust executable reads `jman.toml`, records the complete resolved graph in
`jman.lock`, and keeps command-line and editor behavior aligned.

## Choose your path

### Start a project

Install JMAN, scaffold an application, add a dependency, and run tests in
[Your first project](getting-started/first-project.md).

### Adopt JMAN in a Maven project

Convert the effective Maven reactor into a native JMAN workspace with
[Import a Maven project](tutorials/import-maven.md).

### Build and publish a library

Follow [Publish a library](tutorials/publish-library.md) from metadata and
source archives through local, repository, or Maven Central publication.

### Configure an editor

Configure the same Java language server in VS Code or Neovim with the
[editor integration guide](guides/editors.md).

## The everyday loop

```console
jman sync             # resolve declarations and update jman.lock
jman fmt --check      # verify canonical Java source formatting
jman check            # compile the workspace without packaging
jman test             # stream the JUnit result tree as tests finish
jman build --all      # create thin, fat, source, and Javadoc archives
jman run -- --help    # run the configured main class with application args
```

JMAN chooses the JDK pinned by the project, resolves a deterministic classpath,
and reuses content-addressed caches. Add `--offline` to supported commands when
the lockfile and artifacts are already cached.

## Documentation map

The documentation is organized around how you use it:

- **Getting started** explains installation, the first successful build, and
  the files JMAN owns.
- **Tutorials** walk through complete outcomes with copyable projects.
- **Guides** explain one workflow and its choices.
- **Reference** documents every command, manifest section, output location, and
  environment variable.
- **Troubleshooting** maps common symptoms to concrete checks and fixes.

For a single inventory of what is implemented today, see the
[feature catalog](reference/features.md). For exact boundaries rather than an
overview, see the [product contract](product-contract.md).

!!! note "Current platform"

    JMAN currently ships for 64-bit glibc-based Linux. The editor frontend uses
    Java 25 GraalVM; project builds can use another supported JDK.
