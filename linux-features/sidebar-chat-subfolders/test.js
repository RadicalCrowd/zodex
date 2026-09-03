#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  createFolder,
  deleteFolder,
  listFolders,
  moveThread,
  readStore,
  renameFolder,
  storePath,
  toggleFolderCollapse,
  writeStore,
} = require("./folder-store.js");

const {
  MAIN_BUNDLE_PATTERN,
  SIDEBAR_ASSET_PATTERN,
  SUBFOLDERS_IPC_MARKER,
  SUBFOLDERS_RUNTIME_MARKER,
  applyMainBundlePatch,
  applyWebviewAssetPatch,
  descriptors,
} = require("./patch.js");

test("folder store manages project folders with 0600 file permissions", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-folders-test-"));
  const tempFile = path.join(tempDir, "chat-folders.json");
  const env = { ZODEX_CHAT_FOLDERS_FILE: tempFile };

  try {
    assert.equal(storePath(env), tempFile);
    assert.deepEqual(readStore(env), { version: 1, projects: {} });

    // Create folder
    const folder1 = createFolder("proj_1", "Sprint 1", env);
    assert.ok(folder1.id.startsWith("fld_"));
    assert.equal(folder1.name, "Sprint 1");
    assert.equal(folder1.collapsed, false);
    assert.deepEqual(folder1.threadIds, []);

    // File permissions
    const stat = fs.statSync(tempFile);
    assert.equal(stat.mode & 0o777, 0o600);

    // List folders
    const folders = listFolders("proj_1", env);
    assert.equal(folders.length, 1);
    assert.equal(folders[0].name, "Sprint 1");

    // Move thread
    const moved = moveThread("proj_1", "t_100", folder1.id, env);
    assert.equal(moved, true);
    assert.deepEqual(listFolders("proj_1", env)[0].threadIds, ["t_100"]);

    // Toggle collapse
    const collapsed = toggleFolderCollapse("proj_1", folder1.id, env);
    assert.equal(collapsed, true);

    // Rename folder
    const renamed = renameFolder("proj_1", folder1.id, "Sprint 1 - Done", env);
    assert.equal(renamed, true);
    assert.equal(listFolders("proj_1", env)[0].name, "Sprint 1 - Done");

    // Delete folder
    const deleted = deleteFolder("proj_1", folder1.id, env);
    assert.equal(deleted, true);
    assert.equal(listFolders("proj_1", env).length, 0);
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
});

test("patch descriptors target main process and webview sidebar assets", () => {
  assert.equal(descriptors.length, 2);
  assert.match("main-Cwjv9Ibf.js", MAIN_BUNDLE_PATTERN);
  assert.match("app-initial-BTphDPeq.js", SIDEBAR_ASSET_PATTERN);
});

test("applyMainBundlePatch injects IPC runtime once", () => {
  const source = '"use strict";const electron=require("electron");';
  const patched = applyMainBundlePatch(source);
  assert.match(patched, new RegExp(SUBFOLDERS_IPC_MARKER));
  assert.match(patched, /zodex-chat-folders/);

  // Idempotent
  const doublePatched = applyMainBundlePatch(patched);
  assert.equal(doublePatched, patched);
});

test("applyWebviewAssetPatch injects webview runtime once", () => {
  const source = '"use strict";function sidebar(){return null;}';
  const patched = applyWebviewAssetPatch(source);
  assert.match(patched, new RegExp(SUBFOLDERS_RUNTIME_MARKER));
  assert.match(patched, /__ZODEX_CHAT_SUBFOLDERS__/);

  // Idempotent
  const doublePatched = applyWebviewAssetPatch(patched);
  assert.equal(doublePatched, patched);
});
