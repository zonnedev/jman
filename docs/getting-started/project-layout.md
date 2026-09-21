# Project layout

A JMAN project uses familiar Java source directories and keeps build metadata
small enough to review.

```text
greeting/
├── jman.toml
├── jman.lock
├── src/
│   ├── main/
│   │   ├── java/
│   │   └── resources/
│   ├── test/
│   │   ├── java/
│   │   └── resources/
│   └── integrationTest/
│       ├── java/
│       └── resources/
└── .jman/
    ├── artifacts/
    ├── classes/
    └── reports/
```

## Files to commit

`jman.toml`
: The human-edited project declaration: identity, Java release, dependencies,
  modules, compiler settings, coverage policy, and publishing metadata.

`jman.lock`
: JMAN's generated resolution record. Commit it so collaborators and CI select
  the same artifacts and hashes. Change declarations, then run `jman sync`;
  never maintain the lockfile by hand.

`src/`
: Application, test, integration-test, and resource sources. A module uses the
  same layout beneath its own directory.

## Files not to commit

`.jman/`
: Project-local compiled classes, reports, packaged artifacts, and other
  reproducible state. Add `.jman/` to `.gitignore`.

The shared cache defaults to `~/.cache/jman/` and also does not belong in the
repository. Set `JMAN_CACHE_DIR` to isolate it in CI or development tooling.

## Applications and libraries

An application declares a main class:

```toml
[project]
group = "dev.example"
name = "greeting"
version = "1.0.0"
java-release = 21
packaging = "jar"
main-class = "dev.example.greeting.Application"
```

A library omits `main-class` and can be scaffolded with `jman init --lib`.
`jman run` requires a configured main class, while both project types can be
built and published.

## Multi-module roots

A workspace root lists module directories and normally uses `pom` packaging:

```toml
[project]
group = "dev.example"
name = "platform"
version = "1.0.0"
java-release = 21
packaging = "pom"
modules = ["api", "service"]
```

Each module owns its own `jman.toml` and `jman.lock`. Local module relationships
are explicit under `[path-dependencies]`. The
[multi-module tutorial](../tutorials/multi-module.md) builds one from scratch.

See the [manifest reference](../reference/manifest.md) for every supported
section and [storage](../reference/storage.md) for generated paths and caches.
