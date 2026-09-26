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
Comment framing is normalized and plain prose uses a conservative 100-column
soft limit; code, lists, URLs, diagrams, directives, and other structured
content are never reflowed. Literal contents remain authored. Semantic
rewrites are applied only when javac can prove them safe.

Comments attached before a member move with that member. A comment on the same
line as a type's opening brace remains attached to the type header. Formatting
is idempotent: running it again produces no changes.

### Comments and Javadocs

JJFS normalizes multiline comment framing and wraps only plain prose at a
100-column soft limit:

```java
/**
 * Loads the customer and verifies that it can participate in the requested
 * operation.
 *
 * @param identifier the customer identifier
 * @return the active customer
 */
```

It does not reflow lists, tables, URLs, code and `<pre>` blocks, diagrams,
license headers, generated-file warnings, Markdown `///` documentation, or
formatter/tool directives. Trailing comments retain their authored content.
When a fragment cannot be classified confidently, JJFS preserves it.

## Editor formatting

The language server advertises full-document formatting and returns the same
canonical source as `jman fmt`.

In VS Code, JMAN registers itself as the default Java formatter. Run **JMAN
Java: Format Document with JJFS** or the standard **Format Document** command.
Enable `editor.formatOnSave` under `[java]` to format on save.

In Neovim, run `:JmanFormat` or press `<Leader>jf`. Set
`format_on_save = true` in `require("jman").setup(...)` to format before each
Java buffer write. Both paths restrict formatting to the JMAN client.

Range and on-type formatting are intentionally not advertised, because partial
formatting could disagree with import and member ordering.

If a document has a syntax error, JMAN leaves it unchanged and reports the
first javac diagnostic. Fix that diagnostic, then format again.
