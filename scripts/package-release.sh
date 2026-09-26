#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=platform.sh
source "${project_dir}/scripts/platform.sh"
native_dir="${project_dir}/target/native"
platform="$(jman_release_platform)" || {
  printf 'JMAN releases do not support this host: %s %s\n' "$(uname -s)" "$(uname -m)" >&2
  exit 1
}
native_library_name="$(jman_native_library_name)"
native_library="${native_dir}/${native_library_name}"
dist_dir="${project_dir}/target/release-dist"
stage_root="${project_dir}/target/release-stage"

if [[ ! -f "${native_library}" ]]; then
  printf 'Native frontend is missing; run make native from %s\n' "${project_dir}" >&2
  exit 1
fi

"${project_dir}/scripts/test-java.sh"
"${project_dir}/scripts/build-maven-importer.sh"
"${project_dir}/scripts/build-processor-worker.sh"
"${project_dir}/scripts/build-vineflower.sh"
"${project_dir}/scripts/build-jacoco.sh"
JMAN_JAVAC_FRONTEND_LIB_DIR="${native_dir}" \
  "${project_dir}/scripts/with-native-library.sh" "${native_dir}" \
  cargo build --manifest-path "${project_dir}/Cargo.toml" --release -p jman-cli

version="$("${project_dir}/scripts/with-native-library.sh" "${native_dir}" \
  "${project_dir}/target/release/jman" --version | awk '{print $2}')"
name="jman-${version}-${platform}"
stage="${stage_root}/${name}"
archive="${dist_dir}/${name}.tar.gz"

rm -rf "${stage_root}" "${dist_dir}"
mkdir -p "${stage}/resources/icons" "${stage}/tools" "${dist_dir}"
cp "${project_dir}/target/release/jman" "${stage}/jman"
cp "${native_library}" "${stage}/${native_library_name}"
cp -R "${native_dir}/platform" "${stage}/platform"
cp "${project_dir}/target/processor-worker.jar" "${stage}/processor-worker.jar"
cp "${project_dir}/target/vineflower-1.12.0.jar" "${stage}/vineflower.jar"
cp "${project_dir}/target/jacoco-0.8.15-agent.jar" "${stage}/jacocoagent.jar"
cp "${project_dir}/target/jacoco-0.8.15-cli.jar" "${stage}/jacococli.jar"
cp "${project_dir}/resources/icons/jman.svg" "${stage}/resources/icons/jman.svg"
cp -R "${project_dir}/tools/gradle-importer" "${stage}/tools/gradle-importer"
cp "${project_dir}/target/maven-importer.jar" "${stage}/maven-importer.jar"
cp "${project_dir}/README.md" "${project_dir}/CHANGELOG.md" \
  "${project_dir}/LICENSE" "${project_dir}/THIRD_PARTY_NOTICES.md" "${stage}/"
cp "${project_dir}/mkdocs.yml" "${stage}/mkdocs.yml"
cp -R "${project_dir}/docs" "${stage}/docs"
if [[ "$(jman_host_os)" == macos ]]; then
  codesign --force --sign - "${stage}/jman" "${stage}/${native_library_name}"
  codesign --verify --strict "${stage}/jman" "${stage}/${native_library_name}"
fi

"${project_dir}/scripts/create-release-archive.py" \
  "${stage_root}" "${name}" "${archive}"
(
  cd "${dist_dir}"
  archive_name="$(basename "${archive}")"
  printf '%s  %s\n' "$(jman_sha256_file "${archive_name}")" "${archive_name}" > SHA256SUMS
  test "$(jman_sha256_file "${archive_name}")" = "$(awk '{ print $1 }' SHA256SUMS)"
)
printf 'Release archive: %s\n' "${archive}"
