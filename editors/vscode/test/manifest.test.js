"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const extensionRoot = path.resolve(__dirname, "..");
const manifest = require(path.join(extensionRoot, "package.json"));

assert.equal(manifest.publisher, "zonnedev");
assert.equal(manifest.name, "jman-java");
assert.match(manifest.version, /^\d+\.\d+\.\d+$/);
assert.equal(manifest.preview, true);
assert.equal(manifest.pricing, "Free");
assert.deepEqual(manifest.extensionKind, ["workspace"]);
assert.equal(manifest.capabilities.untrustedWorkspaces.supported, false);
assert.equal(manifest.capabilities.virtualWorkspaces.supported, false);
assert.ok(manifest.description.length >= 40 && manifest.description.length <= 200);

for (const category of ["Programming Languages", "Testing"]) {
  assert.ok(manifest.categories.includes(category), `missing category: ${category}`);
}
for (const keyword of ["java", "jman", "language server", "maven", "gradle"]) {
  assert.ok(manifest.keywords.includes(keyword), `missing keyword: ${keyword}`);
}

const contributedCommands = new Set(
  manifest.contributes.commands.map((entry) => entry.command),
);
for (const activation of manifest.activationEvents) {
  if (activation.startsWith("onCommand:")) {
    const command = activation.slice("onCommand:".length);
    assert.ok(contributedCommands.has(command), `activation without command: ${command}`);
  }
}

for (const [name, property] of Object.entries(manifest.contributes.configuration.properties)) {
  assert.ok(name.startsWith("jman.java."), `unexpected setting namespace: ${name}`);
  assert.ok(property.description, `setting has no description: ${name}`);
}
assert.equal(
  manifest.contributes.configuration.properties["jman.java.buildJavaHome"].default,
  "",
);

assert.equal(
  manifest.scripts.package,
  "vsce package --target linux-x64 --pre-release --no-dependencies",
);
assert.ok(!Object.values(manifest.scripts).some((script) => script.includes("vsce publish")));

for (const file of [
  "README.md",
  "CHANGELOG.md",
  "LICENSE",
  "SUPPORT.md",
  "THIRD_PARTY_NOTICES.md",
]) {
  const filePath = path.join(extensionRoot, file);
  assert.ok(fs.statSync(filePath).size > 0, `missing or empty release document: ${file}`);
}

const changelog = fs.readFileSync(path.join(extensionRoot, "CHANGELOG.md"), "utf8");
assert.ok(changelog.includes(`## ${manifest.version} - `));

const notices = fs.readFileSync(
  path.join(extensionRoot, "THIRD_PARTY_NOTICES.md"),
  "utf8",
);
const lockfile = require(path.join(extensionRoot, "package-lock.json"));
for (const [packagePath, metadata] of Object.entries(lockfile.packages)) {
  if (!packagePath || metadata.dev === true) continue;
  const packageName = packagePath.slice("node_modules/".length);
  assert.ok(
    notices.includes(`\`${packageName}\` | ${metadata.version} |`),
    `missing runtime dependency notice: ${packageName}@${metadata.version}`,
  );
}

const icon = fs.readFileSync(path.join(extensionRoot, manifest.icon));
assert.deepEqual([...icon.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
assert.ok(icon.readUInt32BE(16) >= 256, "Marketplace icon must be at least 256px wide");
assert.ok(icon.readUInt32BE(20) >= 256, "Marketplace icon must be at least 256px high");
