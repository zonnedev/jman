#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi

test_root="${JMAN_TEST_TEMP_ROOT}/toolchain-bootstrap"
toolchains_dir="${test_root}/toolchains"
fake_jdks="${test_root}/jdks"
fake_jman="${test_root}/jman"
command_log="${test_root}/commands"
mkdir -p "${fake_jdks}" "${toolchains_dir}"

for specification in graalvm-community:25 temurin:17 temurin:21 temurin:25; do
  vendor="${specification%%:*}"
  major="${specification##*:}"
  home="${fake_jdks}/${vendor}-${major}"
  mkdir -p "${home}/bin"
  cat > "${home}/bin/java" <<EOF
#!/bin/sh
if [ "\${1:-}" = -version ]; then
  printf 'openjdk version "${major}.0.0"\n' >&2
fi
exit 0
EOF
  printf '#!/bin/sh\nexit 0\n' > "${home}/bin/javac"
  chmod +x "${home}/bin/java" "${home}/bin/javac"
done
printf '#!/bin/sh\nexit 0\n' > "${fake_jdks}/graalvm-community-25/bin/native-image"
chmod +x "${fake_jdks}/graalvm-community-25/bin/native-image"

cat > "${fake_jman}" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -eq 1 && "$1" == --version ]]; then
  echo 'jman 0.7.1'
  exit 0
fi
printf '%s\n' "$*" >> "${JMAN_FAKE_COMMAND_LOG}"
if [[ "$1" == java && "$2" == install ]]; then
  exit 0
fi
if [[ "$1" == java && "$2" == exec ]]; then
  version="$3"
  vendor="$5"
  major="${version%%.*}"
  printf '%s/%s-%s\n' "${JMAN_FAKE_JDKS}" "${vendor}" "${major}"
  exit 0
fi
echo "unexpected fake JMAN command: $*" >&2
exit 64
EOF
chmod +x "${fake_jman}"

bootstrap_environment=(
  "JMAN_TEST_TOOLCHAINS_DIR=${toolchains_dir}"
  "JMAN_TEST_BOOTSTRAP_BINARY=${fake_jman}"
  "JMAN_FAKE_COMMAND_LOG=${command_log}"
  "JMAN_FAKE_JDKS=${fake_jdks}"
)

env "${bootstrap_environment[@]}" \
  "${project_dir}/scripts/setup-test-jdks.sh" --all >/dev/null

# shellcheck disable=SC1090
source "${toolchains_dir}/graalvm.env"
# shellcheck disable=SC1090
source "${toolchains_dir}/compatibility.env"
test "${JMAN_GRAALVM_HOME}" = "${fake_jdks}/graalvm-community-25"
test "${JMAN_TEST_JAVA_17_HOME}" = "${fake_jdks}/temurin-17"
test "${JMAN_TEST_JAVA_21_HOME}" = "${fake_jdks}/temurin-21"
test "${JMAN_TEST_JAVA_25_HOME}" = "${fake_jdks}/temurin-25"
grep -Fqx 'java install 25.3.4.1 --vendor graalvm-community --no-progress' "${command_log}"
grep -Fqx 'java install 17 --vendor temurin --no-progress' "${command_log}"
grep -Fqx 'java install 21 --vendor temurin --no-progress' "${command_log}"
grep -Fqx 'java install 25 --vendor temurin --no-progress' "${command_log}"

initial_commands="$(wc -l < "${command_log}")"
env "${bootstrap_environment[@]}" \
  "${project_dir}/scripts/setup-test-jdks.sh" --all >/dev/null
test "$(wc -l < "${command_log}")" -eq "${initial_commands}"

if env "${bootstrap_environment[@]}" \
  JMAN_GRAALVM_HOME="${test_root}/missing-graalvm" \
  "${project_dir}/scripts/setup-test-jdks.sh" --graalvm >/dev/null 2>&1; then
  echo 'Invalid explicit GraalVM override was accepted' >&2
  exit 1
fi

if env "${bootstrap_environment[@]}" \
  JMAN_TEST_JAVA_21_HOME="${test_root}/missing-java-21" \
  "${project_dir}/scripts/setup-test-jdks.sh" --compatibility >/dev/null 2>&1; then
  echo 'Invalid explicit compatibility JDK override was accepted' >&2
  exit 1
fi
if env "${bootstrap_environment[@]}" \
  JMAN_TEST_JAVA_21_HOME="${fake_jdks}/temurin-17" \
  "${project_dir}/scripts/setup-test-jdks.sh" --compatibility >/dev/null 2>&1; then
  echo 'Compatibility JDK override with the wrong major was accepted' >&2
  exit 1
fi

grep -Fq 'bootstrap_version="0.7.1"' "${project_dir}/scripts/setup-test-jdks.sh"
grep -Fq \
  'bootstrap_sha256="989346585606ce1ebf731c4178033936e9b0e1fce397075de84cd11a559e8a31"' \
  "${project_dir}/scripts/setup-test-jdks.sh"

echo 'JMAN test toolchain bootstrap tests passed'
