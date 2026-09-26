#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi
fixture_dir=""
remote_dir=""
cleanup() {
  if [[ -n "${fixture_dir}" ]]; then
    rm -rf -- "${fixture_dir}"
  fi
  if [[ -n "${remote_dir}" ]]; then
    rm -rf -- "${remote_dir}"
  fi
}
trap cleanup EXIT

fixture_dir="$(mktemp -d "${TMPDIR:-/tmp}/jman-prepare-release.XXXXXX")"
remote_dir="$(mktemp -d "${TMPDIR:-/tmp}/jman-prepare-release-remote.XXXXXX")"

mkdir -p \
  "${fixture_dir}/.github/workflows" \
  "${fixture_dir}/crates/demo/src" \
  "${fixture_dir}/docs" \
  "${fixture_dir}/editors/neovim" \
  "${fixture_dir}/editors/vscode" \
  "${fixture_dir}/scripts"

cat > "${fixture_dir}/Cargo.toml" <<'EOF'
[workspace]
members = ["crates/demo"]
resolver = "2"

[workspace.package]
version = "0.5.0"
edition = "2021"
EOF
cat > "${fixture_dir}/crates/demo/Cargo.toml" <<'EOF'
[package]
name = "demo"
version.workspace = true
edition.workspace = true
EOF
cat > "${fixture_dir}/crates/demo/src/lib.rs" <<'EOF'
pub fn demo() {}
EOF
cat > "${fixture_dir}/editors/vscode/package.json" <<'EOF'
{
  "name": "jman-java",
  "version": "0.5.0"
}
EOF
cat > "${fixture_dir}/CHANGELOG.md" <<'EOF'
# Changelog

## [Unreleased]

### Added

- Prepared feature.

## [0.5.0] - 2026-09-01
EOF
cat > "${fixture_dir}/editors/vscode/CHANGELOG.md" <<'EOF'
# Change Log

## Unreleased

- Prepared editor feature.

## 0.5.0 - 2026-09-01
EOF
cat > "${fixture_dir}/editors/neovim/CHANGELOG.md" <<'EOF'
# Changelog

## Unreleased

- Prepared editor feature.

## 0.5.0 - 2026-09-01
EOF
printf '%s\n' 'example v0.5.0' > "${fixture_dir}/.github/workflows/release.yml"
printf '%s\n' 'jman-java-0.5.0-linux-x64.vsix' > "${fixture_dir}/docs/jman-java.md"
printf '%s\n' 'git tag -a v0.5.0 -m "v0.5.0"' > "${fixture_dir}/docs/releasing.md"
printf '%s\n' 'jman-java-0.5.0-linux-x64.vsix' > "${fixture_dir}/docs/vscode-release-checklist.md"
cat > "${fixture_dir}/scripts/setup-test-jdks.sh" <<'EOF'
#!/usr/bin/env bash
bootstrap_version="0.5.0"
bootstrap_sha256="bootstrap-checksum-is-version-independent"
EOF

cp "${project_dir}/scripts/prepare-release.sh" "${fixture_dir}/scripts/"
cp "${project_dir}/scripts/verify-release-version.sh" "${fixture_dir}/scripts/"
chmod +x "${fixture_dir}/scripts/prepare-release.sh" "${fixture_dir}/scripts/verify-release-version.sh"

(
  cd "${fixture_dir}"
  cargo generate-lockfile
  npm install --package-lock-only --ignore-scripts --prefix editors/vscode >/dev/null
  git init --quiet
  git config user.name 'JMAN Release Test'
  git config user.email 'release-test@jman.invalid'
  git config commit.gpgSign false
  git config tag.gpgSign false
  git add --all
  git commit --quiet -m 'fixture'
  git init --bare --quiet "${remote_dir}"
  git remote add origin "${remote_dir}"

  if ./scripts/prepare-release.sh invalid </dev/null >/dev/null 2>&1; then
    echo "Invalid release version was accepted" >&2
    exit 1
  fi

  printf 'dirty\n' >> docs/jman-java.md
  if ./scripts/prepare-release.sh 0.5.1 </dev/null >/dev/null 2>&1; then
    echo "Dirty worktree was accepted" >&2
    exit 1
  fi
  git restore docs/jman-java.md

  initial_commit="$(git rev-parse HEAD)"
  decline_output="$(
    JMAN_RELEASE_DATE=2026-09-20 ./scripts/prepare-release.sh 0.5.1 <<<'n'
  )"
  printf '%s\n' "${decline_output}"
  grep -Fqx 'Would you like to commit and tag v0.5.1? [y/N]' <<<"${decline_output}"
  grep -Fqx 'Release files remain uncommitted for review; no tag was created.' \
    <<<"${decline_output}"
  grep -q '^version = "0.5.1"$' Cargo.toml
  grep -A2 '^name = "demo"$' Cargo.lock | grep -q '^version = "0.5.1"$'
  test "$(node -p 'require("./editors/vscode/package.json").version')" = 0.5.1
  test "$(node -p 'require("./editors/vscode/package-lock.json").version')" = 0.5.1
  grep -Fqx '## [0.5.1] - 2026-09-20' CHANGELOG.md
  grep -Fqx '## 0.5.1 - 2026-09-20' editors/vscode/CHANGELOG.md
  grep -Fqx '## 0.5.1 - 2026-09-20' editors/neovim/CHANGELOG.md
  grep -q 'v0.5.1' docs/releasing.md
  grep -Fqx 'bootstrap_version="0.5.0"' scripts/setup-test-jdks.sh
  grep -Fqx \
    'bootstrap_sha256="bootstrap-checksum-is-version-independent"' \
    scripts/setup-test-jdks.sh
  test "$(git rev-parse HEAD)" = "${initial_commit}"
  if git show-ref --verify --quiet refs/tags/v0.5.1; then
    echo "Declining release confirmation created a tag" >&2
    exit 1
  fi

  git restore .
  publish_output="$(
    JMAN_RELEASE_DATE=2026-09-20 ./scripts/prepare-release.sh v0.5.1 <<'EOF'
