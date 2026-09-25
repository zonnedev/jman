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
result in every editor and CI environment:

- four spaces per indentation level, LF line endings, and a final newline;
- canonical spacing around declarations, calls, control flow, operators,
  generics, arrays, varargs, lambdas, and method references;
- ordinary imports followed by static imports, with each group sorted;
- resolvable wildcard imports expanded to the types or static members actually
  referenced by a semantically complete compilation unit (wildcards are kept
  when attribution is incomplete so formatting cannot break unresolved code);
- fields and initializer blocks first in their original order, because their
  order can change runtime behavior;
- constructors next, methods ordered by name, and nested types last;
- line comments, block comments, Javadocs, string literals, character literals,
  and text-block contents preserved from the source.

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
