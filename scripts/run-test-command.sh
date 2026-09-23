#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -eq 0 ]]; then
  echo "usage: $0 COMMAND [ARGUMENT ...]" >&2
  exit 2
fi

test_root=""
cleanup() {
  if [[ -n "${test_root}" ]]; then
    rm -rf -- "${test_root}"
  fi
}
trap cleanup EXIT

test_root="$(mktemp -d "${TMPDIR:-/tmp}/jman-test.XXXXXX")"
mkdir -p "${test_root}/tmp" "${test_root}/cache"
export JMAN_TEST_TEMP_ROOT="${test_root}"
export TMPDIR="${test_root}/tmp"
export XDG_CACHE_HOME="${test_root}/cache"
# A Gradle daemon outlives this wrapper and can recreate its inherited TMPDIR
# after cleanup. Test commands use disposable daemons instead.
export GRADLE_OPTS="${GRADLE_OPTS:+${GRADLE_OPTS} }-Dorg.gradle.daemon=false"

"$@"
