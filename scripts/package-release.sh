#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
native_dir="${project_dir}/target/native"
native_library="${native_dir}/libjman_javac_frontend.so"
dist_dir="${project_dir}/target/release-dist"
stage_root="${project_dir}/target/release-stage"

if [[ ! -f "${native_library}" ]]; then
  printf 'Native frontend is missing; run make native from %s\n' "${project_dir}" >&2
  exit 1
fi

"${project_dir}/scripts/test-java.sh"
"${project_dir}/scripts/build-processor-worker.sh"
"${project_dir}/scripts/build-vineflower.sh"
JAVAC_FRONTEND_LIB_DIR="${native_dir}" \
  LD_LIBRARY_PATH="${native_dir}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" \
  cargo build --manifest-path "${project_dir}/Cargo.toml" --release -p jman-cli

version="$(LD_LIBRARY_PATH="${native_dir}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" \
  "${project_dir}/target/release/jman" --version | awk '{print $2}')"
case "$(uname -m)" in
  x86_64) architecture="x86_64" ;;
  aarch64|arm64) architecture="aarch64" ;;
  *) architecture="$(uname -m)" ;;
esac
platform="$(uname -s | tr '[:upper:]' '[:lower:]')-${architecture}"
name="jman-${version}-${platform}"
stage="${stage_root}/${name}"
archive="${dist_dir}/${name}.tar.gz"

rm -rf "${stage_root}" "${dist_dir}"
mkdir -p "${stage}/resources/icons" "${stage}/tools" "${dist_dir}"
cp "${project_dir}/target/release/jman" "${stage}/jman"
cp "${native_library}" "${stage}/libjman_javac_frontend.so"
cp "${project_dir}/target/processor-worker.jar" "${stage}/processor-worker.jar"
cp "${project_dir}/target/vineflower-1.12.0.jar" "${stage}/vineflower.jar"
cp "${project_dir}/resources/icons/jman.svg" "${stage}/resources/icons/jman.svg"
cp -R "${project_dir}/tools/gradle-importer" "${stage}/tools/gradle-importer"
jar --create \
  --date=1980-01-01T00:00:02Z \
  --file "${stage}/maven-importer.jar" \
  -C "${project_dir}/target/java-test-classes" \
  io/github/zonnedev/jman/maven/importer
cp "${project_dir}/README.md" "${project_dir}/CHANGELOG.md" \
  "${project_dir}/LICENSE" "${stage}/"

tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
  -C "${stage_root}" -cf - "${name}" | gzip -n > "${archive}"
(
  cd "${dist_dir}"
  sha256sum "$(basename "${archive}")" > SHA256SUMS
  sha256sum --check SHA256SUMS
)
printf 'Release archive: %s\n' "${archive}"
