#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
workspace_version="$({
  awk '
    $0 == "[workspace.package]" { in_workspace_package = 1; next }
    /^\[/ { in_workspace_package = 0 }
    in_workspace_package && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' "${project_dir}/Cargo.toml"
})"
extension_version="$(node -p 'require(process.argv[1]).version' "${project_dir}/editors/vscode/package.json")"
release_tag="v${workspace_version}"
test_root="${project_dir}/target/release-automation-test"
release_dist_dir="${test_root}/release-dist"
vscode_dist_dir="${test_root}/vscode"
output_dir="${test_root}/github-release"
workflow_dir="${project_dir}/.github/workflows"

rm -rf "${test_root}"
mkdir -p "${release_dist_dir}" "${vscode_dist_dir}"
trap 'rm -rf "${test_root}"' EXIT

printf 'cli fixture\n' > "${release_dist_dir}/jman-${workspace_version}-linux-x86_64.tar.gz"
printf 'vsix fixture\n' > "${vscode_dist_dir}/jman-java-${extension_version}-linux-x64.vsix"

"${project_dir}/scripts/verify-release-version.sh" "${release_tag}"
if "${project_dir}/scripts/verify-release-version.sh" "v0.0.0-release-test" >/dev/null 2>&1; then
  echo "Mismatched release tag was accepted" >&2
  exit 1
fi
if "${project_dir}/scripts/verify-release-version.sh" >/dev/null 2>&1; then
  echo "Missing release tag was accepted" >&2
  exit 1
fi
if JMAN_RELEASE_DIST_DIR="${release_dist_dir}" \
  JMAN_VSCODE_DIST_DIR="${vscode_dist_dir}" \
  JMAN_GITHUB_RELEASE_DIR="/tmp/jman-release-automation-unsafe" \
  "${project_dir}/scripts/stage-release-artifacts.sh" "${release_tag}" >/dev/null 2>&1; then
  echo "Release staging outside the project target directory was accepted" >&2
  exit 1
fi

JMAN_RELEASE_DIST_DIR="${release_dist_dir}" \
JMAN_VSCODE_DIST_DIR="${vscode_dist_dir}" \
JMAN_GITHUB_RELEASE_DIR="${output_dir}" \
  "${project_dir}/scripts/stage-release-artifacts.sh" "${release_tag}"

(
  cd "${output_dir}"
  sha256sum --check SHA256SUMS
)
node -e '
  const fs = require("fs");
  const manifest = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
  if (manifest.releaseTag !== process.argv[2]) throw new Error("release tag mismatch");
  if (manifest.artifacts.length !== 2) throw new Error("artifact count mismatch");
  for (const artifact of manifest.artifacts) {
    if (!/^[0-9a-f]{64}$/.test(artifact.sha256)) throw new Error("invalid digest");
  }
' "${output_dir}/release-manifest.json" "${release_tag}"

test "$(wc -l < "${output_dir}/SHA256SUMS")" -eq 2
cp "${release_dist_dir}/jman-${workspace_version}-linux-x86_64.tar.gz" \
  "${release_dist_dir}/jman-duplicate-linux-x86_64.tar.gz"
if JMAN_RELEASE_DIST_DIR="${release_dist_dir}" \
  JMAN_VSCODE_DIST_DIR="${vscode_dist_dir}" \
  JMAN_GITHUB_RELEASE_DIR="${output_dir}" \
  "${project_dir}/scripts/stage-release-artifacts.sh" "${release_tag}" >/dev/null 2>&1; then
  echo "Duplicate CLI archives were accepted" >&2
  exit 1
fi

for workflow in ci.yml release.yml publish-vscode.yml verify-vscode-marketplace-identity.yml; do
  test -s "${workflow_dir}/${workflow}"
done
while IFS= read -r action_reference; do
  if [[ ! "${action_reference}" =~ @[0-9a-f]{40}$ ]]; then
    echo "GitHub Action is not pinned to a full commit: ${action_reference}" >&2
    exit 1
  fi
done < <(sed -n 's/^[[:space:]]*uses:[[:space:]]*\([^ #]*\).*/\1/p' "${workflow_dir}"/*.yml)
grep -q 'azure/login@532459ea530d8321f2fb9bb10d1e0bcf23869a43' \
  "${workflow_dir}/publish-vscode.yml"
grep -q -- '--azure-credential' "${workflow_dir}/publish-vscode.yml"
grep -q 'environment: vscode-marketplace' \
  "${workflow_dir}/verify-vscode-marketplace-identity.yml"
grep -q '499b84ac-1321-427f-aa17-267ca6975798' \
  "${workflow_dir}/verify-vscode-marketplace-identity.yml"
grep -q 'vsce verify-pat --azure-credential zonnedev' \
  "${workflow_dir}/verify-vscode-marketplace-identity.yml"
if grep -R -q 'VSCE_PAT' "${workflow_dir}"; then
  echo "Marketplace workflow must not use a long-lived PAT" >&2
  exit 1
fi

printf 'Release automation tests passed\n'
