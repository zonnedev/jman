#!/bin/sh
set -eu

repository="${JMAN_REPOSITORY:-zonnedev/jman}"
release_base_url="${JMAN_RELEASE_BASE_URL:-https://github.com/${repository}/releases/download}"
latest_manifest_url="${JMAN_LATEST_MANIFEST_URL:-https://github.com/${repository}/releases/latest/download/release-manifest.json}"
java_version="${JMAN_JAVA_VERSION:-25}"
setup_java="${JMAN_SETUP_JAVA:-auto}"
temporary_dir=""
staging_dir=""
temporary_link=""

say() {
  printf '%s\n' "$*"
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  if [ -n "${temporary_dir}" ] && [ -d "${temporary_dir}" ]; then
    rm -rf -- "${temporary_dir}"
  fi
  if [ -n "${staging_dir}" ] && [ -d "${staging_dir}" ]; then
    rm -rf -- "${staging_dir}"
  fi
  if [ -n "${temporary_link}" ] && [ -L "${temporary_link}" ]; then
    rm -f -- "${temporary_link}"
  fi
}

trap cleanup EXIT HUP INT TERM

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

download() {
  download_url="$1"
  download_destination="$2"
  case "${download_url}" in
    https://*)
      curl --fail --location --silent --show-error \
        --proto '=https' --proto-redir '=https' --tlsv1.2 \
        --output "${download_destination}" "${download_url}"
      ;;
    *)
      if [ "${JMAN_ALLOW_INSECURE_URLS:-0}" != "1" ]; then
        fail "refusing non-HTTPS download URL: ${download_url}"
      fi
      curl --fail --location --silent --show-error \
        --output "${download_destination}" "${download_url}"
      ;;
  esac
}

detect_shell() {
  configured_shell="${SHELL:-}"
  shell_name="${configured_shell##*/}"
  case "${shell_name}" in
    bash | zsh | fish) printf '%s\n' "${shell_name}" ;;
    *) printf '%s\n' bash ;;
  esac
}

shell_config_file() {
  case "$1" in
    zsh) printf '%s\n' '~/.zshrc' ;;
    fish) printf '%s\n' '~/.config/fish/config.fish' ;;
    *) printf '%s\n' '~/.bashrc' ;;
  esac
}

confirm_java_install() {
  case "${setup_java}" in
    1 | true | yes) return 0 ;;
    0 | false | no) return 1 ;;
    auto)
      if [ -r /dev/tty ] && [ -w /dev/tty ]; then
        printf 'Install Temurin Java %s globally and create Java shims now? [Y/n] ' \
          "${java_version}" >/dev/tty
        answer=""
        IFS= read -r answer </dev/tty || true
        case "${answer}" in
          '' | y | Y | yes | YES | Yes) return 0 ;;
          *) return 1 ;;
        esac
      fi
      return 1
      ;;
    *)
      fail 'JMAN_SETUP_JAVA must be auto, 1, or 0'
      ;;
  esac
}

require_command curl
require_command tar
require_command sha256sum
require_command mktemp
require_command awk
require_command sed
require_command grep
require_command uname

case "${setup_java}" in
  auto | 1 | 0 | true | false | yes | no) ;;
  *) fail 'JMAN_SETUP_JAVA must be auto, 1, or 0' ;;
esac

[ "$(uname -s)" = Linux ] || fail 'JMAN releases currently support Linux only'
case "$(uname -m)" in
  x86_64 | amd64) platform=linux-x86_64 ;;
  *) fail "unsupported architecture: $(uname -m); JMAN currently publishes Linux x86-64 releases" ;;
esac
if command -v getconf >/dev/null 2>&1 \
  && getconf GNU_LIBC_VERSION >/dev/null 2>&1; then
  :
elif command -v ldd >/dev/null 2>&1 \
  && ldd --version 2>&1 | grep -Eiq 'glibc|gnu libc'; then
  :
else
  fail 'JMAN releases currently require a glibc-based Linux system'
fi

version="${JMAN_VERSION:-}"
if [ -z "${version}" ]; then
  temporary_dir="$(mktemp -d "${TMPDIR:-/tmp}/jman-install.XXXXXX")"
  manifest_file="${temporary_dir}/release-manifest.json"
  say 'Resolving the latest JMAN release...'
  download "${latest_manifest_url}" "${manifest_file}"
  version="$(sed -n 's/^[[:space:]]*"jmanVersion":[[:space:]]*"\([^"]*\)".*/\1/p' "${manifest_file}" | sed -n '1p')"
fi
version="${version#v}"
if ! printf '%s\n' "${version}" \
  | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z][0-9A-Za-z.-]*)?(\+[0-9A-Za-z][0-9A-Za-z.-]*)?$'; then
  fail "invalid JMAN version: ${version}"
fi

tag="v${version}"
archive_name="jman-${version}-${platform}.tar.gz"
release_url="${release_base_url}/${tag}"

