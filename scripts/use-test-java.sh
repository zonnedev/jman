#!/usr/bin/env bash

# Source this file from repository scripts that require the pinned GraalVM
# frontend toolchain. The pinned JMAN bootstrap provisions it on first use.
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_GRAALVM_HOME:-}" ]]; then
  "${project_dir}/scripts/setup-test-jdks.sh" --graalvm >/dev/null
  # shellcheck disable=SC1091
  source "${JMAN_TEST_TOOLCHAINS_DIR:-${project_dir}/target/test-toolchains}/graalvm.env"
fi

for executable in java javac native-image; do
  if [[ ! -x "${JMAN_GRAALVM_HOME}/bin/${executable}" ]]; then
    echo "GraalVM test toolchain is incomplete at ${JMAN_GRAALVM_HOME}: missing bin/${executable}" >&2
    exit 1
  fi
done

export JAVA_HOME="${JMAN_GRAALVM_HOME}"
export PATH="${JAVA_HOME}/bin:${PATH}"
