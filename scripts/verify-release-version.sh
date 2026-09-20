#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
release_tag="${1:-}"

if [[ -z "${release_tag}" ]]; then
  echo "usage: $0 v<workspace-version>" >&2
  exit 2
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

if [[ -z "${workspace_version}" ]]; then
  echo "Could not read workspace.package.version from Cargo.toml" >&2
  exit 1
fi

expected_tag="v${workspace_version}"
if [[ "${release_tag}" != "${expected_tag}" ]]; then
  echo "Release tag ${release_tag} does not match workspace version ${workspace_version}; expected ${expected_tag}" >&2
  exit 1
fi

extension_version="$(node -p 'require(process.argv[1]).version' \
  "${project_dir}/editors/vscode/package.json")"
extension_lock_version="$(node -p 'require(process.argv[1]).version' \
  "${project_dir}/editors/vscode/package-lock.json")"
if [[ "${extension_version}" != "${workspace_version}" ]]; then
  echo "VS Code extension version ${extension_version} does not match workspace version ${workspace_version}" >&2
  exit 1
fi
if [[ "${extension_lock_version}" != "${workspace_version}" ]]; then
  echo "VS Code lockfile version ${extension_lock_version} does not match workspace version ${workspace_version}" >&2
  exit 1
fi

if ! cargo_metadata="$(cargo metadata \
  --manifest-path "${project_dir}/Cargo.toml" \
  --format-version 1 \
  --no-deps \
  --locked)"; then
  echo "Cargo.lock is not synchronized with Cargo.toml" >&2
  exit 1
fi

mapfile -t workspace_packages < <(
  printf '%s' "${cargo_metadata}" |
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
node - "${project_dir}/Cargo.lock" "${workspace_version}" "${workspace_packages[@]}" <<'NODE'
const fs = require("fs");
const [lockPath, expectedVersion, ...packageNames] = process.argv.slice(2);
const expected = new Set(packageNames);
const found = new Set();
for (const section of fs.readFileSync(lockPath, "utf8").split(/(?=^\[\[package\]\]\n)/m)) {
  const name = section.match(/^name = "([^"]+)"$/m)?.[1];
  if (!name || !expected.has(name) || /^source = /m.test(section)) continue;
  const version = section.match(/^version = "([^"]+)"$/m)?.[1];
  if (version !== expectedVersion) {
    throw new Error(`Cargo.lock has ${name} at ${version ?? "no version"}, expected ${expectedVersion}`);
  }
  found.add(name);
}
const missing = [...expected].filter(name => !found.has(name));
if (missing.length > 0) {
  throw new Error(`Cargo.lock is missing workspace packages: ${missing.join(", ")}`);
}
NODE

printf 'Release version verified: %s\n' "${release_tag}"
