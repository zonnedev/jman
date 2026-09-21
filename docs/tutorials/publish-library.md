# Publish a library

This tutorial prepares a release-quality POM, validates every archive locally,
and then shows the two remote publication paths.

## 1. Start with a library project

```bash
jman init useful-library \
  --lib \
  --group dev.example \
  --version 1.0.0 \
  --java 21
cd useful-library
```

Release versions must not end in `-SNAPSHOT` for Maven Central.

## 2. Add publication metadata

Append and adapt this section in `jman.toml`:

```toml
[publishing]
name = "Useful Library"
description = "Small, focused utilities for Java applications"
url = "https://github.com/example/useful-library"

[[publishing.licenses]]
name = "Apache-2.0"
url = "https://www.apache.org/licenses/LICENSE-2.0.txt"
distribution = "repo"

[[publishing.developers]]
id = "developer"
name = "Example Developer"
email = "developer@example.com"

[publishing.scm]
connection = "scm:git:https://github.com/example/useful-library.git"
developer-connection = "scm:git:ssh://git@github.com/example/useful-library.git"
url = "https://github.com/example/useful-library"
tag = "HEAD"
```

## 3. Validate without delivering

```bash
jman publish --dry-run
```

JMAN builds thin, source, and Javadoc JARs, generates a standalone POM, stages
the Maven repository layout, and writes checksum sidecars. Inspect
`.jman/publications/repository/` before any remote operation.

## 4. Test a local Maven consumer

```bash
jman publish
```

This installs the staged artifacts into `~/.m2/repository`. Use
`--local-repository /tmp/test-repository` for an isolated acceptance test.

## 5. Publish remotely

For a generic HTTPS Maven repository:

```bash
export JMAN_PUBLISH_USERNAME='repository-user'
export JMAN_PUBLISH_PASSWORD='repository-password'
jman publish --to repository \
  --repository-url https://repo.example.com/releases/
```

Use `JMAN_PUBLISH_TOKEN` instead for bearer authentication. Credentials never
belong in `jman.toml` or a repository URL.

For Maven Central, create a Central Portal token and configure GPG:

```bash
export JMAN_CENTRAL_USERNAME='portal-token-username'
export JMAN_CENTRAL_PASSWORD='portal-token-password'
export JMAN_GPG_KEY_ID='full-key-fingerprint'
export JMAN_GPG_PASSPHRASE='key-passphrase'
jman publish --to central
```

The default waits until Central validates the deployment, leaving the final
release action in the Portal. Add `--automatic` to request publication and wait
for `PUBLISHED`.

Remote publication rejects dirty Git worktrees by default. This makes the
released source correspond to a reviewable commit. The complete signing,
authentication, checksum, and Central behavior is in the
[publishing reference](../publishing.md).
