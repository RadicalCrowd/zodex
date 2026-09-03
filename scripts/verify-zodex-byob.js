#!/usr/bin/env node
"use strict";

const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const {
  mapMachineArch,
  parseReleaseSha256,
  selectChatgptPackage,
  verifyIndexedFile,
  verifyInRelease,
} = require("./lib/upstream-linux-package.js");

function verifyByobPackage({ debPath, evidenceDir, architecture = os.arch() }) {
  const arch = mapMachineArch(architecture);
  const key = path.join(evidenceDir, "codex-linux-repository-key.gpg");
  const inRelease = path.join(evidenceDir, "InRelease");
  const packages = path.join(evidenceDir, `Packages.${arch}`);
  for (const file of [debPath, key, inRelease, packages]) {
    const metadata = fs.lstatSync(file);
    if (!metadata.isFile() || metadata.isSymbolicLink()) throw new Error(`BYOB evidence must be a regular non-symlink file: ${file}`);
  }
  const payload = verifyInRelease(inRelease, key);
  const indexed = parseReleaseSha256(payload).get(`main/binary-${arch}/Packages`);
  if (!indexed) throw new Error(`signed InRelease does not index Packages.${arch}`);
  verifyIndexedFile(packages, indexed, `Packages.${arch}`);
  const selected = selectChatgptPackage(fs.readFileSync(packages, "utf8"), arch);
  verifyIndexedFile(debPath, selected, path.basename(debPath));
  const field = (name) => {
    const result = childProcess.spawnSync("dpkg-deb", ["-f", debPath, name], { encoding: "utf8" });
    if (result.status !== 0) throw new Error(`dpkg-deb rejected BYOB package: ${result.stderr.trim()}`);
    return result.stdout.trim();
  };
  const packageName = field("Package");
  const packageArch = field("Architecture");
  const version = field("Version");
  if (packageName !== "chatgpt" || packageArch !== arch || version !== selected.version) {
    throw new Error("BYOB package identity does not match signed metadata");
  }
  return { package: packageName, architecture: packageArch, version, sha256: selected.sha256, size: selected.size };
}

function main() {
  const args = process.argv.slice(2);
  const value = (name) => {
    const index = args.indexOf(name);
    if (index < 0 || index + 1 >= args.length) throw new Error(`missing ${name}`);
    return args[index + 1];
  };
  const result = verifyByobPackage({ debPath: path.resolve(value("--deb")), evidenceDir: path.resolve(value("--evidence")), architecture: args.includes("--arch") ? value("--arch") : os.arch() });
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (require.main === module) {
  try { main(); } catch (error) { console.error(`ERROR: ${error.message}`); process.exit(1); }
}

module.exports = { verifyByobPackage };
