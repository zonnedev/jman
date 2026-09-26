<p align="center">
  <img src="images/jman.png" alt="JMAN logo" width="160">
</p>

# JMAN Java

Native Java language support backed by JMAN's Rust language server and GraalVM
compiler frontend. JMAN Java understands JMAN, Maven, and Gradle workspaces and
keeps editor analysis aligned with the project's real build model.

> **Preview:** this release supports 64-bit glibc-based Linux systems. Windows,
> macOS, ARM, Alpine Linux, virtual workspaces, and untrusted workspaces are not
> supported yet.

## Highlights

- Java diagnostics, completion, hover, signature help, navigation, references,
  symbols, semantic highlighting, inlay hints, formatting, and code actions.
- Workspace import for `jman.toml`, Maven, and Gradle projects.
- Native VS Code Test Explorer integration for unit and integration tests,
  including a JMAN coverage profile and the Test Coverage view.
- Commands for project checks, builds, runs, tests, synchronization, indexing,
  cache maintenance, server status, and restart.
- A bundled `jman` server: no separate JMAN installation is required for normal
  extension operation.

## Requirements

- Visual Studio Code 1.95 or newer.
- A 64-bit x86 processor and Linux distribution using glibc.
- A supported project JDK for builds, tests, and annotation-processor workers.
  The native compiler frontend and its matching platform symbols are bundled.
- A trusted local or remote workspace with its project dependencies available.

JMAN may invoke a Maven or Gradle wrapper when importing those project types.
The wrapper and repositories must therefore be usable from the extension host.
For Gradle wrappers, JMAN selects the newest installed LTS JDK that the wrapper
can run on. This build-tool runtime is independent from the bundled native
compiler frontend, and JMAN never installs a JDK implicitly.

## Getting started

1. Install **JMAN Java** from the Visual Studio Marketplace.
2. Open a trusted folder containing `jman.toml`, `pom.xml`, `build.gradle`, or
   `build.gradle.kts`.
3. Open a Java source file and run **JMAN Java: Show Status** from the Command
   Palette to confirm the selected build system and index state.

The extension registers JMAN as the default Java formatter. Run **JMAN Java:
Format Document with JJFS** or VS Code's standard **Format Document** command.
To format whenever a Java file is saved, add:

```json
{
  "[java]": {
    "editor.defaultFormatter": "zonnedev.jman-java",
    "editor.formatOnSave": true
  }
}
```

Example workspace settings:

```json
{
  "jman.java.buildJavaHome": "",
  "jman.java.buildSystem": "auto",
  "jman.java.buildSync": "prompt"
}
```

## Project selection

The default `auto` mode uses JMAN when `jman.toml` is present, then detects
Gradle or Maven. Select a build system explicitly when a workspace contains
metadata for more than one build tool.

| Setting | Default | Purpose |
| --- | --- | --- |
| `jman.java.server.path` | bundled server | Override the `jman` executable used by the extension. |
| `jman.java.javaHome` | empty | Optional fallback JDK exported to Java workers; the native frontend does not use it. |
| `jman.java.buildJavaHome` | empty | Override the JDK used to run Maven or Gradle; empty lets Gradle select a compatible installed runtime. |
| `jman.java.buildSystem` | `auto` | Choose `auto`, `jman`, `gradle`, or `maven`. |
| `jman.java.buildSync` | `prompt` | Choose `manual`, `prompt`, or `automatic` synchronization. |
| `jman.java.server.extraEnv` | `{}` | Add environment variables to the language-server process. |

The selected build JDK is also used for Gradle and Maven Test Explorer runs and
for the extension's check, build, test, and run commands. This keeps build-tool
runtime selection independent from JMAN's bundled native frontend.

## Commands

| Command | Purpose |
| --- | --- |
| **JMAN Java: Show Status** | Show server, workspace, index, cache, and synchronization state. |
| **JMAN Java: Sync Workspace** | Reload the selected project model. |
| **JMAN Java: Check Project** | Compile and check the workspace. |
| **JMAN Java: Build Project** | Build reproducible project outputs. |
| **JMAN Java: Run Application** | Run the configured application. |
| **JMAN Java: Run All Tests** | Run the complete test suite. |
| **JMAN Java: Format Document with JJFS** | Apply canonical full-document JJFS formatting. |
| **JMAN Java: Change Method Signature** | Apply JMAN's conservative change-signature refactoring. |
| **JMAN Java: Rebuild Project Index** | Recreate the Java symbol index. |
| **JMAN Java: Clear Workspace Cache** | Clear cached project data and rebuild it. |
| **JMAN Java: Restart Server** | Restart the bundled language server. |

Individual tests and classes can also be run from VS Code's Test Explorer and
from test code lenses. Native `jman.toml` workspaces expose **Run with JMAN
Coverage** in Test Explorer and write HTML, XML, and JSON reports beneath
`.jman/reports/coverage/`.

## Compatibility and limitations

JMAN Java is under active development. Maven and Gradle model import
intentionally does not execute arbitrary build plugins inside the
language-server process. Rename and change-signature operations are
conservative, and some framework-generated or reflective references may not yet
be discovered.

Running multiple Java language servers for the same workspace can produce
duplicate diagnostics, competing code actions, and excess resource usage.
Disable other Java language-server extensions in the workspace while evaluating
JMAN Java.

## Privacy and network access

The extension does not collect or transmit analytics or telemetry. JMAN and the
selected build tool may access configured artifact repositories, wrapper
distributions, and the Adoptium API when a requested project or toolchain
operation requires them. Environment variables configured through
`jman.java.server.extraEnv` are passed to the JMAN process; do not place secrets
in settings that you would not normally expose to a local build tool.

## Troubleshooting

Open **View → Output → JMAN Java** for server startup and protocol diagnostics.
If startup fails, confirm the project wrapper is executable and run the
corresponding build tool from a terminal. Use
**JMAN Java: Show Status** to inspect synchronization and indexing failures.
If no installed JDK can run the Gradle wrapper, install the suggested version
with `jman java install <major>` or set `jman.java.buildJavaHome`. This setting
changes the build-tool runtime only; Gradle still owns the project's compiler
toolchain and target release.

Report reproducible problems through [GitHub Issues](https://github.com/zonnedev/jman/issues).
Include the JMAN Java version, VS Code version, Linux distribution, selected
build system, and sanitized output-channel logs. See [SUPPORT.md](SUPPORT.md)
before reporting security-sensitive information.

## License

JMAN Java is licensed under the Apache License 2.0. Bundled third-party software
retains its own license; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
