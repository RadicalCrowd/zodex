"use strict";

const childProcess = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const SOURCE_ACK = "zodex-developer-update-source-v1";
const INSTALL_ACK = "zodex-developer-update-install-v1";
const EXCLUDED_SOURCE_NAMES = new Set([".git", "node_modules", "target", "dist", "dist-next", "codex-app", "codex-app-next", ".venv"]);
const POLL_MS = 15_000;

function xdg(name, fallback) {
  const value = process.env[name] || path.join(os.homedir(), fallback);
  if (!path.isAbsolute(value)) throw new Error(`${name} must be an absolute path.`);
  return path.resolve(value);
}

function paths() {
  const configDir = path.join(xdg("XDG_CONFIG_HOME", ".config"), "zodex-update-manager");
  const stateDir = path.join(xdg("XDG_STATE_HOME", ".local/state"), "zodex-update-manager");
  const cacheDir = path.join(xdg("XDG_CACHE_HOME", ".cache"), "zodex-update-manager");
  return {
    configDir,
    stateDir,
    cacheDir,
    developer: path.join(configDir, "developer.json"),
    state: path.join(stateDir, "state.json"),
    lock: path.join(stateDir, "check.lock"),
    snapshots: path.join(cacheDir, "snapshots"),
    rollback: path.join(cacheDir, "rollback"),
  };
}

function assertNoSymlinkAncestors(target, label) {
  const resolved = path.resolve(target);
  const parts = resolved.split(path.sep).filter(Boolean);
  let current = path.parse(resolved).root;
  for (const part of parts) {
    current = path.join(current, part);
    const stat = fs.lstatSync(current, { throwIfNoEntry: false });
    if (stat?.isSymbolicLink()) throw new Error(`${label} must not use symbolic links.`);
  }
  return resolved;
}

function ensurePrivateDirectory(directory) {
  assertNoSymlinkAncestors(directory, "Manager directory");
  fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
  const stat = fs.lstatSync(directory);
  if (!stat.isDirectory() || stat.uid !== process.getuid() || (stat.mode & 0o077) !== 0) {
    throw new Error("Manager directory is not a private current-user directory.");
  }
}

function writePrivateJson(file, value) {
  ensurePrivateDirectory(path.dirname(file));
  const temporary = `${file}.tmp.${process.pid}`;
  fs.writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
  fs.chmodSync(temporary, 0o600);
  fs.renameSync(temporary, file);
}

