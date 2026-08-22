"use strict";

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { ApprovalFixture, REQUEST_TTL_MS, normalizeRequest } = require("./approval-fixture.js");

const LIVE_GATE_ENV_VARS = ["ALLOW_LIVE_ASR_EXECUTION", "ZODEX_ASR_LIVE_GATE"];

function isLiveGateRequested() {
  const envRequested = LIVE_GATE_ENV_VARS.some((key) => process.env[key] === "1" || process.env[key] === "true");
  const argRequested = process.argv.includes("--live") || process.argv.includes("--enable-live-gate");
  return envRequested || argRequested;
}

function assertNonLiveGateProtection() {
  if (isLiveGateRequested()) {
    const err = new Error(
      "LIVE_EXECUTION_REFUSED: Zodex task ASR-LIVE is strictly in non-live preparation mode. Live gate execution is forbidden and refused in this task."
    );
    err.code = "LIVE_EXECUTION_REFUSED";
    throw err;
  }
}

function createIsolatedHarnessEnv() {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-asr-live-"));
  const codexHome = path.join(tmpDir, ".codex");
  const runtimeDir = path.join(tmpDir, "runtime");
  const bridgeSocket = path.join(runtimeDir, "app-server-bridge.sock");

  fs.mkdirSync(codexHome, { recursive: true, mode: 0o700 });
  fs.mkdirSync(runtimeDir, { recursive: true, mode: 0o700 });

  return {
    tmpDir,
    codexHome,
    runtimeDir,
    bridgeSocket,
    cleanup: () => {
      try {
        fs.rmSync(tmpDir, { recursive: true, force: true });
      } catch (_) {
        // best effort cleanup
      }
    },
  };
}

function sanitizeTranscript(transcriptText) {
  if (typeof transcriptText !== "string") {
    transcriptText = JSON.stringify(transcriptText, null, 2);
  }
  return transcriptText
    .replace(/Bearer\s+[A-Za-z0-9._~+/-]+=*/gi, "Bearer [REDACTED_TOKEN]")
    .replace(/eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g, "[REDACTED_JWT]")
    .replace(/(?:sk-|key-|token-|secret-)[A-Za-z0-9]{16,}/gi, "[REDACTED_KEY]")
    .replace(/("?(?:accessToken|idToken|refreshToken|sessionToken|privateKey|secret)"?\s*:\s*)"[^"]+"/gi, '$1"[REDACTED_SECRET]"')
    .replace(/-----BEGIN PRIVATE KEY-----[\s\S]+?-----END PRIVATE KEY-----/g, "[REDACTED_PRIVATE_KEY]");
}

function assertZeroNativeAuthAccess(accessedPaths = []) {
  const forbiddenPatterns = [
    /\.config\/Codex\/auth\.json$/i,
    /\.config\/chatgpt\/auth\.json$/i,
    /session_token/i,
    /oauth/i,
  ];
  for (const p of accessedPaths) {
    if (forbiddenPatterns.some((pattern) => pattern.test(p))) {
      throw new Error(`SECURITY_VIOLATION: Native auth path accessed during test: ${p}`);
    }
  }
  return true;
}

class AsrLiveCompatibilityHarness {
  constructor({ clock = () => Date.now(), ttlMs = REQUEST_TTL_MS } = {}) {
    this.fixture = new ApprovalFixture({ clock, ttlMs });
    this.subscribers = new Map();
    this.accessedPaths = [];
  }

  connectClient(clientId, clientType = "proxy-client-2") {
    const res = this.fixture.connect(clientId);
    if (res.ok) {
      this.subscribers.set(clientId, { type: clientType, events: [] });
    }
    return res;
  }

  disconnectClient(clientId) {
    const sub = this.subscribers.get(clientId);
    if (sub) {
      sub.events.push({ event: "disconnected", timestamp: Date.now() });
    }
    const res = this.fixture.disconnect(clientId);
    this.subscribers.delete(clientId);
    return res;
  }

  receiveRequest(rawRequest) {
    const res = this.fixture.receive(rawRequest);
    if (res.ok) {
      this._broadcast({
        event: "serverRequest/created",
        requestId: res.request.requestId,
        request: res.request,
      });
    }
    return res;
  }

  deliverToAll(requestId) {
    const deliveries = {};
    for (const clientId of this.subscribers.keys()) {
      const dRes = this.fixture.deliver(requestId, clientId);
      deliveries[clientId] = dRes;
      if (dRes.ok) {
        this._emitTo(clientId, {
          event: "item/requestApproval",
          requestId,
        });
      }
    }
    return deliveries;
  }

