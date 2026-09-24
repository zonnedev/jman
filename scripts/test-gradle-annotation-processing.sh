#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi
# shellcheck source=use-test-java.sh
source "${project_dir}/scripts/use-test-java.sh"
fixture="${project_dir}/tests/fixtures/gradle-annotation-processing"
fixture_copy="${project_dir}/target/integration-fixtures/gradle-annotation-processing"
gradle_user_home="${project_dir}/target/gradle-user-home"
model_output="${project_dir}/target/gradle-annotation-processing-model.ndjson"
semantic_classpath="${project_dir}/target/gradle-annotation-processing-classpath.txt"
worker_request="${project_dir}/target/gradle-processor-worker.properties"
petclinic_gradlew="${project_dir}/target/integration-fixtures/spring-petclinic/gradlew"

if [[ ! -x "${petclinic_gradlew}" ]]; then
  echo "Run make test-gradle-import first to provision the pinned Gradle wrapper" >&2
  exit 1
fi

rm -rf "${fixture_copy}"
mkdir -p "${fixture_copy}"
cp -a "${fixture}/." "${fixture_copy}/"
cp "${petclinic_gradlew}" "${fixture_copy}/gradlew"
cp -a \
  "${project_dir}/target/integration-fixtures/spring-petclinic/gradle" \
  "${fixture_copy}/gradle"

(
  cd "${fixture_copy}"
  export GRADLE_USER_HOME="${gradle_user_home}"
  ./gradlew \
    --console=plain \
    --no-daemon \
    --no-configuration-cache \
    -I "${project_dir}/tools/gradle-importer/javac-frontend-model.init.gradle" \
    javaFrontendModel \
    :app:compileJava
) | sed -n 's/^JAVAC_FRONTEND_MODEL //p' > "${model_output}"

grep -q '"taskPath":":app:compileJava"' "${model_output}"
grep -q '"schemaVersion":2' "${model_output}"
grep -q '"sourceRoots":\[[^]]*/app/src/main/java"' "${model_output}"
grep -q '"projectDependencies":\[":processor"\]' "${model_output}"
grep -q '"annotationProcessorPath":\[[^]]*/processor/build/libs/processor\.jar"' "${model_output}"
grep -q 'lombok-1\.18\.46\.jar' "${model_output}"
grep -q 'mapstruct-processor-1\.6\.3\.jar' "${model_output}"
grep -q '"annotationProcessorOptions":\["-Agreeting.mode=strict"\]' "${model_output}"
grep -q '"generatedSourceDirectories":\[[^]]*/app/build/generated/sources/annotationProcessor/java/main"' "${model_output}"
grep -q '"javaCompilerExecutable":"[^"]*/bin/javac"' "${model_output}"
grep -q '"taskPath":":app:compileIntegrationTestJava"' "${model_output}"
grep -q '"component":"annotationProcessorPath"' "${model_output}"
grep -q '"taskPath":":app:compileProcessorTestJava"' "${model_output}"
jq -e '
  select(.taskPath == ":app:compileProcessorTestJava")
  | (.sourceRoots | any(endswith("/app/src/processorTest/java")))
    and (.annotationProcessorPath | any(endswith("/processor/build/libs/processor.jar")))
    and (.generatedSourceDirectories
      | any(endswith("/app/build/generated/sources/annotationProcessor/java/processorTest")))
' "${model_output}" >/dev/null

generated_source="${fixture_copy}/app/build/generated/sources/annotationProcessor/java/main/io/github/zonnedev/jman/tests/fixture/GeneratedGreeting.java"
generated_class="${fixture_copy}/app/build/classes/java/main/io/github/zonnedev/jman/tests/fixture/GeneratedGreeting.class"
mapstruct_source="${fixture_copy}/app/build/generated/sources/annotationProcessor/java/main/io/github/zonnedev/jman/tests/fixture/PersonMapperImpl.java"
mapstruct_class="${fixture_copy}/app/build/classes/java/main/io/github/zonnedev/jman/tests/fixture/PersonMapperImpl.class"
lombok_class="${fixture_copy}/app/build/classes/java/main/io/github/zonnedev/jman/tests/fixture/Person.class"
test -s "${generated_source}"
test -s "${generated_class}"
test -s "${mapstruct_source}"
test -s "${mapstruct_class}"
test -s "${lombok_class}"
javap -p "${lombok_class}" | grep -q 'getName()'
javap -p "${lombok_class}" | grep -q 'setName(java.lang.String)'

jq -r \
  'select(.taskPath == ":app:compileJava") | .classpath | join(":")' \
  "${model_output}" > "${semantic_classpath}"
JAVAC_FRONTEND_LIB_DIR="${project_dir}/target/native" \
  cargo run \
    --quiet \
    -p javac-frontend \
    --features native-ffi \
    --example semantic_probe \
    -- \
    "${fixture_copy}/app/src/main/java/io/github/zonnedev/jman/tests/fixture/Application.java" \
    25 \
    "${semantic_classpath}" \
    "${fixture_copy}/app/src/main/java:${fixture_copy}/app/build/generated/sources/annotationProcessor/java/main" \
    io.github.zonnedev.jman.tests.fixture.GeneratedGreeting

cargo run \
  --quiet \
  -p jman-java-lsp \
  --example processor_worker_probe \
  -- \
  "${JAVA_HOME}/bin/java" \
  "${project_dir}/target/java-test-classes" \
  "${fixture_copy}/app/src/main/java/io/github/zonnedev/jman/tests/fixture/Application.java" \
  "${fixture_copy}/processor/build/libs/processor.jar" \
  "${fixture_copy}/app/build/jman-java/generated" \
  "${fixture_copy}/app/build/jman-java/classes" \
  "${worker_request}"
