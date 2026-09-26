# Releasing JMAN

JMAN releases are built from existing Git tags by GitHub Actions. The workflow
tests the tagged source, builds every currently supported binary artifact,
publishes checksums and a machine-readable manifest, records GitHub artifact
attestations, and creates the GitHub Release. It never changes source files,
versions, commits, or tags.

## One-time repository setup

1. Enable GitHub Actions and permit workflows to create releases with the
   repository `GITHUB_TOKEN`.
2. Protect `master` and require the `CI / Verify Linux x64` and `CI / Verify
   macOS ARM64` status checks before merging. Both normal CI and tagged releases
   use the standard Apple Silicon `macos-15` runner.
3. Create a GitHub environment named `vscode-marketplace`. Add required
   reviewers so Marketplace publication remains a deliberate approval step.
4. Create an Azure user-assigned managed identity with a federated credential
   for GitHub owner `zonnedev`, repository `jman`, and environment
   `vscode-marketplace`. Assign the identity the Azure subscription `Reader`
   role.
5. Add the identity's client, tenant, and subscription IDs to the environment as
   `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, and `AZURE_SUBSCRIPTION_ID` secrets.
6. Create or use an Azure DevOps organization connected to the same Microsoft
   Entra directory, then add the managed identity to the organization with the
   free `Stakeholder` access level. This creates the Azure DevOps profile used
   by Visual Studio Marketplace.
7. From a federated CI job authenticated as the managed identity, retrieve its
   Visual Studio Marketplace profile ID once with:

   ```bash
   az rest \
     --url https://app.vssps.visualstudio.com/_apis/profile/profiles/me \
     --resource 499b84ac-1321-427f-aa17-267ca6975798 \
     --query id \
     --output tsv
   ```

8. Add that profile ID to the `zonnedev` Marketplace publisher with the
   `Contributor` role.
9. Run **Verify VS Code Marketplace Identity** and confirm that its Marketplace
   publisher access check succeeds.

Marketplace publishing exchanges GitHub OIDC tokens for short-lived Microsoft
Entra credentials. Do not add a `VSCE_PAT` secret; the workflow intentionally
does not support long-lived Marketplace tokens.

## Prepare a release

Record user-facing changes below each `Unreleased` heading. From a clean
reviewed commit, run the portable suite before preparing the tag:

```bash
cargo fmt --all -- --check
make ci
JMAN_JAVAC_FRONTEND_LIB_DIR=target/native LD_LIBRARY_PATH=target/native \
  cargo clippy --workspace --all-targets --all-features -- -D warnings
npm audit --prefix editors/vscode --audit-level=high
```

For a local rehearsal of the tag's full acceptance suite, run
`make release-gates`. It fetches Spring Petclinic and Gson at fixed Git commits
into `target/upstream-fixtures/`. Test setup downloads the checksum-pinned JMAN
bootstrap release configured in `scripts/setup-test-jdks.sh` into
`target/test-toolchains/` and uses it to install GraalVM Community 25.3.4.1
plus Temurin 17, 21, and 25. Maven 3.9.9 and Gradle 8.7/8.14.1/9.1.0 are also
checksum-pinned under `target/compatibility-tools/`. Existing JDK installations
can be supplied with `JMAN_GRAALVM_HOME` and the `JMAN_TEST_JAVA_*_HOME`
variables. The release workflow performs the same self-hosted setup and runs
`make release-gates` against the exact tag in its Linux validation job. The
Apple Silicon packaging job uses the pinned `actions/setup-java` action as the
one-time GraalVM seed because the older Linux-only JMAN bootstrap cannot run on
macOS; released JMAN then manages ordinary project JDKs itself. Publishing
waits for both jobs. Normal pushes and pull requests continue to run the
smaller `make ci` suite.

Then prepare the coordinated release from the same clean worktree:

```bash
make prepare-release VERSION=0.8.1
```

The command synchronizes the Rust workspace, Cargo lockfile, VS Code manifests,
changelogs, workflow example, and versioned documentation. After validation it
asks whether to create `chore(release): prepare v0.8.1` and the annotated tag
`git tag -a v0.8.1 -m "v0.8.1"`. A separate final confirmation can atomically
push both the current branch and tag to `origin`. Declining either confirmation
never pushes anything and leaves the prepared state available for review.

Tags whose version contains a hyphen, such as `-rc.1`, become GitHub
pre-releases. Stable semantic versions become normal releases. A failed or
interrupted release can be retried from **Actions → Release → Run workflow** by
entering the existing tag; the workflow safely replaces assets on an existing
GitHub Release.

## Published artifacts

The Release workflow currently publishes:

- `jman-<version>-linux-x86_64.tar.gz`, containing the CLI, native compiler
  frontend, Java workers, project import support, documentation, and license;
- `jman-<version>-macos-aarch64.tar.gz`, with the same runtime for Apple
  Silicon macOS;
- `jman-java-<version>-linux-x64.vsix`, the Linux x64 VS Code extension;
- `jman-java-<version>-darwin-arm64.vsix`, the Apple Silicon VS Code extension;
- `install.sh`, the user-local bootstrap installer;
- `SHA256SUMS`, covering the CLI, extension, and installer; and
- `release-manifest.json`, recording versions, target platforms, filenames, and
  SHA-256 digests.

The same files remain available as a short-lived workflow artifact for 14 days.
GitHub stores Sigstore-backed build provenance for the CLI archive, VSIX, and
installer. After downloading an artifact, verify both controls:

```bash
grep '  jman-0.8.1-linux-x86_64.tar.gz$' SHA256SUMS \
  | sha256sum --check -
gh attestation verify jman-0.8.1-linux-x86_64.tar.gz \
  --repo zonnedev/jman
```

GraalVM Native Image 25 does not produce byte-identical shared libraries across
fresh builds, even with stable inputs and image identifiers. Each candidate is
therefore checksummed and attested but is not claimed to be bit-for-bit
reproducible. JMAN's Java JAR outputs retain their separate byte-reproducibility
contract.

## Publish the VS Code extension

First install the VSIX from the GitHub Release into a clean VS Code profile and
complete the [VS Code release checklist](vscode-release-checklist.md). Then open
**Actions → Publish VS Code Marketplace → Run workflow**, enter the same release
tag, and approve the `vscode-marketplace` environment deployment.

The publishing workflow downloads both already-reviewed platform VSIX files
from the GitHub Release, verifies them against `SHA256SUMS`, confirms their
publisher and extension identity, and publishes them through Marketplace
trusted publishing. Tags with a prerelease suffix publish to the Marketplace
pre-release channel; stable tags publish to the stable channel. It does not
rebuild or modify either package. Re-running it is safe because duplicate
versions are skipped.

## Local artifact rehearsal

To inspect the artifacts for the current host without creating a tag or
release:

```bash
make release
make package-vscode
```

The release workflow combines the Linux and macOS job outputs before running
`make stage-release`; local staging requires both platform pairs to be copied
into `target/release-dist/` and `target/vscode/`. These commands never publish,
commit, or tag anything.
