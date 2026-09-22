# Manage Java toolchains

JMAN discovers, verifies, installs, removes, and pins JDK distributions. Its
internal catalog model is provider-neutral; the current provider is the Foojay
Disco API, with distribution-specific download-host allowlists enforced before
installation.

## Browse available JDKs

```bash
jman java list
jman java list --major 21
jman java list --lts
jman java list --vendor zulu
jman java list --major 21 --vendor temurin --all
```

The default table combines local and remote releases, marks installed entries,
and shows only the latest matching release per vendor. `--all` expands every
matching remote release. Filters compose.

To show only installed JDKs, with no network access:

```bash
jman java list --installed
```

For scripts and editor integrations:

```bash
jman java list --format json
```

JSON retains the complete matching catalog even when the human table would be
compact.

## Choose the user-wide Java

```bash
jman java install 21 --global
jman java which
jman java setup
```

Temurin is the default vendor. Installation resolves the platform archive and
SHA-256 digest from the provider, requires HTTPS, validates every initial and
redirected host, verifies the downloaded bytes, and only then makes the JDK
available. `--global` also selects the exact installed release as the current
user's default. The selection is durable: catalog and artifact cache cleanup
does not delete installed JDKs.

`jman java setup` creates project-aware shims for the commands supplied by that
JDK and prints the one line to add to Bash, Zsh, or Fish. For example:

```bash
eval "$(jman shell init zsh)"
```

These commands have separate responsibilities: `java setup` creates links in
the JMAN data directory, while `shell init` writes shell code to standard
output. That code activates the shims, synchronizes `JAVA_HOME`, and registers
JMAN command completion generated from the current CLI definition. It completes
subcommands, flags, and fixed choices for Bash, Zsh, and Fish. Evaluating
`shell init` cannot create missing shims. Run `java setup` once before enabling
the shell integration and again if a newly selected JDK provides additional
commands. In an existing Zsh session, run `rehash` after the first setup so Zsh
discards cached command locations.

After shell initialization, `java`, `javac`, `jar`, and the other JDK commands
follow JMAN's effective selection. `JAVA_HOME` is refreshed when the working
directory changes.

Verify all layers of the selection:

```bash
jman java which
echo "$JAVA_HOME"
command -v java
java --version
```

`jman java which` shows the JDK JMAN intends to use. `JAVA_HOME` should be that
JDK's directory, and `command -v java` should resolve to
`~/.local/share/jman/shims/java` by default. The final command confirms what is
actually executed.

To switch to another JDK that is already installed:

```bash
jman java use 21 --vendor corretto --global
```

`java use` never downloads. This keeps selection predictable; use `java
install ... --global` when one command should both install and select.

## Override Java for one project

From a JMAN project:

```bash
jman java install 25 --vendor zulu
jman java use 25 --vendor zulu
jman java which
```

Without `--global`, `java use` writes the version and vendor to the nearest
project's `jman.toml`. A project selection wins over the user-wide selection.
Nested directories discover the nearest manifest automatically, so the same
selection is used by JMAN builds, shell shims, and editor processes.

Consequently, changing the global selection does not override a project pin.
To change the current project, omit `--global`:

```bash
jman java use 25 --vendor zulu
```

To return a project to the global fallback, remove its `[toolchain]` section
from `jman.toml`. There is no hidden shell-level override in between these two
selection scopes.

`jman java which` explains the effective JDK by default, including its exact
version, installation path, and whether it came from `jman.toml` or the global
configuration. Machine-oriented forms are also available:

```bash
jman java which --format json
jman java which --format home
jman java which --format shell
```

Run an individual command with the effective JDK without changing the parent
shell:

```bash
jman java exec -- java -version
jman java exec 27 --vendor zulu -- java -version
```

## Remove installations safely

Preview before removal:

```bash
jman java remove 21 --dry-run
jman java remove 21 --vendor corretto
```

Use `--all` to remove all matching installations and `--force` only to override
a project-pin safety check. A globally selected JDK cannot be removed, even
with `--force`; select another global JDK first. `--path` selects the project
context used by the operation.

## Catalog caching and refresh

The remote catalog is cached beneath `$JMAN_CACHE_DIR/catalog/` for 15 minutes.
Expired entries are conditionally revalidated with an HTTP ETag or
`Last-Modified`. If the provider is unavailable, JMAN reports the failure and
falls back to the last valid cache entry.

```bash
jman java list --refresh
```

`--refresh` bypasses the freshness window but still permits conditional HTTP
revalidation. `--installed` is the strict no-network alternative and composes
with `--major`, `--lts`, `--vendor`, and `--format`.

## Build-tool JDK versus project JDK

For Maven and Gradle workspaces opened through JMAN Java, the JDK that runs the
build tool can differ from the project's compiler toolchain. This matters when,
for example, Gradle 8.7 cannot run on Java 25. JMAN selects a compatible
installed LTS runtime or accepts an explicit editor `buildJavaHome`; Gradle or
Maven still owns the project's language target.

The global JMAN selection is considered before automatically discovered
`JAVA_HOME` or `PATH` candidates, provided it is compatible with the build
tool. Explicit editor `buildJavaHome` and Gradle daemon criteria still win.
JMAN never installs a JDK implicitly for editor import. Install the suggested
runtime explicitly, then synchronize the workspace again.
