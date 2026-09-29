# Library platform: a complete native JMAN workspace

This example deliberately uses no Maven or Gradle. It exercises a Java 25
multi-module build, local module dependencies, cached source generators,
annotation processors, unit tests, integration-test source sets, two runnable
Micronaut applications, and deterministic JAR packaging through JMAN.

## Architecture

```text
domain              DDD entities, aggregates, and value objects; no dependencies
  ↑
application         framework-free use cases and repository ports
  ↑          ↑
http-api     grpc-api        Micronaut inbound adapters and in-memory repositories
     ↑          ↑
 api-contracts              OpenAPI and protobuf contracts plus generated Java
          ↑
        client              one public Java API, HTTP or gRPC selected by its builder
```

`api-contracts` owns the wire formats. JMAN runs two generators before any
consumer compiles:

- a Java 25 source-file tool validates the OpenAPI operations and generates the
  HTTP route/model contract;
- `protoc` generates the protobuf messages used by the gRPC server and client.

Generated sources live below `.jman/generated/` and are never committed.

## Prerequisites

- JMAN built from this repository;
- `protoc` available on `PATH`;
- a Java 25 JDK. JMAN installs the selected Temurin JDK when it is missing.

## Build and test everything

From this directory:

```bash
jman sync
jman generate
jman test
jman build --all
```

Or run the composed acceptance gate:

```bash
jman script verify
```

JMAN contributors can run the same project from an isolated temporary copy:

```bash
make test-library-platform
```

The test layout is intentional:

- `domain/src/test` verifies aggregate invariants;
- `application/src/test` verifies use cases through in-memory ports;
- each API module has direct adapter unit tests;
- `http-api/src/integrationTest` starts a Micronaut HTTP server on an ephemeral
  port and performs a real request;
- `grpc-api/src/integrationTest` starts a real gRPC server and channel;
- `client/src/integrationTest` proves HTTP and gRPC implement the exact same
  public client contract.

Run a narrower tier or module while iterating:

```bash
jman test --source-set unit
jman test --source-set integration
jman test --module library-application
jman test --module library-client --source-set integration
```

## Run either application

The workspace has two entry points, so select one explicitly:

```bash
jman run --module library-http-api
jman run --module library-grpc-api
```

The equivalent named scripts are:

```bash
jman script run-http
jman script run-grpc
```

### HTTP examples

```bash
curl -sS http://localhost:8080/users \
  -H 'content-type: application/json' \
  -d '{"name":"Ada"}'

curl -sS http://localhost:8080/users

curl -sS http://localhost:8080/users/USER_ID/books \
  -H 'content-type: application/json' \
  -d '{"title":"Domain-Driven Design"}'

curl -sS -X PUT http://localhost:8080/books/BOOK_ID/owner \
  -H 'content-type: application/json' \
  -d '{"ownerId":"NEW_OWNER_ID"}'
```

### gRPC examples

With `grpcurl` installed:

```bash
grpcurl -plaintext \
  -import-path api-contracts/src/main/proto \
  -proto library.proto \
  -d '{"name":"Ada"}' \
  localhost:9090 jman.example.library.v1.LibraryService/CreateUser
```

## Use the client library

Application code depends only on `LibraryClient`. Selecting a transport does
not change the operations or return types:

```java
try (var library = LibraryClient.builder()
  .http(URI.create("http://localhost:8080"))
  .build()) {
  var ada = library.createUser("Ada");
  var book = library.registerBook(ada.id(), "Domain-Driven Design");
}
```

Switch only the builder configuration for gRPC:

```java
try (var library = LibraryClient.builder()
  .grpc("localhost", 9090)
  .build()) {
  var users = library.listUsers();
}
```

## What this example proves

JMAN can own the full lifecycle of a non-trivial Java workspace today:

- one checked-in manifest and lockfile per module;
- dependency resolution and local module ordering;
- generated main sources as first-class compiler inputs;
- Micronaut annotation processing;
- concurrent, streamed JUnit execution across unit and integration source sets;
- multiple selectable application entry points;
- thin, fat, source, and Javadoc artifacts.

The in-memory repositories are deliberate: the example tests JMAN, modular
architecture, contracts, and transports without requiring a database or
container runtime.