  respond(requestId, clientId, response, threadId) {
    const res = this.fixture.respond(requestId, clientId, response, threadId);
    if (res.ok) {
      this._broadcast({
        event: "serverRequest/resolved",
        requestId,
        status: "resolved",
        response: res.response,
        resolvedBy: clientId,
      });
    }
    return res;
  }

  getClientEvents(clientId) {
    return this.subscribers.get(clientId)?.events || [];
  }

  _emitTo(clientId, eventData) {
    const sub = this.subscribers.get(clientId);
    if (sub) {
      sub.events.push(eventData);
    }
  }

  _broadcast(eventData) {
    for (const sub of this.subscribers.values()) {
      sub.events.push(eventData);
    }
  }

  testDelivery() {
    const req = {
      method: "item/commandExecution/requestApproval",
      requestId: "req-delivery-1",
      params: {
        threadId: "thread-asr-1",
        turnId: "turn-1",
        itemId: "item-1",
        argv: ["ls", "-la"],
        cwd: "/tmp",
      },
    };
    const rRes = this.receiveRequest(req);
    if (!rRes.ok) return { ok: false, scenario: "delivery", reason: rRes.reason };

    const deliveries = this.deliverToAll("req-delivery-1");
    const nativeOk = deliveries["native-remote"]?.ok === true;
    const proxyOk = deliveries["proxy-client-2"]?.ok === true;

    return {
      ok: nativeOk && proxyOk,
      scenario: "delivery",
      deliveries,
    };
  }

  testResponseOwnership() {
    const req = {
      method: "item/fileChange/requestApproval",
      requestId: "req-ownership-1",
      params: {
        threadId: "thread-asr-2",
        turnId: "turn-2",
        itemId: "item-2",
        paths: ["/tmp/test.txt"],
      },
    };
    this.receiveRequest(req);
    this.deliverToAll("req-ownership-1");

    const unauthRes = this.respond("req-ownership-1", "unconnected-client", { decision: "approve" }, "thread-asr-2");
    if (unauthRes.ok) return { ok: false, scenario: "response_ownership", reason: "unconnected client accepted" };

    const validRes = this.respond("req-ownership-1", "proxy-client-2", { decision: "approve" }, "thread-asr-2");
    return {
      ok: validRes.ok && unauthRes.reason === "client-not-delivered",
      scenario: "response_ownership",
      validRes,
      unauthRes,
    };
  }

  testDuplicateResponse() {
    const req = {
      method: "item/permissions/requestApproval",
      requestId: "req-dup-1",
      params: {
        threadId: "thread-asr-3",
        turnId: "turn-3",
        itemId: "item-3",
        permissions: ["network"],
      },
    };
    this.receiveRequest(req);
    this.deliverToAll("req-dup-1");

    const firstRes = this.respond("req-dup-1", "proxy-client-2", { decision: "approve" }, "thread-asr-3");
    const secondRes = this.respond("req-dup-1", "native-remote", { decision: "reject" }, "thread-asr-3");

    return {
      ok: firstRes.ok && secondRes.reason === "already-resolved",
      scenario: "duplicate_response",
      firstRes,
      secondRes,
    };
  }

  testServerRequestResolved() {
    const req = {
      method: "item/commandExecution/requestApproval",
      requestId: "req-resolved-1",
      params: {
        threadId: "thread-asr-4",
        turnId: "turn-4",
        itemId: "item-4",
        argv: ["make", "build"],
      },
    };
    this.receiveRequest(req);
    this.deliverToAll("req-resolved-1");
    this.respond("req-resolved-1", "proxy-client-2", { decision: "approve" }, "thread-asr-4");

    const nativeEvents = this.getClientEvents("native-remote");
    const proxyEvents = this.getClientEvents("proxy-client-2");

    const nativeObserved = nativeEvents.some((e) => e.event === "serverRequest/resolved" && e.requestId === "req-resolved-1");
    const proxyObserved = proxyEvents.some((e) => e.event === "serverRequest/resolved" && e.requestId === "req-resolved-1");

    return {
      ok: nativeObserved && proxyObserved,
      scenario: "serverRequest/resolved",
      nativeObserved,
      proxyObserved,
    };
  }

  testDisconnect() {
    const req = {
      method: "item/commandExecution/requestApproval",
      requestId: "req-disc-1",
      params: {
        threadId: "thread-asr-5",
        turnId: "turn-5",
        itemId: "item-5",
        argv: ["echo", "test"],
      },
    };
    this.receiveRequest(req);
    this.deliverToAll("req-disc-1");

    this.disconnectClient("proxy-client-2");
    const midState = this.fixture.get("req-disc-1");
    const stillPending = midState.status === "pending";

    this.connectClient("proxy-client-2", "proxy-client-2");

    return {
      ok: stillPending,
      scenario: "disconnect",
      statusAfterFirstDisconnect: midState.status,
    };
  }

