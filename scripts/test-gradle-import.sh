#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="${JAVAC_FRONTEND_PETCLINIC:-/home/jfsanchez/zonnedev/tmp/test/spring-petclinic}"
output="${project_dir}/target/gradle-petclinic-model.ndjson"
fixture_copy="${project_dir}/target/integration-fixtures/spring-petclinic"
gradle_user_home="${project_dir}/target/gradle-user-home"
project_jdk="${JAVAC_FRONTEND_PROJECT_JDK:-/home/jfsanchez/.sdkman/candidates/java/17.0.20-tem}"

rm -rf "${fixture_copy}"
mkdir -p "$(dirname "${fixture_copy}")" "${gradle_user_home}"
cp -a "${fixture}/." "${fixture_copy}/"
(
  cd "${fixture_copy}"
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
