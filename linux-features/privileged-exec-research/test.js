#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const featureDir = __dirname;
const repoRoot = path.resolve(featureDir, "../..");
const { enabledLinuxFeatureStageHooks } = require("../../scripts/lib/linux-features.js");

function temp(prefix) {
  return fs.mkdtempSync(path.join(os.tmpdir(), prefix));
}

function executable(file, contents) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, contents, { mode: 0o755 });
}

test("feature is disabled by default and stages no paths", () => {
  const root = temp("privileged-exec-feature-config-");
  const featuresRoot = path.join(root, "features");
  fs.mkdirSync(path.join(featuresRoot, "privileged-exec-research"), { recursive: true });
  for (const name of ["feature.json", "README.md", "stage.sh"]) {
    fs.copyFileSync(path.join(featureDir, name), path.join(featuresRoot, "privileged-exec-research", name));
  }
  fs.writeFileSync(path.join(featuresRoot, "features.example.json"), '{"enabled":[]}\n');

  assert.deepEqual(enabledLinuxFeatureStageHooks({ featuresRoot }), []);
  assert.equal(fs.existsSync(path.join(root, "install")), false);
});

test("enabled stage copies only the prebuilt backend and exact plugin paths", () => {
  const root = temp("privileged-exec-feature-stage-");
  const installDir = path.join(root, "install");
  const backend = path.join(root, "codex-privileged-exec-linux");
  const marketplace = path.join(
    installDir,
    "resources/plugins/openai-bundled/.agents/plugins/marketplace.json",
  );
  executable(backend, "#!/bin/sh\nexit 0\n");
  fs.mkdirSync(path.dirname(marketplace), { recursive: true });
  fs.writeFileSync(marketplace, JSON.stringify({ plugins: [{ name: "computer-use" }] }));

  execFileSync("bash", [path.join(featureDir, "stage.sh")], {
    cwd: repoRoot,
    env: {
      ...process.env,
      SCRIPT_DIR: repoRoot,
      INSTALL_DIR: installDir,
      CODEX_PRIVILEGED_EXEC_RESEARCH_SOURCE: backend,
    },
    stdio: "pipe",
  });

  const pluginDir = path.join(
    installDir,
    "resources/plugins/openai-bundled/plugins/privileged-exec-research",
  );
  assert.equal(fs.existsSync(path.join(pluginDir, ".codex-plugin/plugin.json")), true);
  assert.equal(fs.existsSync(path.join(pluginDir, ".mcp.json")), true);
  assert.equal(fs.existsSync(path.join(pluginDir, "bin/codex-privileged-exec-linux")), true);
  assert.equal(fs.existsSync(path.join(installDir, ".codex-linux")), false);
  const entries = JSON.parse(fs.readFileSync(marketplace, "utf8")).plugins;
  assert.equal(entries.some((entry) => entry.name === "privileged-exec-research"), true);
});

test("staging consumes an executable artifact and never invokes Cargo or forbidden surfaces", () => {
  const stage = fs.readFileSync(path.join(featureDir, "stage.sh"), "utf8");
  assert.doesNotMatch(stage, /cargo\s+(?:build|install|run)/);
  assert.match(stage, /codex-privileged-exec-linux/);
  assert.doesNotMatch(stage, /interactive-console|zodex-modules/);
  assert.equal(path.resolve(featureDir), path.resolve(repoRoot, "linux-features/privileged-exec-research"));
});
