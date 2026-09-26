#!/usr/bin/env bash

# Shared host-platform helpers for build and packaging scripts.

jman_host_os() {
  case "$(uname -s)" in
    Linux) printf '%s\n' linux ;;
    Darwin) printf '%s\n' macos ;;
    *) return 1 ;;
  esac
}

jman_host_architecture() {
  case "$(uname -m)" in
    x86_64 | amd64) printf '%s\n' x86_64 ;;
    arm64 | aarch64) printf '%s\n' aarch64 ;;
    *) return 1 ;;
  esac
}

jman_release_platform() {
  local os architecture
  os="$(jman_host_os)" || return 1
  architecture="$(jman_host_architecture)" || return 1
  case "${os}-${architecture}" in
    linux-x86_64 | macos-aarch64) printf '%s\n' "${os}-${architecture}" ;;
    *) return 1 ;;
  esac
}

jman_vscode_target() {
  case "$(jman_release_platform)" in
    linux-x86_64) printf '%s\n' linux-x64 ;;
    macos-aarch64) printf '%s\n' darwin-arm64 ;;
    *) return 1 ;;
  esac
}

jman_native_library_name() {
  case "$(jman_host_os)" in
    linux) printf '%s\n' libjman_javac_frontend.so ;;
    macos) printf '%s\n' libjman_javac_frontend.dylib ;;
    *) return 1 ;;
  esac
}

jman_sha256_file() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${file}" | awk '{ print $1 }'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${file}" | awk '{ print $1 }'
  else
    printf 'Neither sha256sum nor shasum is available\n' >&2
    return 1
  fi
}

jman_sha256_stream() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | awk '{ print $1 }'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 | awk '{ print $1 }'
  else
    printf 'Neither sha256sum nor shasum is available\n' >&2
    return 1
  fi
}
