#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi
# shellcheck source=use-test-java.sh
source "${project_dir}/scripts/use-test-java.sh"
classes_dir="${project_dir}/target/java-test-classes"
test_tmp=""

cleanup() {
  if [[ -n "${test_tmp}" ]]; then
    rm -rf -- "${test_tmp}"
  fi
}
trap cleanup EXIT
test_tmp="$(mktemp -d "${TMPDIR:-/tmp}/jman-java-tests.XXXXXX")"

# Keep every temporary resource created by the Java fixtures under one
# script-owned directory so failed assertions cannot leave files in /tmp.
export TMPDIR="${test_tmp}"

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
java -Djava.io.tmpdir="${test_tmp}" -ea -cp "${classes_dir}" io.github.zonnedev.jman.javac.JavacFrontendTest
java -Djava.io.tmpdir="${test_tmp}" -ea -cp "${classes_dir}" io.github.zonnedev.jman.javac.ProcessedSemanticWorkerTest
java -Djava.io.tmpdir="${test_tmp}" -ea -cp "${classes_dir}" io.github.zonnedev.jman.maven.importer.MavenModelImporterTest
java -Djava.io.tmpdir="${test_tmp}" -ea -cp "${classes_dir}" io.github.zonnedev.jman.processor.worker.ProcessorWorkerTest
