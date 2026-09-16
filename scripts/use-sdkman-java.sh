#!/usr/bin/env bash

# Source this file from repository scripts that require the GraalVM toolchain.
# CI and other non-SDKMAN environments may provide JMAN_GRAALVM_HOME or a
# GraalVM-backed JAVA_HOME. Local development falls back to .sdkmanrc.
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sdkman_root="${SDKMAN_DIR:-${HOME}/.sdkman}"
sdkman_java_version="$(sed -n 's/^java=//p' "${project_dir}/.sdkmanrc")"
sdkman_java_home="${sdkman_root}/candidates/java/${sdkman_java_version}"

if [[ -n "${JMAN_GRAALVM_HOME:-}" ]]; then
  selected_java_home="${JMAN_GRAALVM_HOME}"
elif [[ -n "${JAVA_HOME:-}" && -x "${JAVA_HOME}/bin/native-image" ]]; then
  selected_java_home="${JAVA_HOME}"
else
  selected_java_home="${sdkman_java_home}"
fi

for executable in java javac native-image; do
  if [[ ! -x "${selected_java_home}/bin/${executable}" ]]; then
    echo "GraalVM toolchain is incomplete at ${selected_java_home}: missing bin/${executable}" >&2
    echo "Set JMAN_GRAALVM_HOME, set JAVA_HOME to GraalVM, or install ${sdkman_java_version} with SDKMAN." >&2
    exit 1
  fi
done

export JAVA_HOME="${selected_java_home}"
export PATH="${JAVA_HOME}/bin:${PATH}"
