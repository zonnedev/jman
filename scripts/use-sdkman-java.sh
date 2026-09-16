#!/usr/bin/env bash

# Source this file from repository scripts that require the GraalVM toolchain.
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sdkman_root="${SDKMAN_DIR:-/home/jfsanchez/.sdkman}"
sdkman_java_version="$(sed -n 's/^java=//p' "${project_dir}/.sdkmanrc")"
sdkman_java_home="${sdkman_root}/candidates/java/${sdkman_java_version}"

if [[ -z "${sdkman_java_version}" || ! -x "${sdkman_java_home}/bin/java" ]]; then
  echo "SDKMAN Java from .sdkmanrc is not installed: ${sdkman_java_version}" >&2
  exit 1
fi

export JAVA_HOME="${sdkman_java_home}"
export PATH="${JAVA_HOME}/bin:${PATH}"
