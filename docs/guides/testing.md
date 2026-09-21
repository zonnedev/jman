# Test Java projects

`jman test` compiles test source sets, starts one isolated JUnit Platform worker
per selected module, and streams structured events back to the terminal or an
editor.

## Source sets

The default `all` runs both conventional roots:

- `src/test/java` and `src/test/resources` for unit tests.
- `src/integrationTest/java` and `src/integrationTest/resources` for
  integration tests.

Select one set when iterating:

```bash
jman test --source-set unit
jman test --source-set integration
```

## Select modules and tests

```bash
jman test --module api --module application
jman test --tests dev.example.OrderServiceTest
jman test --tests 'dev.example.OrderServiceTest#createsOrder'
```

Both selectors are repeatable. JMAN retains stable JUnit selectors so editor
reruns target the same class or method rather than matching display text.

## Parallel execution and live reporting

```bash
jman test --jobs 4
```

`--jobs` bounds module JVMs, not the internal policy of a test engine. Human
output grows as a module/suite/test tree while cases finish. JSON output emits
newline-delimited `test-module-started`, `test-case-started`, `test-case`, and
`test-module-finished` records and flushes every event immediately.

Test durations come from JUnit execution events. Suite durations include
container lifecycle time that cannot be attributed to an individual method.

## Debug a worker

```bash
jman test --debug --tests dev.example.OrderServiceTest
```

Debug mode configures the test worker for debugger attachment. Prefer a narrow
selector so unrelated tests do not occupy the debug session.

Arguments after `--` are forwarded to the JUnit Platform runner:

```bash
jman test -- --details verbose
```

## Coverage

```bash
jman test --coverage \
  --coverage-format summary \
  --coverage-format html \
  --coverage-min-line 80 \
  --coverage-min-branch 70
```

Repeat `--coverage-include` and `--coverage-exclude` to filter classes, and use
`--coverage-output <directory>` to relocate reports. Put stable policy in
`[test.coverage]`; see [coverage](../coverage.md).

## HTTP tests and containers

JMAN sets `-Djman.test.port=0` by default. Applications that open a test HTTP
server should read that property and let the operating system select an
ephemeral port. Set `JMAN_TEST_PORT` only when a fixed value is intentionally
required.

Docker, Podman, `DOCKER_HOST`, `CONTAINER_HOST`, and Testcontainers environment
settings pass through unchanged. `jman doctor` reports container runtime
availability. Ctrl-C cancels orchestration and terminates worker JVMs on a
best-effort basis; Testcontainers remains responsible for its resources.

## CI output

Use plain human output for readable logs:

```bash
jman --no-progress test
```

Use `--report json` when another process owns presentation or needs individual
test events. A nonzero exit covers compilation failures, worker failures,
failed tests, and missed coverage thresholds.
