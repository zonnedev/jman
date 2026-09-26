"use strict";

const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const extensionRoot = path.resolve(__dirname, "..");
const projectRoot = path.resolve(extensionRoot, "..", "..");

function releasePath(relative) {
  const resolved = path.join(extensionRoot, relative);
  assert.ok(fs.statSync(resolved).size > 0, `missing or empty release file: ${relative}`);
  return resolved;
}

function assertElfX64(relative, executable) {
  const resolved = releasePath(relative);
  const header = Buffer.alloc(20);
  const descriptor = fs.openSync(resolved, "r");
  try {
    assert.equal(fs.readSync(descriptor, header, 0, header.length, 0), header.length);
  } finally {
    fs.closeSync(descriptor);
  }
  assert.deepEqual([...header.subarray(0, 4)], [0x7f, 0x45, 0x4c, 0x46]);
  assert.equal(header[4], 2, `${relative} must be a 64-bit ELF file`);
  assert.equal(header[5], 1, `${relative} must use little-endian ELF encoding`);
  assert.equal(header.readUInt16LE(18), 0x3e, `${relative} must target x86-64`);
  if (executable) {
    assert.notEqual(fs.statSync(resolved).mode & 0o111, 0, `${relative} must be executable`);
  }
}

function assertMachOArm64(relative, executable) {
  const resolved = releasePath(relative);
  const header = Buffer.alloc(12);
  const descriptor = fs.openSync(resolved, "r");
  try {
    assert.equal(fs.readSync(descriptor, header, 0, header.length, 0), header.length);
  } finally {
    fs.closeSync(descriptor);
  }
  assert.equal(header.readUInt32LE(0), 0xfeedfacf, `${relative} must be a 64-bit Mach-O file`);
  assert.equal(header.readUInt32LE(4), 0x0100000c, `${relative} must target ARM64`);
  if (executable) {
    assert.notEqual(fs.statSync(resolved).mode & 0o111, 0, `${relative} must be executable`);
  }
}

if (process.platform === "linux" && process.arch === "x64") {
  assertElfX64("server/jman", true);
  assertElfX64("server/libjman_javac_frontend.so", false);
} else if (process.platform === "darwin" && process.arch === "arm64") {
  assertMachOArm64("server/jman", true);
  assertMachOArm64("server/libjman_javac_frontend.dylib", false);
} else {
  assert.fail(`unsupported release-test host: ${process.platform}-${process.arch}`);
}

const nativeBuild = fs.readFileSync(path.join(projectRoot, "scripts/build-native.sh"), "utf8");
assert.ok(nativeBuild.includes("-march=compatibility"));

for (const relative of [
  "dist/extension.js",
  "server/maven-importer.jar",
  "server/processor-worker.jar",
  "server/vineflower.jar",
  "server/jacocoagent.jar",
  "server/jacococli.jar",
  "server/platform/lib/ct.sym",
  "server/tools/gradle-importer/javac-frontend-model.init.gradle",
]) {
  releasePath(relative);
}

const platformSignature = fs
  .readFileSync(path.join(extensionRoot, "server/platform/lib/ct.sym"))
  .subarray(0, 2);
assert.deepEqual([...platformSignature], [0x50, 0x4b], "ct.sym is not a ZIP archive");

for (const relative of [
  "server/maven-importer.jar",
  "server/processor-worker.jar",
  "server/vineflower.jar",
  "server/jacocoagent.jar",
  "server/jacococli.jar",
]) {
  const signature = fs.readFileSync(path.join(extensionRoot, relative)).subarray(0, 4);
  assert.equal(signature[0], 0x50, `${relative} is not a ZIP/JAR archive`);
  assert.equal(signature[1], 0x4b, `${relative} is not a ZIP/JAR archive`);
}

const version = childProcess.spawnSync(path.join(extensionRoot, "server/jman"), ["--version"], {
  encoding: "utf8",
});
assert.equal(version.status, 0, version.stderr);
assert.match(version.stdout, /^jman \d+\.\d+\.\d+/);
