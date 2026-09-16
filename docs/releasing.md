# Releasing JMAN

Release candidates are built from a clean, reviewed commit. Publishing is a
separate, explicit operation; the repository release target only creates local
artifacts.

## Candidate checklist

```bash
cargo fmt --all -- --check
make gates
make test-compatibility-matrix
JAVAC_FRONTEND_LIB_DIR=target/native LD_LIBRARY_PATH=target/native \
  cargo clippy --workspace --all-targets -- -D warnings
make release
```

Confirm that `git status --short` is empty, the version and changelog agree,
and the generated SHA-256 file verifies. Then create an annotated tag named
`v<version>`, for example `v0.1.0-rc.1`. Uploading artifacts, pushing the tag,
and publishing the VS Code extension require an explicit maintainer action.

## Local artifacts

`make release` writes a timestamp-normalized Linux archive and checksum under
`target/release-dist/`. The archive contains `jman`, its native compiler
frontend, editor-import support, the processor worker, Vineflower, the license,
README, and changelog. `make package-vscode` separately creates the VSIX.

GraalVM Native Image 25 does not produce byte-identical shared libraries across
fresh builds, even with stable inputs and image identifiers. Consequently, each
candidate archive is checksummed and auditable but is not claimed to be
bit-for-bit reproducible. JMAN's Java JAR outputs retain their separate
byte-reproducibility contract.
