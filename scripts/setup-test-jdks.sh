#!/usr/bin/env bash
set -Eeuo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bootstrap_version="0.7.1"
bootstrap_sha256="989346585606ce1ebf731c4178033936e9b0e1fce397075de84cd11a559e8a31"
bootstrap_archive="jman-${bootstrap_version}-linux-x86_64.tar.gz"
bootstrap_url="https://github.com/zonnedev/jman/releases/download/v${bootstrap_version}/${bootstrap_archive}"
toolchains_dir="${JMAN_TEST_TOOLCHAINS_DIR:-${project_dir}/target/test-toolchains}"
bootstrap_dir="${toolchains_dir}/jman/${bootstrap_version}"
bootstrap_binary="${bootstrap_dir}/jman"
graalvm_environment="${toolchains_dir}/graalvm.env"
compatibility_environment="${toolchains_dir}/compatibility.env"
install_graalvm=0
install_compatibility=0
staging_dir=""
environment_staging=""

usage() {
  echo "usage: $0 [--graalvm | --compatibility | --all]" >&2
}

cleanup() {
  if [[ -n "${staging_dir}" ]]; then
    rm -rf -- "${staging_dir}"
  fi
  if [[ -n "${environment_staging}" ]]; then
    rm -f -- "${environment_staging}"
  fi
}
trap cleanup EXIT

if [[ "$#" -eq 0 ]]; then
  install_graalvm=1
  install_compatibility=1
else
  for argument in "$@"; do
    case "${argument}" in
      --graalvm) install_graalvm=1 ;;
      --compatibility) install_compatibility=1 ;;
      --all)
        install_graalvm=1
        install_compatibility=1
        ;;
      -h | --help)
        usage
        exit 0
        ;;
      *)
        usage
        exit 2
        ;;
    esac
  done
fi

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64 | Linux-amd64) ;;
  *)
    echo "JMAN test toolchains currently require Linux x86-64" >&2
    exit 1
    ;;
esac

mkdir -p \
  "${toolchains_dir}" \
  "${toolchains_dir}/jman" \
  "${toolchains_dir}/data" \
  "${toolchains_dir}/cache" \
  "${toolchains_dir}/config"

export JMAN_DATA_DIR="${toolchains_dir}/data"
export JMAN_CACHE_DIR="${toolchains_dir}/cache"
export JMAN_CONFIG_DIR="${toolchains_dir}/config"

jman_version() {
  "$1" --version 2>/dev/null | awk '{ print $2 }'
}

if [[ -n "${JMAN_TEST_BOOTSTRAP_BINARY:-}" ]]; then
  bootstrap_binary="${JMAN_TEST_BOOTSTRAP_BINARY}"
  if [[ ! -x "${bootstrap_binary}" ]]; then
    echo "JMAN test bootstrap is not executable: ${bootstrap_binary}" >&2
    exit 1
  fi
elif [[ ! -x "${bootstrap_binary}" ]]; then
  if [[ -e "${bootstrap_dir}" ]]; then
    echo "JMAN test bootstrap is incomplete: ${bootstrap_dir}" >&2
    echo "Remove that generated directory and retry." >&2
    exit 1
  fi
  for command in curl tar sha256sum mktemp; do
    if ! command -v "${command}" >/dev/null 2>&1; then
      echo "Required JMAN bootstrap command is unavailable: ${command}" >&2
      exit 1
    fi
  done
  staging_dir="$(mktemp -d "${toolchains_dir}/.jman-${bootstrap_version}.XXXXXX")"
  curl --fail --location --silent --show-error \
    --proto '=https' --proto-redir '=https' --tlsv1.2 \
    "${bootstrap_url}" \
    --output "${staging_dir}/${bootstrap_archive}"
  printf '%s  %s\n' \
    "${bootstrap_sha256}" \
    "${staging_dir}/${bootstrap_archive}" | sha256sum --check -
  mkdir -p "${staging_dir}/distribution"
  tar -xzf "${staging_dir}/${bootstrap_archive}" \
    -C "${staging_dir}/distribution" \
    --strip-components=1
  if [[ ! -x "${staging_dir}/distribution/jman" ]]; then
    echo "JMAN ${bootstrap_version} archive does not contain an executable" >&2
    exit 1
  fi
  if [[ "$(jman_version "${staging_dir}/distribution/jman")" != "${bootstrap_version}" ]]; then
    echo "Downloaded JMAN bootstrap reports an unexpected version" >&2
    exit 1
  fi
  mv -- "${staging_dir}/distribution" "${bootstrap_dir}"
  rm -rf -- "${staging_dir}"
  staging_dir=""
fi

if [[ "$(jman_version "${bootstrap_binary}")" != "${bootstrap_version}" ]]; then
  echo "JMAN test bootstrap must report version ${bootstrap_version}: ${bootstrap_binary}" >&2
  exit 1
