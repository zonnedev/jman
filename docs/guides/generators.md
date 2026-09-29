# Generate Java sources and resources

Native JMAN generators are content-addressed build nodes. They declare their
inputs, direct command, dependencies, and typed outputs. JMAN runs the required
generators before compilation and owns their output beneath `.jman/generated/`.

## Generate Protobuf Java sources

```toml
[build.generators.protobuf]
command = ["protoc"]
arguments = [
  "--proto_path=${module.root}/src/main/proto",
  "--java_out=${output.java}",
  "${input}",
]
inputs = ["src/main/proto/**/*.proto"]

[build.generators.protobuf.outputs.java]
kind = "java-sources"
source-set = "main"
```

The repository contains a complete
[Protobuf example](https://github.com/zonnedev/jman/tree/master/examples/protobuf).
Install `protoc`, synchronize its Java runtime dependency, and run it with:

```bash
cd examples/protobuf
jman sync
jman generate
jman run
```

`jman generate` runs generators directly. `compile`, `run`, `test`, `build`,
and `publish` run the generators needed by their source sets automatically.
`sync`, `fmt`, project discovery, and editor startup never execute them.

## Expressions

Generator arguments support a small JMAN-owned expression language:

| Expression | Value |
| --- | --- |
| `${project.root}` | Canonical workspace root. |
| `${module.root}` | Canonical module directory. |
| `${generator.name}` | Generator name from the manifest. |
| `${java.home}` | Selected build JDK. |
| `${input}` | Sorted matching input files, each as a separate process argument. |
| `${output.NAME}` | Temporary directory for a declared named output. |
| `${dependency.GENERATOR.OUTPUT}` | Stable output directory of a direct generator dependency. |
| `${env.NAME}` | A declared or explicitly inherited environment value. |

`${input}` must occupy one complete argument because it expands to zero or
more arguments. Write `$${input}` when a command needs the literal text
`${input}`. JMAN performs no shell evaluation, command substitution, variable
defaults, pipes, or redirection.

Input patterns are module-relative `/` paths. They support `*`, `?`, and `**`.
Matches are canonicalized, sorted, deduplicated, and forbidden from escaping
the module or traversing symbolic links. A generator fails when it matches no
inputs unless it explicitly sets `allow-empty-inputs = true`.

## Typed outputs and source sets

Every output has one kind and one owning source set:

```toml
[build.generators.schema.outputs.java]
kind = "java-sources"
source-set = "main"

[build.generators.schema.outputs.resources]
kind = "resources"
source-set = "main"
```

Supported kinds are `java-sources` and `resources`. Supported source sets are
`main`, `test`, and `integration-test`. Main output is visible to tests, so do
not produce the same class separately for every source set. A generator may
declare multiple outputs when one invocation creates different material.

JMAN assigns each output a stable final directory:

```text
.jman/generated/GENERATOR/OUTPUT/
```

The command receives temporary output paths. JMAN promotes the complete output
tree atomically only after a successful exit. Failure preserves the previous
valid output, and replacement removes stale generated files.

## Generator dependencies

Generators within one module can depend on one another:

```toml
[build.generators.client]
command = ["client-generator"]
arguments = [
  "--schema-sources=${dependency.schema.java}",
  "--java-out=${output.java}",
]
allow-empty-inputs = true
depends-on = ["schema"]

[build.generators.client.outputs.java]
kind = "java-sources"
source-set = "main"
```

JMAN validates dependency names and output references, detects cycles, orders
execution, and includes dependency state in downstream generator keys. Only
direct dependencies can be referenced. Normal workspace module edges remain
the boundary between modules.

## Environment and working directory

Commands run directly without a platform shell. Their default working
directory is the module root. An alternative must remain inside the module:

```toml
working-directory = "tools/schema"
environment = { GENERATOR_MODE = "strict" }
inherit-environment = ["HTTP_PROXY", "HTTPS_PROXY"]
```

JMAN supplies `JAVA_HOME`, `JMAN_JAVA_HOME`, `JMAN_PROJECT_ROOT`,
`JMAN_MODULE_ROOT`, and `JMAN_GENERATOR_NAME`. The selected JDK is first on
`PATH`. These context variables and `PATH` are reserved. Other ambient
variables are cleared unless explicitly inherited. Declared and inherited
values participate in the cache key.

## Cache and invalidation

The generator key includes its normalized configuration, input paths and
bytes, resolved executable bytes, effective environment, and dependency
generator state. A hit is reused only while every declared output still has
the recorded content digest. Missing, changed, or unexpected output forces a
new invocation.

```bash
jman generate
jman generate --generator protobuf
jman generate --source-set test
jman generate --rebuild
```

`--rebuild` bypasses generator reuse once. Generated output and state are
project-local disposable data and must not be committed. `--offline` prevents
JMAN from downloading a missing managed JDK; arbitrary external generator
tools remain responsible for their own network behavior.

## Trust boundary

Generators execute repository-provided programs. Running an explicit CLI build
command authorizes the required generators. Merely opening, discovering, or
synchronizing a repository does not. Editors can index existing generated
roots but must use their normal workspace-trust and build-confirmation flow
before starting generation.

Generators are not arbitrary lifecycle plugins. Version 1 does not accept
class files, JARs, custom artifacts, post-package hooks, cross-module generator
edges, or shell command strings as generated build outputs.
