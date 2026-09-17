# VS Code extension release checklist

JMAN Java releases are prepared as platform-specific VSIX packages. The first
public channel is a Linux x64 pre-release; do not upload it as a universal
extension.

## Build and verify

From a clean reviewed checkout on Linux x86-64:

```bash
make package-vscode
(cd target/vscode && sha256sum --check jman-java-0.3.0-linux-x64.vsix.sha256)
```

The packaging command rebuilds the native frontend, Java workers, bundled JMAN
server, and extension client. It runs the extension tests, validates the native
artifacts as compatibility-targeted Linux x86-64 ELF files, checks the VSIX ZIP
structure, and writes a SHA-256 checksum.

Confirm that `git status --short` contains only the reviewed release changes and
that `npm audit` reports no known vulnerabilities.

## Clean-profile acceptance

Use temporary profile and extension directories so the candidate is isolated
from normal VS Code settings and extensions:

```bash
mkdir -p /tmp/jman-vscode-profile /tmp/jman-vscode-extensions
code \
  --user-data-dir /tmp/jman-vscode-profile \
  --extensions-dir /tmp/jman-vscode-extensions \
  --install-extension target/vscode/jman-java-0.3.0-linux-x64.vsix
```

Launch VS Code with the same two directory arguments and verify:

1. The extension is labeled as a pre-release and displays the expected icon,
   README, changelog, license, and platform support statement.
2. It remains disabled until the workspace is trusted.
3. A JMAN project starts the bundled server and reports `jman` as its build
   system through **JMAN Java: Show Status**.
4. Representative Maven and Gradle projects synchronize with their wrappers.
5. Diagnostics, completion, hover, definition, references, formatting, code
   actions, symbols, semantic highlighting, and inlay hints respond correctly.
6. Test Explorer discovers classes and methods and can run and cancel tests.
7. Check, build, run, test, sync, restart, rebuild-index, and clear-cache
   commands report actionable success or failure messages.
8. **View → Output → JMAN Java** contains no unexpected startup or protocol
   errors.
9. Disabling or uninstalling the extension stops the bundled server.

Repeat the smoke test with another Java language server enabled long enough to
confirm that the documented conflict guidance is accurate, then disable the
other server for the remainder of acceptance testing.

## Marketplace publication

1. Confirm the GitHub Release checksum and provenance, then complete every
   clean-profile check above using its VSIX.
2. Open **Actions → Publish VS Code Marketplace → Run workflow** and enter the
   GitHub Release tag that contains the reviewed package.
3. Approve the protected `vscode-marketplace` environment deployment.
4. Confirm the listing is a pre-release for Linux x64, not a universal stable
   release, and review its rendered README, icon, links, license, pricing,
   support details, and installation controls.
5. Install the Marketplace copy into the clean profile and repeat the startup,
   status, completion, and test-discovery smoke checks.

The workflow exchanges GitHub OIDC tokens for short-lived Microsoft Entra
credentials; it requires no PAT or stored publishing credential. It verifies
and publishes the VSIX already attached to the GitHub Release rather than
rebuilding it. See the [release guide](releasing.md) for the one-time identity
configuration.
