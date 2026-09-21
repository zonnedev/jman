# Import a Maven project

This tutorial converts an existing Maven reactor into native JMAN manifests
without moving or rewriting its Java sources.

## 1. Start from a healthy Maven build

From the directory containing the root `pom.xml`, run the project's existing
verification once:

```bash
./mvnw verify
```

If the wrapper is absent, use `mvn verify`. Fix Maven-model or source failures
before importing; JMAN cannot infer a correct project from a broken effective
model.

## 2. Import the effective reactor

```bash
jman init --import .
```

The importer prefers `./mvnw`, then a system `mvn`. When neither exists, JMAN
uses its built-in Maven model importer. Asking Maven for its effective reactor
captures profiles and model inheritance more faithfully; the native fallback
keeps simple projects importable without a Maven installation.

The operation creates a root `jman.toml` and `jman.lock`, plus module manifests
for a reactor. It does not delete `pom.xml` or alter source files.

## 3. Review the translation

```bash
git diff -- jman.toml '**/jman.toml'
jman tree
jman doctor
```

Check these fields especially:

- `[project]`: coordinates, Java release, packaging, modules, and main class.
- Dependency scopes and local relationships under `[path-dependencies]`.
- Imported BOMs and dependency management under `[maven]`.
- `[annotation-processors]` for Lombok, Micronaut, MapStruct, or other code
  generation required by compilation.
- `[build].compiler-args` for project-specific `javac` flags.

Maven plugins are diagnosed but are not executed or translated into an opaque
plugin system. Express compilation, processors, the main class, and supported
JMAN behavior explicitly in the manifest.

## 4. Compare the native build

```bash
jman sync
jman check
jman test
jman build
```

Resolve any reported unsupported metadata before removing Maven from your
normal workflow. Keep both builds temporarily if consumers or release jobs
still need Maven interoperability.

## 5. Commit the native model

Commit every `jman.toml` and `jman.lock`, and ignore generated state:

```gitignore
.jman/
```

From this point, `jman sync`, `check`, `test`, `build`, and `run` use JMAN's
native resolver and builder; they do not invoke Maven.

!!! tip "Editor-only Maven projects"

    You do not have to convert a project to use JMAN Java. The language server
    can import Maven workspaces directly through the wrapper/system Maven model
    provider. Native conversion is for adopting JMAN as the build tool.
