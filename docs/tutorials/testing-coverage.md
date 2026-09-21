# Test and measure coverage

This tutorial adds unit and integration tests, runs precise selectors, and
turns coverage expectations into a checked project policy.

## 1. Declare a JUnit engine

```bash
jman add org.junit.jupiter:junit-jupiter@5.10.2 --scope test
```

Place fast tests in `src/test/java` and broader tests in
`src/integrationTest/java`. Both source sets receive main outputs and test
dependencies.

## 2. Watch results arrive

```bash
jman test
```

The human reporter builds a module/suite/test tree while JUnit runs. Results are
not buffered until the whole suite ends, and suite totals include lifecycle
time such as shared setup and teardown.

Useful focused runs:

```bash
jman test --source-set unit
jman test --source-set integration
jman test --module application
jman test --tests dev.example.CalculatorTest
jman test --tests 'dev.example.CalculatorTest#addsTwoNumbers'
```

Repeat `--module` or `--tests` to select more than one target. Arguments after
`--` are passed to the JUnit runner.

## 3. Generate coverage

```bash
jman test --coverage --coverage-format html --coverage-format xml
```

Open `.jman/reports/coverage/html/index.html` for the navigable report. XML and
JSON support CI and editor integrations; the terminal summary remains concise.

## 4. Make coverage policy persistent

Add this to `jman.toml`:

```toml
[test.coverage]
enabled = true
formats = ["summary", "html", "xml"]
minimum-line = 80
minimum-branch = 70
include = ["dev.example.*"]
exclude = ["dev.example.generated.*"]
```

Now a plain `jman test` collects coverage and fails when either threshold is
missed. Command-line options can still override report formats, output paths,
thresholds, and filters for one run.

## 5. Feed another tool

```bash
jman test --report json --coverage --coverage-format json
```

JSON mode emits newline-delimited events as modules and cases start and finish,
then adds coverage-file and coverage-summary records. Consumers should process
the stream incrementally rather than waiting for EOF.

See the [testing guide](../guides/testing.md) for debugging, parallelism,
Testcontainers, and failure behavior, and the [coverage contract](../coverage.md)
for exact metrics and paths.
