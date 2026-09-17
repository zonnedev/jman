#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=use-sdkman-java.sh
source "${project_dir}/scripts/use-sdkman-java.sh"
classes="${project_dir}/target/processor-worker-classes"
jar_file="${project_dir}/target/processor-worker.jar"
source_file="${project_dir}/tools/annotation-processor-worker/src/main/java/io/github/zonnedev/jman/processor/worker/ProcessorWorker.java"

rm -rf "${classes}"
mkdir -p "${classes}"
javac --release 17 -Werror -Xlint:all -d "${classes}" "${source_file}"
jar --create --date=1980-01-01T00:00:02Z --file "${jar_file}" -C "${classes}" .
test -s "${jar_file}"
javap \
  -classpath "${jar_file}" \
  -verbose \
  io.github.zonnedev.jman.processor.worker.ProcessorWorker \
  | grep -q 'major version: 61'
