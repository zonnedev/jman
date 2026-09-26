#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi
test_root=""
cleanup() {
  if [[ -n "${test_root}" ]]; then
    rm -rf -- "${test_root}"
  fi
}
trap cleanup EXIT
test_root="$(mktemp -d "${TMPDIR:-/tmp}/jman-external-fixtures.XXXXXX")"

while IFS= read -r test_script; do
  if ! grep -Fq 'scripts/run-test-command.sh' "${test_script}"; then
    echo "Test helper does not isolate temporary resources: ${test_script}" >&2
    exit 1
  fi
done < <(find "${project_dir}/scripts" -maxdepth 1 -type f -name 'test-*.sh' | sort)

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
missing_revision="0000000000000000000000000000000000000000"
if "${project_dir}/scripts/ensure-test-repository.sh" \
  "${source_repo}" "${missing_revision}" "${test_root}/missing" >/dev/null 2>&1; then
  echo 'Missing test revision was accepted' >&2
  exit 1
fi
if compgen -G "${test_root}/.test-repository.*" >/dev/null; then
  echo 'Failed test repository fetch left a staging directory behind' >&2
  exit 1
fi

compatibility_fixture="${test_root}/compatibility-tools"
fake_bin="${compatibility_fixture}/fake-bin"
curl_log="${compatibility_fixture}/curl.log"
mkdir -p "${compatibility_fixture}/scripts" "${fake_bin}"
cp "${project_dir}/scripts/setup-compatibility-tools.sh" "${compatibility_fixture}/scripts/"
cat > "${fake_bin}/curl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${JMAN_FAKE_CURL_LOG:?}"
exit 1
EOF
chmod +x "${fake_bin}/curl"
if JMAN_FAKE_CURL_LOG="${curl_log}" PATH="${fake_bin}:${PATH}" \
  "${compatibility_fixture}/scripts/setup-compatibility-tools.sh" >/dev/null 2>&1; then
  echo 'Compatibility tool setup unexpectedly succeeded with a failing download' >&2
  exit 1
fi
for expected_option in \
  '--connect-timeout 30' \
  '--retry 5' \
  '--retry-all-errors' \
  '--retry-delay 2' \
  '--retry-max-time 180' \
  '--remove-on-error'; do
  if ! grep -Fq -- "${expected_option}" "${curl_log}"; then
    echo "Compatibility tool download omitted resilient curl option: ${expected_option}" >&2
    exit 1
  fi
done
grep -Fq 'archive.apache.org' "${curl_log}"
grep -Fq 'repo.maven.apache.org' "${curl_log}"
if compgen -G "${compatibility_fixture}/target/compatibility-tools/.maven-*" >/dev/null; then
  echo 'Failed compatibility tool download left a staging directory behind' >&2
  exit 1
fi

wrapper_parent="${test_root}/wrapper-parent"
wrapper_probe="${test_root}/wrapper-probe.sh"
mkdir -p "${wrapper_parent}"
cat > "${wrapper_probe}" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
test -n "${JMAN_TEST_TEMP_ROOT:-}"
touch "${TMPDIR}/temporary-file"
mkdir -p "${XDG_CACHE_HOME}/temporary-cache"
exit "${1:-0}"
EOF
chmod +x "${wrapper_probe}"
TMPDIR="${wrapper_parent}" \
  "${project_dir}/scripts/run-test-command.sh" "${wrapper_probe}"
if compgen -G "${wrapper_parent}/*" >/dev/null; then
  echo 'Successful test command left temporary resources behind' >&2
  exit 1
fi
if TMPDIR="${wrapper_parent}" \
  "${project_dir}/scripts/run-test-command.sh" "${wrapper_probe}" 7; then
  echo 'Failing test command unexpectedly succeeded' >&2
  exit 1
fi
if compgen -G "${wrapper_parent}/*" >/dev/null; then
  echo 'Failed test command left temporary resources behind' >&2
  exit 1
fi
echo 'External fixture provisioning tests passed'
