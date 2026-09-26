#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tools_dir="${project_dir}/target/compatibility-tools"
install_maven=0
install_gradle=0

if [[ "$#" -eq 0 ]]; then
  install_maven=1
  install_gradle=1
else
  for argument in "$@"; do
    case "${argument}" in
      --maven) install_maven=1 ;;
      --gradle) install_gradle=1 ;;
      --all)
        install_maven=1
        install_gradle=1
        ;;
      *)
        echo "usage: $0 [--maven | --gradle | --all]" >&2
        exit 2
        ;;
    esac
  done
fi

mkdir -p "${tools_dir}"

download_archive() {
  local destination="$1"
  shift
  local url
  for url in "$@"; do
    rm -f -- "${destination}"
    if curl \
      --fail \
      --location \
      --silent \
      --show-error \
      --connect-timeout 30 \
      --retry 5 \
      --retry-all-errors \
      --retry-delay 2 \
      --retry-max-time 180 \
      --remove-on-error \
      --output "${destination}" \
      "${url}"; then
      return 0
    fi
  done
  printf 'Unable to download compatibility tool from any configured source\n' >&2
  return 1
}

download_gradle() (
  local version="$1" checksum="$2"
  local installation="${tools_dir}/gradle-${version}"
  if [[ -x "${installation}/bin/gradle" ]]; then
    exit 0
  fi
  local staging="" archive
  cleanup() {
    if [[ -n "${staging}" ]]; then
      rm -rf -- "${staging}"
    fi
  }
  trap cleanup EXIT

  staging="$(mktemp -d "${tools_dir}/.gradle-${version}.XXXXXX")"
  archive="${staging}/gradle.zip"
  download_archive \
    "${archive}" \
    "https://services.gradle.org/distributions/gradle-${version}-bin.zip" \
    "https://downloads.gradle.org/distributions/gradle-${version}-bin.zip"
  printf '%s  %s\n' "${checksum}" "${archive}" | sha256sum --check -
  unzip -q "${archive}" -d "${staging}"
  mv -- "${staging}/gradle-${version}" "${installation}"
)

download_maven() (
  local installation="${tools_dir}/apache-maven-3.9.9"
  if [[ -x "${installation}/bin/mvn" ]]; then
    exit 0
  fi
  local staging="" archive
  cleanup() {
    if [[ -n "${staging}" ]]; then
      rm -rf -- "${staging}"
    fi
  }
  trap cleanup EXIT

  staging="$(mktemp -d "${tools_dir}/.maven-3.9.9.XXXXXX")"
  archive="${staging}/maven.tar.gz"
  download_archive \
    "${archive}" \
    https://archive.apache.org/dist/maven/maven-3/3.9.9/binaries/apache-maven-3.9.9-bin.tar.gz \
    https://repo.maven.apache.org/maven2/org/apache/maven/apache-maven/3.9.9/apache-maven-3.9.9-bin.tar.gz
  printf '%s  %s\n' \
    a555254d6b53d267965a3404ecb14e53c3827c09c3b94b5678835887ab404556bfaf78dcfe03ba76fa2508649dca8531c74bca4d5846513522404d48e8c4ac8b \
    "${archive}" | sha512sum --check -
  tar -xzf "${archive}" -C "${staging}"
  mv -- "${staging}/apache-maven-3.9.9" "${installation}"
)

if [[ "${install_maven}" -eq 1 ]]; then
  download_maven
  echo 'Pinned Maven compatibility tool is ready'
fi
if [[ "${install_gradle}" -eq 1 ]]; then
  download_gradle 8.7 544c35d6bd849ae8a5ed0bcea39ba677dc40f49df7d1835561582da2009b961d
  download_gradle 8.14.1 845952a9d6afa783db70bb3b0effaae45ae5542ca2bb7929619e8af49cb634cf
  download_gradle 9.1.0 a17ddd85a26b6a7f5ddb71ff8b05fc5104c0202c6e64782429790c933686c806
  echo 'Pinned Gradle compatibility tools are ready'
fi
