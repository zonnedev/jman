# Releasing JMAN

JMAN releases are built from existing Git tags by GitHub Actions. The workflow
tests the tagged source, builds every currently supported binary artifact,
publishes checksums and a machine-readable manifest, records GitHub artifact
attestations, and creates the GitHub Release. It never changes source files,
versions, commits, or tags.

## One-time repository setup

1. Enable GitHub Actions and permit workflows to create releases with the
   repository `GITHUB_TOKEN`.
2. Protect `master` and require the `CI / Verify Linux x64` status check before
   merging.
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
reviewed commit, run the complete maintainer suite:

```bash
cargo fmt --all -- --check
make gates
make test-compatibility-matrix
JAVAC_FRONTEND_LIB_DIR=target/native LD_LIBRARY_PATH=target/native \
  cargo clippy --workspace --all-targets --all-features -- -D warnings
npm audit --prefix editors/vscode --audit-level=high
```

The complete local suite includes external Maven/Gradle compatibility fixtures
that are intentionally not downloaded by CI. GitHub Actions reruns the portable
Rust, Java, native-frontend, extension, release-contract, formatting, lint, and
dependency-audit checks against the exact tag.

Then prepare the coordinated release from the same clean worktree:

```bash
make prepare-release VERSION=0.7.0
```

The command synchronizes the Rust workspace, Cargo lockfile, VS Code manifests,
changelogs, workflow example, and versioned documentation. After validation it
asks whether to create `chore(release): prepare v0.7.0` and the annotated tag
`git tag -a v0.7.0 -m "v0.7.0"`. A separate final confirmation can atomically
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
- `jman-java-<version>-linux-x64.vsix`, the Linux x64 VS Code extension;
- `install.sh`, the user-local bootstrap installer;
- `SHA256SUMS`, covering the CLI, extension, and installer; and
- `release-manifest.json`, recording versions, target platforms, filenames, and
  SHA-256 digests.

The same files remain available as a short-lived workflow artifact for 14 days.
GitHub stores Sigstore-backed build provenance for the CLI archive, VSIX, and
installer. After downloading an artifact, verify both controls:

```bash
grep '  jman-0.7.0-linux-x86_64.tar.gz$' SHA256SUMS \
  | sha256sum --check -
gh attestation verify jman-0.7.0-linux-x86_64.tar.gz \
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

The publishing workflow downloads the already-reviewed VSIX from the GitHub
Release, verifies it against `SHA256SUMS`, confirms its publisher and extension
identity, and publishes it through Marketplace trusted publishing. Tags with a
prerelease suffix publish to the Marketplace pre-release channel; stable tags
publish to the stable channel. It does not rebuild or modify the package.
Re-running it is safe because duplicate versions are skipped.

## Local artifact rehearsal

To inspect exactly what the workflow will publish without creating a tag or
release:

```bash
make release
make package-vscode
make stage-release TAG=v0.7.0
(cd target/github-release && sha256sum --check SHA256SUMS)
```

The staged files are written to `target/github-release/`. These commands never
publish, commit, or tag anything.
