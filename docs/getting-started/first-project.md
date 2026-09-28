# Your first JMAN project

This tutorial creates a small Java application, runs its generated JUnit test,
and packages the result.

## 1. Create the application

```bash
jman init greeting \
  --group dev.example \
  --version 1.0.0 \
  --java 21 \
  --main-class dev.example.greeting.Application
cd greeting
```

JMAN creates `jman.toml`, resolves `jman.lock`, declares JUnit Platform Console
Standalone in test scope, and generates JJFS-formatted production and test
sources. The generated application is ready to run:

```bash
jman run
```

Its `src/main/java/dev/example/greeting/Application.java` starts as:

```java
package dev.example.greeting;

public final class Application {
  private Application() {
  }

  public static void main(String[] args) {
    System.out.println(greeting());
  }

  static String greeting() {
    return "Hello from greeting!";
  }
}
```

The companion `src/test/java/dev/example/greeting/ApplicationTest.java` is:

```java
package dev.example.greeting;

import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.Test;

final class ApplicationTest {
  @Test
  void createsGreeting() {
    Assertions.assertEquals("Hello from greeting!", Application.greeting());
  }
}
```

## 2. Run the test

```bash
jman test
```

JMAN reports modules, suites, and individual cases as they complete. A final
summary contains the real suite duration and pass/fail/skip totals.

Inspect the generated dependency graph and why the runner is present:

```bash
jman tree
jman why org.junit.platform:junit-platform-console-standalone
```

## 3. Check and package

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

## 4. Try the reproducible path

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
