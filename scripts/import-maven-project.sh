#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 2 ]]; then
  echo "usage: import-maven-project.sh PROJECT_ROOT OUTPUT_NDJSON" >&2
  exit 2
fi

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
workspace="$(cd "$1" && pwd)"
output="$2"
cache_dir="$(cd "$(dirname "${output}")" && pwd)/maven-import"
local_repository="${JAVA_LSP_MAVEN_REPOSITORY:-${project_dir}/target/maven-repository}"
importer_classes="${JAVAC_FRONTEND_IMPORTER_CLASSES:-${project_dir}/target/java-test-classes}"
build_java_home="${JAVA_LSP_BUILD_JAVA_HOME:-/home/jfsanchez/.sdkman/candidates/java/17.0.20-tem}"
maven="${workspace}/mvnw"

if [[ ! -x "${maven}" ]]; then
  maven="${JAVA_LSP_MAVEN:-mvn}"
fi
if [[ ! -f "${importer_classes}/io/github/zonnedev/jman/maven/importer/MavenModelImporter.class" ]]; then
  echo "Maven importer classes are missing; run make test-java" >&2
  exit 1
fi

mkdir -p "${cache_dir}" "${local_repository}"
effective_pom="${cache_dir}/effective-pom.xml"
classpath_file="${cache_dir}/classpath.txt"
(
  cd "${workspace}"
  if [[ -x "${build_java_home}/bin/java" ]]; then
    export JAVA_HOME="${build_java_home}"
    export PATH="${JAVA_HOME}/bin:${PATH}"
  fi
  # Materialize sources contributed by lifecycle plugins (templating,
  # protobuf, annotation setup, and similar generators).
  if ! "${maven}" \
    --batch-mode \
    --no-transfer-progress \
    -q \
    "-Dmaven.repo.local=${local_repository}" \
    -Denforcer.skip=true \
    -DskipTests \
    generate-sources; then
    echo "warning: Maven generated-source collection was partial" >&2
  fi
  "${maven}" \
    --batch-mode \
    --no-transfer-progress \
    -q \
    "-Dmaven.repo.local=${local_repository}" \
    -Denforcer.skip=true \
    -DskipTests \
    help:effective-pom \
    "-Doutput=${effective_pom}"
  # Some reactors contain verification modules which depend on sibling
  # artifacts being installed first (Gson's test-jpms is one example). Retain
  # the classpaths Maven resolved before that failure.
  if ! "${maven}" \
    --batch-mode \
    --no-transfer-progress \
    -q \
    "-Dmaven.repo.local=${local_repository}" \
    -Denforcer.skip=true \
    -DskipTests \
    dependency:build-classpath \
    "-Dmdep.outputFile=${classpath_file}"; then
    echo "warning: Maven classpath collection was partial; importing the resolvable reactor modules" >&2
  fi
)

touch "${classpath_file}"
java -cp "${importer_classes}" \
  io.github.zonnedev.jman.maven.importer.MavenModelImporter \
  "${effective_pom}" \
  "${classpath_file}" \
  "${local_repository}" > "${output}"

test -s "${output}"
