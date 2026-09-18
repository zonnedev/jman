# Publishing Java artifacts

`jman publish` builds every workspace module in dependency order and stages a
Maven-compatible repository under `.jman/publications/repository/`. JAR modules
include the thin JAR, sources, Javadoc, and a generated POM. Aggregator modules
publish only their POM. The staged primary files receive MD5, SHA-1, SHA-256,
and SHA-512 sidecars. `--sign` adds armored GPG signatures; Maven Central always
requires signing.

Use `--dry-run` to run the complete build, metadata validation, staging,
checksum, signing, and Central bundle creation without copying or uploading
anything. `--format json` writes a stable machine-readable report to standard
output and keeps progress output disabled.

## Project metadata

Local and generic repositories can publish with the project coordinates alone.
Maven Central additionally requires a release version and complete project
metadata. Metadata on the workspace root is inherited by modules that do not
define their own section.

```toml
[publishing]
name = "Example Library"
description = "A concise description of the library"
url = "https://github.com/example/example"

[[publishing.licenses]]
name = "Apache-2.0"
url = "https://www.apache.org/licenses/LICENSE-2.0.txt"
distribution = "repo"

[[publishing.developers]]
id = "developer"
name = "Example Developer"
email = "developer@example.com"

[publishing.scm]
connection = "scm:git:https://github.com/example/example.git"
developer-connection = "scm:git:ssh://git@github.com/example/example.git"
url = "https://github.com/example/example"
tag = "HEAD"
```

The generated POM is standalone: direct JMAN dependencies retain their Maven
scope, type, classifier, optional flag, and exclusions; workspace path
dependencies become normal versioned Maven coordinates.

## Local Maven repository

```bash
jman publish
jman publish --local-repository /tmp/maven-repository
```

Files are copied atomically. Publishing the same release again replaces the
matching local files.

## Generic Maven repository

```bash
export JMAN_PUBLISH_USERNAME='repository-user'
export JMAN_PUBLISH_PASSWORD='repository-password'
jman publish --to repository \
  --repository-url https://repo.example.com/releases/
```

Use `JMAN_PUBLISH_TOKEN` instead for bearer authentication. JMAN never reads
credentials from `jman.toml` or a repository URL. HTTPS is mandatory;
`--allow-insecure` permits HTTP only for explicitly selected local-development
servers. Every staged file is uploaded with HTTP PUT at its Maven repository
path.

## Maven Central Portal

Create a user token in the Central Portal, then provide either its username and
password pair or a pre-encoded portal token:

```bash
export JMAN_CENTRAL_USERNAME='portal-token-username'
export JMAN_CENTRAL_PASSWORD='portal-token-password'
# Alternatively: export JMAN_CENTRAL_TOKEN='<base64 username:password>'

export JMAN_GPG_KEY_ID='full key fingerprint' # optional when GPG has a default
export JMAN_GPG_PASSPHRASE='key passphrase'    # optional

jman publish --to central
jman publish --to central --automatic
```

The default is user-managed publication: JMAN waits until Central reports the
deployment as `VALIDATED`, then you publish it in the Portal. `--automatic`
asks Central to release it automatically and waits for `PUBLISHED`. The default
timeout is five minutes and can be changed with `--timeout-seconds`.

Remote publication refuses a dirty Git worktree by default. Use
`--allow-dirty` only when the exact uncommitted contents are intentional.
