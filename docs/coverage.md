# Test coverage

JMAN collects Java code coverage with its pinned, checksum-verified JaCoCo
agent and translates JaCoCo reports into a provider-neutral JMAN model. The
agent and reporting CLI are bundled with release archives and editor packages;
they are never added to the project's dependency graph.

Run the complete test selection with coverage:

```bash
jman test --coverage
```

By default JMAN prints a module tree and writes HTML, XML, and JSON reports to
`.jman/reports/coverage/`. `index.html` is the interactive report,
`coverage.xml` is the native JaCoCo interchange report, and `coverage.json`
contains JMAN's stable provider-neutral model. Restrict persistent formats or
change the destination with:

```bash
jman test --coverage --coverage-format summary --coverage-format xml
jman test --coverage --coverage-output build/reports/coverage
```

Coverage works with the normal `--module`, `--tests`, and `--source-set`
selection flags. Each module test JVM writes an isolated execution-data file;
JMAN aggregates those files only after all selected tests finish. Reports are
still generated when tests or coverage thresholds fail.

## Configuration and CI thresholds

Workspace defaults belong in the root `jman.toml`:

```toml
[test.coverage]
enabled = true
engine = "jacoco"
formats = ["summary", "json", "xml", "html"]
minimum-line = 80
minimum-branch = 70
include = ["com.example.*"]
exclude = ["com.example.generated.*", "com.example.*Application"]
```

Thresholds are aggregate workspace percentages from 0 through 100. Command-line
values override the manifest:

```bash
jman test --coverage --coverage-min-line 85 --coverage-min-branch 75
```

Include and exclude values use JaCoCo class-name wildcards: `*` matches any
sequence and `?` matches one character. Slash-separated and dot-separated class
names are both accepted. Dependencies never contribute to project coverage;
only compiled classes owned by selected workspace modules are reported.

## Structured and editor output

`jman test --coverage --report json` retains the test protocol's
newline-delimited stream and adds `coverage-file` and `coverage-summary`
records. File records contain line, branch, and method counters plus line-level
instruction and branch details. The final summary contains module totals,
workspace totals, output paths, and threshold failures.

VS Code exposes a native **Run with JMAN Coverage** Test Explorer profile and
uses file and line details in its Test Coverage view. Neovim provides
`:JmanCoverage`, `:JmanCoverageNearest`, and `:JmanCoveragePattern`; reports use
the same terminal tree and `.jman/reports/coverage/` artifacts as the CLI.

Coverage descriptors are currently available only for native `jman.toml`
workspaces. Maven and Gradle test execution remains owned by those build tools.
