#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");

const {
  CONFIRMATION_PATTERN,
  PtyManager,
  PtySession,
  SUDO_PROMPT_PATTERN,
} = require("./pty-manager.js");

const {
  CONSOLE_IPC_MARKER,
  CONSOLE_RUNTIME_MARKER,
  MAIN_BUNDLE_PATTERN,
  WEBVIEW_ASSET_PATTERN,
  applyMainBundlePatch,
  applyWebviewAssetPatch,
  descriptors,
} = require("./patch.js");

test("prompt patterns detect interactive prompts correctly", () => {
  assert.ok(SUDO_PROMPT_PATTERN.test("[sudo] password for user:"));
  assert.ok(SUDO_PROMPT_PATTERN.test("[sudo] password for akif: "));
  assert.ok(SUDO_PROMPT_PATTERN.test("Password:"));
  assert.ok(!SUDO_PROMPT_PATTERN.test("Password reset link sent."));

  assert.ok(CONFIRMATION_PATTERN.test("Do you want to continue? (y/n)"));
  assert.ok(CONFIRMATION_PATTERN.test("Proceed with upgrade? [y/N] "));
  assert.ok(CONFIRMATION_PATTERN.test("Delete file? [Y/n]"));
  assert.ok(!CONFIRMATION_PATTERN.test("Option y/n selected"));
});

test("PtyManager spawns, streams output, and tracks session status", async () => {
  const manager = new PtyManager();
  const session = manager.createSession({
    command: process.execPath,
    args: ["-e", "console.log('Hello Zodex Console');"],
  });

  assert.ok(session.id.startsWith("term_"));
  assert.equal(session.command, process.execPath);

  await new Promise((resolve) => {
    session.on("close", resolve);
  });

  assert.equal(session.status, "completed");
  assert.equal(session.exitCode, 0);
  assert.match(session.outputBuffer, /Hello Zodex Console/);

  const list = manager.listSessions();
  assert.equal(list.length, 1);
  assert.equal(list[0].id, session.id);
  assert.equal(list[0].status, "completed");
});

test("PtySession writeInput handles regular input and password zeroing", async () => {
  const session = new PtySession({
    command: process.execPath,
    args: [
      "-e",
      "process.stdin.on('data', (d) => { console.log('Answer received:', d.toString().trim()); process.exit(0); });",
    ],
  });

  session.start();

  // Wait for spawn
  await new Promise((r) => setTimeout(r, 100));

  // Write input with password mode
  const wrote = session.writeInput("SecretPassword123", { isPassword: true });
  assert.equal(wrote, true);

  await new Promise((resolve) => {
    session.on("close", resolve);
  });

  assert.equal(session.status, "completed");
  assert.match(session.outputBuffer, /Answer received: SecretPassword123/);
});

test("patch descriptors target main and webview bundles", () => {
  assert.equal(descriptors.length, 2);
  assert.match("main-Cwjv9Ibf.js", MAIN_BUNDLE_PATTERN);
  assert.match("app-initial-BTphDPeq.js", WEBVIEW_ASSET_PATTERN);
});

test("applyMainBundlePatch injects IPC handler and is idempotent", () => {
  const source = '"use strict";const electron=require("electron");';
  const patched = applyMainBundlePatch(source);
  assert.match(patched, new RegExp(CONSOLE_IPC_MARKER));
  assert.match(patched, /zodex-console-sessions/);
  assert.match(patched, /zodex-console-spawn/);

  const doublePatched = applyMainBundlePatch(patched);
  assert.equal(doublePatched, patched);
});

test("applyWebviewAssetPatch injects webview runtime and is idempotent", () => {
  const source = '"use strict";function app(){return null;}';
  const patched = applyWebviewAssetPatch(source);
  assert.match(patched, new RegExp(CONSOLE_RUNTIME_MARKER));
  assert.match(patched, /__ZODEX_CONSOLE__/);

  const doublePatched = applyWebviewAssetPatch(patched);
  assert.equal(doublePatched, patched);
});
