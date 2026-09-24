#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi
# shellcheck source=use-test-jdks.sh
source "${project_dir}/scripts/use-test-jdks.sh"
fixture="${JMAN_JAVAC_FRONTEND_PETCLINIC:-}"
if [[ -z "${fixture}" ]]; then
  fixture="$("${project_dir}/scripts/ensure-test-repository.sh" \
    https://github.com/spring-projects/spring-petclinic.git \
    f182358d02e4a68e52bdbabf55ca7800288511e7 \
    "${project_dir}/target/upstream-fixtures/spring-petclinic")"
fi
output="${project_dir}/target/gradle-petclinic-model.ndjson"
fixture_copy="${project_dir}/target/integration-fixtures/spring-petclinic"
gradle_user_home="${project_dir}/target/gradle-user-home"
project_jdk="${JMAN_JAVAC_FRONTEND_PROJECT_JDK:-${JMAN_TEST_JAVA_17_HOME}}"

rm -rf "${fixture_copy}"
mkdir -p "$(dirname "${fixture_copy}")" "${gradle_user_home}"
cp -a "${fixture}/." "${fixture_copy}/"
(
  cd "${fixture_copy}"
  export JAVA_HOME="${project_jdk}"
  export PATH="${JAVA_HOME}/bin:${PATH}"
  export GRADLE_USER_HOME="${gradle_user_home}"
  ./gradlew \
    --console=plain \
    --no-configuration-cache \
    "-Dorg.gradle.java.installations.paths=${project_jdk}" \
    -I "${project_dir}/tools/gradle-importer/javac-frontend-model.init.gradle" \
    javaFrontendModel
) | sed -n 's/^JAVAC_FRONTEND_MODEL //p' > "${output}"

if [[ ! -s "${output}" ]]; then
  echo "Gradle importer emitted no Java compile models" >&2
  exit 1
fi

grep -q '"taskPath":":compileJava"' "${output}"
grep -q '"javaLanguageVersion":17' "${output}"
grep -q '"annotationProcessorPath":' "${output}"
