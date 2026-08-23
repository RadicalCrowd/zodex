"use strict";

const fs = require("node:fs");
const path = require("node:path");

const CONFIG_VERSION = 1;
const CONFIG_RELATIVE_PATH = path.join("zodex", "config.json");
const ACKNOWLEDGEMENT_RISK = "zodex-oauth-risk-v1";
const ACKNOWLEDGEMENT_EFFECTS = "zodex-oauth-effects-v1";
const ACKNOWLEDGEMENTS = Object.freeze([
  ACKNOWLEDGEMENT_RISK,
  ACKNOWLEDGEMENT_EFFECTS,
]);
const SECRET_KEY_PATTERN = /(?:api[-_]?key|access[-_]?token|refresh[-_]?token|password|secret|credential|private[-_]?key)/iu;
const ID_PATTERN = /^[a-z0-9][a-z0-9-]*$/u;
const BROKER_PROVIDERS = Object.freeze({
  omniroute: new Set(["anthropic", "google", "opencode", "kilo"]),
  "opencode-community": new Set(),
});

function isObject(value) {
  return value != null && typeof value === "object" && !Array.isArray(value);
}

function assertKnownKeys(value, allowed, location) {
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) throw new Error(`${location} contains unknown key '${key}'`);
  }
}

function assertNoSecrets(value, location = "config") {
  if (Array.isArray(value)) {
    value.forEach((entry, index) => assertNoSecrets(entry, `${location}[${index}]`));
    return;
  }
  if (!isObject(value)) return;
  for (const [key, entry] of Object.entries(value)) {
    if (SECRET_KEY_PATTERN.test(key)) {
      throw new Error(`${location}.${key} is forbidden; credentials never belong in Zodex JSON`);
    }
    assertNoSecrets(entry, `${location}.${key}`);
  }
}

function parseConnection(value, location) {
  if (!isObject(value)) throw new Error(`${location} must be an object`);
  assertKnownKeys(value, ["enabled", "acknowledgements"], location);
  if (typeof value.enabled !== "boolean") throw new Error(`${location}.enabled must be a boolean`);
  if (!Array.isArray(value.acknowledgements) || value.acknowledgements.some((item) => typeof item !== "string")) {
    throw new Error(`${location}.acknowledgements must be an array of strings`);
  }
  const acknowledgements = [...new Set(value.acknowledgements)];
  const unknown = acknowledgements.filter((item) => !ACKNOWLEDGEMENTS.includes(item));
  if (unknown.length) throw new Error(`${location} has unknown acknowledgement '${unknown[0]}'`);
  return { enabled: value.enabled, acknowledgements };
}

function parseBroker(value, location, brokerId) {
  if (!isObject(value)) throw new Error(`${location} must be an object`);
  assertKnownKeys(value, ["enabled", "providers"], location);
  if (typeof value.enabled !== "boolean") throw new Error(`${location}.enabled must be a boolean`);
  if (!isObject(value.providers)) throw new Error(`${location}.providers must be an object`);
  const providers = {};
  for (const [providerId, connection] of Object.entries(value.providers)) {
    if (!ID_PATTERN.test(providerId)) throw new Error(`${location}.providers has invalid id '${providerId}'`);
    if (!BROKER_PROVIDERS[brokerId].has(providerId)) {
      throw new Error(`${location}.providers.${providerId} is not an audited ${brokerId} provider`);
    }
    providers[providerId] = parseConnection(connection, `${location}.providers.${providerId}`);
  }
  return { enabled: value.enabled, providers };
}

