"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { ApprovalFixture, REQUEST_TTL_MS, normalizeRequest } = require("./approval-fixture.js");
const {
  AsrLiveCompatibilityHarness,
  assertNonLiveGateProtection,
  assertZeroNativeAuthAccess,
  isLiveGateRequested,
  runDryRunSuite,
  sanitizeTranscript,
} = require("./asr-live-harness.js");

const command = (overrides = {}) => ({
  method: "item/commandExecution/requestApproval",
  requestId: "req-1",
  params: { threadId: "thread-1", turnId: "turn-1", itemId: "item-1", argv: ["sudo", "-n", "true"], cwd: "/tmp", ...overrides },
});

const input = (overrides = {}) => ({
  method: "item/tool/requestUserInput",
  requestId: "req-input",
  params: {
    threadId: "thread-1",
    turnId: "turn-1",
    itemId: "item-input",
    questions: [{ id: "confirm", title: "Continue?", options: [{ id: "yes", label: "Yes" }, { id: "no", label: "No" }] }],
    ...overrides,
  },
});

test("normalizes each supported request shape", () => {
  for (const raw of [
    command(),
    { ...command(), method: "item/fileChange/requestApproval", params: { threadId: "t", turnId: "u", itemId: "i", paths: ["a.txt"] } },
    { ...command(), method: "item/permissions/requestApproval", params: { threadId: "t", turnId: "u", itemId: "i", permissions: ["network"] } },
    input(),
  ]) {
    assert.equal(normalizeRequest(raw, 100).ok, true);
  }
});

test("rejects unknown, malformed, free-form, secret, and stale requests", () => {
  assert.equal(normalizeRequest({ method: "item/nope", params: {} }).ok, false);
  assert.equal(normalizeRequest(command({ argv: "sudo true" }), 100).ok, false);
  assert.equal(normalizeRequest(input({ questions: [{ id: "password", title: "Password", type: "text" }] }), 100).ok, false);
  assert.equal(normalizeRequest(command({ expiresAt: 100 + REQUEST_TTL_MS + 1 }), 100).ok, false);
});

test("first valid response wins and duplicate responses fail", () => {
  const f = new ApprovalFixture({ clock: () => 100 });
  f.connect("desktop");
  f.connect("phone");
  assert.equal(f.receive(command()).ok, true);
  f.deliver("req-1", "desktop");
  f.deliver("req-1", "phone");
  assert.deepEqual(f.respond("req-1", "phone", { decision: "approve" }, "thread-1"), { ok: true, response: { decision: "approve" } });
  assert.equal(f.respond("req-1", "desktop", { decision: "reject" }, "thread-1").reason, "already-resolved");
});

test("rejects cross-thread and invalid client responses", () => {
  const f = new ApprovalFixture({ clock: () => 100 });
  f.connect("desktop");
  f.receive(command());
  f.deliver("req-1", "desktop");
  assert.equal(f.respond("req-1", "desktop", { decision: "approve" }, "other-thread").reason, "cross-thread-response");
  assert.equal(f.respond("req-1", "desktop", { decision: "maybe" }, "thread-1").reason, "invalid-response");
});

test("expires exactly at the five-minute boundary", () => {
  let now = 100;
  const f = new ApprovalFixture({ clock: () => now });
  f.connect("desktop");
  f.receive(command());
  f.deliver("req-1", "desktop");
  now += REQUEST_TTL_MS;
  assert.equal(f.respond("req-1", "desktop", { decision: "approve" }, "thread-1").reason, "expired");
  assert.equal(f.get("req-1").status, "expired");
});

test("models server resolution, cancellation, and disconnect", () => {
  const f = new ApprovalFixture({ clock: () => 100 });
  f.connect("desktop");
  f.receive(command());
  f.deliver("req-1", "desktop");
  assert.equal(f.resolve("req-1").status, "resolved");
  f.receive({ ...command(), requestId: "req-2" });
  f.cancel("req-2", "turn-cancelled");
  assert.equal(f.get("req-2").status, "cancelled");
  f.receive({ ...command(), requestId: "req-3" });
  f.deliver("req-3", "desktop");
  f.disconnect("desktop");
  assert.equal(f.get("req-3").reason, "disconnect");
});

test("accepts only enumerated tool answers", () => {
  const f = new ApprovalFixture({ clock: () => 100 });
  f.connect("desktop");
  f.receive(input());
  f.deliver("req-input", "desktop");
  assert.equal(f.respond("req-input", "desktop", { answers: { confirm: "maybe" } }, "thread-1").reason, "invalid-response");
  assert.equal(f.respond("req-input", "desktop", { answers: { confirm: "yes" } }, "thread-1").ok, true);
});

test("ASR-LIVE harness runs dry-run suite covering all 7 compatibility scenarios", () => {
  const { report, sanitizedOutput } = runDryRunSuite();
  assert.equal(report.ok, true);
  assert.equal(report.mode, "dry-run");
  assert.equal(report.task, "ASR-LIVE");
  assert.equal(report.scenarios.delivery.ok, true);
  assert.equal(report.scenarios.responseOwnership.ok, true);
  assert.equal(report.scenarios.duplicateResponse.ok, true);
  assert.equal(report.scenarios.serverRequestResolved.ok, true);
  assert.equal(report.scenarios.disconnect.ok, true);
  assert.equal(report.scenarios.expiry.ok, true);
  assert.equal(report.scenarios.remoteContinuity.ok, true);
  assert.doesNotMatch(sanitizedOutput, /Bearer\s+eyJ/);
});

test("ASR-LIVE harness sanitizes sensitive tokens and paths", () => {
  const raw = "Authorization: Bearer secret_token_123456789";
  const sanitized = sanitizeTranscript(raw);
  assert.equal(sanitized.includes("secret_token_123456789"), false);
  assert.equal(sanitized.includes("[REDACTED_TOKEN]"), true);
});

test("ASR-LIVE harness refuses live gate execution during preparation phase", () => {
  process.env.ALLOW_LIVE_ASR_EXECUTION = "1";
  try {
    assert.throws(() => assertNonLiveGateProtection(), (err) => err.code === "LIVE_EXECUTION_REFUSED");
  } finally {
    delete process.env.ALLOW_LIVE_ASR_EXECUTION;
  }
});
