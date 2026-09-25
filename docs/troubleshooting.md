# Troubleshooting

Start with the project-aware diagnostic command and detailed plain output:

```bash
jman doctor
jman --no-progress -vv check
```

`doctor` reports the selected project/JDK and container runtime. `-vv` exposes
the operation context without an animated terminal UI.
Successful checks end with `✓`. An unavailable Docker or Podman service is
marked `!` because it is optional unless your tests need containers.

## A managed JDK is missing

Inspect the pin and installed catalog:

```bash
jman java which
jman java list --installed
```

Install the requested major/vendor explicitly, or select an existing one:

```bash
jman java install 21 --vendor temurin --global
jman java use 21 --vendor temurin --global
```

Omit `--global` from `java use` when setting a project override. `java use`
only selects an existing installation; `java install ... --global` performs
both operations.

JMAN does not install editor build runtimes implicitly.

## The shell still runs another Java

Check the complete path from configuration to the executable:

```bash
jman java which
echo "$JAVA_HOME"
command -v java
java --version
```

Then follow the first matching case:

1. If `jman java which` reports a project selection, that project's
   `[toolchain]` section overrides the global selection. Run `jman java use 25
   --vendor <vendor>` without `--global` to change that project, or remove its
   `[toolchain]` section to inherit the global selection.
2. If `command -v java` does not report
   `~/.local/share/jman/shims/java`, create the shims with `jman java setup`,
   then evaluate `jman shell init` for the current shell. `shell init` prints
   activation code; it does not create the shims.
3. In an already-running Zsh session, run `rehash` after creating the shims so
   Zsh forgets the previously cached Java path.
4. If the shim directory is present but loses precedence later, move the JMAN
   initialization near the end of the shell startup file. Any later `PATH` or
   `JAVA_HOME` assignment can override it.

For Zsh, the complete repair sequence is:

```zsh
jman java setup --shell zsh
eval "$(jman shell init zsh)"
rehash
jman java which
echo "$JAVA_HOME"
command -v java
java --version
```

When `JMAN_DATA_DIR` is configured, the expected shim path is
`$JMAN_DATA_DIR/shims/java` instead of the default path. `jman doctor` reports
selection and `PATH` disagreements after these checks.

## Gradle reports an unsupported class-file major version

The JDK running Gradle is newer than that Gradle version can parse. This is
different from the Java release targeted by the source code.

In VS Code set `jman.java.buildJavaHome`; in Neovim set `build_java_home` to a
compatible installed JDK (often Java 21), then synchronize. The JMAN
status report shows the selected Gradle version, Java version, home, and source
of that selection.

Restricted native-access warnings from older Gradle native-platform libraries
are warnings; the later build exception is the actionable failure.

## A thin JAR fails with `NoClassDefFoundError`

A thin JAR intentionally contains only project classes and resources. `jman
run` succeeds because it launches with the full resolved runtime classpath.

Build and run the standalone archive instead:

```bash
jman build --fat
java -jar .jman/artifacts/<name>-<version>-fat.jar
```

Use the exact artifact path printed by the build.

## Dependencies cannot resolve offline

Offline mode is strict. Run an online synchronization once to populate the
exact lock graph and cache:

```bash
jman sync --refresh
jman sync --offline
```

Check `JMAN_CACHE_DIR` when the online and offline commands run in different
shells or CI containers. A lockfile does not embed artifact bytes.

## Annotation-generated members are missing

Make sure the processor is declared on the processor path, not only as a
compile dependency:

```toml
[annotation-processors]
"org.projectlombok:lombok" = "1.18.34"
```

Then run `jman sync`, rebuild the editor index, and inspect the JMAN output log
for processor warnings. Processor failures retain the last good generated
tree, but missing upstream module classes or invalid processor options must be
fixed in the project model.

For Maven/Gradle workspaces, confirm the build tool itself exposes the correct
annotation processor path and generated source roots.

## Navigation opens decompiled code for a workspace class

First synchronize and rebuild the project index. Workspace source should win
over attached source and bytecode fallback:

- VS Code: **JMAN Java: Sync Workspace**, then **Rebuild Project Index**.
- Neovim: `:JmanSync`, then `:JmanRebuildIndex`.

If it persists, clear only the workspace cache and retry. Include the source
path, target symbol, selected build system, and sanitized status/output logs in
a bug report; local-source precedence has regression coverage.

## The editor starts another Java server too

Disable other Java language-server extensions for the workspace. Two servers
can publish duplicate diagnostics, race on edits, and attach overlapping test
items.

## VS Code fails to start JMAN Java

Open **View → Output → JMAN Java**, then verify:

1. The bundled/default server or `jman.java.server.path` is executable and has
   its adjacent `platform/lib/ct.sym` runtime data.
2. A compatible project/build JDK is installed.
3. The workspace is trusted and uses x86-64 glibc Linux.
4. `mvnw`/`gradlew` is executable when importing that build system.

Run **JMAN Java: Show Status** after recovery.

## Neovim does not attach

Run:

```vim
:checkhealth jman
:JmanStatus
:LspInfo
```

Confirm Neovim 0.11+, the plugin runtime path, and the configured `cmd` or
`JMAN_BIN`. A local checkout must add `editors/neovim` to `runtimepath`, not
only the repository root.

## Tests appear to hang

The active spinner remains visible during compilation and test execution. Use
plain verbose output to identify the current stage:

```bash
jman --no-progress -vv test --tests 'package.Class#method'
```

For Testcontainers failures, run `jman doctor`, check Docker/Podman endpoints,
and inspect containers left by an earlier hard kill. Ctrl-C cleanup is best
effort and cannot guarantee cleanup after SIGKILL or a host crash.

## Publication is rejected

Run the full pipeline without delivery:

```bash
jman publish --dry-run -vv
```

For Central, check that the version is not a snapshot, required `[publishing]`
metadata is complete, GPG can sign non-interactively, credentials come from
the documented environment variables, and the Git worktree is clean. Generic
repositories require HTTPS unless local-development HTTP is explicitly allowed.

## Report a reproducible issue

Open [GitHub Issues](https://github.com/zonnedev/jman/issues) with:

- `jman --version` and `jman doctor` output;
- Linux distribution/architecture and relevant JDK/build-tool versions;
- the smallest manifest/project that reproduces the behavior;
- the exact command and sanitized `-vv` or editor output logs.

Do not include repository tokens, Central credentials, GPG passphrases, private
URLs, or proprietary source code.
