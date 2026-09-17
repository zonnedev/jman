#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 2 ]]; then
  echo "usage: import-gradle-project.sh PROJECT_ROOT OUTPUT_NDJSON" >&2
  exit 2
fi

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
workspace="$(cd "$1" && pwd)"
output="$2"
gradle_user_home="${JAVA_LSP_GRADLE_USER_HOME:-${project_dir}/target/gradle-user-home}"
gradle_project_cache="${JAVA_LSP_GRADLE_PROJECT_CACHE_DIR:-${project_dir}/target/gradle-project-cache}"
build_java_home="${JAVA_LSP_BUILD_JAVA_HOME:-${JAVA_HOME:-/home/jfsanchez/.sdkman/candidates/java/25.0.4-graal}}"
gradle="${workspace}/gradlew"

if [[ ! -x "${gradle}" ]]; then
  gradle="${JAVA_LSP_GRADLE:-gradle}"
fi

mkdir -p "$(dirname "${output}")" "${gradle_user_home}" "${gradle_project_cache}"
(
  cd "${workspace}"
  export GRADLE_USER_HOME="${gradle_user_home}"
  if [[ -x "${build_java_home}/bin/java" ]]; then
    export JAVA_HOME="${build_java_home}"
    export PATH="${JAVA_HOME}/bin:${PATH}"
  fi
  "${gradle}" \
    --console=plain \
    --no-configuration-cache \
    --project-cache-dir "${gradle_project_cache}" \
    -I "${project_dir}/tools/gradle-importer/javac-frontend-model.init.gradle" \
    javaFrontendModel
) | sed -n 's/^JAVAC_FRONTEND_MODEL //p' > "${output}"

test -s "${output}"
