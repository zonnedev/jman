#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi
# shellcheck source=use-test-java.sh
source "${project_dir}/scripts/use-test-java.sh"
fixture_dir="${project_dir}/tests/fixtures/publishing"
work_dir="${project_dir}/target/publishing-acceptance"
repository="${work_dir}/repository"
jman_binary="${JMAN_TEST_BINARY:-${project_dir}/target/debug/jman}"
maven_binary="${JAVA_LSP_MATRIX_MAVEN:-${project_dir}/target/compatibility-tools/apache-maven-3.9.9/bin/mvn}"
gradle_binary="${JAVA_LSP_MATRIX_GRADLE_9_1_0:-${project_dir}/target/compatibility-tools/gradle-9.1.0/bin/gradle}"
expected="hello from published modules"

for executable in \
  "${jman_binary}" \
  "${JAVA_HOME}/bin/java" \
  "${maven_binary}" \
  "${gradle_binary}"; do
  if [[ ! -x "${executable}" ]]; then
    echo "Publishing acceptance executable is unavailable: ${executable}" >&2
    exit 1
  fi
done

rm -rf "${work_dir}"
mkdir -p "${work_dir}"
cp -a "${fixture_dir}/." "${work_dir}/"

"${jman_binary}" --quiet publish "${work_dir}/producer" \
  --local-repository "${repository}" \
  --format json > "${work_dir}/publication.json"

group_path="io/github/zonnedev/jman/fixture"
api_dir="${repository}/${group_path}/greeting-api/1.0.0"
library_dir="${repository}/${group_path}/greeting-library/1.0.0"
root_dir="${repository}/${group_path}/publishing-fixture/1.0.0"

for artifact in \
  "${root_dir}/publishing-fixture-1.0.0.pom" \
  "${api_dir}/greeting-api-1.0.0.jar" \
  "${api_dir}/greeting-api-1.0.0-sources.jar" \
  "${api_dir}/greeting-api-1.0.0-javadoc.jar" \
  "${library_dir}/greeting-library-1.0.0.jar" \
  "${library_dir}/greeting-library-1.0.0-sources.jar" \
  "${library_dir}/greeting-library-1.0.0-javadoc.jar" \
  "${library_dir}/greeting-library-1.0.0.pom"; do
  test -s "${artifact}"
  test -s "${artifact}.md5"
  test -s "${artifact}.sha1"
  test -s "${artifact}.sha256"
  test -s "${artifact}.sha512"
  checksum="$(sha256sum "${artifact}")"
  test "${checksum%% *}" = "$(< "${artifact}.sha256")"
done

grep -q '<artifactId>greeting-api</artifactId>' \
  "${library_dir}/greeting-library-1.0.0.pom"
grep -q '<version>1.0.0</version>' \
  "${library_dir}/greeting-library-1.0.0.pom"

repository_url="file://${repository}"
sed -i "s|@PUBLISHING_REPOSITORY@|${repository_url}|g" \
  "${work_dir}/jman-consumer/jman.toml"
JMAN_CACHE_DIR="${work_dir}/jman-cache" \
  "${jman_binary}" --quiet sync "${work_dir}/jman-consumer"
jman_output="$(
  JMAN_CACHE_DIR="${work_dir}/jman-cache" \
    "${jman_binary}" --quiet run "${work_dir}/jman-consumer"
)"
test "${jman_output}" = "${expected}"

(
  cd "${work_dir}/maven-consumer"
  "${maven_binary}" --batch-mode --no-transfer-progress -q \
    "-Dmaven.repo.local=${work_dir}/maven-cache" \
    "-Dpublishing.repository=${repository_url}" \
    compile \
    org.apache.maven.plugins:maven-dependency-plugin:3.9.0:build-classpath \
    -Dmdep.outputFile=target/publishing-classpath.txt
)
maven_classpath="$(< "${work_dir}/maven-consumer/target/publishing-classpath.txt")"
case "${maven_classpath}" in
  *greeting-library-1.0.0.jar*greeting-api-1.0.0.jar*) ;;
  *)
    echo "Maven did not resolve both published modules: ${maven_classpath}" >&2
    exit 1
    ;;
esac
maven_output="$(
  "${JAVA_HOME}/bin/java" -cp "${work_dir}/maven-consumer/target/classes:${maven_classpath}" \
    io.github.zonnedev.jman.fixture.consumer.Application
)"
test "${maven_output}" = "${expected}"

gradle_output="$(
  cd "${work_dir}/gradle-consumer"
  GRADLE_USER_HOME="${work_dir}/gradle-user-home" \
    "${gradle_binary}" --no-daemon --console=plain -q \
      "-PpublishingRepository=${repository_url}" run
)"
test "${gradle_output}" = "${expected}"

printf 'Publishing acceptance passed for JMAN, Maven, and Gradle consumers.\n'
