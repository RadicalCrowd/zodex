"use strict";

const MODULE_API_VERSION = 1;
const MODULE_ID_PATTERN = /^[a-z0-9][a-z0-9-]*$/u;
const REMOTE_CONTROL_CAPABILITIES = Object.freeze([
  "remote-control.status:read",
  "remote-control.conversation:observe",
]);

function validateModuleManifest(manifest) {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error("module manifest must be an object");
  }
  const allowed = ["apiVersion", "id", "displayName", "defaultEnabled", "capabilities"];
  for (const key of Object.keys(manifest)) {
    if (!allowed.includes(key)) throw new Error(`module ${manifest.id || "unknown"} contains unknown key '${key}'`);
  }
  if (manifest.apiVersion !== MODULE_API_VERSION) throw new Error(`module apiVersion must be ${MODULE_API_VERSION}`);
  if (!MODULE_ID_PATTERN.test(manifest.id || "")) throw new Error("module id is invalid");
  if (typeof manifest.displayName !== "string" || !manifest.displayName.trim()) {
    throw new Error(`module ${manifest.id} requires displayName`);
  }
  if (manifest.defaultEnabled !== false) throw new Error(`module ${manifest.id} must be disabled by default`);
  if (!Array.isArray(manifest.capabilities)) throw new Error(`module ${manifest.id} capabilities must be an array`);
  const capabilities = [...new Set(manifest.capabilities)];
  const unsupported = capabilities.filter((capability) => !REMOTE_CONTROL_CAPABILITIES.includes(capability));
  if (unsupported.length) throw new Error(`module ${manifest.id} requests unsupported capability '${unsupported[0]}'`);
  return { ...manifest, capabilities };
}

function planRemoteControlModules(manifests, remoteControlConfig, availableCapabilities = []) {
  const available = new Set(availableCapabilities);
  const selected = new Set(remoteControlConfig.extensions || []);
  const known = new Set();
  const modules = manifests.map((input) => {
    const manifest = validateModuleManifest(input);
    if (known.has(manifest.id)) throw new Error(`duplicate module id '${manifest.id}'`);
    known.add(manifest.id);
    if (!remoteControlConfig.enabled || !selected.has(manifest.id)) {
      return { id: manifest.id, state: "off", capabilities: manifest.capabilities };
    }
    const missing = manifest.capabilities.filter((capability) => !available.has(capability));
    return missing.length
      ? { id: manifest.id, state: "unavailable", capabilities: manifest.capabilities, missing }
      : { id: manifest.id, state: "ready", capabilities: manifest.capabilities };
  });
  const unknown = [...selected].filter((id) => !known.has(id));
  return { modules, unknown };
}

module.exports = {
  MODULE_API_VERSION,
  REMOTE_CONTROL_CAPABILITIES,
  planRemoteControlModules,
  validateModuleManifest,
};
