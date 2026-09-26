#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${JMAN_TEST_TEMP_ROOT:-}" ]]; then
  exec "${project_dir}/scripts/run-test-command.sh" "$0" "$@"
fi
workspace_version="$({
  awk '
    $0 == "[workspace.package]" { in_workspace_package = 1; next }
    /^\[/ { in_workspace_package = 0 }
    in_workspace_package && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' "${project_dir}/Cargo.toml"
})"
extension_version="$(node -p 'require(process.argv[1]).version' "${project_dir}/editors/vscode/package.json")"
release_tag="v${workspace_version}"
test_root="${project_dir}/target/release-automation-test"
unsafe_root=""
release_dist_dir="${test_root}/release-dist"
vscode_dist_dir="${test_root}/vscode"
output_dir="${test_root}/github-release"
workflow_dir="${project_dir}/.github/workflows"
workspace_rust_version="$({
  awk '
    $0 == "[workspace.package]" { in_workspace_package = 1; next }
    /^\[/ { in_workspace_package = 0 }
    in_workspace_package && $1 == "rust-version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' "${project_dir}/Cargo.toml"
})"

for documentation in README.md docs/jman-java.md docs/releasing.md; do
  grep -Fq 'scripts/setup-test-jdks.sh' "${project_dir}/${documentation}"
done

cleanup() {
  rm -rf -- "${test_root}"
  if [[ -n "${unsafe_root}" ]]; then
    rm -rf -- "${unsafe_root}"
  fi
}
trap cleanup EXIT

unsafe_root="$(mktemp -d "/tmp/jman-release-automation-unsafe.XXXXXX")"
rm -rf "${test_root}"
mkdir -p "${release_dist_dir}" "${vscode_dist_dir}"

printf 'cli fixture\n' > "${release_dist_dir}/jman-${workspace_version}-linux-x86_64.tar.gz"
printf 'vsix fixture\n' > "${vscode_dist_dir}/jman-java-${extension_version}-linux-x64.vsix"

"${project_dir}/scripts/verify-release-version.sh" "${release_tag}"
if "${project_dir}/scripts/verify-release-version.sh" "v0.0.0-release-test" >/dev/null 2>&1; then
  echo "Mismatched release tag was accepted" >&2
  exit 1
fi
if "${project_dir}/scripts/verify-release-version.sh" >/dev/null 2>&1; then
  echo "Missing release tag was accepted" >&2
  exit 1
fi
if JMAN_RELEASE_DIST_DIR="${release_dist_dir}" \
  JMAN_VSCODE_DIST_DIR="${vscode_dist_dir}" \
  JMAN_GITHUB_RELEASE_DIR="${unsafe_root}/github-release" \
  "${project_dir}/scripts/stage-release-artifacts.sh" "${release_tag}" >/dev/null 2>&1; then
  echo "Release staging outside the project target directory was accepted" >&2
  exit 1
fi

JMAN_RELEASE_DIST_DIR="${release_dist_dir}" \
JMAN_VSCODE_DIST_DIR="${vscode_dist_dir}" \
JMAN_GITHUB_RELEASE_DIR="${output_dir}" \
  "${project_dir}/scripts/stage-release-artifacts.sh" "${release_tag}"

(
  cd "${output_dir}"
  sha256sum --check SHA256SUMS
)
node -e '
  const fs = require("fs");
  const manifest = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
  if (manifest.releaseTag !== process.argv[2]) throw new Error("release tag mismatch");
  if (manifest.artifacts.length !== 3) throw new Error("artifact count mismatch");
  for (const artifact of manifest.artifacts) {
    if (!/^[0-9a-f]{64}$/.test(artifact.sha256)) throw new Error("invalid digest");
  }
' "${output_dir}/release-manifest.json" "${release_tag}"

test "$(wc -l < "${output_dir}/SHA256SUMS")" -eq 3
test -x "${output_dir}/install.sh"
grep -Fq 'releases/latest/download/release-manifest.json' "${output_dir}/install.sh"
cp "${release_dist_dir}/jman-${workspace_version}-linux-x86_64.tar.gz" \
  "${release_dist_dir}/jman-duplicate-linux-x86_64.tar.gz"
if JMAN_RELEASE_DIST_DIR="${release_dist_dir}" \
  JMAN_VSCODE_DIST_DIR="${vscode_dist_dir}" \
  JMAN_GITHUB_RELEASE_DIR="${output_dir}" \
  "${project_dir}/scripts/stage-release-artifacts.sh" "${release_tag}" >/dev/null 2>&1; then
  echo "Duplicate CLI archives were accepted" >&2
  exit 1
fi

for workflow in ci.yml release.yml publish-vscode.yml verify-vscode-marketplace-identity.yml; do
  test -s "${workflow_dir}/${workflow}"
done
for workflow in ci.yml release.yml; do
  grep -Eq "rustup toolchain install ${workspace_rust_version}([.]|[[:space:]])" \
    "${workflow_dir}/${workflow}"
  grep -q 'neovim/releases/download/v0.11.7/nvim-linux-x86_64.tar.gz' \
    "${workflow_dir}/${workflow}"
  grep -q '38a7c6317f94503841096c00e8fde05ef04b9472fc9d7d62b6e033cecd6f7991' \
    "${workflow_dir}/${workflow}"
  grep -q 'GITHUB_PATH' "${workflow_dir}/${workflow}"
