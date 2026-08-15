#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  ACKNOWLEDGEMENT_EFFECTS,
  ACKNOWLEDGEMENT_RISK,
  brokerConnectionStates,
  parseConfig,
  readConfig,
} = require("./config.js");
const { planRemoteControlModules, validateModuleManifest } = require("./module-host.js");
const { applyMainBundlePatch, applyWebviewRuntimePatch, descriptors } = require("./patch.js");
const {
  enabledLinuxFeatureInstallPlan,
  loadLinuxFeaturePatchDescriptors,
} = require("../../scripts/lib/linux-features.js");

function config(connection = { enabled: false, acknowledgements: [] }) {
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

test("Zodex OAuth connections require both file acknowledgements in order", () => {
  const first = brokerConnectionStates(parseConfig(config({ enabled: true, acknowledgements: [] })))[0];
  assert.equal(first.state, "warning-one");
  assert.equal(first.active, false);
  assert.match(first.message, new RegExp(ACKNOWLEDGEMENT_RISK));

  const second = brokerConnectionStates(parseConfig(config({
    enabled: true,
    acknowledgements: [ACKNOWLEDGEMENT_RISK],
  })))[0];
  assert.equal(second.state, "warning-two");
  assert.equal(second.active, false);
  assert.match(second.message, new RegExp(ACKNOWLEDGEMENT_EFFECTS));

  const active = brokerConnectionStates(parseConfig(config({
    enabled: true,
    acknowledgements: [ACKNOWLEDGEMENT_RISK, ACKNOWLEDGEMENT_EFFECTS],
  })))[0];
  assert.equal(active.state, "enabled");
  assert.equal(active.active, true);
});

test("disabled broker or connection remains off without warnings", () => {
  const value = config({ enabled: false, acknowledgements: [] });
  assert.equal(brokerConnectionStates(parseConfig(value))[0].state, "off");
  value.oauth.brokers.omniroute.enabled = false;
  value.oauth.brokers.omniroute.providers.anthropic.enabled = true;
  assert.equal(brokerConnectionStates(parseConfig(value))[0].state, "off");
});

test("Zodex JSON rejects credentials and unknown fields", () => {
  const withKey = config();
  withKey.oauth.brokers.omniroute.apiKey = "must-not-be-here";
  assert.throws(() => parseConfig(withKey), /credentials never belong in Zodex JSON/);
  const withUnknown = config();
  withUnknown.remoteControl.magic = true;
  assert.throws(() => parseConfig(withUnknown), /unknown key 'magic'/);

  const unknownBroker = config();
  unknownBroker.oauth.brokers.unreviewed = unknownBroker.oauth.brokers.omniroute;
  delete unknownBroker.oauth.brokers.omniroute;
  assert.throws(() => parseConfig(unknownBroker), /not an audited broker/);

  const unknownProvider = config();
  unknownProvider.oauth.brokers.omniroute.providers.unreviewed =
    unknownProvider.oauth.brokers.omniroute.providers.anthropic;
  delete unknownProvider.oauth.brokers.omniroute.providers.anthropic;
  assert.throws(() => parseConfig(unknownProvider), /not an audited omniroute provider/);

  const futureVersion = config();
  futureVersion.version = 2;
  assert.throws(() => parseConfig(futureVersion), /config.version must be 1/);
});

test("config reader reports missing, invalid, and valid files without exposing contents", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-config-test-"));
  const file = path.join(root, "config.json");
  try {
    assert.equal(readConfig({ ZODEX_CONFIG_FILE: file }).state, "missing");
    fs.writeFileSync(file, "{broken\n", { mode: 0o600 });
    const invalid = readConfig({ ZODEX_CONFIG_FILE: file });
    assert.equal(invalid.state, "invalid");
    assert.equal("contents" in invalid, false);
    fs.writeFileSync(file, `${JSON.stringify(config(), null, 2)}\n`, { mode: 0o600 });
    assert.equal(readConfig({ ZODEX_CONFIG_FILE: file }).state, "valid");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("config reader refuses group-readable files", { skip: process.platform === "win32" }, () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-config-mode-test-"));
  const file = path.join(root, "config.json");
  try {
    fs.writeFileSync(file, `${JSON.stringify(config())}\n`, { mode: 0o644 });
    const result = readConfig({ ZODEX_CONFIG_FILE: file });
    assert.equal(result.state, "invalid");
    assert.match(result.error, /permissions must be 0600/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("remote-control modules are typed, off by default, and capability-gated", () => {
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
  const unavailable = planRemoteControlModules([manifest], { enabled: true, extensions: [manifest.id] });
  assert.deepEqual(unavailable.modules[0].missing, ["remote-control.status:read"]);
  const ready = planRemoteControlModules(
    [manifest],
    { enabled: true, extensions: [manifest.id, "not-installed"] },
    ["remote-control.status:read"],
  );
  assert.equal(ready.modules[0].state, "ready");
  assert.deepEqual(ready.unknown, ["not-installed"]);
  assert.throws(
    () => validateModuleManifest({ ...manifest, defaultEnabled: true }),
    /disabled by default/,
  );
});

test("feature patches expose a read-only status bridge and warning runtime", () => {
  const main = applyMainBundlePatch('"use strict";const handlers={"native-desktop-apps":async()=>{}};');
  assert.match(main, /"zodex-module-status":async/);
  assert.match(main, /CODEX_LINUX_FEATURES_DIR is unavailable/);
  assert.equal(applyMainBundlePatch(main), main);

  const webview = applyWebviewRuntimePatch("const app=true;\n");
  assert.match(webview, /zodexModuleWarningVersion/);
  assert.match(webview, /role`,\`alertdialog/);
  assert.match(webview, /warning-one/);
  assert.match(webview, /third-party OAuth active/);
  assert.equal(applyWebviewRuntimePatch(webview), webview);
  assert.deepEqual(descriptors.map(({ id }) => id), ["module-status-handler", "oauth-warning-runtime"]);
});

test("feature is disabled by default and stages only its public validator and example", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zodex-feature-test-"));
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
