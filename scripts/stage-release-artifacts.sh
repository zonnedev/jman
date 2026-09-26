#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=platform.sh
source "${project_dir}/scripts/platform.sh"
release_tag="${1:-}"
release_dist_dir="${JMAN_RELEASE_DIST_DIR:-${project_dir}/target/release-dist}"
vscode_dist_dir="${JMAN_VSCODE_DIST_DIR:-${project_dir}/target/vscode}"
output_dir="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).resolve())' \
  "${JMAN_GITHUB_RELEASE_DIR:-${project_dir}/target/github-release}")"

if [[ "${output_dir}" != "${project_dir}/target/"* ]]; then
  echo "Release staging directory must be inside ${project_dir}/target: ${output_dir}" >&2
  exit 1
fi

"${project_dir}/scripts/verify-release-version.sh" "${release_tag}"
workspace_version="${release_tag#v}"
extension_version="$(node -p 'require(process.argv[1]).version' "${project_dir}/editors/vscode/package.json")"

shopt -s nullglob
cli_archives=("${release_dist_dir}"/jman-"${workspace_version}"-*.tar.gz)
vscode_packages=("${vscode_dist_dir}"/jman-java-"${extension_version}"-*.vsix)
shopt -u nullglob

if (( ${#cli_archives[@]} == 0 )); then
  echo "No CLI archives found in ${release_dist_dir}" >&2
  exit 1
fi
if (( ${#vscode_packages[@]} != ${#cli_archives[@]} )); then
  echo "Every CLI platform must have exactly one VSIX: ${#cli_archives[@]} CLI, ${#vscode_packages[@]} VSIX" >&2
  exit 1
fi

rm -rf "${output_dir}"
mkdir -p "${output_dir}"
cp "${project_dir}/install.sh" "${output_dir}/"

artifact_names=()
artifact_platforms=()
for cli_archive in "${cli_archives[@]}"; do
  cli_name="$(basename "${cli_archive}")"
  release_platform="${cli_name#jman-${workspace_version}-}"
  release_platform="${release_platform%.tar.gz}"
  case "${release_platform}" in
    linux-x86_64) vscode_platform=linux-x64 ;;
    macos-aarch64) vscode_platform=darwin-arm64 ;;
    *)
      echo "Unsupported release platform in ${cli_name}" >&2
      exit 1
      ;;
  esac
  vscode_name="jman-java-${extension_version}-${vscode_platform}.vsix"
  vscode_package="${vscode_dist_dir}/${vscode_name}"
  if [[ ! -f "${vscode_package}" ]]; then
    echo "Missing ${vscode_platform} VSIX for ${cli_name}" >&2
    exit 1
  fi
  cp "${cli_archive}" "${vscode_package}" "${output_dir}/"
  artifact_names+=("${cli_name}" "${vscode_name}")
  artifact_platforms+=("${release_platform}" "${vscode_platform}")
done
artifact_names+=(install.sh)
artifact_platforms+=(portable-shell)

(
  cd "${output_dir}"
  : > SHA256SUMS
  for artifact in "${artifact_names[@]}"; do
    printf '%s  %s\n' "$(jman_sha256_file "${artifact}")" "${artifact}" >> SHA256SUMS
  done
  while read -r expected artifact; do
    test "$(jman_sha256_file "${artifact}")" = "${expected}"
  done < SHA256SUMS
)

{
  cat <<EOF
{
  "schemaVersion": 1,
  "releaseTag": "${release_tag}",
  "jmanVersion": "${workspace_version}",
  "vscodeExtensionVersion": "${extension_version}",
  "artifacts": [
EOF
  for index in "${!artifact_names[@]}"; do
    artifact="${artifact_names[index]}"
    platform="${artifact_platforms[index]}"
    sha256="$(jman_sha256_file "${output_dir}/${artifact}")"
    if (( index > 0 )); then
      printf ',\n'
    fi
    printf '    {"name":"%s","platform":"%s","sha256":"%s"}' \
      "${artifact}" "${platform}" "${sha256}"
  done
  cat <<'EOF'

  ]
}
EOF
} > "${output_dir}/release-manifest.json"

node -e '
  const fs = require("fs");
  const manifest = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
  if (manifest.schemaVersion !== 1 || manifest.artifacts.length < 3) process.exit(1);
  const platforms = new Set(manifest.artifacts.map(artifact => artifact.platform));
  if (!platforms.has("portable-shell")) process.exit(1);
' "${output_dir}/release-manifest.json"

printf 'GitHub release assets staged in %s\n' "${output_dir}"