if [ -z "${temporary_dir}" ]; then
  temporary_dir="$(mktemp -d "${TMPDIR:-/tmp}/jman-install.XXXXXX")"
fi
archive_file="${temporary_dir}/${archive_name}"
checksums_file="${temporary_dir}/SHA256SUMS"

say "Downloading JMAN ${version}..."
download "${release_url}/SHA256SUMS" "${checksums_file}"
download "${release_url}/${archive_name}" "${archive_file}"

expected_sha256="$(awk -v name="${archive_name}" '$2 == name || $2 == "*" name { print $1 }' "${checksums_file}")"
case "${expected_sha256}" in
  '' | *[!0-9a-fA-F]*) fail "${archive_name} has no valid SHA-256 entry in SHA256SUMS" ;;
esac
if [ "${#expected_sha256}" -ne 64 ]; then
  fail "${archive_name} has an invalid SHA-256 entry in SHA256SUMS"
fi
actual_sha256="$(sha256sum "${archive_file}" | awk '{ print $1 }')"
if [ "${actual_sha256}" != "${expected_sha256}" ]; then
  fail "SHA-256 verification failed for ${archive_name}"
fi
say "Verified ${archive_name}."

data_home="${XDG_DATA_HOME:-${HOME}/.local/share}"
install_root="${JMAN_INSTALL_ROOT:-${data_home}/jman/versions}"
install_dir="${install_root}/${version}"
bin_dir="${JMAN_BIN_DIR:-${HOME}/.local/bin}"
jman_link="${bin_dir}/jman"

mkdir -p "${install_root}" "${bin_dir}"
if [ -d "${install_dir}" ]; then
  if [ ! -x "${install_dir}/jman" ]; then
    fail "existing installation is incomplete: ${install_dir}"
  fi
  installed_version="$(${install_dir}/jman --version 2>/dev/null | awk '{ print $2 }')"
  if [ "${installed_version}" != "${version}" ]; then
    fail "existing installation at ${install_dir} reports version ${installed_version:-unknown}"
  fi
  say "JMAN ${version} is already installed in ${install_dir}."
else
  staging_dir="$(mktemp -d "${install_root}/.install-${version}.XXXXXX")"
  tar -xzf "${archive_file}" -C "${staging_dir}" --strip-components=1
  [ -x "${staging_dir}/jman" ] || fail 'release archive does not contain an executable jman'
  installed_version="$(${staging_dir}/jman --version 2>/dev/null | awk '{ print $2 }')"
  [ "${installed_version}" = "${version}" ] || \
    fail "release archive reports version ${installed_version:-unknown}, expected ${version}"
  mv "${staging_dir}" "${install_dir}"
  staging_dir=""
  say "Installed JMAN ${version} in ${install_dir}."
fi

if [ -e "${jman_link}" ] && [ ! -L "${jman_link}" ]; then
  fail "refusing to replace non-symlink ${jman_link}"
fi
temporary_link="${bin_dir}/.jman-link.$$"
rm -f -- "${temporary_link}"
ln -s "${install_dir}/jman" "${temporary_link}"
mv -Tf "${temporary_link}" "${jman_link}"
temporary_link=""
say "Linked ${jman_link} to JMAN ${version}."

shell_name="$(detect_shell)"
configured_java=0
setup_output="${temporary_dir}/java-setup-output"
case "${setup_java}" in
  0 | false | no) ;;
  *)
    if "${install_dir}/jman" java setup --shell "${shell_name}" --no-progress \
      >"${setup_output}" 2>&1; then
      configured_java=1
    elif ! grep -Fq 'no global Java is selected' "${setup_output}"; then
      cat "${setup_output}" >&2
      fail 'could not configure Java command shims'
    elif confirm_java_install; then
      "${install_dir}/jman" java install "${java_version}" --global --no-progress
      if ! "${install_dir}/jman" java setup --shell "${shell_name}" --no-progress \
        >"${setup_output}" 2>&1; then
        cat "${setup_output}" >&2
        fail 'could not configure Java command shims'
      fi
      configured_java=1
    fi
    ;;
esac

if [ "${configured_java}" -eq 1 ]; then
  say 'Java command shims are ready.'
else
  say ''
  say 'Java setup was skipped. Finish it later with:'
  say "  ${jman_link} java install ${java_version} --global"
  say "  ${jman_link} java setup --shell ${shell_name}"
fi

config_file="$(shell_config_file "${shell_name}")"
say ''
say "Add the following lines near the end of ${config_file}:"
say ''
escaped_bin_dir="$(printf '%s' "${bin_dir}" | sed "s/'/'\\\\''/g")"
if [ "${shell_name}" = fish ]; then
  say "  fish_add_path '${escaped_bin_dir}'"
else
  say "  export PATH='${escaped_bin_dir}':\"\$PATH\""
fi
say "  eval \"\$(jman shell init ${shell_name})\""
say ''
say "Then restart ${shell_name}, or evaluate those lines in the current shell."
say "JMAN ${version} installation is complete."
