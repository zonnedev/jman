# Protobuf generator example

This native JMAN project uses `protoc` to generate Java sources before
compilation. Install `protoc`, then run:

```bash
jman sync
jman generate
jman run
```

`jman compile`, `jman test`, `jman run`, `jman build`, and `jman publish` run
the required main-source generator automatically. Generated files live under
`.jman/generated/protobuf/java/` and must not be committed.

Run `jman generate --rebuild` to bypass generator reuse. Editing
`src/main/proto/greeting.proto`, changing the generator command, or changing
the resolved `protoc` executable invalidates the generator cache.
