# Environment variables

JMAN keeps reproducible project configuration in `jman.toml`. Environment
variables are reserved for machine-local paths, credentials, and runtime
integration.

## CLI and build

| Variable | Purpose |
| --- | --- |
| `JMAN_CACHE_DIR` | Override the shared artifact, catalog, and audit cache root. |
| `JMAN_DATA_DIR` | Override the durable managed-JDK, `current`, and shim root. |
| `JMAN_CONFIG_DIR` | Override the user-wide configuration root. |
| `JMAN_TEST_PORT` | Override the default `0` value of the `jman.test.port` system property. |
| `DOCKER_HOST` | Docker endpoint inherited by tests and Testcontainers. |
| `CONTAINER_HOST` | Alternative container endpoint inherited by tests. |

## Publishing credentials

| Variable | Purpose |
| --- | --- |
| `JMAN_PUBLISH_USERNAME` | Username for a generic Maven repository. |
| `JMAN_PUBLISH_PASSWORD` | Password for a generic Maven repository. |
| `JMAN_PUBLISH_TOKEN` | Bearer token alternative for a generic repository. |
| `JMAN_CENTRAL_USERNAME` | Maven Central Portal token username. |
| `JMAN_CENTRAL_PASSWORD` | Maven Central Portal token password. |
| `JMAN_CENTRAL_TOKEN` | Pre-encoded Central token alternative. |
| `JMAN_GPG_KEY_ID` | GPG key ID or full fingerprint; otherwise use the GPG default. |
| `JMAN_GPG_PASSPHRASE` | Optional passphrase for non-interactive signing. |

Never commit these values. In CI, store credentials in the platform's secret
store rather than plain workflow variables.

## Editor integration

| Variable | Purpose |
| --- | --- |
| `JMAN_BIN` | Neovim executable fallback when `cmd` is not configured. |
| `JAVA_LSP_BUILD_JAVA_HOME` | Override the JDK used to run Maven or Gradle model import. |
| `JAVA_LSP_TELEMETRY` | Enable local performance diagnostics; no remote analytics are sent. |
| `JAVA_LSP_PARSE_WORKERS` | Advanced override for structural parsing workers. |
| `JAVA_LSP_SEMANTIC_WORKERS` | Advanced override for semantic-analysis workers. |

Prefer the editor's `buildJavaHome`/`build_java_home` setting over exporting
`JAVA_LSP_BUILD_JAVA_HOME` interactively. Worker-count overrides are diagnostic
tuning controls, not normal project configuration.

Without an explicit build-runtime override or Gradle daemon criterion, editor
imports consider JMAN's global Java before automatically discovered SDKMAN,
`JAVA_HOME`, and `PATH` installations. The selected runtime still has to be
compatible with the Maven or Gradle version.

The VS Code `jman.java.server.extraEnv` and Neovim `extra_env` settings pass
additional variables into the server and its build/test tasks. Treat their
contents like local build-tool configuration and do not place secrets in a
checked-in editor settings file.

## Advanced audit provider

`JMAN_AUDIT_OSV_URL` points auditing at an OSV-compatible HTTPS endpoint. It is
primarily intended for compatible mirrors and test infrastructure. JMAN still
validates provider responses and keys cached results by the exact dependency
graph.

## Maintainer-only variables

The repository's build, integration-test, and release scripts define additional
`JMAN_*` and `JAVA_LSP_*` variables for bundled tool paths and fixtures. They
are not part of the supported end-user configuration surface. Consult the
specific script and [release guide](../releasing.md) only when developing JMAN
itself.
