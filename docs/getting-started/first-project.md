# Your first JMAN project

This tutorial creates a small Java application, adds a real test dependency,
runs a JUnit test, and packages the result.

## 1. Create the application

```bash
jman init greeting \
  --group dev.example \
  --version 1.0.0 \
  --java 21 \
  --main-class dev.example.greeting.Application
cd greeting
```

JMAN creates `jman.toml`, `jman.lock`, and the standard Java source/resource
directories. Open `src/main/java/dev/example/greeting/Application.java`; the
generated class is already runnable.

```bash
jman run
```

## 2. Add JUnit

Declare JUnit only on the test classpath:

```bash
jman add org.junit.jupiter:junit-jupiter@5.10.2 --scope test
```

`jman add` updates `jman.toml`, resolves the graph, and rewrites `jman.lock` as
one operation. Inspect what was selected:

```bash
jman tree
jman why org.junit.jupiter:junit-jupiter
```

## 3. Write a test

Create `src/test/java/dev/example/greeting/GreetingTest.java`:

```java
package dev.example.greeting;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

final class GreetingTest {
    @Test
    void buildsAGreeting() {
        assertEquals("Hello, Ada!", "Hello, " + "Ada" + "!");
    }
}
```

Run it:

```bash
jman test
```

JMAN reports modules, suites, and individual cases as they complete. A final
summary contains the real suite duration and pass/fail/skip totals.

## 4. Check and package

```bash
jman check
jman build --all
```

The default build produces a thin application JAR. `--all` additionally
creates an executable fat JAR, a source JAR, and a Javadoc JAR under
`.jman/artifacts/`.

Run the executable archive directly:

```bash
java -jar .jman/artifacts/greeting-1.0.0-fat.jar
```

The exact fat-JAR suffix is shown by `jman build`; use that reported path when
the project name or version differs.

## 5. Try the reproducible path

Once dependencies and the JDK are cached, confirm the project works without
network access:

```bash
jman check --offline
jman test --offline
jman build --offline
```

Commit both `jman.toml` and `jman.lock`. Do not commit `.jman/`; it contains
reproducible project-local outputs and state.

Next, learn the [project layout](project-layout.md), build a
[multi-module workspace](../tutorials/multi-module.md), or configure an
[editor](../guides/editors.md).
