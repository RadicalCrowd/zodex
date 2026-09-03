"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");

const SCHEMA_VERSION = 1;
const ALGORITHM = "aes-256-gcm";

function requireInstallationKey(keyring) {
  const key = keyring.get("zodex", "installation-key");
  if (!Buffer.isBuffer(key) || key.length !== 32) {
    throw new Error("KEYRING_UNAVAILABLE: no plaintext fallback is permitted");
  }
  return key;
}

function encryptRecord(keyring, record, authenticatedMetadata) {
  const key = requireInstallationKey(keyring);
  const nonce = crypto.randomBytes(12);
  const aad = Buffer.from(JSON.stringify(authenticatedMetadata));
  const cipher = crypto.createCipheriv(ALGORITHM, key, nonce);
  cipher.setAAD(aad);
  const ciphertext = Buffer.concat([cipher.update(JSON.stringify(record), "utf8"), cipher.final()]);
  return {
    schemaVersion: SCHEMA_VERSION,
    algorithm: ALGORITHM,
    nonce: nonce.toString("base64"),
    aad: aad.toString("base64"),
    ciphertext: ciphertext.toString("base64"),
    tag: cipher.getAuthTag().toString("base64"),
  };
}

function decryptRecord(keyring, envelope) {
  if (envelope.schemaVersion !== SCHEMA_VERSION || envelope.algorithm !== ALGORITHM) {
    throw new Error("STORE_MIGRATION_REQUIRED");
  }
  const key = requireInstallationKey(keyring);
  const decipher = crypto.createDecipheriv(ALGORITHM, key, Buffer.from(envelope.nonce, "base64"));
  decipher.setAAD(Buffer.from(envelope.aad, "base64"));
  decipher.setAuthTag(Buffer.from(envelope.tag, "base64"));
  const plaintext = Buffer.concat([
    decipher.update(Buffer.from(envelope.ciphertext, "base64")),
    decipher.final(),
  ]);
  return JSON.parse(plaintext.toString("utf8"));
}

function writeEnvelope(file, envelope) {
  const temporary = `${file}.new`;
  fs.writeFileSync(temporary, `${JSON.stringify(envelope)}\n`, { mode: 0o600, flag: "wx" });
  fs.renameSync(temporary, file);
}

module.exports = { decryptRecord, encryptRecord, requireInstallationKey, writeEnvelope };
