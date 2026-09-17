#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
matrix_dir="${project_dir}/target/compatibility-matrix"
maven_fixture="${project_dir}/tests/fixtures/compatibility/maven"
gradle_fixture="${project_dir}/tests/fixtures/compatibility/gradle"
maven_bin="${JAVA_LSP_MATRIX_MAVEN:-/home/jfsanchez/.sdkman/candidates/maven/3.9.9/bin/mvn}"

java_homes=(
  "/home/jfsanchez/.sdkman/candidates/java/17.0.20-tem"
  "/home/jfsanchez/.sdkman/candidates/java/21.0.2-open"
  "/home/jfsanchez/.sdkman/candidates/java/25-open"
)
gradle_bins=(
  "${JAVA_LSP_MATRIX_GRADLE_8_7:-/home/jfsanchez/.sdkman/candidates/gradle/8.7/bin/gradle}"
  "/home/jfsanchez/.sdkman/candidates/gradle/8.14.1/bin/gradle"
  "/home/jfsanchez/.sdkman/candidates/gradle/9.1.0/bin/gradle"
)

mkdir -p "${matrix_dir}"
test -x "${maven_bin}"

for java_home in "${java_homes[@]}"; do
  test -x "${java_home}/bin/java"
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
  test -x "${gradle_bin}"
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
