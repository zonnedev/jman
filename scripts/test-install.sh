#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi
version="$({
  awk '
    $0 == "[workspace.package]" { in_workspace_package = 1; next }
    /^\[/ { in_workspace_package = 0 }
    in_workspace_package && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' "${project_dir}/Cargo.toml"
})"
test_root="${project_dir}/target/install-script-test"
release_dir="${test_root}/releases/download/v${version}"
payload_name="jman-${version}-linux-x86_64"
archive_name="${payload_name}.tar.gz"
payload_dir="${test_root}/payload/${payload_name}"
state_dir="${test_root}/state"

rm -rf "${test_root}"
mkdir -p "${release_dir}" "${payload_dir}" "${state_dir}"
trap 'rm -rf "${test_root}"' EXIT

cat > "${payload_dir}/jman" <<EOF
#!/bin/sh
set -eu
case "\$*" in
  --version)
    printf 'jman %s\n' '${version}'
    ;;
  'java which --format home')
    test -f "\${JMAN_TEST_STATE_DIR}/global-java"
    printf '%s\n' "\${JMAN_TEST_STATE_DIR}/fake-java-home"
    ;;
  'java install 25 --global --no-progress')
    printf '%s\n' "\$*" >> "\${JMAN_TEST_STATE_DIR}/commands"
    touch "\${JMAN_TEST_STATE_DIR}/global-java"
    ;;
  'java setup --shell zsh --no-progress')
    if [ ! -f "\${JMAN_TEST_STATE_DIR}/global-java" ]; then
      printf 'error: no global Java is selected\n' >&2
      exit 1
    fi
    printf '%s\n' "\$*" >> "\${JMAN_TEST_STATE_DIR}/commands"
    ;;
  *)
    printf 'unexpected fake jman arguments: %s\n' "\$*" >&2
    exit 64
    ;;
esac
EOF
chmod +x "${payload_dir}/jman"
printf 'runtime fixture\n' > "${payload_dir}/libjman_javac_frontend.so"
tar -czf "${release_dir}/${archive_name}" -C "${test_root}/payload" "${payload_name}"
(
  cd "${release_dir}"
  sha256sum "${archive_name}" > SHA256SUMS
)
cat > "${test_root}/latest-release-manifest.json" <<EOF
{
  "schemaVersion": 1,
  "releaseTag": "v${version}",
  "jmanVersion": "${version}",
  "artifacts": []
}
EOF

home_dir="${test_root}/home"
output_file="${test_root}/install-output"
mkdir -p "${home_dir}"
env \
  HOME="${home_dir}" \
  SHELL=/usr/bin/zsh \
  JMAN_ALLOW_INSECURE_URLS=1 \
  JMAN_LATEST_MANIFEST_URL="file://${test_root}/latest-release-manifest.json" \
  JMAN_RELEASE_BASE_URL="file://${test_root}/releases/download" \
  JMAN_SETUP_JAVA=1 \
  JMAN_TEST_STATE_DIR="${state_dir}" \
  sh "${project_dir}/install.sh" > "${output_file}"

install_dir="${home_dir}/.local/share/jman/versions/${version}"
test -x "${install_dir}/jman"
test -L "${home_dir}/.local/bin/jman"
test "$(readlink "${home_dir}/.local/bin/jman")" = "${install_dir}/jman"
grep -Fxq 'java install 25 --global --no-progress' "${state_dir}/commands"
grep -Fxq 'java setup --shell zsh --no-progress' "${state_dir}/commands"
grep -Fq "export PATH='${home_dir}/.local/bin':\"\$PATH\"" "${output_file}"
grep -Fq 'eval "$(jman shell init zsh)"' "${output_file}"
grep -Fq "JMAN ${version} installation is complete." "${output_file}"

