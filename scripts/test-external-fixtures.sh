#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/jman-external-fixtures.XXXXXX")"
trap 'rm -rf -- "${test_root}"' EXIT
source_repo="${test_root}/source"
checkout="${test_root}/checkout"
mkdir -p "${source_repo}"
git -C "${source_repo}" init -q
printf 'fixture\n' > "${source_repo}/sample.txt"
git -C "${source_repo}" add sample.txt
git -C "${source_repo}" -c user.name=JMAN -c user.email=jman@example.invalid \
  -c commit.gpgsign=false commit -qm fixture
revision="$(git -C "${source_repo}" rev-parse HEAD)"

result="$("${project_dir}/scripts/ensure-test-repository.sh" "${source_repo}" "${revision}" "${checkout}")"
test "${result}" = "${checkout}"
test "$(git -C "${checkout}" rev-parse HEAD)" = "${revision}"
test "$("${project_dir}/scripts/ensure-test-repository.sh" "${source_repo}" "${revision}" "${checkout}")" = "${checkout}"

printf 'changed\n' > "${checkout}/sample.txt"
if "${project_dir}/scripts/ensure-test-repository.sh" "${source_repo}" "${revision}" "${checkout}" >/dev/null 2>&1; then
  echo 'Dirty test checkout was accepted' >&2
  exit 1
fi
if "${project_dir}/scripts/ensure-test-repository.sh" "${source_repo}" invalid "${test_root}/invalid" >/dev/null 2>&1; then
  echo 'Invalid test revision was accepted' >&2
  exit 1
fi
echo 'External fixture provisioning tests passed'
