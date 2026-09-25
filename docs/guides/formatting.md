# Format Java source

JMAN ships one opinionated formatter for the command line, VS Code, and
Neovim. The formatter runs inside JMAN's native javac frontend, so it uses the
project's Java release and resolved compile model rather than a separate Java
parser.

## Format a project

Run the formatter from a JMAN, Maven, or Gradle project:

```console
jman fmt
```

Pass a directory to format only Java sources below it, or a single `.java`
file for a targeted edit:

```console
jman fmt src/main/java
jman fmt src/main/java/com/example/Application.java
```

JMAN ignores `.git`, `.gradle`, `.idea`, `.jman`, `build`, and `target`
directories. It computes every result before replacing any source, rejects
syntactically invalid input, and replaces changed files atomically.

## Check formatting in CI

`--check` performs the same parse, attribution, import expansion, and rendering
without writing files:

```console
jman fmt --check
```

The command succeeds when every source is canonical. It lists files that would
change and exits unsuccessfully otherwise.

## Canonical style

The format is intentionally not configurable. A repository therefore has one
result in every editor and CI environment. The normative rules are documented
in the [JMAN Java Formatting Style 1 specification](../reference/jjfs-v1.md).

JJFS 1 uses two-space indentation, a structural 140-column limit, canonical
imports without static or wildcard imports, API-first member ordering, and
AST-aware wrapping for declarations, calls, conditions, and fluent chains.
Comments and literal contents remain authored. Semantic rewrites are applied
only when javac can prove them safe.

Comments attached before a member move with that member. A comment on the same
line as a type's opening brace remains attached to the type header. Formatting
is idempotent: running it again produces no changes.

## Editor formatting

The language server advertises full-document formatting. Use the editor's
normal **Format Document** command; it returns the same canonical source as
`jman fmt`. Range and on-type formatting are intentionally not advertised,
because partial formatting could disagree with import and member ordering.

If a document has a syntax error, JMAN leaves it unchanged and reports the
first javac diagnostic. Fix that diagnostic, then format again.