before_count="$(wc -l < "${state_dir}/commands")"
env \
  HOME="${home_dir}" \
  SHELL=/usr/bin/zsh \
  JMAN_ALLOW_INSECURE_URLS=1 \
  JMAN_VERSION="${version}" \
  JMAN_RELEASE_BASE_URL="file://${test_root}/releases/download" \
  JMAN_SETUP_JAVA=0 \
  JMAN_TEST_STATE_DIR="${state_dir}" \
  sh "${project_dir}/install.sh" > "${test_root}/reinstall-output"
after_count="$(wc -l < "${state_dir}/commands")"
test "${after_count}" -eq "${before_count}"
grep -Fq "JMAN ${version} is already installed" "${test_root}/reinstall-output"

protected_home="${test_root}/protected-home"
mkdir -p "${protected_home}/.local/bin"
printf 'keep me\n' > "${protected_home}/.local/bin/jman"
if env \
  HOME="${protected_home}" \
  SHELL=/bin/bash \
  JMAN_ALLOW_INSECURE_URLS=1 \
  JMAN_VERSION="${version}" \
  JMAN_RELEASE_BASE_URL="file://${test_root}/releases/download" \
  JMAN_SETUP_JAVA=0 \
  JMAN_TEST_STATE_DIR="${state_dir}" \
  sh "${project_dir}/install.sh" > "${test_root}/protected-output" 2>&1; then
  echo 'Installer replaced a non-symlink jman executable' >&2
  exit 1
fi
grep -Fxq 'keep me' "${protected_home}/.local/bin/jman"
grep -Fq 'refusing to replace non-symlink' "${test_root}/protected-output"

fish_home="${test_root}/fish-home"
mkdir -p "${fish_home}"
env \
  HOME="${fish_home}" \
  SHELL=/usr/bin/fish \
  JMAN_ALLOW_INSECURE_URLS=1 \
  JMAN_VERSION="${version}" \
  JMAN_RELEASE_BASE_URL="file://${test_root}/releases/download" \
  JMAN_SETUP_JAVA=0 \
  JMAN_TEST_STATE_DIR="${state_dir}" \
  sh "${project_dir}/install.sh" > "${test_root}/fish-output"
grep -Fq "fish_add_path '${fish_home}/.local/bin'" "${test_root}/fish-output"
grep -Fq 'eval "$(jman shell init fish)"' "${test_root}/fish-output"

invalid_home="${test_root}/invalid-home"
mkdir -p "${invalid_home}"
if env \
  HOME="${invalid_home}" \
  SHELL=/bin/bash \
  JMAN_ALLOW_INSECURE_URLS=1 \
  JMAN_VERSION=.. \
  JMAN_RELEASE_BASE_URL="file://${test_root}/releases/download" \
  JMAN_SETUP_JAVA=0 \
  JMAN_TEST_STATE_DIR="${state_dir}" \
  sh "${project_dir}/install.sh" > "${test_root}/invalid-output" 2>&1; then
  echo 'Installer accepted an unsafe version' >&2
  exit 1
fi
test ! -e "${invalid_home}/.local/bin/jman"
grep -Fq 'invalid JMAN version' "${test_root}/invalid-output"

printf 'corrupt\n' >> "${release_dir}/${archive_name}"
checksum_home="${test_root}/checksum-home"
mkdir -p "${checksum_home}"
if env \
  HOME="${checksum_home}" \
  SHELL=/bin/bash \
  JMAN_ALLOW_INSECURE_URLS=1 \
  JMAN_VERSION="${version}" \
  JMAN_RELEASE_BASE_URL="file://${test_root}/releases/download" \
  JMAN_SETUP_JAVA=0 \
  JMAN_TEST_STATE_DIR="${state_dir}" \
  sh "${project_dir}/install.sh" > "${test_root}/checksum-output" 2>&1; then
  echo 'Installer accepted an archive with an invalid checksum' >&2
  exit 1
fi
test ! -e "${checksum_home}/.local/bin/jman"
grep -Fq 'SHA-256 verification failed' "${test_root}/checksum-output"

printf 'Remote installer tests passed\n'