function readPrivateJson(file, fallback) {
  if (!fs.existsSync(file)) return fallback;
  assertNoSymlinkAncestors(file, "Manager file");
  const stat = fs.lstatSync(file);
  if (!stat.isFile() || stat.uid !== process.getuid() || (stat.mode & 0o077) !== 0 || stat.size > 64 * 1024) {
    throw new Error(`Unsafe manager file: ${file}`);
  }
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function resolveConfigFile() {
  if (process.env.ZODEX_CONFIG_FILE) return assertNoSymlinkAncestors(process.env.ZODEX_CONFIG_FILE, "Zodex configuration");
  const configRoot = process.env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config");
  return assertNoSymlinkAncestors(path.join(configRoot, "zodex", "config.json"), "Zodex configuration");
}

function developerUpdatesActive() {
  const file = resolveConfigFile();
  if (!fs.existsSync(file)) return false;
  const stat = fs.lstatSync(file);
  if (!stat.isFile() || stat.uid !== process.getuid() || (stat.mode & 0o077) !== 0 || stat.size > 64 * 1024) {
    throw new Error("Zodex configuration is unsafe.");
  }
  const config = JSON.parse(fs.readFileSync(file, "utf8"));
  if (config == null || typeof config !== "object" || Array.isArray(config) || config.version !== 1) {
    throw new Error("Zodex configuration is invalid.");
  }
  const allowed = new Set(["version", "oauth", "remoteControl", "developerUpdates"]);
  if (Object.keys(config).some((key) => !allowed.has(key)) || config.oauth == null || typeof config.oauth !== "object" ||
      config.remoteControl == null || typeof config.remoteControl !== "object") {
    throw new Error("Zodex configuration is invalid.");
  }
  const updates = config.developerUpdates;
  return updates?.enabled === true && Array.isArray(updates.acknowledgements) &&
    updates.acknowledgements.includes(SOURCE_ACK) && updates.acknowledgements.includes(INSTALL_ACK);
}

function assertCheckout(checkout) {
  if (typeof checkout !== "string" || !path.isAbsolute(checkout)) throw new Error("Checkout must be an absolute path.");
  assertNoSymlinkAncestors(checkout, "Developer checkout");
  const resolved = fs.realpathSync(checkout);
  if (resolved !== path.resolve(checkout)) throw new Error("Developer checkout must not resolve through a symbolic link.");
  const stat = fs.lstatSync(resolved);
  if (!stat.isDirectory() || stat.uid !== process.getuid()) throw new Error("Checkout must be a current-user directory.");
  if (!fs.existsSync(path.join(resolved, "scripts", "build-zodex-arch-candidate.sh"))) {
    throw new Error("Checkout is not a Zodex source tree.");
  }
  return resolved;
}

function digestSnapshot(root) {
  const hash = crypto.createHash("sha256");
  function visit(directory, relative = "") {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      if (EXCLUDED_SOURCE_NAMES.has(entry.name)) continue;
      const absolute = path.join(directory, entry.name);
      const next = path.join(relative, entry.name);
      const stat = fs.lstatSync(absolute);
      if (stat.isSymbolicLink()) throw new Error(`Developer source contains a symlink: ${next}`);
      if (stat.uid !== process.getuid()) throw new Error(`Developer source contains a non-owner entry: ${next}`);
      if (stat.isDirectory()) {
        hash.update(`d\0${next}\0`);
        visit(absolute, next);
      } else if (stat.isFile()) {
        hash.update(`f\0${next}\0${stat.mode & 0o777}\0`);
        hash.update(fs.readFileSync(absolute));
      } else {
        throw new Error(`Developer source contains an unsupported entry: ${next}`);
      }
    }
  }
  visit(root);
  return hash.digest("hex");
}

function copySnapshot(source, destination) {
  function copyDirectory(input, output) {
    fs.mkdirSync(output, { recursive: true, mode: 0o700 });
    for (const entry of fs.readdirSync(input, { withFileTypes: true })) {
      if (EXCLUDED_SOURCE_NAMES.has(entry.name)) continue;
      const from = path.join(input, entry.name);
      const to = path.join(output, entry.name);
      const stat = fs.lstatSync(from);
      if (stat.isSymbolicLink()) throw new Error(`Developer source contains a symlink: ${entry.name}`);
      if (stat.uid !== process.getuid()) throw new Error(`Developer source contains a non-owner entry: ${entry.name}`);
      if (stat.isDirectory()) copyDirectory(from, to);
      else if (stat.isFile()) fs.copyFileSync(from, to, fs.constants.COPYFILE_EXCL);
      else throw new Error(`Developer source contains an unsupported entry: ${entry.name}`);
    }
  }
  copyDirectory(source, destination);
}

function acquireLock(file) {
  ensurePrivateDirectory(path.dirname(file));
  let descriptor;
  try { descriptor = fs.openSync(file, "wx", 0o600); } catch (error) {
    if (error.code === "EEXIST") return null;
    throw error;
  }
  return () => { fs.closeSync(descriptor); fs.unlinkSync(file); };
}

