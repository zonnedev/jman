#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
release_tag="${1:-}"

if [[ -z "${release_tag}" ]]; then
  echo "usage: $0 v<workspace-version>" >&2
  exit 2
fi

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

if [[ -z "${workspace_version}" ]]; then
  echo "Could not read workspace.package.version from Cargo.toml" >&2
  exit 1
fi

expected_tag="v${workspace_version}"
if [[ "${release_tag}" != "${expected_tag}" ]]; then
  echo "Release tag ${release_tag} does not match workspace version ${workspace_version}; expected ${expected_tag}" >&2
  exit 1
fi

printf 'Release version verified: %s\n' "${release_tag}"
