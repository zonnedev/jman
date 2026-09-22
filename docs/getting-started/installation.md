# Install JMAN

JMAN releases are self-contained archives for 64-bit glibc-based Linux. Keep
the archive together: it contains the `jman` executable, the native compiler
frontend, Java workers, the Maven/Gradle model importers, JaCoCo, and
Vineflower.

## Install a release

1. Download the Linux x64 archive and its `SHA256SUMS` file from the
   [latest GitHub release](https://github.com/zonnedev/jman/releases/latest).
2. Verify the download from the directory containing both files:

   ```bash
   sha256sum --check SHA256SUMS
   ```

3. Extract it into a versioned directory. Replace `<version>` and `<archive>`
   with the downloaded release:

   ```bash
   mkdir -p "$HOME/.local/share/jman/<version>"
   tar -xzf <archive> -C "$HOME/.local/share/jman/<version>" --strip-components=1
   mkdir -p "$HOME/.local/bin"
   ln -sfn "$HOME/.local/share/jman/<version>/jman" "$HOME/.local/bin/jman"
   ```

4. Ensure `~/.local/bin` is on `PATH`, then verify the installation:

   ```bash
   jman --version
   ```

Do not move only the `jman` executable out of the extracted directory. JMAN
locates its bundled runtime files relative to that executable.

## Requirements

- Linux on an x86-64 processor with glibc.
- A supported JDK for project compilation. JMAN can install and select one with
  `jman java install --global`.
- Java 25 GraalVM when using the Java language server. This runtime is separate
  from the JDK used to compile a project.
- Network access for uncached Maven artifacts, the JDK catalog, vulnerability
  data, or publication. Locked, cached builds can run offline.

## Make JMAN your Java manager

List the compact multi-vendor catalog, install Java 21 as your user-wide
default, and create project-aware command shims:

```bash
jman java list --lts
jman java install 21 --global
jman java setup
eval "$(jman shell init zsh)" # use bash or fish when appropriate
jman java which
jman doctor
```

`jman java setup` creates the command shims. `jman shell init` only prints the
shell code that adds those shims to `PATH`, updates `JAVA_HOME`, and enables
JMAN command and option completion; evaluating it does not create any files.
Completions come from the same command model as `jman --help`, so installed
releases automatically expose their current commands, flags, and fixed-value
choices. Add the `eval` command near the end of the matching shell startup
file. When creating the shims for the first time in an already-running Zsh
session, run `rehash`.

Verify the complete setup rather than relying only on the prompt's Java icon:

```bash
jman java which
echo "$JAVA_HOME"
command -v java
java --version
```

`command -v java` should report `~/.local/share/jman/shims/java` unless
`JMAN_DATA_DIR` changes the data location. Inside a project, `jman java which`
may report a project selection instead of the global default; project
configuration intentionally has higher precedence.

Temurin is the default distribution. Pass `--vendor <name>` consistently to
install and select another distribution, for example `zulu` or `corretto`.
See [Java toolchains](../guides/java-toolchains.md) for catalog filters,
verification, cache behavior, project overrides, and shell integration.

## Upgrade or remove JMAN

Install each release into a new versioned directory and repoint the
`~/.local/bin/jman` symlink. This makes rollback explicit and leaves no partial
in-place upgrade. After choosing the version you want to keep, remove older
version directories manually.

JMAN's artifact and catalog cache is independent of the installation. Its
default location is `~/.cache/jman`; see [storage](../reference/storage.md).
