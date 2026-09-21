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

For local state only, with no network access:

```bash
jman java list --local
```

For scripts and editor integrations:

```bash
jman java list --format json
```

JSON retains the complete matching catalog even when the human table would be
compact.

## Install and select a JDK

```bash
jman java install 21
jman java install 21 --vendor corretto
jman java use 21 --vendor corretto
jman java which
```

Temurin is the default vendor. Installation resolves the platform archive and
SHA-256 digest from the provider, requires HTTPS, validates every initial and
redirected host, verifies the downloaded bytes, and only then makes the JDK
available. `java use` records both version and vendor in the current project.

`jman java which` explains the JDK selected for a project. The project pin wins
over ambient shell defaults, keeping CLI, CI, and editor actions aligned.

## Remove installations safely

Preview before removal:

```bash
jman java remove 21 --dry-run
jman java remove 21 --vendor corretto
```

Use `--all` to remove all matching installations and `--force` only when JMAN
reports that a safety check needs an explicit override. `--path` selects the
project context used by the operation.

## Catalog caching and refresh

The remote catalog is cached beneath `$JMAN_CACHE_DIR/catalog/` for 15 minutes.
Expired entries are conditionally revalidated with an HTTP ETag or
`Last-Modified`. If the provider is unavailable, JMAN reports the failure and
falls back to the last valid cache entry.

```bash
jman java list --refresh
```

`--refresh` bypasses the freshness window but still permits conditional HTTP
revalidation. `--local` is the strict no-network alternative.

## Build-tool JDK versus project JDK

For Maven and Gradle workspaces opened through JMAN Java, the JDK that runs the
build tool can differ from the project's compiler toolchain. This matters when,
for example, Gradle 8.7 cannot run on Java 25. JMAN selects a compatible
installed LTS runtime or accepts an explicit editor `buildJavaHome`; Gradle or
Maven still owns the project's language target.

JMAN never installs a JDK implicitly for editor import. Install the suggested
runtime explicitly, then synchronize the workspace again.
