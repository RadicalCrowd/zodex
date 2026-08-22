"use strict";

const { createHash } = require("node:crypto");

const REQUEST_TTL_MS = 5 * 60 * 1000;

const METHODS = new Map([
  ["item/commandExecution/requestApproval", "command"],
  ["item/fileChange/requestApproval", "file-change"],
  ["item/permissions/requestApproval", "permissions"],
  ["item/tool/requestUserInput", "tool-input"],
]);

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requiredString(value) {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (isRecord(value)) {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function reject(reason) {
  return { ok: false, reason };
}

function normalizeQuestion(question) {
  if (!isRecord(question)) return null;
  const id = requiredString(question.id);
  const title = requiredString(question.title);
  const options = question.options;
  if (!id || !title || !Array.isArray(options) || options.length < 2 || options.length > 16) return null;
  const normalized = options.map((option) => {
    if (!isRecord(option)) return null;
    const optionId = requiredString(option.id);
    const label = requiredString(option.label);
    return optionId && label ? { id: optionId, label } : null;
  });
  if (normalized.some((option) => option === null)) return null;
  if (new Set(normalized.map((option) => option.id)).size !== normalized.length) return null;
  return { id, title, options: normalized };
}

function normalizeRequest(raw, receivedAt = Date.now()) {
  if (!isRecord(raw) || !METHODS.has(raw.method) || !isRecord(raw.params)) return reject("unknown-or-malformed-request");
  if (!Number.isSafeInteger(receivedAt) || receivedAt < 0) return reject("invalid-received-at");
  const params = raw.params;
  const requestId = requiredString(raw.requestId) || requiredString(params.requestId);
  const threadId = requiredString(params.threadId);
  const turnId = requiredString(params.turnId);
  const itemId = requiredString(params.itemId);
  if (!requestId || !threadId || !turnId || !itemId) return reject("missing-request-identity");

  const kind = METHODS.get(raw.method);
  let action;
  if (kind === "command") {
    if (!Array.isArray(params.argv) || params.argv.length === 0 || params.argv.some((arg) => typeof arg !== "string")) return reject("invalid-command");
    action = { argv: [...params.argv], cwd: requiredString(params.cwd) || null, reason: requiredString(params.reason) || null };
  } else if (kind === "file-change") {
    if (!Array.isArray(params.paths) || params.paths.length === 0 || params.paths.some((path) => !requiredString(path))) return reject("invalid-file-change");
    action = { paths: [...params.paths], reason: requiredString(params.reason) || null };
  } else if (kind === "permissions") {
    if (!Array.isArray(params.permissions) || params.permissions.length === 0 || params.permissions.some((permission) => !requiredString(permission))) return reject("invalid-permissions");
    action = { permissions: [...params.permissions], reason: requiredString(params.reason) || null };
  } else {
    if (!Array.isArray(params.questions) || params.questions.length === 0 || params.questions.some((question) => normalizeQuestion(question) === null)) return reject("free-form-or-invalid-input");
    action = { questions: params.questions.map(normalizeQuestion) };
  }

  if (params.expiresAt !== undefined && (!Number.isSafeInteger(params.expiresAt) || params.expiresAt <= receivedAt || params.expiresAt > receivedAt + REQUEST_TTL_MS)) {
    return reject("invalid-expiry");
  }
  const normalizedAction = Object.freeze(action);
  const actionDigest = createHash("sha256").update(canonical({ method: raw.method, threadId, turnId, itemId, action: normalizedAction })).digest("hex");
  return {
    ok: true,
    request: Object.freeze({
      requestId,
      method: raw.method,
      kind,
      threadId,
      turnId,
      itemId,
      action: normalizedAction,
      actionDigest,
      receivedAt,
      expiresAt: params.expiresAt === undefined ? receivedAt + REQUEST_TTL_MS : params.expiresAt,
    }),
  };
}

function validDecision(request, response) {
  if (!isRecord(response)) return false;
  if (request.kind === "tool-input") {
    if (!isRecord(response.answers)) return false;
    return request.action.questions.every((question) => {
      const value = response.answers[question.id];
      return typeof value === "string" && question.options.some((option) => option.id === value);
    }) && Object.keys(response.answers).length === request.action.questions.length;
  }
  return response.decision === "approve" || response.decision === "reject";
}

class ApprovalFixture {
  constructor({ clock = () => Date.now(), ttlMs = REQUEST_TTL_MS } = {}) {
    if (!Number.isSafeInteger(ttlMs) || ttlMs <= 0 || ttlMs > REQUEST_TTL_MS) throw new RangeError("ttlMs must be within five minutes");
    this.clock = clock;
    this.ttlMs = ttlMs;
    this.requests = new Map();
    this.clients = new Set();
  }

  connect(clientId) {
    if (!requiredString(clientId)) return { ok: false, reason: "invalid-client" };
    this.clients.add(clientId);
    return { ok: true };
  }

  disconnect(clientId) {
    this.clients.delete(clientId);
    for (const state of this.requests.values()) {
      if (state.status === "pending") {
        state.deliveredTo.delete(clientId);
        if (state.deliveredTo.size === 0 && state.hadDelivery) this._finish(state, "cancelled", "disconnect");
      }
    }
    return { ok: true };
  }

  receive(raw) {
    const result = normalizeRequest(raw, this.clock());
    if (!result.ok) return result;
    if (result.request.expiresAt > result.request.receivedAt + this.ttlMs) return reject("expiry-exceeds-fixture-ttl");
    if (this.requests.has(result.request.requestId)) return reject("duplicate-request");
    const state = { request: result.request, status: "pending", deliveredTo: new Set(), hadDelivery: false, response: null, reason: null };
    this.requests.set(result.request.requestId, state);
    return { ok: true, request: clone(result.request) };
  }

  deliver(requestId, clientId) {
    const state = this.requests.get(requestId);
    if (!state || state.status !== "pending" || !this.clients.has(clientId)) return { ok: false, reason: "not-deliverable" };
    if (this.clock() >= state.request.expiresAt) { this._finish(state, "expired", "ttl"); return { ok: false, reason: "expired" }; }
    state.deliveredTo.add(clientId);
    state.hadDelivery = true;
    return { ok: true };
  }

  respond(requestId, clientId, response, threadId) {
    const state = this.requests.get(requestId);
    if (!state || state.status !== "pending") return { ok: false, reason: state?.status === "resolved" ? "already-resolved" : "not-pending" };
    if (!this.clients.has(clientId) || !state.deliveredTo.has(clientId)) return { ok: false, reason: "client-not-delivered" };
    if (threadId !== state.request.threadId) return { ok: false, reason: "cross-thread-response" };
    if (this.clock() >= state.request.expiresAt) { this._finish(state, "expired", "ttl"); return { ok: false, reason: "expired" }; }
    if (!validDecision(state.request, response)) return { ok: false, reason: "invalid-response" };
    this._finish(state, "resolved", "response");
    state.response = clone(response);
    return { ok: true, response: clone(response) };
  }

  resolve(requestId) { return this._transition(requestId, "resolved", "serverRequest/resolved"); }
  cancel(requestId, reason = "cancelled") { return this._transition(requestId, "cancelled", reason); }

  get(requestId) {
    const state = this.requests.get(requestId);
    return state ? { request: clone(state.request), status: state.status, response: state.response && clone(state.response), reason: state.reason } : null;
  }

  _transition(requestId, status, reason) {
    const state = this.requests.get(requestId);
    if (!state) return { ok: false, reason: "unknown-request" };
    if (state.status !== "pending") return { ok: false, reason: "already-finished" };
    this._finish(state, status, reason);
    return { ok: true, status };
  }

  _finish(state, status, reason) { state.status = status; state.reason = reason; state.deliveredTo.clear(); }
}

module.exports = { ApprovalFixture, REQUEST_TTL_MS, normalizeRequest };
