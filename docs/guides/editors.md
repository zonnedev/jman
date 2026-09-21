# Use JMAN in an editor

JMAN Java is the same native language server in VS Code and Neovim. It imports
native `jman.toml`, Maven, and Gradle workspaces, then combines project-aware
classpath data with a GraalVM-powered compiler frontend.

## Supported language features

- Diagnostics, completion, hover, signature help, document/workspace symbols,
  semantic highlighting, and inlay hints.
- Go to declaration/definition, implementations, references, and call-aware
  navigation across source and dependency symbols.
- Rename, conservative change-signature, organize imports, and code actions.
- Annotation processing as a normal project-model concern, including Lombok
  generated members when Lombok is on the processor path.
- Decompiled dependency source fallback when no attached sources exist, while
  workspace declarations always prefer their local source files.
- JUnit CodeLens, test discovery and selection, live execution, reruns, and
  coverage for native JMAN projects.

## VS Code

1. Install **JMAN Java** from the Visual Studio Marketplace.
2. Open a trusted folder containing `jman.toml`, `pom.xml`, `build.gradle`, or
   `build.gradle.kts`.
3. Configure the Java 25 GraalVM used by the frontend:

   ```json
   {
     "jman.java.javaHome": "/path/to/graalvm-jdk-25",
     "jman.java.buildJavaHome": "",
     "jman.java.buildSystem": "auto",
     "jman.java.buildSync": "prompt"
   }
   ```

4. Open a Java file and run **JMAN Java: Show Status**.

Use VS Code's Test Explorer for individual tests and coverage, or commands such
as **Sync Workspace**, **Check Project**, **Build Project**, **Run Application**,
**Rebuild Project Index**, and **Clear Workspace Cache**.

The complete extension command/setting table is in its
[Marketplace README](https://github.com/zonnedev/jman/tree/main/editors/vscode).

## Neovim 0.11+

Install the repository with your plugin manager and configure the executable:

```lua
{
  "zonnedev/jman",
  ft = "java",
  opts = {
    cmd = "/path/to/jman",
    build_sync = "prompt",
  },
}
```

Then run `:checkhealth jman` and `:JmanStatus`. The plugin exposes commands for
sync, check, build, run, tests, coverage, code actions, imports, index rebuild,
cache clearing, and server restart. Standard LSP mappings continue to handle
definitions, references, symbols, hover, and rename.

See the [Neovim plugin guide](https://github.com/zonnedev/jman/tree/main/editors/neovim)
or `:help jman.nvim` for all options, commands, keymaps, and statusline support.

## Project detection and synchronization

In `auto` mode the clients prefer `jman.toml`, then Gradle, then Maven. Choose a
build system explicitly when a repository contains multiple models.

Synchronization modes are:

- `manual`: model changes wait for an explicit sync command.
- `prompt`: the editor asks before reimporting.
- `automatic`: relevant model changes trigger synchronization.

Maven/Gradle import prefers project wrappers. A separate compatible build JDK
can run an older wrapper while the compiler frontend remains on Java 25
GraalVM. Inspect the chosen runtime in status output.

## Important boundaries

Disable other Java language servers in the workspace to avoid duplicate
diagnostics and competing edits. Maven and Gradle import obtain their real
models from the build tools but do not execute arbitrary plugins inside the
language-server process. Generated/reflection-only relationships can still be
outside conservative rename and reference analysis.

The extension sends no analytics or telemetry. It can access artifact
repositories, wrapper distributions, and configured toolchain services when a
requested operation requires them.
