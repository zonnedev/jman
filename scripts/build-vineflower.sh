#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(tr -d '[:space:]' < "${project_dir}/tools/vineflower/VERSION")"
expected_sha256="$(tr -d '[:space:]' < "${project_dir}/tools/vineflower/SHA256")"
jar="${project_dir}/target/vineflower-${version}.jar"
url="https://repo1.maven.org/maven2/org/vineflower/vineflower/${version}/vineflower-${version}.jar"

if [[ ! -f "${jar}" ]] || [[ "$(sha256sum "${jar}" | cut -d' ' -f1)" != "${expected_sha256}" ]]; then
  mkdir -p "$(dirname "${jar}")"
  curl -L --fail --output "${jar}.tmp" "${url}"
  actual="$(sha256sum "${jar}.tmp" | cut -d' ' -f1)"
  if [[ "${actual}" != "${expected_sha256}" ]]; then
    echo "Vineflower checksum mismatch: ${actual}" >&2
    exit 1
  fi
  mv "${jar}.tmp" "${jar}"
fi
