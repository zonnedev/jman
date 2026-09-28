# `jman.toml` reference

`jman.toml` is the single source of truth for a directory's Java selection and
for native JMAN projects. Keys use kebab-case, dependency maps are sorted when
JMAN rewrites them, and unknown schema versions are rejected rather than
guessed.

## Toolchain-only example

A directory, Maven project, or Gradle project can select Java without becoming
a native JMAN project:

```toml
manifest-version = 1

[toolchain]
jdk = "25"
vendor = "zulu"
```

`jman java install 25 --vendor zulu` creates this form when no `jman.toml`
exists. `jman java use` updates it with an already installed JDK. Maven and
Gradle detection ignores the toolchain-only form, while shell shims, `java
which`, `java exec`, and editor processes inherit it. `jman init` upgrades it
in place and preserves `[toolchain]`.

## Application example

```toml
manifest-version = 1

[project]
group = "dev.example"
name = "orders"
version = "1.2.0"
java-release = 21
packaging = "jar"
modules = []
main-class = "dev.example.orders.Application"

[toolchain]
jdk = "21"
vendor = "temurin"

[build]
encoding = "UTF-8"
compiler-args = ["-parameters", "-Xlint:all"]

[scripts]
dev = "jman run -- --spring.profiles.active=dev"

[scripts.verify]
description = "Run all project checks"
steps = ["format-check", "test"]

[dependencies.compile]
"com.fasterxml.jackson.core:jackson-databind" = "2.17.2"

[dependencies.runtime]
"org.postgresql:postgresql" = "42.7.4"

[dependencies.provided]
"jakarta.servlet:jakarta.servlet-api" = "6.1.0"

[dependencies.test]
"org.junit.jupiter:junit-jupiter" = "5.10.2"

[annotation-processors]
"org.projectlombok:lombok" = "1.18.34"

[[repositories]]
id = "central"
url = "https://repo.maven.apache.org/maven2"
```

For a native project, `manifest-version` and `[project]` are mandatory. A
toolchain-only manifest instead supports only `manifest-version` and
`[toolchain]`; project sections such as dependencies are rejected until
`[project]` is added. The example deliberately shows common application
settings; the focused sections below cover local modules, test policy,
publishing, auditing, and advanced Maven metadata.

## Workspace example

An aggregator lists module directories and connects modules through their own
manifests:

```toml
# jman.toml
manifest-version = 1

[project]
group = "dev.example"
name = "orders-platform"
version = "1.0.0"
java-release = 21
packaging = "pom"
modules = ["orders-api", "orders-application"]
```

```toml
# orders-application/jman.toml
manifest-version = 1

[project]
group = "dev.example"
name = "orders-application"
version = "1.0.0"
java-release = 21
packaging = "jar"
main-class = "dev.example.orders.Application"

[path-dependencies]
"dev.example:orders-api" = "../orders-api"
```

Run `jman sync` at the root after adding or moving a module.

## Top-level schema

`manifest-version`
: Required integer. The current public manifest schema is `1`.

`[project]`
: Required for a native JMAN project; omitted by a toolchain-only manifest.
  Defines identity, compilation target, packaging, modules, and entry point.

`[toolchain]`
: Optional JDK version and vendor pin.

`[maven]`
: Optional advanced Maven metadata, mainly retained during import.

`[build]`, `[test]`, `[publishing]`, `[audit]`
: Optional workflow configuration.

`[scripts]`
: Explicitly invoked project commands and composed tasks.

`[[repositories]]`
: Ordered additional artifact repositories.

`[dependencies.*]`, `[annotation-processors]`, `[path-dependencies]`
: Direct external and local relationships.

## `[project]`

| Key | Required | Meaning |
| --- | --- | --- |
| `group` | yes | Maven-compatible group ID and Java package prefix. |
| `name` | yes | Artifact ID and module display name. |
| `version` | yes | Project/artifact version. |
| `java-release` | yes | `javac --release` target. |
| `packaging` | no | `jar` by default; use `pom` for an aggregator. |
| `modules` | no | Child module directories relative to this manifest. |
| `main-class` | no | Fully qualified application entry point. |

## `[toolchain]`

```toml
[toolchain]
jdk = "21.0.5+11"
vendor = "temurin"
```

`jdk` accepts a feature release or exact version. `vendor` defaults to
`temurin`. Prefer `jman java install` to install and select in one operation,
or `jman java use` to switch to an existing installation. The nearest
directory or project selection overrides the user's global Java.

## Dependencies

External dependency keys are `group:artifact`; values are versions:

```toml
[dependencies.compile]
"org.example:api" = "2.1.0"

[dependencies.runtime]
"org.example:runtime" = "2.1.0"

[dependencies.provided]
"org.example:container-api" = "1.0.0"

[dependencies.test]
"org.example:test-support" = "3.0.0"
```

Annotation processors use the same shape but receive a dedicated processor
path and are not packaged as runtime libraries:

```toml
[annotation-processors]
"org.projectlombok:lombok" = "1.18.34"
```