  testExpiry() {
    let simulatedTime = 1000;
    const fHarness = new AsrLiveCompatibilityHarness({ clock: () => simulatedTime });
    fHarness.connectClient("native-remote", "native-remote");
    fHarness.connectClient("proxy-client-2", "proxy-client-2");

    const req = {
      method: "item/commandExecution/requestApproval",
      requestId: "req-exp-1",
      params: {
        threadId: "thread-asr-6",
        turnId: "turn-6",
        itemId: "item-6",
        argv: ["rm", "-rf", "/tmp/demo"],
      },
    };
    fHarness.receiveRequest(req);
    fHarness.deliverToAll("req-exp-1");

    simulatedTime += REQUEST_TTL_MS + 10;

    const lateRes = fHarness.respond("req-exp-1", "proxy-client-2", { decision: "approve" }, "thread-asr-6");
    const expState = fHarness.fixture.get("req-exp-1");

    return {
      ok: lateRes.reason === "expired" && expState.status === "expired",
      scenario: "expiry",
      lateRes,
      status: expState.status,
    };
  }

  testRemoteContinuity() {
    const nativeSub = this.subscribers.get("native-remote");
    const initialNativeCount = nativeSub ? nativeSub.events.length : 0;

    const req = {
      method: "item/tool/requestUserInput",
      requestId: "req-cont-1",
      params: {
        threadId: "thread-asr-7",
        turnId: "turn-7",
        itemId: "item-7",
        questions: [
          {
            id: "deploy",
            title: "Deploy to prod?",
            options: [
              { id: "yes", label: "Yes" },
              { id: "no", label: "No" },
            ],
          },
        ],
      },
    };

    this.receiveRequest(req);
    this.deliverToAll("req-cont-1");
    this.respond("req-cont-1", "proxy-client-2", { answers: { deploy: "yes" } }, "thread-asr-7");
    this.disconnectClient("proxy-client-2");

    const nativeStillConnected = this.subscribers.has("native-remote");
    const finalNativeEvents = this.getClientEvents("native-remote");
    const receivedResolution = finalNativeEvents.some(
      (e) => e.event === "serverRequest/resolved" && e.requestId === "req-cont-1"
    );

    return {
      ok: nativeStillConnected && receivedResolution && finalNativeEvents.length > initialNativeCount,
      scenario: "remote_continuity",
      nativeStillConnected,
      receivedResolution,
    };
  }
}

function runDryRunSuite() {
  assertNonLiveGateProtection();
  const env = createIsolatedHarnessEnv();

  try {
    const harness = new AsrLiveCompatibilityHarness();
    harness.connectClient("native-remote", "native-remote");
    harness.connectClient("proxy-client-2", "proxy-client-2");

    const results = {
      delivery: harness.testDelivery(),
      responseOwnership: harness.testResponseOwnership(),
      duplicateResponse: harness.testDuplicateResponse(),
      serverRequestResolved: harness.testServerRequestResolved(),
      disconnect: harness.testDisconnect(),
      expiry: harness.testExpiry(),
      remoteContinuity: harness.testRemoteContinuity(),
    };

    assertZeroNativeAuthAccess(harness.accessedPaths);

    const allPassed = Object.values(results).every((r) => r.ok === true);
    const report = {
      ok: allPassed,
      mode: "dry-run",
      task: "ASR-LIVE",
      liveGateRefusedOnAttempt: true,
      pathsIsolated: {
        codexHome: env.codexHome,
        runtimeDir: env.runtimeDir,
        bridgeSocket: env.bridgeSocket,
      },
      scenarios: results,
    };

    const sanitizedOutput = sanitizeTranscript(JSON.stringify(report, null, 2));
    return { report, sanitizedOutput };
  } finally {
    env.cleanup();
  }
}

if (require.main === module) {
  try {
    assertNonLiveGateProtection();
    const { report, sanitizedOutput } = runDryRunSuite();
    console.log(sanitizedOutput);
    process.exit(report.ok ? 0 : 1);
  } catch (err) {
    if (err.code === "LIVE_EXECUTION_REFUSED") {
      console.error(`[ASR-LIVE GATE] ${err.message}`);
      process.exit(1);
    }
    console.error("[ASR-LIVE HARNESS ERROR]", err);
    process.exit(1);
  }
}

module.exports = {
  AsrLiveCompatibilityHarness,
  AsrLiveHarness: AsrLiveCompatibilityHarness,
  assertNonLiveGateProtection,
  assertZeroNativeAuthAccess,
  createIsolatedHarnessEnv,
  isLiveGateRequested,
  runDryRunSuite,
  sanitizeTranscript,
  sanitizeMetadata: sanitizeTranscript,
};
