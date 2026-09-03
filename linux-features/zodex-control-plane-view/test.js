#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { applyMainBundlePatch, applyWebviewPatch, descriptors } = require("./patch.js");

test("feature is optional and idempotently adds read-only handler", () => {
  const source = '"use strict";const handlers={"native-desktop-apps":async()=>{}};';
  const patched = applyMainBundlePatch(source);
  assert.match(patched, /zodex-control-plane-status/);
  assert.match(patched, /method:\`inventory\`/);
  assert.match(patched, /client_id:\`zodex-desktop\`/);
  assert.equal(applyMainBundlePatch(patched), patched);
  assert.ok(descriptors.every((descriptor) => descriptor.ciPolicy === "optional"));
});

test("handler fails closed on absent, foreign, open, oversized, and invalid sockets", () => {
  const patched = applyMainBundlePatch('"use strict";const handlers={"native-desktop-apps":async()=>{}};');
  assert.match(patched, /st\.uid!==process\.getuid\(\)/);
  assert.match(patched, /\(st\.mode&63\)!==0/);
  assert.match(patched, /size>65536/);
  assert.match(patched, /state:\`refused\`/);
});

test("webview renders only sanitized status and agent count", () => {
  const patched = applyWebviewPatch("const app=true;");
  assert.match(patched, /zodexControlPlaneViewV1/);
  assert.match(patched, /managed agent/);
  assert.doesNotMatch(patched, /\bprompt\b|tool\.output|tmux_session/);
  assert.equal(applyWebviewPatch(patched), patched);
});
