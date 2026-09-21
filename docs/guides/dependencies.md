# Manage dependencies

JMAN resolves Maven-compatible coordinates into a deterministic lockfile and
provides separate commands for declaration changes, graph explanation,
upgrades, and security policy.

## Add and remove direct dependencies

The add coordinate format is `group:artifact@version`:

```bash
jman add com.fasterxml.jackson.core:jackson-databind@2.17.2
jman add org.postgresql:postgresql@42.7.4 --scope runtime
jman add jakarta.servlet:jakarta.servlet-api@6.1.0 --scope provided
jman add org.junit.jupiter:junit-jupiter@5.10.2 --scope test
jman add org.projectlombok:lombok@1.18.34 --scope processor
```

Scopes control classpath membership:

| Scope | Main compile | Runtime | Test | Meaning |
| --- | ---: | ---: | ---: | --- |
| `compile` | yes | yes | yes | Normal API and implementation dependency |
| `runtime` | no | yes | yes | Runtime implementation such as a database driver |
| `provided` | yes | no | yes | Supplied by the deployment environment |
| `test` | no | no | yes | Tests only |
| `processor` | processor path | no | as needed | Compile-time annotation processor |

Remove a direct declaration by coordinate:

```bash
jman remove org.junit.jupiter:junit-jupiter
```

Use `--path <project>` with `add` or `remove` when not running in that module's
directory. Both commands regenerate lockfiles transactionally.

## Understand the selected graph

```bash
jman tree
jman why com.fasterxml.jackson.core:jackson-core
```

`tree` renders the resolved graph as a hierarchy. `why` renders every shortest
route from a workspace root to the selected artifact; shared portions remain a
tree instead of becoming ambiguous arrow-delimited text.

Maven nearest-wins mediation chooses one version for a conflict. Dependency
management and imported BOMs constrain versions before graph resolution.

## Find and apply updates

```bash
jman outdated
jman outdated --include-prerelease
jman outdated --format json

jman update --dry-run
jman update org.example:library --level minor
jman update --level major
jman update --level latest --include-prerelease
```

The default update level is `patch`. JMAN edits all matching workspace
declarations and regenerates lockfiles as one transaction; resolution failure
restores every touched file. Local path dependencies are excluded.

The exact version-selection and JSON contracts are documented in
[dependency maintenance](../dependency-maintenance.md).

## Work offline

After a successful online resolution:

```bash
jman sync --offline
jman check --offline
jman test --offline
```

Offline mode never silently goes online. It fails with the missing coordinate
or JDK when the lockfile references data absent from the cache.

## Repositories, BOMs, and exclusions

Declare additional HTTPS repositories and advanced Maven metadata directly in
`jman.toml`. JMAN supports parents, properties, dependency management, BOM
imports, scopes, optional dependencies, exclusions, classifiers, types,
snapshots, relocation, and reactor-local dependencies. See the
[manifest reference](../reference/manifest.md) for syntax and the
[product contract](../product-contract.md) for the exact compatibility boundary.

## Audit the graph

```bash
jman audit
jman audit --severity high
jman audit --deny high
jman audit --offline
```

Audits query OSV for every locked package and render shortest dependency paths.
Use `--deny` as a CI policy threshold and reasoned, expiring suppressions for
accepted risk. See [security auditing](../security-auditing.md).