function packageIsolated(artifact) {
  const name = childProcess.spawnSync("pacman", ["-Qp", "--qf", "%n", artifact], { encoding: "utf8" });
  if (name.status !== 0 || name.stdout.trim() !== "zodex") throw new Error("Candidate package is not the Zodex package.");
  const listed = childProcess.spawnSync("pacman", ["-Qlp", artifact], { encoding: "utf8" });
  if (listed.status !== 0) throw new Error("Could not inspect candidate package paths.");
  const allowed = ["/opt/zodex/", "/usr/bin/zodex", "/usr/bin/zodex-update-manager", "/usr/lib/zodex/", "/usr/lib/systemd/user/zodex-update-manager.service", "/usr/share/applications/zodex.desktop", "/usr/share/icons/", "/etc/apparmor.d/zodex"];
  if (listed.stdout.split("\n").filter(Boolean).some((file) => !allowed.some((prefix) => file === prefix.slice(0, -1) || file.startsWith(prefix)))) {
    throw new Error("Candidate package contains a non-Zodex path.");
  }
}

function zodexRunning() {
  const result = childProcess.spawnSync("ps", ["-eo", "args="], { encoding: "utf8" });
  return result.status === 0 && result.stdout.split("\n").some((line) => line.includes("/opt/zodex/ChatGPT"));
}

function knownGoodPackage() {
  const version = childProcess.spawnSync("pacman", ["-Q", "zodex"], { encoding: "utf8" });
  if (version.status !== 0) return undefined;
  const [, installed] = version.stdout.trim().split(/\s+/, 2);
  const cache = "/var/cache/pacman/pkg";
  try { return fs.readdirSync(cache).map((name) => path.join(cache, name)).find((file) => path.basename(file).startsWith(`zodex-${installed}-`) && file.endsWith(".pkg.tar.zst")); } catch { return undefined; }
}

function retainKnownGoodPackage() {
  const source = knownGoodPackage();
  if (!source) return undefined;
  const managerPaths = paths();
  ensurePrivateDirectory(managerPaths.rollback);
  const target = path.join(managerPaths.rollback, path.basename(source));
  if (!fs.existsSync(target)) fs.copyFileSync(source, target, fs.constants.COPYFILE_EXCL);
  fs.chmodSync(target, 0o600);
  return target;
}

function installCandidate(state, automatic) {
  if (!state.candidate?.artifact || !fs.existsSync(state.candidate.artifact)) return { installed: false, reason: "no-candidate" };
  if (zodexRunning()) return { installed: false, reason: "app-running" };
  packageIsolated(state.candidate.artifact);
  const result = childProcess.spawnSync("pkexec", ["pacman", "-U", "--noconfirm", state.candidate.artifact], { stdio: "inherit" });
  if (result.status !== 0) throw new Error(automatic ? "Automatic Zodex installation was declined or failed." : "Zodex installation failed.");
  state.previousArtifact = state.knownGoodArtifact;
  state.installedSnapshot = state.candidate.snapshot;
  delete state.candidate;
  return { installed: true };
}

function configure(checkout) {
  const resolved = assertCheckout(checkout);
  const managerPaths = paths();
  writePrivateJson(managerPaths.developer, { version: 1, checkout: resolved, autoInstallOnExit: true });
  const service = childProcess.spawnSync("systemctl", ["--user", "enable", "--now", "zodex-update-manager.service"], { encoding: "utf8" });
  if (service.error || service.status !== 0) throw new Error("Could not enable the Zodex developer update manager service.");
  return { configured: true, checkout: resolved };
}

