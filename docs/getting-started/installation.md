# Install JMAN

JMAN releases are self-contained archives for 64-bit glibc-based Linux. Keep
the archive together: it contains the `jman` executable, the native compiler
frontend, Java workers, the Maven/Gradle model importers, JaCoCo, and
Vineflower.

## Quick installation

Run the installer published with the latest GitHub release:

```bash
curl -fsSL https://github.com/zonnedev/jman/releases/latest/download/install.sh | sh
```

The installer:

1. detects the supported operating system and architecture;
2. resolves the latest stable release;
3. downloads the archive and verifies it against the release's
   `SHA256SUMS` entry;
4. extracts it into `~/.local/share/jman/versions/<version>` and atomically
   points `~/.local/bin/jman` at that version;
5. reuses an existing global JMAN Java selection or asks whether to install
   Temurin Java 25 globally;
6. creates project-aware Java command shims; and
7. prints the exact `PATH` and `jman shell init` lines for the detected shell.

The script does not modify `.bashrc`, `.zshrc`, or Fish configuration. Review
the printed lines and add them yourself. A subprocess cannot update the parent
shell, so restart the shell or evaluate the lines after adding them.

To inspect the installer before executing it:

```bash
curl -fsSLO https://github.com/zonnedev/jman/releases/latest/download/install.sh
less install.sh
sh install.sh
```

The one-line command is interactive when a terminal is available. In an
unattended environment, choose explicitly whether Java should also be
installed:

```bash
curl -fsSL https://github.com/zonnedev/jman/releases/latest/download/install.sh \
  | JMAN_SETUP_JAVA=1 sh
```

Supported installer controls are:

| Variable | Default | Purpose |
| --- | --- | --- |
| `JMAN_VERSION` | latest stable | Install a specific version, with or without the leading `v`. |
| `JMAN_JAVA_VERSION` | `25` | Java feature release offered during first-time setup. |
| `JMAN_SETUP_JAVA` | `auto` | Use `1` to install Java without prompting or `0` to skip Java setup. |
| `JMAN_INSTALL_ROOT` | `~/.local/share/jman/versions` | Override versioned JMAN installations. |
| `JMAN_BIN_DIR` | `~/.local/bin` | Override the directory containing the `jman` symlink. |

Re-running the installer is safe. An existing valid version is reused and the
`jman` symlink is updated. The installer refuses to overwrite a regular file at
the link location and refuses archives whose checksum or embedded version does
not match.

## Manual installation

1. Download the Linux x64 archive and its `SHA256SUMS` file from the
   [latest GitHub release](https://github.com/zonnedev/jman/releases/latest).
2. Verify the download from the directory containing both files:

   ```bash
   grep '  jman-<version>-linux-x86_64.tar.gz$' SHA256SUMS \
     | sha256sum --check -
   ```

3. Extract it into a versioned directory. Replace `<version>` and `<archive>`
   with the downloaded release:

   ```bash
   mkdir -p "$HOME/.local/share/jman/versions/<version>"
   tar -xzf <archive> -C "$HOME/.local/share/jman/versions/<version>" --strip-components=1
   mkdir -p "$HOME/.local/bin"
   ln -sfn "$HOME/.local/share/jman/versions/<version>/jman" "$HOME/.local/bin/jman"
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

Temurin is the default download distribution. Pass `--vendor <name>` when
installing another distribution, for example `zulu` or `corretto`; later
installed-JDK commands infer the vendor from project/global selection or a
unique installed match.
See [Java toolchains](../guides/java-toolchains.md) for catalog filters,
verification, cache behavior, project overrides, and shell integration.

## Upgrade or remove JMAN

Install each release into a new versioned directory and repoint the
`~/.local/bin/jman` symlink. This makes rollback explicit and leaves no partial
in-place upgrade. After choosing the version you want to keep, remove older
version directories manually.

JMAN's artifact and catalog cache is independent of the installation. Its
default location is `~/.cache/jman`; see [storage](../reference/storage.md).
