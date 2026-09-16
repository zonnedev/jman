#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=use-sdkman-java.sh
source "${project_dir}/scripts/use-sdkman-java.sh"
classes_dir="${project_dir}/target/java-test-classes"

rm -rf "${classes_dir}"
mkdir -p "${classes_dir}"

mapfile -t main_sources < <(
  find \
    "${project_dir}/tools/javac-bridge/src/main/java" \
    "${project_dir}/tools/maven-importer/src/main/java" \
    "${project_dir}/tools/annotation-processor-worker/src/main/java" \
    -name '*.java' -print | sort
)
mapfile -t test_sources < <(
  find \
    "${project_dir}/tools/javac-bridge/src/test/java" \
    "${project_dir}/tools/maven-importer/src/test/java" \
    "${project_dir}/tools/annotation-processor-worker/src/test/java" \
    -name '*.java' -print | sort
)

javac -Werror -Xlint:all -d "${classes_dir}" "${main_sources[@]}" "${test_sources[@]}"
java -ea -cp "${classes_dir}" io.github.zonnedev.jman.javac.JavacFrontendTest
java -ea -cp "${classes_dir}" io.github.zonnedev.jman.maven.importer.MavenModelImporterTest
java -ea -cp "${classes_dir}" io.github.zonnedev.jman.processor.worker.ProcessorWorkerTest
