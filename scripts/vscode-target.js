"use strict";

const targets = {
  "darwin-arm64": "darwin-arm64",
  "linux-x64": "linux-x64",
};
const host = `${process.platform}-${process.arch}`;
const target = targets[host];
if (!target) {
  console.error(`unsupported JMAN VS Code host: ${host}`);
  process.exit(1);
}
process.stdout.write(target);
