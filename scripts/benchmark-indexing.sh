#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
workspace="${1:?usage: benchmark-indexing.sh PROJECT_ROOT [BUILD_SYSTEM]}"
build_system="${2:-auto}"
report="${project_dir}/target/indexing-benchmark-$(basename "${workspace}").jsonl"
binary="${project_dir}/target/release/jman-java-lsp"
# shellcheck source=use-test-java.sh
source "${project_dir}/scripts/use-test-java.sh"

JAVAC_FRONTEND_LIB_DIR="${project_dir}/target/native" \
  cargo build --release -p jman-java-lsp --features native-ffi >/dev/null

env \
  JAVA_HOME="${JAVA_HOME}" \
  LD_LIBRARY_PATH="${project_dir}/target/native" \
  JAVA_LSP_PROCESSOR_WORKER_CLASSPATH="${project_dir}/target/processor-worker.jar" \
  node "${project_dir}/scripts/benchmark-indexing.mjs" \
    "${binary}" "${workspace}" "${build_system}" \
    2> "${report}"

grep '"jman.javaMetric"' "${report}"
