#!/usr/bin/env node
"use strict";

const childProcess = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const output = path.resolve(process.argv[2] || path.join(root, "dist-next", "zodex-aur-rc"));
fs.mkdirSync(output, { recursive: true, mode: 0o700 });
const listed = childProcess.execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard", "-z"], { cwd: root });
const files = listed.toString("utf8").split("\0").filter(Boolean).sort();
const forbidden = files.filter((file) => /(?:^|\/)(?:app\.asar|[^/]+\.(?:deb|rpm|AppImage|pkg\.tar\.(?:zst|xz)))$/i.test(file));
if (forbidden.length) throw new Error(`proprietary/package payload refused: ${forbidden.join(", ")}`);
const archive = path.join(output, "zodex-source.tar.gz");
const tar = childProcess.spawnSync("tar", [
  "--create", "--gzip", "--file", archive,
  "--sort=name", "--mtime=1970-01-01", "--owner=0", "--group=0", "--numeric-owner",
  "--transform", "s,^,zodex-source/ ,".replace("/ ", "/"),
  "--null", "--files-from", "-",
], {
  cwd: root,
  input: Buffer.from(`${files.join("\0")}\0`),
  env: { ...process.env, GZIP: "-n" },
});
if (tar.status !== 0) throw new Error(`source archive failed: ${tar.stderr}`);
const sha256 = crypto.createHash("sha256").update(fs.readFileSync(archive)).digest("hex");
const version = `0.1.0.rc.${new Date().toISOString().slice(0, 10).replaceAll("-", "")}`;
const template = fs.readFileSync(path.join(root, "packaging/aur/PKGBUILD.template"), "utf8");
fs.writeFileSync(path.join(output, "PKGBUILD"), template.replace("__PKGVER__", version).replace("__SOURCE_SHA256__", sha256));
for (const file of ["zodex.desktop", "zodex-desktop"]) fs.copyFileSync(path.join(root, "packaging/aur", file), path.join(output, file));
fs.writeFileSync(path.join(output, "manifest.json"), `${JSON.stringify({ schemaVersion: 1, version, sourceSha256: sha256, sourceFiles: files.length, proprietaryPayloads: 0 }, null, 2)}\n`);
process.stdout.write(`${output}\n`);
