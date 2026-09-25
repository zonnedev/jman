#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=use-test-java.sh
source "${project_dir}/scripts/use-test-java.sh"
classes="${project_dir}/target/processor-worker-classes"
jar_file="${project_dir}/target/processor-worker.jar"
processor_source="${project_dir}/tools/annotation-processor-worker/src/main/java/io/github/zonnedev/jman/processor/worker/ProcessorWorker.java"
mapfile -t semantic_sources < <(
  find "${project_dir}/tools/javac-bridge/src/main/java" \
    -name '*.java' \
    ! -name 'NativeBridge.java' \
    ! -name 'JavaFormatter.java' \
    ! -name 'SemanticSession.java' \
    ! -name 'SemanticSessions.java' \
    -print | sort
)

rm -rf "${classes}"
mkdir -p "${classes}"
javac --release 17 -Werror -Xlint:all -d "${classes}" \
  "${processor_source}" "${semantic_sources[@]}"
jar --create --date=1980-01-01T00:00:02Z --file "${jar_file}" -C "${classes}" .
test -s "${jar_file}"
javap \
  -classpath "${jar_file}" \
  -verbose \
  io.github.zonnedev.jman.processor.worker.ProcessorWorker \
  | grep -q 'major version: 61'
javap \
  -classpath "${jar_file}" \
  -verbose \
  io.github.zonnedev.jman.javac.ProcessedSemanticWorker \
  | grep -q 'major version: 61'
