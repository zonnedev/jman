#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
gson="${GSON_DIR:-${project_dir}/target/integration-fixtures/gson-maven}"
model="${project_dir}/target/gson-maven-model.ndjson"
semantic_fixture="${project_dir}/tests/fixtures/compatibility/maven"

"${project_dir}/scripts/import-maven-project.sh" "${gson}" "${model}"

jq -e '
  select(.projectPath == "gson" and .taskPath == "compile")
  | (.sourceFiles | any(endswith("/module-info.java")))
    and (.modulePath | length > 0)
    and (.classpath | length == 0)
' "${model}" >/dev/null

JAVAC_FRONTEND_LIB_DIR="${project_dir}/target/native" \
  cargo build -p jman-java-lsp --features native-ffi
JAVA_HOME="${JAVA_HOME:-/home/jfsanchez/.sdkman/candidates/java/25.0.4-graal}" \
JAVA_LSP_MAVEN_REPOSITORY="${project_dir}/target/maven-repository" \
JAVA_LSP_DISABLE_ANNOTATION_PROCESSING=1 \
JAVA_LSP_PROCESSOR_WORKER_CLASSPATH="${project_dir}/target/processor-worker.jar" \
LD_LIBRARY_PATH="${project_dir}/target/native" \
  cargo run --quiet -p jman-java-lsp --example jpms_correctness_probe -- \
    "${project_dir}/target/debug/jman-java-lsp" \
    "${semantic_fixture}" \
    "src/main/java/module-info.java" \
    "src/main/java/io/github/zonnedev/jman/tests/compatibility/Greeting.java" \
    "io.github.zonnedev.jman.tests.compatibility.maven"
