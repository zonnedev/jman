#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=use-test-java.sh
source "${project_dir}/scripts/use-test-java.sh"
classes="${project_dir}/target/jman-runner-classes"
jar_file="${project_dir}/target/jman-runner.jar"
source_file="${project_dir}/tools/jman-runner/src/main/java/io/github/zonnedev/jman/runner/JmanRunner.java"

rm -rf "${classes}"
mkdir -p "${classes}"
javac -Werror -Xlint:all -d "${classes}" "${source_file}"
jar --create --date=1980-01-01T00:00:02Z --file "${jar_file}" -C "${classes}" .
test -s "${jar_file}"
