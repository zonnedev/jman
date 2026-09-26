#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi
cd "${project_dir}"

status=0
markdown_files=()
while IFS= read -r source; do
  markdown_files+=("${source}")
done < <(
  {
    printf '%s\n' README.md
    find docs -type f -name '*.md'
  } | sort
)

for source in "${markdown_files[@]}"; do
  while IFS= read -r markdown_link; do
    target="${markdown_link#*](}"
    target="${target%)}"
    target="${target%%#*}"

    case "${target}" in
      "" | http://* | https://* | mailto:* )
        continue
        ;;
    esac

    if [[ "${target}" == /* ]]; then
      resolved="${target}"
    else
      resolved="$(realpath -m "$(dirname "${source}")/${target}")"
    fi

    if [[ ! -e "${resolved}" ]]; then
      printf 'broken documentation link: %s -> %s\n' "${source}" "${target}" >&2
      status=1
    fi
  done < <(grep -Eo '\[[^]]+\]\([^)]+\)' "${source}" || true)
done

commands=()
while IFS= read -r command; do
  commands+=("${command}")
done < <(
  sed -n '/^enum Command {/,/^}/p' crates/jman-cli/src/main.rs \
    | sed -n 's/^    \([A-Z][A-Za-z]*\)(.*/\1/p' \
    | tr '[:upper:]' '[:lower:]'
)
for command in "${commands[@]}"; do
  if ! grep -Fq "## jman ${command}" docs/reference/cli.md; then
    printf 'missing CLI reference section: jman %s\n' "${command}" >&2
    status=1
  fi
done

java_commands=()
while IFS= read -r command; do
  java_commands+=("${command}")
done < <(
  sed -n '/^enum JavaCommand {/,/^}/p' crates/jman-cli/src/main.rs \
    | sed -n 's/^    \([A-Z][A-Za-z]*\)(.*/\1/p' \
    | tr '[:upper:]' '[:lower:]'
)
for command in "${java_commands[@]}"; do
  if ! grep -Fq "## jman java ${command}" docs/reference/cli.md; then
    printf 'missing Java CLI reference section: jman java %s\n' "${command}" >&2
    status=1
  fi
done

manifest_sections=(
  project toolchain dependencies annotation-processors path-dependencies
  repositories build test.coverage publishing audit maven
)
for section in "${manifest_sections[@]}"; do
  if ! grep -Fq "${section}" docs/reference/manifest.md; then
    printf 'missing manifest reference: %s\n' "${section}" >&2
    status=1
  fi
done

if grep -ERq 'jman add [^[:space:]]+:[^[:space:]@]+:[^[:space:]]+' README.md docs; then
  printf 'documentation contains obsolete group:artifact:version add syntax\n' >&2
  status=1
fi

if ! grep -Fq \
  'DOCS_RUN := $(UV) run --isolated --no-project --with-requirements docs/requirements.txt' \
  Makefile; then
  printf 'documentation targets must provision the pinned requirements\n' >&2
  status=1
fi

if grep -Fq 'mkdocs-material' docs/requirements.txt \
    || ! grep -Fq 'name: mkdocs' mkdocs.yml; then
  printf 'documentation must use the built-in MkDocs theme\n' >&2
  status=1
fi

if grep -Eq 'approaches a stable 0[.]3[.]0|contract for JMAN 0[.]1[.]0|0[.]1[.]x contract' \
    CHANGELOG.md docs/product-contract.md; then
  printf 'current release documentation contains a stale historical version contract\n' >&2
  status=1
fi

if [[ "${status}" -ne 0 ]]; then
  exit "${status}"
fi

printf 'Documentation links and public command/configuration coverage passed.\n'
