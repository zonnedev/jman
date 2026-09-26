#!/usr/bin/env bash

# Source this file from compatibility tests that require JDK 17, 21, and 25.
# The pinned JMAN bootstrap provisions isolated Temurin installations on first use.
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"${project_dir}/scripts/setup-test-jdks.sh" --compatibility >/dev/null
# shellcheck disable=SC1091
source "${JMAN_TEST_TOOLCHAINS_DIR:-${project_dir}/target/test-toolchains}/compatibility.env"

for variable in JMAN_TEST_JAVA_17_HOME JMAN_TEST_JAVA_21_HOME JMAN_TEST_JAVA_25_HOME; do
  home="${!variable}"
  if [[ ! -x "${home}/bin/java" || ! -x "${home}/bin/javac" ]]; then
    echo "JMAN compatibility toolchain is incomplete: ${variable}=${home}" >&2
    exit 1
  fi
  export "${variable}"
done
