#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
release_tag="${1:-}"
release_dist_dir="${JMAN_RELEASE_DIST_DIR:-${project_dir}/target/release-dist}"
vscode_dist_dir="${JMAN_VSCODE_DIST_DIR:-${project_dir}/target/vscode}"
output_dir="$(realpath -m "${JMAN_GITHUB_RELEASE_DIR:-${project_dir}/target/github-release}")"

if [[ "${output_dir}" != "${project_dir}/target/"* ]]; then
  echo "Release staging directory must be inside ${project_dir}/target: ${output_dir}" >&2
  exit 1
fi

"${project_dir}/scripts/verify-release-version.sh" "${release_tag}"

shopt -s nullglob
cli_archives=("${release_dist_dir}"/jman-*-linux-x86_64.tar.gz)
vscode_packages=("${vscode_dist_dir}"/jman-java-*-linux-x64.vsix)
shopt -u nullglob

if (( ${#cli_archives[@]} != 1 )); then
  echo "Expected exactly one Linux x86-64 CLI archive in ${release_dist_dir}; found ${#cli_archives[@]}" >&2
  exit 1
fi
if (( ${#vscode_packages[@]} != 1 )); then
  echo "Expected exactly one Linux x64 VSIX in ${vscode_dist_dir}; found ${#vscode_packages[@]}" >&2
  exit 1
fi

workspace_version="${release_tag#v}"
extension_version="$(node -p 'require(process.argv[1]).version' "${project_dir}/editors/vscode/package.json")"
cli_name="$(basename "${cli_archives[0]}")"
vscode_name="$(basename "${vscode_packages[0]}")"

if [[ "${cli_name}" != "jman-${workspace_version}-linux-x86_64.tar.gz" ]]; then
  echo "CLI archive name does not match the release version: ${cli_name}" >&2
  exit 1
fi
if [[ "${vscode_name}" != "jman-java-${extension_version}-linux-x64.vsix" ]]; then
  echo "VSIX name does not match package.json: ${vscode_name}" >&2
  exit 1
fi

rm -rf "${output_dir}"
mkdir -p "${output_dir}"
cp "${cli_archives[0]}" "${vscode_packages[0]}" "${output_dir}/"

(
  cd "${output_dir}"
  sha256sum "${cli_name}" "${vscode_name}" > SHA256SUMS
  sha256sum --check SHA256SUMS
)

cli_sha256="$(sha256sum "${output_dir}/${cli_name}" | cut -d' ' -f1)"
vscode_sha256="$(sha256sum "${output_dir}/${vscode_name}" | cut -d' ' -f1)"
cat > "${output_dir}/release-manifest.json" <<EOF
{
  "schemaVersion": 1,
  "releaseTag": "${release_tag}",
  "jmanVersion": "${workspace_version}",
  "vscodeExtensionVersion": "${extension_version}",
  "artifacts": [
    {
      "name": "${cli_name}",
      "platform": "linux-x86_64",
      "sha256": "${cli_sha256}"
    },
    {
      "name": "${vscode_name}",
      "platform": "linux-x64",
      "sha256": "${vscode_sha256}"
    }
  ]
}
EOF

node -e '
  const fs = require("fs");
  const manifest = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
  if (manifest.schemaVersion !== 1 || manifest.artifacts.length !== 2) process.exit(1);
' "${output_dir}/release-manifest.json"

printf 'GitHub release assets staged in %s\n' "${output_dir}"
