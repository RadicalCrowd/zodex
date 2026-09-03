#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { decryptRecord, encryptRecord, writeEnvelope } = require("./storage.js");

class SyntheticKeyring {
  constructor() { this.values = new Map(); this.locked = false; }
  set(service, account, value) {
    if (this.locked) throw new Error("KEYRING_LOCKED");
    this.values.set(`${service}:${account}`, Buffer.from(value));
  }
  get(service, account) {
    if (this.locked) throw new Error("KEYRING_LOCKED");
    return this.values.get(`${service}:${account}`);
  }
}

test("keyring-held key encrypts synthetic content at rest and authenticates metadata", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-m1-storage-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const keyring = new SyntheticKeyring();
  keyring.set("zodex", "installation-key", crypto.randomBytes(32));
  const sentinel = "SYNTHETIC_PRIVATE_CONVERSATION_SENTINEL";
  const metadata = { workspaceId: "workspace-1", profileRevision: "profile@1" };
  const envelope = encryptRecord(keyring, { message: sentinel }, metadata);
  const file = path.join(root, "thread.json.enc");
  writeEnvelope(file, envelope);

  const raw = fs.readFileSync(file, "utf8");
  assert.doesNotMatch(raw, new RegExp(sentinel));
  assert.equal(fs.statSync(file).mode & 0o077, 0);
  assert.deepEqual(decryptRecord(keyring, JSON.parse(raw)), { message: sentinel });

  const tampered = { ...envelope, aad: Buffer.from('{"workspaceId":"foreign"}').toString("base64") };
  assert.throws(() => decryptRecord(keyring, tampered));
});

test("locked/missing keyring and incompatible migration fail without plaintext", () => {
  const keyring = new SyntheticKeyring();
  const record = { message: "synthetic" };
  assert.throws(() => encryptRecord(keyring, record, {}), /KEYRING_UNAVAILABLE/);
  keyring.set("zodex", "installation-key", crypto.randomBytes(32));
  const envelope = encryptRecord(keyring, record, {});
  keyring.locked = true;
  assert.throws(() => decryptRecord(keyring, envelope), /KEYRING_LOCKED/);
  keyring.locked = false;
  assert.throws(() => decryptRecord(keyring, { ...envelope, schemaVersion: 999 }), /STORE_MIGRATION_REQUIRED/);
});

test("archive is ciphertext-only and delete leaves a metadata-only tombstone", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-m1-archive-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const keyring = new SyntheticKeyring();
  keyring.set("zodex", "installation-key", crypto.randomBytes(32));
  const content = path.join(root, "thread.enc");
  const archive = path.join(root, "archive.enc");
  writeEnvelope(content, encryptRecord(keyring, { message: "synthetic secret" }, { threadId: "thread-1" }));
  fs.renameSync(content, archive);
  assert.doesNotMatch(fs.readFileSync(archive, "utf8"), /synthetic secret/);
  fs.unlinkSync(archive);
  const tombstone = path.join(root, "thread.tombstone.json");
  fs.writeFileSync(tombstone, `${JSON.stringify({ schemaVersion: 1, threadId: "thread-1", deleted: true })}\n`, { mode: 0o600 });
  assert.equal(fs.existsSync(archive), false);
  assert.doesNotMatch(fs.readFileSync(tombstone, "utf8"), /message|secret/i);
});
