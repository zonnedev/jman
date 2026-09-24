#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi
# shellcheck source=use-test-jdks.sh
source "${project_dir}/scripts/use-test-jdks.sh"
petclinic_fixture="${JAVAC_FRONTEND_PETCLINIC:-}"
gson_fixture="${JAVAC_FRONTEND_GSON:-}"
if [[ -z "${petclinic_fixture}" ]]; then
  petclinic_fixture="$("${project_dir}/scripts/ensure-test-repository.sh" \
    https://github.com/spring-projects/spring-petclinic.git \
    f182358d02e4a68e52bdbabf55ca7800288511e7 \
    "${project_dir}/target/upstream-fixtures/spring-petclinic")"
fi
if [[ -z "${gson_fixture}" ]]; then
  gson_fixture="$("${project_dir}/scripts/ensure-test-repository.sh" \
    https://github.com/google/gson.git \
    aebc51a56ca0793c13b841c29f73433b82446695 \
    "${project_dir}/target/upstream-fixtures/gson")"
fi
local_repository="${project_dir}/target/maven-repository"
classes_dir="${project_dir}/target/java-test-classes"
maven_java_home="${JAVAC_FRONTEND_MAVEN_JAVA_HOME:-${JMAN_TEST_JAVA_17_HOME}}"
java_21_home="${JMAN_TEST_JAVA_21_HOME}"
maven_bin="${JAVA_LSP_MATRIX_MAVEN:-${project_dir}/target/compatibility-tools/apache-maven-3.9.9/bin/mvn}"

if [[ ! -x "${maven_java_home}/bin/java" ]]; then
  echo "Maven integration JDK 17 is not installed: ${maven_java_home}" >&2
  exit 1
fi
if [[ ! -x "${maven_bin}" ]]; then
  echo "Maven 3.9.9 is unavailable: ${maven_bin} (set JAVA_LSP_MATRIX_MAVEN)" >&2
  exit 1
fi

import_project() {
  local fixture="$1"
  local name="$2"
  local classpath_project="${3:-}"
  local project_java_home="${4:-${maven_java_home}}"
  local fixture_copy="${project_dir}/target/integration-fixtures/${name}"
  local model_output="${project_dir}/target/${name}-model.ndjson"

  rm -rf "${fixture_copy}"
  mkdir -p "${fixture_copy}" "${local_repository}"
  cp -a "${fixture}/." "${fixture_copy}/"

  (
    cd "${fixture_copy}"
    export JAVA_HOME="${project_java_home}"
    export PATH="${JAVA_HOME}/bin:${PATH}"
    "${maven_bin}" \
      --batch-mode \
      --no-transfer-progress \
      -q \
      "-Dmaven.repo.local=${local_repository}" \
      -Denforcer.skip=true \
      -DskipTests \
      help:effective-pom \
      -Doutput=target/javac-frontend-effective-pom.xml
    if [[ -n "${classpath_project}" ]]; then
      "${maven_bin}" \
        --batch-mode \
        --no-transfer-progress \
        -q \
        "-Dmaven.repo.local=${local_repository}" \
        -Denforcer.skip=true \
        -DskipTests \
        -pl "${classpath_project}" \
        dependency:build-classpath \
        -Dmdep.outputFile=target/javac-frontend-classpath.txt
    else
      "${maven_bin}" \
        --batch-mode \
        --no-transfer-progress \
        -q \
        "-Dmaven.repo.local=${local_repository}" \
        -Denforcer.skip=true \
        -DskipTests \
      dependency:build-classpath \
      -Dmdep.outputFile=target/javac-frontend-classpath.txt
    fi
  )

  : > "${model_output}"
  while IFS= read -r effective_pom; do
    local module_target
    local classpath_file
    module_target="$(dirname "${effective_pom}")"
    classpath_file="${module_target}/javac-frontend-classpath.txt"
    java -cp "${classes_dir}" \
      io.github.zonnedev.jman.maven.importer.MavenModelImporter \
      "${effective_pom}" \
      "${classpath_file}" \
      "${local_repository}" >> "${model_output}"
  done < <(find "${fixture_copy}" -path '*/target/javac-frontend-effective-pom.xml' -print | sort)

  test -s "${model_output}"
  grep -q '"schemaVersion":2' "${model_output}"
  grep -q '"sourceRoots":' "${model_output}"
  grep -q '"generatedSourceDirectories":' "${model_output}"
  grep -q '"buildSystem":"maven"' "${model_output}"
}

import_project "${petclinic_fixture}" "spring-petclinic-maven"
grep -q '"projectPath":"spring-petclinic"' "${project_dir}/target/spring-petclinic-maven-model.ndjson"
grep -q '"release":"17"' "${project_dir}/target/spring-petclinic-maven-model.ndjson"
grep -q 'spring-context-' "${project_dir}/target/spring-petclinic-maven-model.ndjson"

import_project \
  "${gson_fixture}" \
  "gson-maven" \
  "gson" \
  "${java_21_home}"
gson_model="${project_dir}/target/gson-maven-model.ndjson"
JAVA_HOME="${java_21_home}" \
  PATH="${java_21_home}/bin:${PATH}" \
  "${maven_bin}" \
    --batch-mode \
    --no-transfer-progress \
    -q \
    "-Dmaven.repo.local=${local_repository}" \
    dependency:get \
    -Dartifact=com.google.errorprone:error_prone_core:2.50.0
grep -q '"projectPath":"gson"' "${gson_model}"
grep -q '"release":"8"' "${gson_model}"
grep -q 'error_prone_core-2.50.0.jar' "${gson_model}"
grep -q -- '-Xplugin:ErrorProne' "${gson_model}"
test -s \
  "${local_repository}/com/google/errorprone/error_prone_core/2.50.0/error_prone_core-2.50.0.jar"
