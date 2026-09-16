# Editor integration contract

All editor integrations start the same stdio language server:

```text
jman lsp
```

The LSP implementation is a Rust library embedded in `jman`; editors do not
launch a separate `jman-java-lsp` executable.

## Initialization options

- `buildSystem`: `auto`, `jman`, `gradle`, or `maven`. `auto` prefers JMAN when
  `jman.toml` is present.
- `buildSync`: `manual`, `prompt`, or `automatic`.

Clients should watch `jman.toml`, `jman.lock`, Maven POM/wrapper files, and Gradle
settings, build, wrapper, version-catalog, and `buildSrc` files.

## Custom server contract

- Notification `jman.java/buildSyncStatus` reports `ready`, `required`,
  `syncing`, or `failed`.
- Command `jman.java.status` reports indexing, cache, and synchronization state.
- Command `jman.java.syncWorkspace` reloads the selected workspace model.
- Command `jman.java.rebuildIndex` rebuilds the project index.
- Command `jman.java.clearWorkspaceCache` clears and rebuilds project caches.
- Request `jman.java/changeSignature` performs the conservative Java
  change-signature operation.
- Request `jman.java/tests/discover` returns protocol version 2 test items with
  stable IDs, parent IDs, source ranges, JUnit tags, and exact JMAN selectors.
- Request `jman.java/tests/run` validates selectors and returns a versioned JMAN
  invocation descriptor. The editor launches its configured `jman` executable,
  streams output into its native test UI, and owns cancellation.
- Command `jman.java.test` is used by standard `textDocument/codeLens` entries
  to run the exact test method selected by the user.

The initialization result advertises these extensions under
`capabilities.experimental.jmanJavaTesting`. Version 2 reports
`streamingEvents: true`; per-test results use the versioned `jman-runner`
protocol rather than being inferred from console text.

Editor integrations own only process startup, initialization options, file
watchers, command UI, and status presentation. Java analysis, workspace
imports, indexing, and annotation processing remain in JMAN.