yes
yes
EOF
  )"
  printf '%s\n' "${publish_output}"
  grep -Fqx 'Would you like to commit and tag v0.5.1? [y/N]' <<<"${publish_output}"
  grep -Fqx 'Would you like to push release v0.5.1? [y/N]' <<<"${publish_output}"
  test "$(git log -1 --pretty=%s)" = 'chore(release): prepare v0.5.1'
  test "$(git cat-file -t v0.5.1)" = tag
  test "$(git tag -l v0.5.1 --format='%(contents:subject)')" = v0.5.1
  test -z "$(git status --porcelain --untracked-files=normal)"
  branch="$(git symbolic-ref --short HEAD)"
  test "$(git rev-parse HEAD)" = \
    "$(git --git-dir="${remote_dir}" rev-parse "refs/heads/${branch}")"
  test "$(git rev-parse v0.5.1^{})" = \
    "$(git --git-dir="${remote_dir}" rev-parse refs/tags/v0.5.1^{})"

  if ./scripts/prepare-release.sh 0.5.1 </dev/null >/dev/null 2>&1; then
    echo "Existing release tag was accepted" >&2
    exit 1
  fi

  prerelease_output="$(
    JMAN_RELEASE_DATE=2026-09-21 ./scripts/prepare-release.sh 0.6.0-rc.1 <<<'n'
  )"
  printf '%s\n' "${prerelease_output}"
  grep -Fqx 'Would you like to commit and tag v0.6.0-rc.1? [y/N]' \
    <<<"${prerelease_output}"
  grep -q '^version = "0.6.0-rc.1"$' Cargo.toml
  grep -A2 '^name = "demo"$' Cargo.lock | grep -q '^version = "0.6.0-rc.1"$'
  test "$(node -p 'require("./editors/vscode/package.json").version')" = 0.6.0-rc.1
  test "$(node -p 'require("./editors/vscode/package-lock.json").version')" = 0.6.0-rc.1
  grep -Fqx '## [0.6.0-rc.1] - 2026-09-21' CHANGELOG.md
  grep -Fqx '## 0.6.0-rc.1 - 2026-09-21' editors/vscode/CHANGELOG.md
  grep -Fqx '## 0.6.0-rc.1 - 2026-09-21' editors/neovim/CHANGELOG.md
  grep -q 'v0.6.0-rc.1' docs/releasing.md
  grep -Fqx 'bootstrap_version="0.5.0"' scripts/setup-test-jdks.sh
  grep -Fqx \
    'bootstrap_sha256="bootstrap-checksum-is-version-independent"' \
    scripts/setup-test-jdks.sh
  test "$(git rev-parse HEAD)" = \
    "$(git --git-dir="${remote_dir}" rev-parse refs/heads/"$(git symbolic-ref --short HEAD)")"
  if git show-ref --verify --quiet refs/tags/v0.6.0-rc.1; then
    echo 'Declining prerelease confirmation created a tag' >&2
    exit 1
  fi
  git restore .

  sed -i '/^## \[Unreleased\]$/d' CHANGELOG.md
  git add CHANGELOG.md
  git commit --quiet -m 'break changelog fixture'
  if ./scripts/prepare-release.sh 0.5.2 </dev/null >/dev/null 2>&1; then
    echo "Missing changelog heading was accepted" >&2
    exit 1
  fi
  if compgen -G 'CHANGELOG.md.*' >/dev/null; then
    echo "Failed changelog update left a temporary file behind" >&2
    exit 1
  fi
)

printf 'Release preparation tests passed\n'
