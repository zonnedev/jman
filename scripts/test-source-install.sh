#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi

test_root="${JMAN_TEST_TEMP_ROOT}/source-install"
stage_dir="${test_root}/stage"
home_dir="${test_root}/home"
state_dir="${test_root}/state"
version="0.8.3+dev.1.g123456789abc.dirty"
mkdir -p "${stage_dir}" "${home_dir}" "${state_dir}"

cat >"${stage_dir}/jman" <<EOF
#!/bin/sh
set -eu
case "\$*" in
  --version)
    printf 'jman %s\n' '${version}'
    ;;
  'java setup --shell zsh --no-progress')
    printf '%s\n' "\$*" >>"\${JMAN_TEST_STATE_DIR}/commands"
    ;;
  *)
    printf 'unexpected fake jman arguments: %s\n' "\$*" >&2
    exit 64
    ;;
esac
EOF
chmod +x "${stage_dir}/jman"
printf 'first payload\n' >"${stage_dir}/runtime-marker"

HOME="${home_dir}" \
SHELL=/usr/bin/zsh \
JMAN_TEST_STATE_DIR="${state_dir}" \
  "${project_dir}/scripts/install-from-source.sh" "${stage_dir}" \
  >"${test_root}/first-output"

install_dir="${home_dir}/.local/share/jman/versions/${version}"
test -x "${install_dir}/jman"
grep -Fxq 'first payload' "${install_dir}/runtime-marker"
test -L "${home_dir}/.local/bin/jman"
test "$(readlink "${home_dir}/.local/bin/jman")" = "${install_dir}/jman"
grep -Fxq 'java setup --shell zsh --no-progress' "${state_dir}/commands"
grep -Fq "Installed JMAN ${version} from source" "${test_root}/first-output"

printf 'replacement payload\n' >"${stage_dir}/runtime-marker"
HOME="${home_dir}" \
SHELL=/usr/bin/zsh \
JMAN_TEST_STATE_DIR="${state_dir}" \
  "${project_dir}/scripts/install-from-source.sh" "${stage_dir}" \
  >"${test_root}/second-output"
grep -Fxq 'replacement payload' "${install_dir}/runtime-marker"
test "$(wc -l <"${state_dir}/commands")" -eq 2
if find "${home_dir}/.local/share/jman/versions" -maxdepth 1 \
  \( -name '.replaced-*' -o -name '.install-*' \) | grep -q .; then
  printf 'source installation left replacement files behind\n' >&2
  exit 1
fi

protected_home="${test_root}/protected-home"
mkdir -p "${protected_home}/.local/bin"
printf 'keep me\n' >"${protected_home}/.local/bin/jman"
if HOME="${protected_home}" \
  SHELL=/bin/bash \
  JMAN_TEST_STATE_DIR="${state_dir}" \
  "${project_dir}/scripts/install-from-source.sh" "${stage_dir}" \
  >"${test_root}/protected-output" 2>&1; then
  printf 'source installer replaced a non-symlink jman executable\n' >&2
  exit 1
fi
grep -Fxq 'keep me' "${protected_home}/.local/bin/jman"
grep -Fq 'refusing to replace non-symlink' "${test_root}/protected-output"

printf 'Source installation tests passed\n'
