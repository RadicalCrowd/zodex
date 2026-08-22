"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { ApprovalFixture, REQUEST_TTL_MS, normalizeRequest } = require("./approval-fixture.js");

const command = (overrides = {}) => ({ method: "item/commandExecution/requestApproval", requestId: "req-1", params: { threadId: "thread-1", turnId: "turn-1", itemId: "item-1", argv: ["sudo", "-n", "true"], cwd: "/tmp", ...overrides } });
const input = (overrides = {}) => ({ method: "item/tool/requestUserInput", requestId: "req-input", params: { threadId: "thread-1", turnId: "turn-1", itemId: "item-input", questions: [{ id: "confirm", title: "Continue?", options: [{ id: "yes", label: "Yes" }, { id: "no", label: "No" }] }], ...overrides } });

test("normalizes each supported request shape", () => {
  for (const raw of [command(), { ...command(), method: "item/fileChange/requestApproval", params: { threadId: "t", turnId: "u", itemId: "i", paths: ["a.txt"] } }, { ...command(), method: "item/permissions/requestApproval", params: { threadId: "t", turnId: "u", itemId: "i", permissions: ["network"] } }, input()]) assert.equal(normalizeRequest(raw, 100).ok, true);
});

test("rejects unknown, malformed, free-form, secret, and stale requests", () => {
  assert.equal(normalizeRequest({ method: "item/nope", params: {} }).ok, false);
  assert.equal(normalizeRequest(command({ argv: "sudo true" }), 100).ok, false);
  assert.equal(normalizeRequest(input({ questions: [{ id: "password", title: "Password", type: "text" }] }), 100).ok, false);
  assert.equal(normalizeRequest(command({ expiresAt: 100 + REQUEST_TTL_MS + 1 }), 100).ok, false);
});

test("first valid response wins and duplicate responses fail", () => {
  const f = new ApprovalFixture({ clock: () => 100 }); f.connect("desktop"); f.connect("phone");
  assert.equal(f.receive(command()).ok, true); f.deliver("req-1", "desktop"); f.deliver("req-1", "phone");
  assert.deepEqual(f.respond("req-1", "phone", { decision: "approve" }, "thread-1"), { ok: true, response: { decision: "approve" } });
  assert.equal(f.respond("req-1", "desktop", { decision: "reject" }, "thread-1").reason, "already-resolved");
});

test("rejects cross-thread and invalid client responses", () => {
  const f = new ApprovalFixture({ clock: () => 100 }); f.connect("desktop"); f.receive(command()); f.deliver("req-1", "desktop");
  assert.equal(f.respond("req-1", "desktop", { decision: "approve" }, "other-thread").reason, "cross-thread-response");
  assert.equal(f.respond("req-1", "desktop", { decision: "maybe" }, "thread-1").reason, "invalid-response");
});

test("expires exactly at the five-minute boundary", () => {
  let now = 100; const f = new ApprovalFixture({ clock: () => now }); f.connect("desktop"); f.receive(command()); f.deliver("req-1", "desktop"); now += REQUEST_TTL_MS;
  assert.equal(f.respond("req-1", "desktop", { decision: "approve" }, "thread-1").reason, "expired"); assert.equal(f.get("req-1").status, "expired");
});

test("models server resolution, cancellation, and disconnect", () => {
  const f = new ApprovalFixture({ clock: () => 100 }); f.connect("desktop"); f.receive(command()); f.deliver("req-1", "desktop"); assert.equal(f.resolve("req-1").status, "resolved");
  f.receive({ ...command(), requestId: "req-2" }); f.cancel("req-2", "turn-cancelled"); assert.equal(f.get("req-2").status, "cancelled");
  f.receive({ ...command(), requestId: "req-3" }); f.deliver("req-3", "desktop"); f.disconnect("desktop"); assert.equal(f.get("req-3").reason, "disconnect");
});

test("accepts only enumerated tool answers", () => {
  const f = new ApprovalFixture({ clock: () => 100 }); f.connect("desktop"); f.receive(input()); f.deliver("req-input", "desktop");
  assert.equal(f.respond("req-input", "desktop", { answers: { confirm: "maybe" } }, "thread-1").reason, "invalid-response");
  assert.equal(f.respond("req-input", "desktop", { answers: { confirm: "yes" } }, "thread-1").ok, true);
});
