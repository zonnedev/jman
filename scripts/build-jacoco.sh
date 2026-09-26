#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=platform.sh
source "${project_dir}/scripts/platform.sh"
version="$(tr -d '[:space:]' < "${project_dir}/tools/jacoco/VERSION")"
agent_sha256="$(tr -d '[:space:]' < "${project_dir}/tools/jacoco/AGENT_SHA256")"
cli_sha256="$(tr -d '[:space:]' < "${project_dir}/tools/jacoco/CLI_SHA256")"
repository="https://repo.maven.apache.org/maven2/org/jacoco"

download() {
  local name=$1
  local expected=$2
  local url=$3
  local destination="${project_dir}/target/jacoco-${version}-${name}.jar"

  if [[ -f "${destination}" ]] &&
    [[ "$(jman_sha256_file "${destination}")" == "${expected}" ]]; then
    return
  fi
  mkdir -p "$(dirname "${destination}")"
  curl --fail --location --silent --show-error --output "${destination}.tmp" "${url}"
  local actual
  actual="$(jman_sha256_file "${destination}.tmp")"
  if [[ "${actual}" != "${expected}" ]]; then
    rm -f "${destination}.tmp"
    printf 'JaCoCo %s checksum mismatch: expected %s, got %s\n' \
      "${name}" "${expected}" "${actual}" >&2
    exit 1
  fi
  mv "${destination}.tmp" "${destination}"
}

download agent "${agent_sha256}" \
  "${repository}/org.jacoco.agent/${version}/org.jacoco.agent-${version}-runtime.jar"
download cli "${cli_sha256}" \
  "${repository}/org.jacoco.cli/${version}/org.jacoco.cli-${version}-nodeps.jar"
