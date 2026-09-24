#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi
# shellcheck source=use-test-jdks.sh
source "${project_dir}/scripts/use-test-jdks.sh"
matrix_dir="${project_dir}/target/compatibility-matrix"
maven_fixture="${project_dir}/tests/fixtures/compatibility/maven"
gradle_fixture="${project_dir}/tests/fixtures/compatibility/gradle"
tools_dir="${project_dir}/target/compatibility-tools"
maven_bin="${JAVA_LSP_MATRIX_MAVEN:-${tools_dir}/apache-maven-3.9.9/bin/mvn}"

java_homes=(
  "${JMAN_TEST_JAVA_17_HOME}"
  "${JMAN_TEST_JAVA_21_HOME}"
  "${JMAN_TEST_JAVA_25_HOME}"
)
gradle_bins=(
  "${JAVA_LSP_MATRIX_GRADLE_8_7:-${tools_dir}/gradle-8.7/bin/gradle}"
  "${JAVA_LSP_MATRIX_GRADLE_8_14_1:-${tools_dir}/gradle-8.14.1/bin/gradle}"
  "${JAVA_LSP_MATRIX_GRADLE_9_1_0:-${tools_dir}/gradle-9.1.0/bin/gradle}"
)

mkdir -p "${matrix_dir}"
if [[ ! -x "${maven_bin}" ]]; then
  echo "Maven 3.9.9 is unavailable: ${maven_bin} (set JAVA_LSP_MATRIX_MAVEN)" >&2
  exit 1
fi

for java_home in "${java_homes[@]}"; do
  if [[ ! -x "${java_home}/bin/java" ]]; then
    echo "Matrix JDK is unavailable: ${java_home} (set JMAN_TEST_JAVA_{17,21,25}_HOME)" >&2
    exit 1
  fi
  release="$("${java_home}/bin/java" -version 2>&1 | sed -n '1s/.*version "\([0-9][0-9]*\).*/\1/p')"
  output="${matrix_dir}/maven-jdk-${release}.ndjson"
  JAVA_LSP_MAVEN="${maven_bin}" \
  JAVA_LSP_BUILD_JAVA_HOME="${java_home}" \
  JAVA_LSP_MAVEN_REPOSITORY="${project_dir}/target/maven-repository" \
    "${project_dir}/scripts/import-maven-project.sh" "${maven_fixture}" "${output}"
  jq -e '
    select(.projectPath == "compatibility-maven" and .taskPath == "compile")
    | .release == "17"
      and (.sourceFiles | any(endswith("/module-info.java")))
      and (.modulePath | type == "array")
  ' "${output}" >/dev/null
done

for gradle_bin in "${gradle_bins[@]}"; do
  if [[ ! -x "${gradle_bin}" ]]; then
    echo "Matrix Gradle is unavailable: ${gradle_bin} (set JAVA_LSP_MATRIX_GRADLE_*)" >&2
    exit 1
  fi
  gradle_version="$(
    JAVA_HOME="${java_homes[1]}" \
    PATH="${java_homes[1]}/bin:${PATH}" \
      "${gradle_bin}" --version | sed -n 's/^Gradle //p'
  )"
  for java_home in "${java_homes[@]}"; do
    release="$("${java_home}/bin/java" -version 2>&1 | sed -n '1s/.*version "\([0-9][0-9]*\).*/\1/p')"
    if [[ "${release}" == 25 && "${gradle_version}" != 9.1.0 ]]; then
      continue
    fi
    output="${matrix_dir}/gradle-${gradle_version}-jdk-${release}.ndjson"
    JAVA_LSP_GRADLE="${gradle_bin}" \
    JAVA_LSP_BUILD_JAVA_HOME="${java_home}" \
      "${project_dir}/scripts/import-gradle-project.sh" "${gradle_fixture}" "${output}"
    jq -e '
      select(.projectPath == ":" and .taskPath == ":compileJava")
      | .javaLanguageVersion == 17
        and (.javaCompilerExecutable | endswith("/bin/javac"))
        and (.projectDependencies | index(":model") != null)
        and (.sourceFiles | any(endswith("/module-info.java")))
        and (.modulePath | type == "array")
        and (.resolutionErrors | length == 0)
    ' "${output}" >/dev/null
  done
done

echo "Compatibility matrix passed: Maven 3.9.9 and Gradle 8.7/8.14.1/9.1.0 on their supported JDK 17/21/25 cells"
