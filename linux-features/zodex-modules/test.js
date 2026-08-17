#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const {
  ACKNOWLEDGEMENT_EFFECTS,
  ACKNOWLEDGEMENT_RISK,
  CONFIG_VERSION,
  assertNoSecrets,
  brokerConnectionStates,
  configPath,
  connectionState,
  parseConfig,
  readConfig,
} = require("./config.js");
const { planRemoteControlModules, validateModuleManifest } = require("./module-host.js");
const { applyMainBundlePatch, applyWebviewRuntimePatch, descriptors } = require("./patch.js");
const {
  enabledLinuxFeatureInstallPlan,
  loadLinuxFeaturePatchDescriptors,
} = require("../../scripts/lib/linux-features.js");

function validConfig(connection = { enabled: false, acknowledgements: [] }) {
  return {
    version: 1,
    oauth: {
      brokers: {
        omniroute: {
          enabled: true,
          providers: { anthropic: connection },
        },
      },
    },
    remoteControl: { enabled: false, extensions: [] },
  };
}

function compileMainBundleStatus(sourceWithPatch, env = {}) {
  const sandbox = {
    require: (mod) => {
      if (mod === "node:path" || mod === "path") return require("node:path");
      if (mod === "node:fs" || mod === "fs") return require("node:fs");
      return require(mod);
    },
    process: {
      env: { ...env },
      platform: process.platform,
      getuid: process.getuid,
      resourcesPath: env.resourcesPath,
    },
    console,
  };
  const wrapped = `${sourceWithPatch}\n;globalThis.__zodexModuleStatus = zodexModuleStatus;`;
  vm.createContext(sandbox);
  vm.runInContext(wrapped, sandbox);
  return () => {
    const res = sandbox.__zodexModuleStatus();
    if (res && res.connections) {
      res.connections = Array.from(res.connections);
    }
    return res;
  };
}

function createWebviewTestEnvironment(statusPayload) {
  const elements = [];
  const headElements = [];
  let dialogElement = null;

  const doc = {
    readyState: "complete",
    head: {
      appendChild(el) { headElements.push(el); },
    },
    body: {
      appendChild(el) {
        elements.push(el);
        if (el.className === "zodex-module-warning") dialogElement = el;
      },
    },
    documentElement: null,
    getElementById(id) {
      if (id === "zodex-module-warning-style") return headElements.find((e) => e.id === id) || null;
      if (id === "zodex-oauth-active") return elements.find((e) => e.id === id) || null;
      return null;
    },
    createElement(tag) {
      const attrs = new Map();
      const children = [];
      const el = {
        tag,
        id: "",
        className: "",
        textContent: "",
        type: "",
        setAttribute(k, v) { attrs.set(k, v); },
        getAttribute(k) { return attrs.get(k); },
        append(...items) { children.push(...items); },
        appendChild(child) { children.push(child); },
        addEventListener(name, handler) {
          if (name === "click") el._onClick = handler;
        },
        remove() {
          const idx = elements.indexOf(el);
          if (idx !== -1) elements.splice(idx, 1);
          if (dialogElement === el) dialogElement = null;
        },
        children,
      };
      return el;
    },
  };

  const messageListeners = [];
  const sandbox = {
    document: doc,
    window: {
      addEventListener(name, handler) {
        if (name === "message") messageListeners.push(handler);
      },
      dispatchEvent(event) {
        if (event.type === "codex-message-from-view" && event.detail?.url === "vscode://codex/zodex-module-status") {
          const requestId = event.detail.requestId;
          setImmediate(() => {
            const responseEvent = {
              data: {
                type: "fetch-response",
                requestId,
                responseType: "success",
                bodyJsonString: JSON.stringify(statusPayload),
              },
            };
            messageListeners.forEach((fn) => fn(responseEvent));
          });
        }
      },
      electronBridge: {
        sendMessageFromView: async () => {},
      },
    },
    CustomEvent: class CustomEvent {
      constructor(type, init = {}) {
        this.type = type;
        this.detail = init.detail;
      }
    },
    setTimeout,
    clearTimeout,
    console,
  };
  sandbox.globalThis = sandbox;
  return { sandbox, doc, getDialog: () => dialogElement, getElements: () => elements };
}

