# `jman.toml` reference

`jman.toml` is the only project file intended for manual editing. Keys use
kebab-case, dependency maps are sorted when JMAN rewrites them, and unknown
schema versions are rejected rather than guessed.

## Complete example

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

Only `manifest-version` and `[project]` are mandatory.

## Top-level schema

`manifest-version`
: Required integer. The current public manifest schema is `1`.

`[project]`
: Required identity, compilation target, packaging, modules, and entry point.

`[toolchain]`
: Optional JDK version and vendor pin.

`[maven]`
: Optional advanced Maven metadata, mainly retained during import.

`[build]`, `[test]`, `[publishing]`, `[audit]`
: Optional workflow configuration.

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
`temurin`. Prefer `jman java use` so the selected identifiers match the catalog.

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
