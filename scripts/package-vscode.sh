#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=platform.sh
source "${project_dir}/scripts/platform.sh"
extension_dir="${project_dir}/editors/vscode"
server_dir="${extension_dir}/server"
output_dir="${project_dir}/target/vscode"
native_dir="${project_dir}/target/native"
target_platform="$(jman_vscode_target)" || {
  printf 'JMAN Java does not support this host: %s %s\n' "$(uname -s)" "$(uname -m)" >&2
  exit 1
}
native_library_name="$(jman_native_library_name)"
native_library="${native_dir}/${native_library_name}"
extension_version="$(node -p 'require(process.argv[1]).version' "${extension_dir}/package.json")"
release_tag="${JMAN_RELEASE_TAG:-}"
channel="stable"
package_flags=(
  --target "${target_platform}"
  --no-dependencies
  --out "${output_dir}/jman-java-${extension_version}-${target_platform}.vsix"
)
if [[ "${release_tag#v}" == *-* ]]; then
  channel="pre-release"
  package_flags+=(--pre-release)
fi

if [[ ! -f "${native_library}" ]]; then
  printf 'Native frontend is missing; run make native from %s\n' "${project_dir}" >&2
  exit 1
fi

"${project_dir}/scripts/build-processor-worker.sh"
"${project_dir}/scripts/build-maven-importer.sh"
"${project_dir}/scripts/build-vineflower.sh"
"${project_dir}/scripts/build-jacoco.sh"
JMAN_JAVAC_FRONTEND_LIB_DIR="${native_dir}" \
  CARGO_TARGET_DIR="${project_dir}/target" \
  cargo build \
    --manifest-path "${project_dir}/Cargo.toml" \
    --release \
    -p jman-cli

rm -rf "${server_dir}" "${output_dir}"
mkdir -p "${server_dir}" "${output_dir}"
cp "${project_dir}/target/release/jman" "${server_dir}/jman"
if [[ "$(jman_host_os)" == macos ]]; then
  strip -x "${server_dir}/jman"
else
  strip --strip-unneeded "${server_dir}/jman"
fi
cp "${native_library}" "${server_dir}/${native_library_name}"
if [[ "$(jman_host_os)" == macos ]]; then
  codesign --force --sign - "${server_dir}/jman" "${server_dir}/${native_library_name}"
  codesign --verify --strict "${server_dir}/jman" "${server_dir}/${native_library_name}"
fi
cp -R "${native_dir}/platform" "${server_dir}/platform"
cp "${project_dir}/target/processor-worker.jar" "${server_dir}/processor-worker.jar"
cp "${project_dir}/target/vineflower-1.12.0.jar" "${server_dir}/vineflower.jar"
cp "${project_dir}/target/jacoco-0.8.15-agent.jar" "${server_dir}/jacocoagent.jar"
cp "${project_dir}/target/jacoco-0.8.15-cli.jar" "${server_dir}/jacococli.jar"
mkdir -p "${server_dir}/tools"
cp -R "${project_dir}/tools/gradle-importer" "${server_dir}/tools/gradle-importer"
cp "${project_dir}/target/maven-importer.jar" "${server_dir}/maven-importer.jar"

(
  cd "${extension_dir}"
  npm ci
  npx vsce package "${package_flags[@]}"
)

package="${output_dir}/jman-java-${extension_version}-${target_platform}.vsix"
checksum="${package}.sha256"

unzip -tq "${package}"
(
  cd "${output_dir}"
  package_name="$(basename "${package}")"
  printf '%s  %s\n' "$(jman_sha256_file "${package_name}")" "${package_name}" \
    > "$(basename "${checksum}")"
  test "$(jman_sha256_file "${package_name}")" = "$(awk '{ print $1 }' "$(basename "${checksum}")")"
)

printf 'VS Code %s package: %s\n' "${channel}" "${package}"
printf 'SHA-256 checksum: %s\n' "${checksum}"
