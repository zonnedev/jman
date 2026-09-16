#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
petclinic_fixture="${JAVAC_FRONTEND_PETCLINIC:-/home/jfsanchez/zonnedev/tmp/test/spring-petclinic}"
gson_fixture="${JAVAC_FRONTEND_GSON:-/home/jfsanchez/zonnedev/tmp/test/gson}"
local_repository="${project_dir}/target/maven-repository"
classes_dir="${project_dir}/target/java-test-classes"
maven_java_home="${JAVAC_FRONTEND_MAVEN_JAVA_HOME:-/home/jfsanchez/.sdkman/candidates/java/17.0.20-tem}"

if [[ ! -x "${maven_java_home}/bin/java" ]]; then
  echo "Maven integration JDK 17 is not installed: ${maven_java_home}" >&2
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
    mvn \
      --batch-mode \
      --no-transfer-progress \
      -q \
      "-Dmaven.repo.local=${local_repository}" \
      -Denforcer.skip=true \
      -DskipTests \
      help:effective-pom \
      -Doutput=target/javac-frontend-effective-pom.xml
    if [[ -n "${classpath_project}" ]]; then
      mvn \
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
      mvn \
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
  "/home/jfsanchez/.sdkman/candidates/java/21.0.2-open"
gson_model="${project_dir}/target/gson-maven-model.ndjson"
JAVA_HOME="/home/jfsanchez/.sdkman/candidates/java/21.0.2-open" \
  PATH="/home/jfsanchez/.sdkman/candidates/java/21.0.2-open/bin:${PATH}" \
  mvn \
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
