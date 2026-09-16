#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
workspace="${MICRONAUT_CORE_DIR:-${project_dir}/target/integration-fixtures/micronaut-core}"
expected_revision="${MICRONAUT_CORE_REVISION:-7978e6342398a68b35faa736d9cb32141e3d6c7a}"
repository="https://github.com/micronaut-projects/micronaut-core.git"
model="${project_dir}/target/micronaut-core-model.ndjson"

if [[ ! -d "${workspace}/.git" ]]; then
  git clone --depth 1 "${repository}" "${workspace}"
fi

actual_revision="$(git -C "${workspace}" rev-parse HEAD)"
if [[ "${actual_revision}" != "${expected_revision}" ]]; then
  echo "Micronaut Core revision ${actual_revision}; expected ${expected_revision}" >&2
  exit 1
fi

JAVA_HOME="${JAVA_HOME:-/home/jfsanchez/.sdkman/candidates/java/25.0.4-graal}" \
  "${project_dir}/scripts/import-gradle-project.sh" "${workspace}" "${model}"

test "$(wc -l < "${model}")" -ge 150
jq -e -s '
  length >= 150
  and (map(.sourceFiles | length) | add) >= 3500
  and (map(select((.resolutionErrors // []) | length > 0)) | length) == 0
  and (map(select(.annotationProcessorPath | length > 0)) | length) >= 100
  and (map(.release // .javaLanguageVersion) | all(. == 25 or . == "25"))
' "${model}" >/dev/null

JAVAC_FRONTEND_LIB_DIR="${project_dir}/target/native" \
  cargo build -p jman-java-lsp --features native-ffi

JAVA_HOME="${JAVA_HOME:-/home/jfsanchez/.sdkman/candidates/java/25.0.4-graal}" \
JAVA_LSP_PROCESSOR_WORKER_CLASSPATH="${project_dir}/target/processor-worker.jar" \
LD_LIBRARY_PATH="${project_dir}/target/native" \
  cargo run --quiet -p jman-java-lsp --example micronaut_correctness_probe -- \
    "${project_dir}/target/debug/jman-java-lsp" "${workspace}"

if [[ -n "${XDG_CACHE_HOME:-}" ]]; then
  cache_root="${XDG_CACHE_HOME}/io.github.zonnedev.jman.lsp/cache"
else
  cache_root="${TMPDIR:-/tmp}/io.github.zonnedev.jman.lsp/cache"
fi
main_generated_class="$(find "${cache_root}/projects" -path "*/processors/_micronaut-context__micronaut-context_compileJava/current/build/classes/java/main/io/micronaut/logging/\$LogLevel\$Introspection.class" -print -quit)"
test_generated_class="$(find "${cache_root}/projects" -path "*/processors/_micronaut-module-info-runtime__micronaut-module-info-runtime_compileTestJava/current/build/classes/java/test/io/micronaut/module/info/runtime/\$MicronautModuleRuntimeInfoFactoryTest\$DummyModulesFactory\$Definition.class" -print -quit)"
test -f "${main_generated_class}"
test -f "${test_generated_class}"
