# Lockfile, cache, and outputs

JMAN separates reviewable project state, durable user data, shared downloaded
data, and disposable build output.

## `jman.lock`

The generated lockfile records:

- schema and manifest/workspace hashes;
- target platform and selected JDK;
- resolved packages, versions, repositories, hashes, and dependency edges;
- module compile, runtime, test, and processor classpaths.

Commit every workspace lockfile. JMAN writes it deterministically and
atomically. Never edit it by hand; change `jman.toml` and run `jman sync`.

Lock schema changes are explicit. A JMAN release refuses unsupported newer
schemas rather than producing a potentially different graph.

## Shared cache

The default root is `~/.cache/jman/`; `JMAN_CACHE_DIR` overrides it.

| Directory | Contents |
| --- | --- |
| `repository/` | Maven artifacts, POMs, and metadata. |
| `catalog/` | Remote JDK catalog responses and validators. |
| `audit/osv/` | Vulnerability responses keyed by the exact graph. |

The cache is reusable across workspaces. Offline commands require the relevant
entries to exist and never hide a cache miss with network access.

Set an isolated cache in CI when jobs should not share state:

```bash
export JMAN_CACHE_DIR="$PWD/.ci-cache/jman"
jman sync
```

Do not commit the shared cache.

## Durable Java data and configuration

Installed JDKs and command shims are user data, not a cache. On Linux they are
stored under `~/.local/share/jman/`; `JMAN_DATA_DIR` overrides the root.

| Path | Contents |
| --- | --- |
| `jdks/` | Verified JDK installations managed by JMAN. |
| `current` | Atomic link to the exact globally selected JDK. |
| `shims/` | Project-aware JDK command links created by `jman java setup`. |

The user-wide selection is stored separately in
`~/.config/jman/config.toml`; `JMAN_CONFIG_DIR` overrides that configuration
root. The file records an exact version and vendor. Do not manually edit the
configuration or the `current` link; use `jman java install ... --global` or
`jman java use ... --global` so they change together.

Cache cleanup is safe for installed JDKs. Removing the data directory is not:
it deletes the managed installations and shims.

## Project-local `.jman/`

JMAN writes reproducible or ephemeral project state beneath `.jman/`:

| Path | Purpose |
| --- | --- |
| `.jman/classes/` | Compiled production and test classes. |
| `.jman/artifacts/` | Thin, fat, source, and Javadoc JARs. |
| `.jman/reports/coverage/` | Coverage summary, JSON, XML, and HTML. |
| `.jman/publications/repository/` | Staged Maven repository layout. |

Module outputs live under that module's directory. Add `.jman/` to
`.gitignore`; rebuild it from committed source and lockfiles.

## Language-server cache

The language server keeps project-specific structural/semantic indexes,
external symbols, generated processor output, and decompiled-source fallbacks
under its platform cache directory. **Clear Workspace Cache** in VS Code or
`:JmanClearCache` in Neovim removes the current workspace's cached index and
rebuilds it. This does not delete source files or the Maven artifact cache.

Use editor status output to see the active cache directory, hit/miss counts,
project ID, and indexing time before diagnosing performance.
