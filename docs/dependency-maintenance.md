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
