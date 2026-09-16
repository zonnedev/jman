#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
extension_dir="${project_dir}/editors/vscode"
server_dir="${extension_dir}/server"
output_dir="${project_dir}/target/vscode"
native_dir="${project_dir}/target/native"
extension_version="$(node -p 'require(process.argv[1]).version' "${extension_dir}/package.json")"

"${project_dir}/scripts/build-native.sh"
"${project_dir}/scripts/build-processor-worker.sh"
"${project_dir}/scripts/build-vineflower.sh"
JAVAC_FRONTEND_LIB_DIR="${native_dir}" \
  CARGO_TARGET_DIR="${project_dir}/target" \
  cargo build \
    --manifest-path "${project_dir}/Cargo.toml" \
    --release \
    -p jman-cli

rm -rf "${server_dir}" "${output_dir}"
mkdir -p "${server_dir}" "${output_dir}"
cp "${project_dir}/target/release/jman" "${server_dir}/jman"
cp "${native_dir}/libjman_javac_frontend.so" "${server_dir}/libjman_javac_frontend.so"
cp "${project_dir}/target/processor-worker.jar" "${server_dir}/processor-worker.jar"
cp "${project_dir}/target/vineflower-1.12.0.jar" "${server_dir}/vineflower.jar"
mkdir -p "${server_dir}/tools"
cp -R "${project_dir}/tools/gradle-importer" "${server_dir}/tools/gradle-importer"
jar --create \
  --date=1980-01-01T00:00:02Z \
  --file "${server_dir}/maven-importer.jar" \
  -C "${project_dir}/target/java-test-classes" \
  io/github/zonnedev/jman/maven/importer

(
  cd "${extension_dir}"
  if [[ ! -d node_modules ]]; then
    npm ci
  fi
  npm run check
  npm run bundle
  npx vsce package \
    --no-dependencies \
    --out "${output_dir}/jman-java-${extension_version}.vsix"
)
