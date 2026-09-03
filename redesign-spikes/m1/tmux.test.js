#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

function tmux(socket, ...args) {
  return spawnSync("tmux", ["-S", socket, ...args], { encoding: "utf8" });
}

function writeStickyPanic(file) {
  const temporary = `${file}.new`;
  fs.writeFileSync(temporary, `${JSON.stringify({ schemaVersion: 1, panicked: true })}\n`, {
    mode: 0o600,
    flag: "wx",
  });
  fs.renameSync(temporary, file);
}

function validateAuthorityLock(lock, expected) {
  if (lock.schemaVersion !== 1) return { ok: false, reason: "unsupported-version" };
  if (lock.uid !== expected.uid) return { ok: false, reason: "foreign-uid" };
  if (lock.socket !== expected.socket) return { ok: false, reason: "foreign-socket" };
  if (lock.session !== expected.session) return { ok: false, reason: "foreign-session" };
  if (!Number.isSafeInteger(lock.pid) || lock.pid <= 0) return { ok: false, reason: "invalid-pid" };
  return { ok: true };
}

test("dedicated socket has one inert authority and survives client exit", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-m1-tmux-"));
  const socket = path.join(root, "control.sock");
  const session = "zodex-control-m1";
  t.after(() => {
    tmux(socket, "kill-server");
    fs.rmSync(root, { recursive: true, force: true });
  });

  const started = tmux(socket, "new-session", "-d", "-s", session, "sleep", "30");
  assert.equal(started.status, 0, started.stderr);
  assert.ok(fs.statSync(socket).isSocket());
  assert.equal(fs.statSync(socket).mode & 0o077, 0, "socket must be owner-only");

  const duplicate = tmux(socket, "new-session", "-d", "-s", session, "sleep", "30");
  assert.notEqual(duplicate.status, 0, "duplicate authority must be refused");

  const observed = tmux(socket, "display-message", "-p", "-t", session, "#{session_name}:#{pane_current_command}");
  assert.equal(observed.status, 0, observed.stderr);
  assert.equal(observed.stdout.trim(), `${session}:sleep`);
});

test("foreign/stale authority records fail closed and panic persists", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-m1-authority-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const expected = {
    uid: typeof process.getuid === "function" ? process.getuid() : 0,
    socket: path.join(root, "control.sock"),
    session: "zodex-control-m1",
  };
  const valid = { schemaVersion: 1, ...expected, pid: process.pid };
  assert.deepEqual(validateAuthorityLock(valid, expected), { ok: true });
  assert.equal(validateAuthorityLock({ ...valid, pid: 0 }, expected).reason, "invalid-pid");
  assert.equal(validateAuthorityLock({ ...valid, uid: expected.uid + 1 }, expected).reason, "foreign-uid");
  assert.equal(validateAuthorityLock({ ...valid, socket: "/tmp/foreign.sock" }, expected).reason, "foreign-socket");
  assert.equal(validateAuthorityLock({ ...valid, session: "foreign" }, expected).reason, "foreign-session");

  const panicFile = path.join(root, "panic.json");
  writeStickyPanic(panicFile);
  const afterRestart = JSON.parse(fs.readFileSync(panicFile, "utf8"));
  assert.equal(afterRestart.panicked, true);
  assert.equal(fs.statSync(panicFile).mode & 0o077, 0);
  writeStickyPanic(panicFile);
  assert.equal(JSON.parse(fs.readFileSync(panicFile, "utf8")).panicked, true);
});