function parseConfig(value) {
  if (!isObject(value)) throw new Error("config must be an object");
  assertNoSecrets(value);
  assertKnownKeys(value, ["version", "oauth", "remoteControl"], "config");
  if (value.version !== CONFIG_VERSION) throw new Error(`config.version must be ${CONFIG_VERSION}`);
  if (!isObject(value.oauth)) throw new Error("config.oauth must be an object");
  assertKnownKeys(value.oauth, ["brokers"], "config.oauth");
  if (!isObject(value.oauth.brokers)) throw new Error("config.oauth.brokers must be an object");
  const brokers = {};
  for (const [brokerId, broker] of Object.entries(value.oauth.brokers)) {
    if (!ID_PATTERN.test(brokerId)) throw new Error(`config.oauth.brokers has invalid id '${brokerId}'`);
    if (!Object.hasOwn(BROKER_PROVIDERS, brokerId)) {
      throw new Error(`config.oauth.brokers.${brokerId} is not an audited broker`);
    }
    brokers[brokerId] = parseBroker(broker, `config.oauth.brokers.${brokerId}`, brokerId);
  }
  if (!isObject(value.remoteControl)) throw new Error("config.remoteControl must be an object");
  assertKnownKeys(value.remoteControl, ["enabled", "extensions"], "config.remoteControl");
  if (typeof value.remoteControl.enabled !== "boolean") {
    throw new Error("config.remoteControl.enabled must be a boolean");
  }
  if (!Array.isArray(value.remoteControl.extensions) || value.remoteControl.extensions.some((id) => !ID_PATTERN.test(id))) {
    throw new Error("config.remoteControl.extensions must be an array of module ids");
  }
  return {
    version: CONFIG_VERSION,
    oauth: { brokers },
    remoteControl: {
      enabled: value.remoteControl.enabled,
      extensions: [...new Set(value.remoteControl.extensions)],
    },
  };
}

function configPath(environment = process.env) {
  if (typeof environment.ZODEX_CONFIG_FILE === "string" && environment.ZODEX_CONFIG_FILE.trim()) {
    return path.resolve(environment.ZODEX_CONFIG_FILE);
  }
  const home = environment.HOME;
  const root = environment.XDG_CONFIG_HOME || (home ? path.join(home, ".config") : undefined);
  return root ? path.join(root, CONFIG_RELATIVE_PATH) : undefined;
}

function readConfig(environment = process.env) {
  const file = configPath(environment);
  if (!file || !fs.existsSync(file)) return { state: "missing", file, config: null };
  try {
    let text;
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const before = fs.statSync(file);
      if (!before.isFile()) throw new Error("config path is not a regular file");
      if (before.size > 64 * 1024) throw new Error("config file exceeds 64 KiB");
      if (typeof process.getuid === "function" && before.uid !== process.getuid()) {
        throw new Error("config file is not owned by the current user");
      }
      if (process.platform !== "win32" && (before.mode & 0o077) !== 0) {
        throw new Error("config file permissions must be 0600");
      }
      text = fs.readFileSync(file, "utf8");
      const after = fs.statSync(file);
      if (
        before.ino === after.ino &&
        before.size === after.size &&
        before.mtimeMs === after.mtimeMs
      ) break;
      text = undefined;
    }
    if (text === undefined) throw new Error("config changed while it was being read; refresh again");
    return { state: "valid", file, config: parseConfig(JSON.parse(text)) };
  } catch (error) {
    return { state: "invalid", file, config: null, error: error instanceof Error ? error.message : String(error) };
  }
}

function connectionState({ brokerId, providerId, broker, connection }) {
  const base = { brokerId, providerId, active: false };
  if (!broker.enabled || !connection.enabled) return { ...base, state: "off" };
  if (!connection.acknowledgements.includes(ACKNOWLEDGEMENT_RISK)) {
    return {
      ...base,
      state: "warning-one",
      title: `${providerId} OAuth remains off`,
      message: `Warning 1 of 2: ${brokerId} is a third-party OAuth broker. It can receive prompts, responses, and account tokens, and the upstream provider may restrict this use. Add '${ACKNOWLEDGEMENT_RISK}' to this connection only after you accept that risk.`,
    };
  }
  if (!connection.acknowledgements.includes(ACKNOWLEDGEMENT_EFFECTS)) {
    return {
      ...base,
      state: "warning-two",
      title: `${providerId} OAuth is ready for final acknowledgement`,
      message: `Warning 2 of 2: enabling this connection sends its selected model requests through the local ${brokerId} service and the signed-in ${providerId} account. It does not replace ChatGPT login or change other Codex providers. Add '${ACKNOWLEDGEMENT_EFFECTS}' to activate it on the next refresh.`,
    };
  }
  return { ...base, state: "enabled", active: true };
}

function brokerConnectionStates(config) {
  const states = [];
  for (const [brokerId, broker] of Object.entries(config.oauth.brokers)) {
    for (const [providerId, connection] of Object.entries(broker.providers)) {
      states.push(connectionState({ brokerId, providerId, broker, connection }));
    }
  }
  return states;
}

module.exports = {
  ACKNOWLEDGEMENT_EFFECTS,
  ACKNOWLEDGEMENT_RISK,
  CONFIG_VERSION,
  assertNoSecrets,
  brokerConnectionStates,
  configPath,
  connectionState,
  parseConfig,
  readConfig,
};
