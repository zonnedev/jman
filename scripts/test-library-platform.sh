#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi

jman_binary="${JMAN_TEST_BINARY:-${project_dir}/target/debug/jman}"
fixture_dir="${project_dir}/examples/library-platform"
work_dir="${JMAN_TEST_TEMP_ROOT}/library-platform"

if [[ ! -x "${jman_binary}" ]]; then
  echo "JMAN test binary is unavailable: ${jman_binary}" >&2
  exit 1
fi
if ! command -v protoc >/dev/null 2>&1; then
  echo 'Library-platform acceptance requires protoc on PATH' >&2
  exit 1
fi

mkdir -p "${work_dir}"
tar -C "${fixture_dir}" --exclude='.jman' -cf - . | tar -C "${work_dir}" -xf -

"${jman_binary}" --no-progress sync "${work_dir}"
"${jman_binary}" --no-progress generate "${work_dir}"
"${jman_binary}" --no-progress generate "${work_dir}"
"${jman_binary}" --no-progress fmt --check "${work_dir}"
"${jman_binary}" --no-progress test "${work_dir}" --source-set unit
"${jman_binary}" --no-progress test "${work_dir}" --source-set integration
"${jman_binary}" --no-progress build "${work_dir}" --all

printf 'Library-platform acceptance passed with native JMAN.\n'
