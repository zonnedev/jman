#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=use-test-java.sh
source "${project_dir}/scripts/use-test-java.sh"
classes_dir="${project_dir}/target/maven-importer-classes"
jar_file="${project_dir}/target/maven-importer.jar"

sources=()
while IFS= read -r source; do
  sources+=("${source}")
done < <(find "${project_dir}/tools/maven-importer/src/main/java" -name '*.java' -print | sort)
if [[ "${#sources[@]}" -eq 0 ]]; then
  echo 'Maven importer has no Java sources' >&2
  exit 1
fi

rm -rf -- "${classes_dir}"
mkdir -p "${classes_dir}"
"${JAVA_HOME}/bin/javac" \
  --release 17 \
  -Werror \
  -Xlint:all \
  -d "${classes_dir}" \
  "${sources[@]}"
"${JAVA_HOME}/bin/jar" \
  --create \
  --date=1980-01-01T00:00:02Z \
  --file "${jar_file}" \
  -C "${classes_dir}" \
  .
test -s "${jar_file}"
"${JAVA_HOME}/bin/javap" \
  -classpath "${jar_file}" \
  -verbose \
  io.github.zonnedev.jman.maven.importer.MavenModelImporter \
  | grep -q 'major version: 61'