function checkNow() {
  if (!developerUpdatesActive()) return { checked: false, reason: "developer-updates-off" };
  const managerPaths = paths();
  const release = acquireLock(managerPaths.lock);
  if (!release) return { checked: false, reason: "already-checking" };
  try {
    const developer = readPrivateJson(managerPaths.developer, null);
    if (developer?.version !== 1) throw new Error("Developer source is not configured.");
    const checkout = assertCheckout(developer.checkout);
    const snapshot = digestSnapshot(checkout);
    const state = readPrivateJson(managerPaths.state, { version: 1 });
    if (state.candidate?.snapshot === snapshot || state.installedSnapshot === snapshot) return { checked: true, changed: false, snapshot };
    ensurePrivateDirectory(managerPaths.snapshots);
    const target = path.join(managerPaths.snapshots, snapshot);
    if (!fs.existsSync(target)) copySnapshot(checkout, target);
    const build = childProcess.spawnSync(path.join(target, "scripts", "build-zodex-arch-candidate.sh"), [], {
      cwd: target,
      env: { ...process.env, ZODEX_DEVELOPER_UPDATER: "1", ZODEX_SOURCE_SNAPSHOT_ID: snapshot.slice(0, 12) },
      stdio: "inherit",
    });
    if (build.status !== 0) throw new Error("Zodex developer candidate build failed.");
    const dist = path.join(target, "dist-next", "zodex-pacman");
    const artifact = fs.readdirSync(dist).filter((name) => name.endsWith(".pkg.tar.zst") && !name.includes("latest")).sort().at(-1);
    if (!artifact) throw new Error("Developer build produced no Zodex package.");
    const artifactPath = path.join(dist, artifact);
    packageIsolated(artifactPath);
    state.knownGoodArtifact = retainKnownGoodPackage();
    state.candidate = { snapshot, artifact: artifactPath, createdAt: new Date().toISOString() };
    writePrivateJson(managerPaths.state, state);
    return { checked: true, changed: true, snapshot, artifact: artifactPath };
  } finally { release(); }
}

function status() {
  return readPrivateJson(paths().state, { version: 1 });
}

function redactedState(value) {
  const state = { ...value };
  if (state.candidate?.artifact) {
    state.candidate = { ...state.candidate, artifact: path.basename(state.candidate.artifact) };
  }
  delete state.knownGoodArtifact;
  delete state.previousArtifact;
  return state;
}

function rollback() {
  const state = status();
  if (!state.previousArtifact || !fs.existsSync(state.previousArtifact)) throw new Error("No retained Zodex package is available for rollback.");
  if (zodexRunning()) throw new Error("Close Zodex before rolling back.");
  const result = childProcess.spawnSync("pkexec", ["pacman", "-U", "--noconfirm", state.previousArtifact], { stdio: "inherit" });
  if (result.status !== 0) throw new Error("Zodex rollback failed.");
  return { rolledBack: true };
}

async function daemon() {
  for (;;) {
    try {
      const state = status();
      if (state.candidate?.artifact && developerUpdatesActive() && !zodexRunning()) {
        const outcome = installCandidate(state, true);
        if (outcome.installed) writePrivateJson(paths().state, state);
      }
    } catch {
      // The candidate/state keeps the diagnostic surface; retry after the next owner action.
    }
    await new Promise((resolve) => setTimeout(resolve, POLL_MS));
  }
}

async function main(argv = process.argv.slice(2)) {
  const [command, ...rest] = argv;
  if (command === "developer" && rest[0] === "configure") {
    const index = rest.indexOf("--checkout");
    if (index === -1 || !rest[index + 1]) throw new Error("Usage: zodex-update-manager developer configure --checkout /absolute/path");
    configure(rest[index + 1]);
    return { configured: true };
  }
  if (command === "check-now") {
    const outcome = checkNow();
    return outcome.artifact ? { ...outcome, artifact: path.basename(outcome.artifact) } : outcome;
  }
  if (command === "status") return redactedState(status());
  if (command === "diagnose") return { state: redactedState(status()), configured: readPrivateJson(paths().developer, null) != null, appRunning: zodexRunning() };
  if (command === "install-ready") { const state = status(); const outcome = installCandidate(state, false); if (outcome.installed) writePrivateJson(paths().state, state); return outcome; }
  if (command === "daemon") return daemon();
  throw new Error("Usage: zodex-update-manager developer configure|check-now|status|diagnose|install-ready|rollback|daemon");
}

if (require.main === module) {
  main().then((result) => { if (result !== undefined) process.stdout.write(`${JSON.stringify(result)}\n`); }).catch((error) => { process.stderr.write(`${error.message}\n`); process.exitCode = 1; });
}

module.exports = { assertCheckout, checkNow, configure, digestSnapshot, packageIsolated, paths, redactedState, zodexRunning };
