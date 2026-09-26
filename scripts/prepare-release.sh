#!/usr/bin/env bash
set -Eeuo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
requested_version="${1:-}"

if [[ -z "${requested_version}" ]]; then
  echo "usage: $0 <version>" >&2
  exit 2
fi

version="${requested_version#v}"
if [[ ! "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]]; then
  echo "Invalid release version: ${requested_version}" >&2
  exit 2
fi

tag="v${version}"
release_date="${JMAN_RELEASE_DATE:-$(date +%F)}"
versioned_files=(
  Cargo.toml
  Cargo.lock
  CHANGELOG.md
  .github/workflows/release.yml
  docs/jman-java.md
  docs/releasing.md
  docs/vscode-release-checklist.md
  editors/neovim/CHANGELOG.md
  editors/vscode/CHANGELOG.md
  editors/vscode/package.json
  editors/vscode/package-lock.json
)

cd "${project_dir}"

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "Release preparation must run inside a Git worktree" >&2
  exit 1
fi
if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
  echo "Release preparation requires a clean worktree" >&2
  exit 1
fi
if git show-ref --verify --quiet "refs/tags/${tag}"; then
  echo "Release tag ${tag} already exists" >&2
  exit 1
fi
if ! branch="$(git symbolic-ref --quiet --short HEAD)"; then
  echo "Release preparation requires a checked-out branch, not detached HEAD" >&2
  exit 1
fi

old_version="$({
  awk '
    $0 == "[workspace.package]" { in_workspace_package = 1; next }
    /^\[/ { in_workspace_package = 0 }
    in_workspace_package && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' Cargo.toml
})"
if [[ -z "${old_version}" ]]; then
  echo "Could not read workspace.package.version from Cargo.toml" >&2
  exit 1
fi
if [[ "${old_version}" == "${version}" ]]; then
  echo "Workspace is already at version ${version}" >&2
  exit 1
fi

workspace_packages=()
while IFS= read -r package; do
  workspace_packages+=("${package}")
done < <(
  cargo metadata --format-version 1 --no-deps --locked |
    node -e '
      let input = "";
      process.stdin.setEncoding("utf8");
      process.stdin.on("data", chunk => input += chunk);
      process.stdin.on("end", () => {
        const metadata = JSON.parse(input);
        const members = new Set(metadata.workspace_members);
        for (const pkg of metadata.packages) {
          if (members.has(pkg.id)) console.log(pkg.name);
        }
      });
    '
)
if [[ "${#workspace_packages[@]}" -eq 0 ]]; then
  echo "Could not discover Cargo workspace packages" >&2
  exit 1
fi

restore_on_error=1
restore_release_files() {
  local exit_code=$?
  if [[ "${restore_on_error}" -eq 1 ]]; then
    git restore -- "${versioned_files[@]}"
    echo "Release preparation failed; restored versioned files" >&2
  fi
  exit "${exit_code}"
}
trap restore_release_files ERR

old_version_pattern="${old_version//./\\.}"
sed -i -E "/^\[workspace\.package\]$/,/^\[/ {
  s/^version = \"${old_version_pattern}\"$/version = \"${version}\"/
}" Cargo.toml

node - Cargo.lock "${old_version}" "${version}" "${workspace_packages[@]}" <<'NODE'
const fs = require("fs");
const [lockPath, oldVersion, newVersion, ...packageNames] = process.argv.slice(2);
const expected = new Set(packageNames);
const updated = new Set();
const contents = fs.readFileSync(lockPath, "utf8");
const sections = contents.split(/(?=^\[\[package\]\]\n)/m).map(section => {
  const name = section.match(/^name = "([^"]+)"$/m)?.[1];
  if (!name || !expected.has(name) || /^source = /m.test(section)) return section;
  const version = section.match(/^version = "([^"]+)"$/m)?.[1];
  if (version !== oldVersion) {
    throw new Error(`Cargo.lock has ${name} at ${version ?? "no version"}, expected ${oldVersion}`);
  }
  updated.add(name);
  return section.replace(/^version = "[^"]+"$/m, `version = "${newVersion}"`);
});
const missing = [...expected].filter(name => !updated.has(name));
if (missing.length > 0) {
  throw new Error(`Cargo.lock is missing workspace packages: ${missing.join(", ")}`);
}
fs.writeFileSync(lockPath, sections.join(""));
NODE

npm version "${version}" --no-git-tag-version --ignore-scripts \
  --prefix editors/vscode >/dev/null
cargo metadata --format-version 1 --no-deps --locked >/dev/null

replace_documented_version() {
  local file=$1
  sed -i -E \
    -e "s/v${old_version_pattern}/v${version}/g" \
    -e "s/${old_version_pattern}/${version}/g" \
    "${file}"
}

replace_documented_version .github/workflows/release.yml
replace_documented_version docs/jman-java.md
replace_documented_version docs/releasing.md
replace_documented_version docs/vscode-release-checklist.md

add_changelog_release() (
  local file=$1
  local unreleased_heading=$2
  local release_heading=$3
  local temporary_file=""

  cleanup() {
    if [[ -n "${temporary_file}" ]]; then
      rm -f -- "${temporary_file}"
    fi
  }
  trap cleanup EXIT

  if grep -Fqx "${release_heading}" "${file}"; then
    exit 0
  fi
  temporary_file="$(mktemp "${file}.XXXXXX")"
  awk -v unreleased="${unreleased_heading}" -v release="${release_heading}" '
    { print }
    !inserted && $0 == unreleased {
      print ""
      print release
      inserted = 1
    }
    END {
      if (!inserted) {
        exit 1
      }
    }
  ' "${file}" > "${temporary_file}"
  mv "${temporary_file}" "${file}"
  temporary_file=""
)

add_changelog_release \
  CHANGELOG.md \
  '## [Unreleased]' \
  "## [${version}] - ${release_date}"
add_changelog_release \
  editors/vscode/CHANGELOG.md \
  '## Unreleased' \
  "## ${version} - ${release_date}"
add_changelog_release \
  editors/neovim/CHANGELOG.md \
  '## Unreleased' \
  "## ${version} - ${release_date}"

./scripts/verify-release-version.sh "${tag}"
git diff --check

restore_on_error=0
trap - ERR

printf '\nPrepared release %s.\n' "${tag}"
git status --short
printf '\nWould you like to commit and tag %s? [y/N]\n' "${tag}"
if ! read -r answer; then
  answer=''
fi
case "${answer}" in
  y | Y | yes | YES | Yes)
    git add -- "${versioned_files[@]}"
    git commit -m "chore(release): prepare ${tag}"
    git tag -a "${tag}" -m "${tag}"
    printf '\nWould you like to push release %s? [y/N]\n' "${tag}"
    if ! read -r push_answer; then
      push_answer=''
    fi
    case "${push_answer}" in
      y | Y | yes | YES | Yes)
        git push --atomic origin \
          "HEAD:refs/heads/${branch}" \
          "refs/tags/${tag}"
        printf 'Pushed branch %s and release tag %s to origin.\n' "${branch}" "${tag}"
        ;;
      *)
        printf 'Commit and tag remain local. Push them later with:\n'
        printf '  git push --atomic origin HEAD:refs/heads/%s refs/tags/%s\n' \
          "${branch}" "${tag}"
        ;;
    esac
    ;;
  *)
    printf 'Release files remain uncommitted for review; no tag was created.\n'
    ;;
esac