fi

graalvm_override="${JMAN_GRAALVM_HOME:-}"
java_17_override="${JMAN_TEST_JAVA_17_HOME:-}"
java_21_override="${JMAN_TEST_JAVA_21_HOME:-}"
java_25_override="${JMAN_TEST_JAVA_25_HOME:-}"
if [[ -f "${graalvm_environment}" ]]; then
  # shellcheck disable=SC1090
  source "${graalvm_environment}"
fi
if [[ -f "${compatibility_environment}" ]]; then
  # shellcheck disable=SC1090
  source "${compatibility_environment}"
fi
if [[ -n "${graalvm_override}" ]]; then
  JMAN_GRAALVM_HOME="${graalvm_override}"
fi
if [[ -n "${java_17_override}" ]]; then
  JMAN_TEST_JAVA_17_HOME="${java_17_override}"
fi
if [[ -n "${java_21_override}" ]]; then
  JMAN_TEST_JAVA_21_HOME="${java_21_override}"
fi
if [[ -n "${java_25_override}" ]]; then
  JMAN_TEST_JAVA_25_HOME="${java_25_override}"
fi

valid_jdk() {
  local home="$1" expected_major="$2" actual_major
  [[ -x "${home}/bin/java" && -x "${home}/bin/javac" ]] || return 1
  actual_major="$(
    "${home}/bin/java" -version 2>&1 \
      | sed -n '1s/.*version "\([0-9][0-9]*\).*/\1/p'
  )"
  [[ "${actual_major}" == "${expected_major}" ]]
}

valid_graalvm() {
  local home="$1" expected_major="$2"
  valid_jdk "${home}" "${expected_major}" && [[ -x "${home}/bin/native-image" ]]
}

resolve_jdk_home() {
  local version="$1" vendor="$2"
  "${bootstrap_binary}" java exec "${version}" --vendor "${vendor}" -- \
    /bin/sh -c 'printf "%s\n" "$JAVA_HOME"'
}

install_jdk() {
  local variable="$1" version="$2" major="$3" vendor="$4" requirement="$5" explicit="$6"
  local home="${!variable:-}"
  if [[ -n "${home}" ]]; then
    if "${requirement}" "${home}" "${major}"; then
      printf -v "${variable}" '%s' "${home}"
      export "${variable}"
      return
    fi
    if [[ -n "${explicit}" ]]; then
      echo "Configured ${variable} is incomplete: ${home}" >&2
      exit 1
    fi
  fi

  "${bootstrap_binary}" java install "${version}" --vendor "${vendor}" --no-progress
  home="$(resolve_jdk_home "${version}" "${vendor}")"
  if ! "${requirement}" "${home}" "${major}"; then
    echo "JMAN installed an incomplete ${vendor} JDK ${version}: ${home}" >&2
    exit 1
  fi
  printf -v "${variable}" '%s' "${home}"
  export "${variable}"
}

write_environment() {
  local destination="$1"
  shift
  environment_staging="$(mktemp "${destination}.XXXXXX")"
  : > "${environment_staging}"
  for variable in "$@"; do
    printf 'export %s=%q\n' "${variable}" "${!variable}" >> "${environment_staging}"
  done
  mv -- "${environment_staging}" "${destination}"
  environment_staging=""
}

if [[ "${install_graalvm}" -eq 1 ]]; then
  install_jdk \
    JMAN_GRAALVM_HOME \
    25.3.4.1 \
    25 \
    graalvm-community \
    valid_graalvm \
    "${graalvm_override}"
  write_environment "${graalvm_environment}" JMAN_GRAALVM_HOME
  printf 'JMAN test GraalVM: %s\n' "${JMAN_GRAALVM_HOME}"
fi

if [[ "${install_compatibility}" -eq 1 ]]; then
  install_jdk JMAN_TEST_JAVA_17_HOME 17 17 temurin valid_jdk "${java_17_override}"
  install_jdk JMAN_TEST_JAVA_21_HOME 21 21 temurin valid_jdk "${java_21_override}"
  install_jdk JMAN_TEST_JAVA_25_HOME 25 25 temurin valid_jdk "${java_25_override}"
  write_environment \
    "${compatibility_environment}" \
    JMAN_TEST_JAVA_17_HOME \
    JMAN_TEST_JAVA_21_HOME \
    JMAN_TEST_JAVA_25_HOME
  printf 'JMAN compatibility JDKs: 17=%s 21=%s 25=%s\n' \
    "${JMAN_TEST_JAVA_17_HOME}" \
    "${JMAN_TEST_JAVA_21_HOME}" \
    "${JMAN_TEST_JAVA_25_HOME}"
fi
