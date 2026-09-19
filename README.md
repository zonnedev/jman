<p align="center">
  <img src="resources/icons/jman.svg" alt="JMAN logo" width="180">
</p>

# JMAN

[![CI](https://github.com/zonnedev/jman/actions/workflows/ci.yml/badge.svg)](https://github.com/zonnedev/jman/actions/workflows/ci.yml)
[![GitHub Release](https://img.shields.io/github/v/release/zonnedev/jman?include_prereleases)](https://github.com/zonnedev/jman/releases)

JMAN (`jman`) is a native Rust build tool and Java language server for deterministic,
locked Java workspaces. It resolves Maven repositories itself; normal build,
test, run, and editor operations do not invoke Maven or Gradle.

The public name and executable are **JMAN** and `jman`. Its project format,
cache, environment, and editor namespaces consistently use `jman.toml`,
`jman.lock`, `.jman/`, `JMAN_*`, and `jman.java.*`.

## Quick start

```bash
jman init hello
cd hello
jman check
jman test
jman run
```

Import an existing Maven project with `jman init --import .`. Import prefers the
project's `mvnw`, then system Maven, and uses JMAN's native Maven importer only
when neither is present. Maven is used once to export its effective reactor;
normal JMAN operations do not invoke it. Use `jman sync` after dependency
changes, `jman build --all` for all reproducible archives, and `jman doctor` to
inspect the selected JDK and optional container runtime.

Publish a library to the local Maven repository, a generic Maven repository,
or Maven Central:

```bash
jman publish                         # install into ~/.m2/repository
jman publish --dry-run --format json # build and validate without delivery
jman publish --to repository --repository-url https://repo.example/releases
jman publish --to central            # validate in the Central Portal
jman publish --to central --automatic
```

Publishing creates deterministic thin, source, and Javadoc JARs, a standalone
POM, and Maven-compatible checksums. Central publications are also GPG-signed
and uploaded as a repository-layout bundle. Credentials are accepted only from
environment variables; see [the publishing guide](docs/publishing.md).
The acceptance suite publishes a multi-module fixture into an isolated local
repository and consumes its transitive API through independent JMAN, Maven, and
Gradle applications.

Manage Java toolchains directly through JMAN:

```bash
jman java list                   # compact table: latest release per vendor
jman java list --local           # installed JDKs only; no network access
jman java list --all             # every matching release in the remote catalog
jman java list --major 21        # filter one Java feature release
jman java list --lts             # show only LTS release lines
jman java list --vendor corretto # filter one vendor
jman java list --refresh         # revalidate the cached remote catalog
jman java list --format json     # stable structured output for integrations
jman java install 21             # install Temurin (the default vendor)
jman java install 21 --vendor zulu
jman java use 21 --vendor zulu   # pin the project to a vendor and version
```

The platform-specific multi-distribution catalog is discovered through the
Foojay Disco API and translated into JMAN's provider-neutral model. It is cached under
`$JMAN_CACHE_DIR/catalog/` (normally `~/.cache/jman/catalog/`) for 15 minutes.
Expired entries are revalidated with HTTP ETags or `Last-Modified`. If Foojay is temporarily
unreachable, JMAN reports the problem and falls back to the last valid cached
catalog. `--refresh` bypasses the freshness window while retaining conditional
requests; `--local` never contacts the network. Installation resolves the
vendor archive and SHA-256 digest through the provider, then requires HTTPS and
checks every download and redirect host against a distribution-scoped allowlist
before accepting checksum-verified bytes. Human-readable listings merge local
and remote releases into one table with an `INSTALLED` column. Filters narrow
the compact table, `--all` expands it to every matching release, and JSON always
retains the complete matching catalog. Temurin remains the default.

The precise supported metadata boundary and release gates are defined in
[the product contract](docs/product-contract.md). Changes are recorded in the
[changelog](CHANGELOG.md).

Inspect direct dependency upgrades without changing the workspace:

```bash
jman outdated
jman outdated --include-prerelease
jman outdated --offline
jman outdated --format json
```

Workspace declarations are aggregated by coordinate and current version, while
local path dependencies are excluded. JMAN reports the newest patch, minor,
major, and overall update using each module's configured Maven repositories.
Stable releases are preferred unless prereleases are explicitly requested.
See [dependency maintenance](docs/dependency-maintenance.md) for the complete
contract.

Apply repository updates transactionally, with patch-only selection by default:

```bash
jman update --dry-run
jman update
jman update org.example:library --level minor
jman update --level major
jman update --level latest --include-prerelease
```

`jman update` edits every matching workspace declaration and regenerates its
lockfiles as one operation. If dependency resolution fails, all touched
manifests and lockfiles are restored.

Audit the complete locked dependency graph against known vulnerabilities:

```bash
jman audit
jman audit --severity high
jman audit --deny high
jman audit --offline
jman audit --format json
```

Audit reports include shortest dependency paths, fixed versions, cached and
offline operation, CI policy thresholds, and validated reasoned suppressions
with mandatory expiry dates. See [security auditing](docs/security-auditing.md)
for the provider, severity, cache, and suppression contracts.

Human dependency explanations use merged trees instead of flattened arrow
chains. `jman why group:artifact` renders every shortest route from the project
to the selected dependency, while each audit finding groups its shortest routes
under the affected workspace module. JSON reports retain explicit path arrays
for programmatic consumers.

## Editor integrations

The VS Code extension and Neovim 0.11+ plugin use the same JMAN Java language
server for native JMAN, Maven, and Gradle projects. Both provide project-model
synchronization, navigation, diagnostics, refactorings, CodeLens tests,
build-tool wrappers, and compatible build-JDK selection. Neovim users can
install this repository directly and configure `require("jman").setup()`; see
[the Neovim guide](editors/neovim/README.md) and `:help jman.nvim`.

## Development

Use `make gates` for the complete local acceptance suite, `cargo test
--workspace` for Rust tests, and `make test-java` for the repository's Java
components. `make ci` runs the portable suite used by GitHub Actions. `cargo
run -p jman-cli -- --help` shows the command surface. Release automation and
maintainer setup are documented in [the release guide](docs/releasing.md).

## Supported Maven metadata

The native resolver supports parent inheritance, properties, dependency
management and imported BOMs, compile/runtime/provided/test scopes, optional
dependencies, exclusions, classifiers and types, repositories, snapshots,
relocation, and Maven nearest-wins mediation. Reactor imports preserve modules,
local parents, and inter-module dependencies. Maven build plugins are detected
and reported but are deliberately not executed or translated; compiler options,
processors, and main classes belong in `jman.toml`.

Compatibility is validated by the fixtures and differential harness described
in [docs/compatibility-matrix.md](docs/compatibility-matrix.md). The current MVP
does not claim full Maven 3/4 model compatibility, arbitrary plugin behavior, or
100% agreement with every public Maven project.

## Test contract

`jman test` compiles `src/test/java` and `src/integrationTest/java`, then runs each
selected module in a separate JVM. `--source-set unit` or `--source-set
integration` selects one source set; the default `all` runs both. Module JVMs
are bounded by `--jobs`. Individual results are reported as soon as JUnit
finishes each test. Human output grows as a module/suite/test tree above the
active spinner, including nested suites and failure details; interleaved parallel
suites reopen their branch explicitly instead of mixing unrelated children.
Final module summaries remain sorted for reproducible output.

The cached `jman-runner.jar` is compiled from the repository-owned Java source
with the selected project JDK. Rust and runner use protocol version 3. A JUnit
Platform listener emits framed events while the worker is running; `--report
json` exposes them as flushed, newline-delimited `test-module-started`,
`test-case-started`, `test-case`, and `test-module-finished` events. Test cases
retain a stable selector plus a unique display/invocation identity, exact failure
message/details, duration, and retry attempt. Class-container events report total
suite time and lifecycle time outside child test methods, including shared setup
and teardown. JUnit XML remains the authoritative reconciliation fallback.
Reruns use exact class or `Class#method` selectors. The protocol exposes debug
descriptors and an explicit unsupported coverage response instead of silently
pretending coverage was collected.

Tests receive `-Djman.test.port=0` by default. Applications that need an HTTP port
must read `jman.test.port` and bind port zero so the operating system chooses an
ephemeral port. Set `JMAN_TEST_PORT` to opt into a fixed value; JMAN does not
reserve a port and does not claim collision-free handoff.

Docker, Podman, `DOCKER_HOST`, `CONTAINER_HOST`, and Testcontainers environment
configuration are inherited unchanged. `jman doctor` probes Docker and Podman and
reports the explicit endpoint when configured. Ctrl-C cancels the orchestration
future and runner JVMs use kill-on-drop for best-effort process cleanup.
Testcontainers' resource reaper remains responsible for containers. SIGKILL,
host crashes, and disabled reapers can still leave resources behind.

## MVP limitations

- Coverage collection is not bundled.
- Cancellation is best effort across operating-system process boundaries.
- Generated source metadata is read-only guidance; clients enforce editing UX.
- Workspace file notifications trigger a safe reindex and never perform an
  implicit destructive filesystem mutation.
- External Maven/Gradle editor providers require project-local wrappers or the
  corresponding executable and remain separate from the native JMAN build path.
