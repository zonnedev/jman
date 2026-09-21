#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${project_dir}"

status=0
mapfile -t markdown_files < <(
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
  done < <(rg --no-filename --only-matching '\[[^]]+\]\([^)]+\)' "${source}")
done

mapfile -t commands < <(
  sed -n '/^enum Command {/,/^}/p' crates/jman-cli/src/main.rs \
    | sed -n 's/^    \([A-Z][A-Za-z]*\)(.*/\1/p' \
    | tr '[:upper:]' '[:lower:]'
)
for command in "${commands[@]}"; do
  if ! rg --quiet --fixed-strings "## jman ${command}" docs/reference/cli.md; then
    printf 'missing CLI reference section: jman %s\n' "${command}" >&2
    status=1
  fi
done

mapfile -t java_commands < <(
  sed -n '/^enum JavaCommand {/,/^}/p' crates/jman-cli/src/main.rs \
    | sed -n 's/^    \([A-Z][A-Za-z]*\)(.*/\1/p' \
    | tr '[:upper:]' '[:lower:]'
)
for command in "${java_commands[@]}"; do
  if ! rg --quiet --fixed-strings "## jman java ${command}" docs/reference/cli.md; then
    printf 'missing Java CLI reference section: jman java %s\n' "${command}" >&2
    status=1
  fi
done

manifest_sections=(
  project toolchain dependencies annotation-processors path-dependencies
  repositories build test.coverage publishing audit maven
)
for section in "${manifest_sections[@]}"; do
  if ! rg --quiet --fixed-strings "${section}" docs/reference/manifest.md; then
    printf 'missing manifest reference: %s\n' "${section}" >&2
    status=1
  fi
done

if rg --quiet 'jman add [^[:space:]]+:[^[:space:]@]+:[^[:space:]]+' README.md docs; then
  printf 'documentation contains obsolete group:artifact:version add syntax\n' >&2
  status=1
fi

if ! rg --quiet --fixed-strings \
  'DOCS_RUN := $(UV) run --isolated --no-project --with-requirements docs/requirements.txt' \
  Makefile; then
  printf 'documentation targets must provision the pinned requirements\n' >&2
  status=1
fi

if rg --quiet --fixed-strings 'mkdocs-material' docs/requirements.txt \
    || ! rg --quiet --fixed-strings 'name: mkdocs' mkdocs.yml; then
  printf 'documentation must use the built-in MkDocs theme\n' >&2
  status=1
fi

if [[ "${status}" -ne 0 ]]; then
  exit "${status}"
fi

printf 'Documentation links and public command/configuration coverage passed.\n'
