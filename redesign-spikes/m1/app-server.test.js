#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const readline = require("node:readline");
const test = require("node:test");

const codexBin = process.env.ZODEX_NATIVE_CODEX || "/home/akif/.local/bin/codex";
const allThreadSourceKinds = [
  "cli", "vscode", "exec", "appServer", "subAgent", "subAgentReview",
  "subAgentCompact", "subAgentThreadSpawn", "subAgentOther", "unknown",
];

class AppServerClient {
  constructor({ codexHome, cwd }) {
    this.child = spawn(codexBin, ["app-server", "--listen", "stdio://"], {
      cwd,
      env: { ...process.env, CODEX_HOME: codexHome },
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.nextId = 1;
    this.pending = new Map();
    this.stderr = "";
    this.child.stderr.setEncoding("utf8");
    this.child.stderr.on("data", (chunk) => { this.stderr += chunk; });
    readline.createInterface({ input: this.child.stdout }).on("line", (line) => {
      let message;
      try { message = JSON.parse(line); } catch (_) { return; }
      if (message.id == null) return;
      const pending = this.pending.get(String(message.id));
      if (pending == null) return;
      this.pending.delete(String(message.id));
      if (message.error != null) pending.reject(new Error(JSON.stringify(message.error)));
      else pending.resolve(message.result);
    });
  }

  notify(method, params = {}) {
    this.child.stdin.write(`${JSON.stringify({ method, params })}\n`);
  }

  request(method, params = {}, timeoutMs = 10_000) {
    const id = String(this.nextId++);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`timed out waiting for ${method}; stderr=${this.stderr.slice(-500)}`));
      }, timeoutMs);
      this.pending.set(id, {
        resolve: (value) => { clearTimeout(timer); resolve(value); },
        reject: (error) => { clearTimeout(timer); reject(error); },
      });
      this.child.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
    });
  }

  async initialize() {
    const result = await this.request("initialize", {
      clientInfo: { name: "zodex-m1-spike", version: "0.1.0" },
      capabilities: { experimentalApi: true },
    });
    this.notify("initialized");
    return result;
  }

  async close() {
    if (this.child.exitCode != null) return;
    const closed = new Promise((resolve) => this.child.once("close", resolve));
    this.child.kill("SIGTERM");
    await closed;
  }
}

test("no-turn thread supports live read/update but is not falsely treated as durable", async (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-m1-app-server-"));
  const codexHome = path.join(root, "codex-home");
  const workspace = path.join(root, "workspace");
  fs.mkdirSync(codexHome, { recursive: true, mode: 0o700 });
  fs.mkdirSync(workspace, { recursive: true, mode: 0o700 });
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  let client = new AppServerClient({ codexHome, cwd: workspace });
  t.after(async () => client.close());
  await client.initialize();

  const started = await client.request("thread/start", {
    cwd: workspace,
    ephemeral: false,
    environments: [],
  });
  const threadId = started.thread.id;
  assert.match(threadId, /^[0-9a-f-]{36}$/i);

  await client.request("thread/inject_items", {
    threadId,
    items: [{
      type: "message",
      role: "user",
      content: [{ type: "input_text", text: "M1 synthetic persistence sentinel" }],
    }],
  });
  await client.request("thread/name/set", { threadId, name: "M1 synthetic thread" });
  const read = await client.request("thread/read", { threadId, includeTurns: true });
  assert.equal(read.thread.id, threadId);
  assert.deepEqual(read.thread.turns, []);

  const firstPage = await client.request("thread/list", { limit: 1, sourceKinds: allThreadSourceKinds });
  assert.equal(firstPage.data.length, 0, "a no-turn thread is not durable/listable");
  assert.ok(Object.hasOwn(firstPage, "nextCursor"));

  const turns = await client.request("thread/turns/list", {
    threadId,
    limit: 1,
    sortDirection: "desc",
  });
  assert.deepEqual(turns.data, []);
  assert.equal(turns.nextCursor, null);

  await client.request("thread/archive", { threadId });
  const archived = await client.request("thread/list", {
    archived: true,
    limit: 10,
    sourceKinds: allThreadSourceKinds,
  });
  assert.equal(archived.data.length, 0, "archive must not invent durable history");

  await client.close();
  client = new AppServerClient({ codexHome, cwd: workspace });
  await client.initialize();
  await assert.rejects(client.request("thread/resume", { threadId, excludeTurns: false }));
});

test("invalid cancellation and approval resolution fail closed without a live turn", async (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-m1-app-server-negative-"));
  const codexHome = path.join(root, "codex-home");
  fs.mkdirSync(codexHome, { recursive: true, mode: 0o700 });
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const client = new AppServerClient({ codexHome, cwd: root });
  t.after(async () => client.close());
  await client.initialize();

  await assert.rejects(
    client.request("turn/interrupt", { threadId: "00000000-0000-0000-0000-000000000000", turnId: "missing" }),
  );
  await assert.rejects(
    client.request("serverRequest/resolved", { requestId: "missing", response: { decision: "decline" } }),
  );
});
