#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 3 ]]; then
  echo "usage: ensure-test-repository.sh REPOSITORY_URL COMMIT DESTINATION" >&2
  exit 2
fi

repository="$1"
revision="$2"
destination="$3"

if [[ ! "${revision}" =~ ^[0-9a-f]{40}$ ]]; then
  echo "Test repository revision must be a full Git commit SHA: ${revision}" >&2
  exit 2
fi

if [[ -e "${destination}" ]]; then
  if [[ ! -d "${destination}/.git" ]]; then
    echo "Test repository destination is not a Git checkout: ${destination}" >&2
    exit 1
  fi
  actual="$(git -C "${destination}" rev-parse HEAD)"
  if [[ "${actual}" != "${revision}" ]] || [[ -n "$(git -C "${destination}" status --porcelain)" ]]; then
    echo "Test repository at ${destination} is not a clean checkout of ${revision}" >&2
    exit 1
  fi
  printf '%s\n' "${destination}"
  exit 0
fi

parent="$(dirname "${destination}")"
mkdir -p "${parent}"
staging=""
cleanup() {
  if [[ -n "${staging}" ]]; then
    rm -rf -- "${staging}"
  fi
}
trap cleanup EXIT
staging="$(mktemp -d "${parent}/.test-repository.XXXXXX")"
git -C "${staging}" init -q
git -C "${staging}" remote add origin "${repository}"
git -C "${staging}" fetch -q --depth 1 origin "${revision}"
git -C "${staging}" checkout -q --detach FETCH_HEAD
actual="$(git -C "${staging}" rev-parse HEAD)"
if [[ "${actual}" != "${revision}" ]]; then
  echo "Fetched ${actual} instead of ${revision} from ${repository}" >&2
  exit 1
fi
mv -- "${staging}" "${destination}"
staging=""
trap - EXIT
printf '%s\n' "${destination}"
