# Build a multi-module workspace

This tutorial creates a small `api` library consumed by an `application`
module. JMAN resolves the local relationship without publishing the library.

## 1. Scaffold the workspace

```bash
jman init example-platform \
  --group dev.example \
  --version 1.0.0 \
  --java 21 \
  --modules api,application
cd example-platform
```

The root manifest lists both modules and uses aggregator packaging. Each module
has its own manifest and lockfile; create source roots as you add code.

## 2. Make the API reusable

In `api/jman.toml`, make sure the project is a JAR without a main class:

```toml
[project]
group = "dev.example"
name = "api"
version = "1.0.0"
java-release = 21
packaging = "jar"
```

Create the package directory and
`api/src/main/java/dev/example/api/Greeting.java`:

```bash
mkdir -p api/src/main/java/dev/example/api
```

```java
package dev.example.api;

public final class Greeting {
    private Greeting() {}

    public static String forName(String name) {
        return "Hello, " + name + "!";
    }
}
```

## 3. Connect the application

Add this relationship to `application/jman.toml`:

```toml
[path-dependencies]
"dev.example:api" = "../api"
```

Create the application package and class:

```bash
mkdir -p application/src/main/java/dev/example/application
```

Write `application/src/main/java/dev/example/application/Application.java`:

```java
package dev.example.application;

import dev.example.api.Greeting;

public final class Application {
    public static void main(String[] args) {
        System.out.println(Greeting.forName("JMAN"));
    }
}
```

Ensure the application manifest declares the class:

```toml
[project]
main-class = "dev.example.application.Application"
```

## 4. Resolve, test, and run

Run commands from the workspace root:

```bash
jman sync
jman check
jman test
jman run
```

JMAN topologically orders modules and compiles independent modules in parallel
up to the `--jobs` limit. `jman run` chooses the runnable application module.

## 5. Package everything

```bash
jman build --all
```

Each JAR module writes artifacts beneath its own `.jman/artifacts/` directory.
The application fat JAR contains runtime dependencies and merges service and
framework metadata deterministically. The API produces its thin, source, and
Javadoc JARs and can be published independently.

Path dependencies become ordinary versioned Maven coordinates in generated
publication POMs, so consumers never see local filesystem paths.