Local modules use Maven coordinates mapped to a relative directory:

```toml
[path-dependencies]
"dev.example:orders-api" = "../api"
```

## `[[repositories]]`

```toml
[[repositories]]
id = "company-releases"
url = "https://repo.example.com/releases"
```

Repository IDs identify the source in diagnostics. Use HTTPS for remote
repositories. Credentials are not part of the public manifest schema.

## `[build]`

```toml
[build]
encoding = "UTF-8"
compiler-args = ["-parameters"]
```

Encoding defaults to `UTF-8`. Compiler arguments are passed to `javac` for the
module; keep portable language targeting in `java-release`.

## `[scripts]`

The concise form is a command evaluated by the platform shell:

```toml
[scripts]
dev = "jman run -- --spring.profiles.active=dev"
format-check = "jman fmt --check"
database = "docker compose up -d postgres"
```

The structured form executes an argument array directly, without shell
parsing:

```toml
[scripts.database]
description = "Start the development database"
command = ["docker", "compose", "up", "-d", "postgres"]
working-directory = "deployment"
environment = { POSTGRES_DB = "orders" }
```

`working-directory` is relative to the manifest and must remain inside the
project. `environment` overlays the invoking process environment. JMAN adds
`JMAN_PROJECT_ROOT`, `JMAN_MANIFEST_PATH`, and `JMAN_SCRIPT_NAME`; when a
project or global JDK is selected it also sets `JAVA_HOME`, `JMAN_JAVA_HOME`,
and prepends the selected JDK's `bin` directory to `PATH`.

Use `steps` instead of `command` to compose existing scripts sequentially:

```toml
[scripts.verify]
description = "Run the standard project checks"
steps = ["format-check", "unit-test"]
environment = { CI = "true" }
```

A structured script must define exactly one of `command` or `steps`. Steps
must name declared scripts; cycles are rejected during execution. Parent
environment and working-directory settings flow into their steps, while a
child may override them. Extra CLI arguments are accepted by command scripts
but rejected for composed scripts because there is no unambiguous destination.

Run `jman script`, `jman script --format json`, or `jman script NAME` to list
and execute declarations. Shell initialization completes current project
script names dynamically. JMAN never invokes scripts implicitly during
project loading, synchronization, builds, tests, or editor startup.

## `[test.coverage]`

```toml
[test.coverage]
enabled = true
engine = "jacoco"
formats = ["summary", "json", "xml", "html"]
minimum-line = 80
minimum-branch = 70
include = ["dev.example.*"]
exclude = ["dev.example.generated.*"]
```

The only current engine is `jacoco`. Thresholds are integer percentages.
Include/exclude values use JaCoCo class-name patterns.

## `[publishing]`

```toml
[publishing]
name = "Orders API"
description = "Public API for order processing"
url = "https://github.com/example/orders"

[[publishing.licenses]]
name = "Apache-2.0"
url = "https://www.apache.org/licenses/LICENSE-2.0.txt"
distribution = "repo"

[[publishing.developers]]
id = "alice"
name = "Alice Example"
email = "alice@example.com"
organization = "Example"
organization-url = "https://example.com"

[publishing.scm]
connection = "scm:git:https://github.com/example/orders.git"
developer-connection = "scm:git:ssh://git@github.com/example/orders.git"
url = "https://github.com/example/orders"
tag = "HEAD"
```

The section is inherited from a workspace root by modules that omit it. Maven
Central requires a release version and complete name, description, URL,
license, developer, and SCM metadata.

## `[audit]`

Suppressions require an advisory ID, a reason, and an expiry date:

```toml
[[audit.suppressions]]
id = "GHSA-xxxx-yyyy-zzzz"
reason = "Not reachable; tracked in SEC-123"
expires = "2027-01-31"
```

Expired or malformed suppressions do not hide a finding. See
[security auditing](../security-auditing.md).

## `[maven]`

This section preserves Maven semantics that do not fit a simple version map:

```toml
[maven]
parent = "org.example:parent:1.0.0"
bom-imports = ["org.example:platform-bom:4.0.0"]

[[maven.dependency-management]]
group = "org.example"
artifact = "library"
version = "4.0.0"
scope = "compile"
dependency-type = "jar"
optional = false
exclusions = ["org.unwanted:legacy"]

[maven.dependencies."compile:org.example:library"]
dependency-type = "test-jar"
classifier = "tests"
optional = true
exclusions = ["org.unwanted:legacy"]
```

Metadata-map keys are `scope:group:artifact`. Managed entries support `group`,
`artifact`, optional `version`, `scope`, `dependency-type`, `classifier`,
`optional`, and `exclusions`. Prefer letting Maven import generate this section;
manual edits require a following `jman sync`.

## After editing

Run `jman sync`, review both manifest and lockfile changes, then validate with
`jman check` or `jman test`. JMAN rejects invalid coordinates, unsafe vendor
identifiers, unsupported schema versions, and inconsistent workspace metadata
before building.
