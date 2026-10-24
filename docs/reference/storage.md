# Lockfile, cache, and outputs

JMAN separates reviewable project state, durable user data, shared downloaded
data, and disposable build output.

## `jman.lock`

The generated lockfile records:

- schema and manifest/workspace hashes;
- target platform and selected JDK;
- resolved packages, versions, repositories, hashes, and dependency edges;
- workspace dependency coordinates, scopes, normalized paths, and target versions;
- module compile, runtime, test, and processor classpaths.

Commit every workspace lockfile. JMAN writes it deterministically and
atomically. Never edit it by hand; change `jman.toml` and run `jman sync`.

Lock schema changes are explicit. A JMAN release refuses unsupported newer
schemas rather than producing a potentially different graph.

## Shared cache

`JMAN_CACHE_DIR` overrides the shared cache root. Without it, JMAN uses the
platform cache directory:

| Platform | Default cache root |
| --- | --- |
| Linux | `~/.cache/jman/` (or `$XDG_CACHE_HOME/jman/`) |
| macOS | `~/Library/Caches/jman/` |

| Directory | Contents |
| --- | --- |
| `repository/` | Maven POMs, artifacts, and metadata, separated by repository identity. |
| `artifacts/` | Content-addressed resolved dependency artifacts. |
| `catalog/` | Remote JDK catalog responses and validators. |
| `audit/osv/` | Vulnerability responses keyed by the exact graph. |
| `imports/` | Effective Maven import models. |
| `tools/` | Regenerable JMAN test and coverage runner state. |
| `lsp/` | Language-server indexes, generated output, external sources, decompiled sources, and isolated Gradle state. |

The cache is reusable across workspaces. Offline commands require the relevant
entries to exist and never hide a cache miss with network access.
Repository entries from older JMAN versions lacked repository identity and are
not reused. Run `jman sync --refresh` online once after upgrading to populate
the new entries before relying on offline resolution.

Set an isolated cache in CI when jobs should not share state:

```bash
export JMAN_CACHE_DIR="$PWD/.ci-cache/jman"
jman sync
```

Do not commit the shared cache.

Inspect or reclaim it without manually deleting directories:

```bash
jman cache dir
jman cache status
jman cache prune --older-than 30
jman cache clear dependencies --dry-run
```

`jman cache clear` accepts a specific cache area or `all`. Both clearing and
pruning support JSON reports and dry runs. JMAN does not follow symbolic links
while measuring or deleting cache entries, and it refuses global LSP cleanup
while an active language server owns a cache lock.

## Durable Java data and configuration

Installed JDKs and command shims are user data, not a cache.
`JMAN_DATA_DIR` overrides their root; `JMAN_CONFIG_DIR` overrides the
user-configuration root.

| Platform | Data root | Configuration file |
| --- | --- | --- |
| Linux | `~/.local/share/jman/` or `$XDG_DATA_HOME/jman/` | `~/.config/jman/config.toml` or `$XDG_CONFIG_HOME/jman/config.toml` |
| macOS | `~/Library/Application Support/jman/` | `~/Library/Application Support/jman/config.toml` |

| Path | Contents |
| --- | --- |
| `jdks/` | Verified JDK installations managed by JMAN. |
| `current` | Atomic link to the exact globally selected JDK. |
| `shims/` | Project-aware JDK command links created by `jman java setup`. |

The user-wide selection file records an exact version and vendor. Do not
manually edit the configuration or the `current` link; use
`jman java install ... --global` or `jman java use ... --global` so they change
together.

Cache cleanup is safe for installed JDKs. Removing the data directory is not:
it deletes the managed installations and shims.

## Project-local `.jman/`

JMAN writes reproducible or ephemeral project state beneath `.jman/`:

| Path | Purpose |
| --- | --- |
| `.jman/output/` | Compiled production/test classes and annotation-processor output. |
| `.jman/artifacts/` | Thin, fat, source, and Javadoc JARs. |
| `.jman/generated/` | Atomically promoted source and resource generator outputs. |
| `.jman/generator-state/` | Content keys and output digests used for generator reuse. |
| `.jman/reports/coverage/` | Coverage summary, JSON, XML, and HTML. |
| `.jman/publications/repository/` | Staged Maven repository layout. |

Module outputs live under that module's directory. Add `.jman/` to
`.gitignore`; rebuild it from committed source and lockfiles.

`jman clean` removes `.jman/` from every module in the selected workspace. It
does not remove `jman.toml`, `jman.lock`, sources, resources, or shared caches.
Use `jman clean --dry-run --format json` to inspect the operation first.

## Language-server cache

The language server keeps project-specific structural/semantic indexes,
external symbols, generated processor output, Gradle import state, and
decompiled-source fallbacks under `lsp/` in JMAN's shared cache directory.
**Clear Workspace Cache** in VS Code or
`:JmanClearCache` in Neovim removes the current workspace's cached index and
rebuilds it. This does not delete source files or the Maven artifact cache.

`jman cache clear lsp` clears all LSP workspaces and regenerable LSP state when
no language-server process is active. Prefer the editor command when only the
current workspace needs rebuilding.

Use editor status output to see the active cache directory, hit/miss counts,
project ID, and indexing time before diagnosing performance.