done
grep -q 'make ci' "${workflow_dir}/ci.yml"
grep -q 'make release-gates' "${workflow_dir}/release.yml"
grep -q 'scripts/setup-compatibility-tools.sh' "${workflow_dir}/release.yml"
grep -q 'scripts/setup-test-jdks.sh --graalvm' "${workflow_dir}/ci.yml"
grep -q 'scripts/setup-test-jdks.sh --all' "${workflow_dir}/release.yml"
if grep -R -Eq 'actions/setup-java|graalvm/setup-graalvm' \
  "${workflow_dir}/ci.yml" "${workflow_dir}/release.yml"; then
  echo "CI and release workflows must provision test JDKs through JMAN" >&2
  exit 1
fi
grep -Fq 'bootstrap_version="0.7.1"' "${project_dir}/scripts/setup-test-jdks.sh"
grep -Fq \
  'bootstrap_sha256="989346585606ce1ebf731c4178033936e9b0e1fce397075de84cd11a559e8a31"' \
  "${project_dir}/scripts/setup-test-jdks.sh"
if grep -R -Eq '^rust-version[[:space:]]*=[[:space:]]*"' \
  "${project_dir}/crates"/*/Cargo.toml; then
  echo "Workspace crates must inherit workspace.package.rust-version" >&2
  exit 1
fi
grep -Eq '^test-rust:.*[[:space:]]vineflower([[:space:]]|$)' \
  "${project_dir}/Makefile"
grep -Eq '^test-rust:.*[[:space:]]jacoco([[:space:]]|$)' \
  "${project_dir}/Makefile"
for packaging_target in release package-vscode; do
  if ! grep -Eq "^${packaging_target}:.*[[:space:]]native([[:space:]]|$)" \
    "${project_dir}/Makefile"; then
    echo "${packaging_target} must reuse Make's native frontend artifact" >&2
    exit 1
  fi
  if ! grep -Eq "^${packaging_target}:.*[[:space:]]jacoco([[:space:]]|$)" \
    "${project_dir}/Makefile"; then
    echo "${packaging_target} must prepare pinned JaCoCo coverage tools" >&2
    exit 1
  fi
done
if grep -q 'scripts/build-native.sh' \
  "${project_dir}/scripts/package-release.sh" \
  "${project_dir}/scripts/package-vscode.sh"; then
  echo "Packaging scripts must not rebuild the native frontend" >&2
  exit 1
fi
for packaging_script in package-release.sh package-vscode.sh; do
  grep -q 'jacoco-0.8.15-agent.jar' "${project_dir}/scripts/${packaging_script}"
  grep -q 'jacoco-0.8.15-cli.jar' "${project_dir}/scripts/${packaging_script}"
  grep -q 'native_dir}/platform' "${project_dir}/scripts/${packaging_script}"
done
grep -Fq '"${project_dir}/docs" "${stage}/docs"' \
  "${project_dir}/scripts/package-release.sh"
while IFS= read -r action_reference; do
  if [[ ! "${action_reference}" =~ @[0-9a-f]{40}$ ]]; then
    echo "GitHub Action is not pinned to a full commit: ${action_reference}" >&2
    exit 1
  fi
done < <(sed -n 's/^[[:space:]]*uses:[[:space:]]*\([^ #]*\).*/\1/p' "${workflow_dir}"/*.yml)
grep -q 'azure/login@532459ea530d8321f2fb9bb10d1e0bcf23869a43' \
  "${workflow_dir}/publish-vscode.yml"
grep -q -- '--azure-credential' "${workflow_dir}/publish-vscode.yml"
grep -q 'JMAN_RELEASE_TAG#v' "${workflow_dir}/publish-vscode.yml"
grep -q 'publish_args+=(--pre-release)' "${workflow_dir}/publish-vscode.yml"
grep -q 'release_tag="${JMAN_RELEASE_TAG:-}"' "${project_dir}/scripts/package-vscode.sh"
grep -q 'package_flags+=(--pre-release)' "${project_dir}/scripts/package-vscode.sh"
grep -q 'environment: vscode-marketplace' \
  "${workflow_dir}/verify-vscode-marketplace-identity.yml"
grep -q 'vsce verify-pat --azure-credential zonnedev' \
  "${workflow_dir}/verify-vscode-marketplace-identity.yml"
if grep -R -q 'VSCE_PAT' "${workflow_dir}"; then
  echo "Marketplace workflow must not use a long-lived PAT" >&2
  exit 1
fi

configuration_paths=(
  .github Makefile README.md crates docs editors scripts
)
legacy_environment_names=(
  "JAVA""_LSP_"
  "GSON""_DIR"
  "RELEASE""_TAG"
)
for legacy_name in "${legacy_environment_names[@]}"; do
  if matches="$(git -C "${project_dir}" grep -En \
      "(^|[^A-Z0-9_])${legacy_name}" -- "${configuration_paths[@]}" || true)" \
      && [[ -n "${matches}" ]]; then
    printf 'JMAN-owned environment variable lacks the JMAN_ prefix:\n%s\n' \
      "${matches}" >&2
    exit 1
  fi
done

legacy_frontend="JAVAC""_FRONTEND_"
if matches="$(git -C "${project_dir}" grep -En \
    "(^|[^A-Z0-9_])${legacy_frontend}" -- "${configuration_paths[@]}" \
    | grep -v 'JAVAC_FRONTEND_MODEL' || true)" \
    && [[ -n "${matches}" ]]; then
  printf 'JMAN-owned environment variable lacks the JMAN_ prefix:\n%s\n' \
    "${matches}" >&2
  exit 1
fi

printf 'Release automation tests passed\n'
