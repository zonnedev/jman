#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=platform.sh
source "${project_dir}/scripts/platform.sh"
version="$(tr -d '[:space:]' < "${project_dir}/tools/vineflower/VERSION")"
expected_sha256="$(tr -d '[:space:]' < "${project_dir}/tools/vineflower/SHA256")"
jar="${project_dir}/target/vineflower-${version}.jar"
url="https://repo1.maven.org/maven2/org/vineflower/vineflower/${version}/vineflower-${version}.jar"

if [[ ! -f "${jar}" ]] || [[ "$(jman_sha256_file "${jar}")" != "${expected_sha256}" ]]; then
  mkdir -p "$(dirname "${jar}")"
  curl -L --fail --output "${jar}.tmp" "${url}"
  actual="$(jman_sha256_file "${jar}.tmp")"
  if [[ "${actual}" != "${expected_sha256}" ]]; then
    echo "Vineflower checksum mismatch: ${actual}" >&2
    exit 1
  fi
  mv "${jar}.tmp" "${jar}"
fi
