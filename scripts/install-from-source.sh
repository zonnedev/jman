#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=platform.sh
source "${project_dir}/scripts/platform.sh"

stage_dir="${1:-}"
staging_dir=""
backup_dir=""
temporary_link=""
setup_output=""

cleanup() {
  if [[ -n "${staging_dir}" && -d "${staging_dir}" ]]; then
    rm -rf -- "${staging_dir}"
  fi
  if [[ -n "${backup_dir}" && -d "${backup_dir}" ]]; then
    rm -rf -- "${backup_dir}"
  fi
  if [[ -n "${temporary_link}" && -L "${temporary_link}" ]]; then
    rm -f -- "${temporary_link}"
  fi
  if [[ -n "${setup_output}" && -f "${setup_output}" ]]; then
    rm -f -- "${setup_output}"
  fi
}
trap cleanup EXIT HUP INT TERM

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

detect_shell() {
  configured_shell="${SHELL:-}"
  shell_name="${configured_shell##*/}"
  case "${shell_name}" in
    bash | zsh | fish) printf '%s\n' "${shell_name}" ;;
    *) printf '%s\n' bash ;;
  esac
}

platform="$(jman_release_platform)" || fail "unsupported source-install platform: $(uname -s) $(uname -m)"
if [[ -z "${stage_dir}" ]]; then
  release_binary="${project_dir}/target/release/jman"
  [[ -x "${release_binary}" ]] || fail 'release binary is missing; run `make release` first'
  version="$(
    "${project_dir}/scripts/with-native-library.sh" "${project_dir}/target/native" \
      "${release_binary}" --version | awk '{ print $2 }'
  )"
  stage_dir="${project_dir}/target/release-stage/jman-${version}-${platform}"
fi

[[ -d "${stage_dir}" ]] || fail "source installation payload is missing: ${stage_dir}"
[[ -x "${stage_dir}/jman" ]] || fail "source installation payload has no executable jman: ${stage_dir}"
version="$(${stage_dir}/jman --version | awk '{ print $2 }')"
if ! printf '%s\n' "${version}" \
  | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z][0-9A-Za-z.-]*)?(\+[0-9A-Za-z][0-9A-Za-z.-]*)?$'; then
  fail "source installation payload reports an invalid JMAN version: ${version:-unknown}"
fi

data_home="${XDG_DATA_HOME:-${HOME}/.local/share}"
install_root="${JMAN_INSTALL_ROOT:-${data_home}/jman/versions}"
install_dir="${install_root}/${version}"
bin_dir="${JMAN_BIN_DIR:-${HOME}/.local/bin}"
jman_link="${bin_dir}/jman"

mkdir -p "${install_root}" "${bin_dir}"
if [[ -e "${jman_link}" && ! -L "${jman_link}" ]]; then
  fail "refusing to replace non-symlink ${jman_link}"
fi
if [[ -e "${install_dir}" && ( ! -d "${install_dir}" || -L "${install_dir}" ) ]]; then
  fail "refusing to replace non-directory installation ${install_dir}"
fi

staging_dir="$(mktemp -d "${install_root}/.install-${version}.XXXXXX")"
cp -R "${stage_dir}/." "${staging_dir}/"
staged_version="$(${staging_dir}/jman --version | awk '{ print $2 }')"
[[ "${staged_version}" == "${version}" ]] \
  || fail "staged source installation reports ${staged_version:-unknown}, expected ${version}"

if [[ -d "${install_dir}" ]]; then
  backup_dir="${install_root}/.replaced-${version}.$$"
  [[ ! -e "${backup_dir}" ]] || fail "temporary replacement path already exists: ${backup_dir}"
  mv "${install_dir}" "${backup_dir}"
fi
if ! mv "${staging_dir}" "${install_dir}"; then
  if [[ -n "${backup_dir}" && -d "${backup_dir}" ]]; then
    mv "${backup_dir}" "${install_dir}"
    backup_dir=""
  fi
  fail "could not activate source installation ${install_dir}"
fi
staging_dir=""
if [[ -n "${backup_dir}" ]]; then
  rm -rf -- "${backup_dir}"
  backup_dir=""
fi

temporary_link="${bin_dir}/.jman-link.$$"
rm -f -- "${temporary_link}"
ln -s "${install_dir}/jman" "${temporary_link}"
if [[ "$(jman_host_os)" == linux ]]; then
  mv -Tf "${temporary_link}" "${jman_link}"
else
  rm -f -- "${jman_link}"
  mv -f "${temporary_link}" "${jman_link}"
fi
temporary_link=""

shell_name="$(detect_shell)"
setup_output="$(mktemp "${TMPDIR:-/tmp}/jman-source-setup.XXXXXX")"
if "${install_dir}/jman" java setup --shell "${shell_name}" --no-progress \
  >"${setup_output}" 2>&1; then
  printf 'Refreshed Java command shims through JMAN %s.\n' "${version}"
elif grep -Fq 'no global Java is selected' "${setup_output}"; then
  printf 'Java shim refresh skipped because no global Java is selected.\n'
else
  cat "${setup_output}" >&2
  fail 'JMAN was installed, but its Java command shims could not be refreshed'
fi

printf 'Installed JMAN %s from source in %s.\n' "${version}" "${install_dir}"
printf 'Linked %s to the source installation.\n' "${jman_link}"
printf 'Restart %s or run: rehash\n' "${shell_name}"
