# Dependency maintenance

`jman outdated` checks direct compile, runtime, provided, test, and annotation
processor dependencies without modifying `jman.toml` or `jman.lock`.

```bash
jman outdated
jman outdated path/to/workspace
jman outdated --include-prerelease
jman outdated --offline
jman outdated --format json
```

For a multi-module workspace, matching declarations with the same coordinate
and current version become one result containing all declaring modules and
scopes. Workspace path dependencies are excluded because they are built from
local source rather than selected from a repository.

## Version policy

By default, alpha, beta, milestone, release-candidate, early-access, preview,
and snapshot versions are excluded. `--include-prerelease` opts into them.
Numeric Maven versions are classified relative to the current declaration:

- patch: the major and minor components are unchanged;
- minor: the major component is unchanged and the minor component increases;
- major: the major component increases;
- other: the version does not have a comparable numeric release line.

The human table shows the newest overall update and its change class. JSON also
provides the newest candidate in each patch, minor, and major class.

## Repository metadata and offline use

Online checks refresh `group/artifact/maven-metadata.xml` through the
repositories declared by each module, or Maven Central when none are declared.
Successful metadata is cached under `$JMAN_CACHE_DIR/repository/` (normally
`~/.cache/jman/repository/`). If repositories are temporarily unavailable,
JMAN falls back to the last cached catalog and warns that it may be stale.

`--offline` never contacts a repository. Dependencies without cached metadata
are reported as `unavailable`; one unavailable catalog does not hide results
for the rest of the workspace.

`--format json` suppresses progress output and returns stable counters plus one
entry per aggregated dependency. Each entry contains its status, current and
available versions, change class, scopes, modules, metadata source, and any
coordinate-specific error.

## Applying updates

`jman update` uses the same repository catalog and prerelease policy, then
updates matching declarations and regenerates the workspace lockfiles:

```bash
jman update --dry-run
jman update
jman update org.example:library
jman update --level minor
jman update --level major
jman update --level latest
jman update --offline
jman update --format json
```

Patch is the default level. `minor` selects the newest patch or minor update;
`major` selects the newest patch, minor, or major update. `latest` additionally
allows versions classified as `other`. Prereleases remain excluded unless
`--include-prerelease` is supplied. A coordinate argument limits the operation
to that direct dependency across all workspace modules.

`--dry-run` performs discovery and prints the exact plan without changing any
files. A real update snapshots all workspace manifests and lockfiles, changes
every declaration matching both the coordinate and its current version, and
runs a refreshed synchronization. Any serialization, repository, resolution,
or lockfile failure restores the complete snapshot, including removing
lockfiles created during the failed attempt. Unavailable metadata prevents a
partial update plan from being applied.