// -----------------------------------------------------------------------------
// Scenario A: Disabled / Unconfigured Optional Modules (No Warning Modal)
// -----------------------------------------------------------------------------

test("Scenario A: status reports ok/missing when CODEX_LINUX_FEATURES_DIR and CODEX_LINUX_APP_DIR are unset", () => {
  const patchedSource = applyMainBundlePatch('"use strict";const handlers={"native-desktop-apps":async()=>{}};');
  const getStatus = compileMainBundleStatus(patchedSource, {});
  const status = getStatus();

  assert.equal(status.ok, true);
  assert.equal(status.state, "missing");
  assert.equal(status.file, null);
  assert.equal(status.error, undefined);
  assert.deepEqual(status.connections, []);
});

test("Scenario A: status reports ok/missing when staged features exist but config.json is absent", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-unconfigured-test-"));
  try {
    const stagedDir = path.join(root, ".codex-linux", "features", "zodex-modules");
    fs.mkdirSync(stagedDir, { recursive: true });
    fs.copyFileSync(path.join(__dirname, "config.js"), path.join(stagedDir, "config.js"));

    const patchedSource = applyMainBundlePatch('"use strict";const handlers={"native-desktop-apps":async()=>{}};');
    const getStatus = compileMainBundleStatus(patchedSource, {
      CODEX_LINUX_APP_DIR: root,
      HOME: root, // no ~/.config/zodex/config.json
    });
    const status = getStatus();

    assert.equal(status.ok, true);
    assert.equal(status.state, "missing");
    assert.equal(typeof status.file, "string");
    assert.equal(status.error, undefined);
    assert.deepEqual(status.connections, []);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("Scenario A: webview runtime does not show warning modal when state is missing or unconfigured", async () => {
  const env = createWebviewTestEnvironment({ ok: true, state: "missing", file: null, connections: [] });
  vm.createContext(env.sandbox);
  const runtime = applyWebviewRuntimePatch("const app = true;\n");
  vm.runInContext(runtime, env.sandbox);

  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(env.getDialog(), null, "Missing/unconfigured state must not display warning dialog");
  assert.equal(env.sandbox.document.getElementById("zodex-oauth-active"), null);
});

// -----------------------------------------------------------------------------
// Scenario B: Explicitly Enabled Valid Directory & Config (Warning 1, 2, Enabled)
// -----------------------------------------------------------------------------

test("Scenario B: valid config transitions through Warning 1, Warning 2, and Enabled states", () => {
  const unacknowledged = validConfig({ enabled: true, acknowledgements: [] });
  const first = brokerConnectionStates(parseConfig(unacknowledged))[0];
  assert.equal(first.state, "warning-one");
  assert.equal(first.active, false);
  assert.match(first.message, new RegExp(ACKNOWLEDGEMENT_RISK));

  const riskAck = validConfig({ enabled: true, acknowledgements: [ACKNOWLEDGEMENT_RISK] });
  const second = brokerConnectionStates(parseConfig(riskAck))[0];
  assert.equal(second.state, "warning-two");
  assert.equal(second.active, false);
  assert.match(second.message, new RegExp(ACKNOWLEDGEMENT_EFFECTS));

  const fullyAck = validConfig({ enabled: true, acknowledgements: [ACKNOWLEDGEMENT_RISK, ACKNOWLEDGEMENT_EFFECTS] });
  const active = brokerConnectionStates(parseConfig(fullyAck))[0];
  assert.equal(active.state, "enabled");
  assert.equal(active.active, true);

  const disabled = validConfig({ enabled: false, acknowledgements: [] });
  assert.equal(brokerConnectionStates(parseConfig(disabled))[0].state, "off");
});

test("Scenario B: status reports valid state and connections when CODEX_LINUX_FEATURES_DIR is valid", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-valid-env-test-"));
  try {
    const configFile = path.join(root, "config.json");
    fs.writeFileSync(
      configFile,
      JSON.stringify(validConfig({ enabled: true, acknowledgements: [ACKNOWLEDGEMENT_RISK] }), null, 2),
      { mode: 0o600 },
    );

    const patchedSource = applyMainBundlePatch('"use strict";const handlers={"native-desktop-apps":async()=>{}};');
    const getStatus = compileMainBundleStatus(patchedSource, {
      CODEX_LINUX_FEATURES_DIR: path.resolve(__dirname, ".."),
      ZODEX_CONFIG_FILE: configFile,
    });
    const status = getStatus();

    assert.equal(status.ok, true);
    assert.equal(status.state, "valid");
    assert.equal(status.file, configFile);
    assert.equal(status.connections.length, 1);
    assert.equal(status.connections[0].state, "warning-two");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("Scenario B: webview runtime displays warning-one and warning-two modals and enabled banner", async () => {
  // Test Warning 1 Modal
  const env1 = createWebviewTestEnvironment({
    ok: true,
    state: "valid",
    file: "/path/to/config.json",
    connections: [{
      state: "warning-one",
      title: "anthropic OAuth remains off",
      message: `Warning 1 of 2: omniroute is a third-party OAuth broker. Add '${ACKNOWLEDGEMENT_RISK}'`,
    }],
  });
  vm.createContext(env1.sandbox);
  vm.runInContext(applyWebviewRuntimePatch(""), env1.sandbox);
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));

  const dialog1 = env1.getDialog();
  assert.notEqual(dialog1, null);
  assert.match(dialog1.children[0].children[0].textContent, /anthropic OAuth remains off/);
  assert.match(dialog1.children[0].children[1].textContent, new RegExp(ACKNOWLEDGEMENT_RISK));

  // Test Active Banner when Enabled
  const env2 = createWebviewTestEnvironment({
    ok: true,
    state: "valid",
    file: "/path/to/config.json",
    connections: [{ brokerId: "omniroute", providerId: "anthropic", state: "enabled", active: true }],
  });
  vm.createContext(env2.sandbox);
  vm.runInContext(applyWebviewRuntimePatch(""), env2.sandbox);
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(env2.getDialog(), null);
  const banner = env2.sandbox.document.getElementById("zodex-oauth-active");
  assert.notEqual(banner, null);
  assert.match(banner.textContent, /omniroute\/anthropic/);
});

// -----------------------------------------------------------------------------
// Scenario C: Explicitly Enabled Missing/Invalid Directory / Corrupted Config (Fails Closed)
// -----------------------------------------------------------------------------

test("Scenario C: explicitly set missing CODEX_LINUX_FEATURES_DIR fails closed with state invalid", () => {
  const patchedSource = applyMainBundlePatch('"use strict";const handlers={"native-desktop-apps":async()=>{}};');
  const getStatus = compileMainBundleStatus(patchedSource, {
    CODEX_LINUX_FEATURES_DIR: "/nonexistent/features/dir/path",
  });
  const status = getStatus();

  assert.equal(status.ok, false);
  assert.equal(status.state, "invalid");
  assert.match(status.error, /CODEX_LINUX_FEATURES_DIR is invalid/);
  assert.deepEqual(status.connections, []);
});

test("Scenario C: corrupted JSON, bad file mode, non-regular file, and oversized config fail closed", {
  skip: process.platform === "win32",
}, () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-invalid-config-test-"));
  const configFile = path.join(root, "config.json");
  try {
    // 1. Broken JSON
    fs.writeFileSync(configFile, "{broken-json-content\n", { mode: 0o600 });
    const invalidJson = readConfig({ ZODEX_CONFIG_FILE: configFile });
    assert.equal(invalidJson.state, "invalid");
    assert.match(invalidJson.error, /Unexpected token|Expected/);

    // 2. Group/World readable mode (0644)
    fs.writeFileSync(configFile, JSON.stringify(validConfig()), { mode: 0o644 });
    fs.chmodSync(configFile, 0o644);
    const invalidMode = readConfig({ ZODEX_CONFIG_FILE: configFile });
    assert.equal(invalidMode.state, "invalid");
    assert.match(invalidMode.error, /permissions must be 0600/);

    // 3. Directory instead of regular file
    const dirConfig = path.join(root, "dir-config.json");
    fs.mkdirSync(dirConfig);
    const notFile = readConfig({ ZODEX_CONFIG_FILE: dirConfig });
    assert.equal(notFile.state, "invalid");
    assert.match(notFile.error, /not a regular file/);

    // 4. Exceeds 64 KiB
    const largeConfig = path.join(root, "large-config.json");
    const largePadding = " ".repeat(65 * 1024);
    fs.writeFileSync(largeConfig, `{"version":1,"padding":"${largePadding}"}`, { mode: 0o600 });
    const oversized = readConfig({ ZODEX_CONFIG_FILE: largeConfig });
    assert.equal(oversized.state, "invalid");
    assert.match(oversized.error, /exceeds 64 KiB/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("Scenario C: webview runtime displays error modal when status reports state invalid", async () => {
  const env = createWebviewTestEnvironment({
    ok: false,
    state: "invalid",
    file: "/home/user/.config/zodex/config.json",
    error: "config file permissions must be 0600",
    connections: [],
  });
  vm.createContext(env.sandbox);
  vm.runInContext(applyWebviewRuntimePatch(""), env.sandbox);
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));

  const dialog = env.getDialog();
  assert.notEqual(dialog, null);
  assert.match(dialog.children[0].children[0].textContent, /Zodex configuration is invalid/);
  assert.match(dialog.children[0].children[1].textContent, /permissions must be 0600/);
});

// -----------------------------------------------------------------------------
// Scenario D: Malformed Configuration & Secret-Shaped Values Rejected
// -----------------------------------------------------------------------------

test("Scenario D: assertNoSecrets and parseConfig reject secret-shaped keys at any depth", () => {
  const secretKeys = [
    "apiKey", "api_key", "api-key", "API_KEY",
    "accessToken", "access_token", "access-token",
    "refreshToken", "refresh_token", "refresh-token",
    "password", "secret", "credential", "private_key", "private-key",
  ];

  for (const secretKey of secretKeys) {
    const rootSecret = validConfig();
    rootSecret[secretKey] = "prohibited-secret";
    assert.throws(() => parseConfig(rootSecret), /credentials never belong in Zodex JSON/);

    const brokerSecret = validConfig();
    brokerSecret.oauth.brokers.omniroute[secretKey] = "prohibited-secret";
    assert.throws(() => parseConfig(brokerSecret), /credentials never belong in Zodex JSON/);

    const providerSecret = validConfig();
    providerSecret.oauth.brokers.omniroute.providers.anthropic[secretKey] = "prohibited-secret";
    assert.throws(() => parseConfig(providerSecret), /credentials never belong in Zodex JSON/);

    const arraySecret = validConfig();
    arraySecret.remoteControl.extensions = [{ [secretKey]: "prohibited-secret" }];
    assert.throws(() => parseConfig(arraySecret), /credentials never belong in Zodex JSON/);
  }
});

test("Scenario D: parseConfig rejects unknown keys, unreviewed brokers/providers, and invalid modes", () => {
  const unknownTop = validConfig();
  unknownTop.extraField = true;
  assert.throws(() => parseConfig(unknownTop), /config contains unknown key 'extraField'/);

  const unknownOauth = validConfig();
  unknownOauth.oauth.extra = true;
  assert.throws(() => parseConfig(unknownOauth), /config.oauth contains unknown key 'extra'/);

  const unknownBroker = validConfig();
  unknownBroker.oauth.brokers.unreviewed = { enabled: true, providers: {} };
  assert.throws(() => parseConfig(unknownBroker), /not an audited broker/);

  const unknownProvider = validConfig();
  unknownProvider.oauth.brokers.omniroute.providers.unreviewed = { enabled: true, acknowledgements: [] };
  assert.throws(() => parseConfig(unknownProvider), /not an audited omniroute provider/);

  const invalidBrokerId = validConfig();
  invalidBrokerId.oauth.brokers["INVALID_ID"] = { enabled: true, providers: {} };
  assert.throws(() => parseConfig(invalidBrokerId), /has invalid id 'INVALID_ID'/);

  const unknownAck = validConfig();
  unknownAck.oauth.brokers.omniroute.providers.anthropic.acknowledgements = ["unreviewed-ack-v1"];
  assert.throws(() => parseConfig(unknownAck), /has unknown acknowledgement 'unreviewed-ack-v1'/);

  const invalidVersion = validConfig();
  invalidVersion.version = 2;
  assert.throws(() => parseConfig(invalidVersion), /config.version must be 1/);

  const nonBooleanEnabled = validConfig();
  nonBooleanEnabled.remoteControl.enabled = "yes";
  assert.throws(() => parseConfig(nonBooleanEnabled), /config.remoteControl.enabled must be a boolean/);
});

// -----------------------------------------------------------------------------
// Upstream Baseline Preservation & Patch Idempotency
// -----------------------------------------------------------------------------

test("feature is disabled by default, staging 0 patches or files until selected", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-feature-baseline-test-"));
  try {
    fs.cpSync(__dirname, path.join(root, "zodex-modules"), { recursive: true });
    fs.writeFileSync(path.join(root, "features.example.json"), '{"enabled":[]}\n');
    fs.writeFileSync(path.join(root, "features.json"), '{"enabled":[]}\n');

    assert.deepEqual(loadLinuxFeaturePatchDescriptors({ featuresRoot: root }), []);

    fs.writeFileSync(path.join(root, "features.json"), '{"enabled":["zodex-modules"]}\n');
    assert.deepEqual(
      loadLinuxFeaturePatchDescriptors({ featuresRoot: root }).map(({ name }) => name),
      [
        "feature:zodex-modules:module-status-handler",
        "feature:zodex-modules:oauth-warning-runtime",
      ],
    );
    assert.deepEqual(
      enabledLinuxFeatureInstallPlan({ featuresRoot: root }).resources.map(({ target }) => target),
      [
        ".codex-linux/features/zodex-modules/config.js",
        ".codex-linux/features/zodex-modules/config.example.json",
      ],
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("patch application is idempotent and preserves exact descriptors", () => {
  const initialMain = '"use strict";const handlers={"native-desktop-apps":async()=>{}};';
  const patchedMain = applyMainBundlePatch(initialMain);
  assert.match(patchedMain, /"zodex-module-status":async/);
  assert.match(patchedMain, /zodexConfigModulePath/);
  assert.equal(applyMainBundlePatch(patchedMain), patchedMain);

  const initialWebview = "const app=true;\n";
  const patchedWebview = applyWebviewRuntimePatch(initialWebview);
  assert.match(patchedWebview, /zodexModuleWarningVersion/);
  assert.equal(applyWebviewRuntimePatch(patchedWebview), patchedWebview);

  assert.deepEqual(descriptors.map(({ id }) => id), ["module-status-handler", "oauth-warning-runtime"]);
});

test("remote-control modules are capability-gated and off by default", () => {
  const manifest = validateModuleManifest({
    apiVersion: 1,
    id: "example-observer",
    displayName: "Example observer",
    defaultEnabled: false,
    capabilities: ["remote-control.status:read"],
  });
  assert.equal(manifest.defaultEnabled, false);
  const off = planRemoteControlModules([manifest], { enabled: false, extensions: [manifest.id] });
  assert.equal(off.modules[0].state, "off");
  const ready = planRemoteControlModules(
    [manifest],
    { enabled: true, extensions: [manifest.id] },
    ["remote-control.status:read"],
  );
  assert.equal(ready.modules[0].state, "ready");
});
