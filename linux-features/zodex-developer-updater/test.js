"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const { assertCheckout, digestSnapshot } = require("./zodex-update-manager.cjs");

test("developer snapshot includes dirty source files but excludes generated output", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-update-manager-"));
  try {
    fs.mkdirSync(path.join(root, "scripts"), { recursive: true });
    fs.writeFileSync(path.join(root, "scripts", "build-zodex-arch-candidate.sh"), "#!/bin/sh\n");
    fs.writeFileSync(path.join(root, "dirty.js"), "one\n");
    fs.mkdirSync(path.join(root, "dist-next"));
    fs.writeFileSync(path.join(root, "dist-next", "ignored"), "one\n");
    const first = digestSnapshot(root);
    fs.writeFileSync(path.join(root, "dist-next", "ignored"), "two\n");
    assert.equal(digestSnapshot(root), first);
    fs.writeFileSync(path.join(root, "dirty.js"), "two\n");
    assert.notEqual(digestSnapshot(root), first);
    assert.equal(assertCheckout(root), fs.realpathSync(root));
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("developer snapshot rejects symlinks rather than following arbitrary source", { skip: process.platform === "win32" }, () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-update-manager-link-"));
  try {
    fs.symlinkSync("/etc/passwd", path.join(root, "unsafe"));
    assert.throws(() => digestSnapshot(root), /symlink/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("developer checkout path itself must not be a symbolic link", { skip: process.platform === "win32" }, () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-update-manager-checkout-"));
  const link = `${root}-link`;
  try {
    fs.mkdirSync(path.join(root, "scripts"), { recursive: true });
    fs.writeFileSync(path.join(root, "scripts", "build-zodex-arch-candidate.sh"), "#!/bin/sh\n");
    fs.symlinkSync(root, link);
    assert.throws(() => assertCheckout(link), /symbolic link/);
  } finally {
    fs.rmSync(link, { force: true });
    fs.rmSync(root, { recursive: true, force: true });
  }
});
