"use strict";

const { spawn } = require("node:child_process");
const { EventEmitter } = require("node:events");

const SUDO_PROMPT_PATTERN = /(\[sudo\]\s+password\s+for\s+\w+:|Password:)\s*$/i;
const CONFIRMATION_PATTERN = /(\(y\/n\)|\[y\/N\]|\[Y\/n\])\s*$/i;
const MAX_HISTORY_BYTES = 512 * 1024; // 512KB scrollback buffer

class PtySession extends EventEmitter {
  constructor({ id, command, args = [], cwd = process.cwd(), env = process.env }) {
    super();
    this.id = id || `term_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 7)}`;
    this.command = command;
    this.args = args;
    this.cwd = cwd;
    this.status = "starting"; // starting, running, waiting_for_input, completed, failed
    this.inputPromptType = null; // "sudo", "confirm", "text"
    this.exitCode = null;
    this.outputBuffer = "";
    this.child = null;
    this.startedAt = Date.now();
    this.completedAt = null;
  }

  start() {
    try {
      this.child = spawn(this.command, this.args, {
        cwd: this.cwd,
        env: { ...process.env, TERM: "xterm-256color" },
        stdio: ["pipe", "pipe", "pipe"],
      });

      this.status = "running";
      this.emit("status", { status: this.status });

      const onData = (chunk) => {
        const text = chunk.toString("utf8");
        this.appendOutput(text);

        // Scan for interactive prompts
        if (SUDO_PROMPT_PATTERN.test(text.trimEnd())) {
          this.status = "waiting_for_input";
          this.inputPromptType = "sudo";
          this.emit("waiting_for_input", { type: "sudo", text });
        } else if (CONFIRMATION_PATTERN.test(text.trimEnd())) {
          this.status = "waiting_for_input";
          this.inputPromptType = "confirm";
          this.emit("waiting_for_input", { type: "confirm", text });
        } else if (this.status === "waiting_for_input") {
          this.status = "running";
          this.inputPromptType = null;
          this.emit("status", { status: this.status });
        }
      };

      this.child.stdout?.on("data", onData);
      this.child.stderr?.on("data", onData);

      this.child.on("close", (code) => {
        this.exitCode = code;
        this.completedAt = Date.now();
        this.status = code === 0 ? "completed" : "failed";
        this.emit("close", { exitCode: code, status: this.status });
      });

      this.child.on("error", (err) => {
        this.appendOutput(`\n[Process Error: ${err.message}]\n`);
        this.status = "failed";
        this.emit("error", err);
      });

      return true;
    } catch (err) {
      this.status = "failed";
      this.appendOutput(`\n[Spawn Failure: ${err.message}]\n`);
      this.emit("error", err);
      return false;
    }
  }

  appendOutput(text) {
    this.outputBuffer += text;
    if (this.outputBuffer.length > MAX_HISTORY_BYTES) {
      this.outputBuffer = this.outputBuffer.slice(-MAX_HISTORY_BYTES);
    }
    this.emit("data", text);
  }

  writeInput(input, { isPassword = false } = {}) {
    if (!this.child || !this.child.stdin || this.child.stdin.destroyed) {
      return false;
    }
    try {
      const inputBuffer = Buffer.from(String(input) + "\n", "utf8");
      if (isPassword || this.inputPromptType === "sudo") {
        this.child.stdin.write(inputBuffer, () => {
          inputBuffer.fill(0);
        });
      } else {
        this.child.stdin.write(inputBuffer);
        this.appendOutput(`${input}\n`);
      }

      this.status = "running";
      this.inputPromptType = null;
      this.emit("status", { status: this.status });
      return true;
    } catch (err) {
      this.emit("error", err);
      return false;
    }
  }

  kill(signal = "SIGTERM") {
    if (this.child && !this.child.killed) {
      this.child.kill(signal);
      return true;
    }
    return false;
  }
}

class PtyManager {
  constructor() {
    this.sessions = new Map();
  }

  createSession(options) {
    const session = new PtySession(options);
    this.sessions.set(session.id, session);
    session.on("close", () => {
      // Keep up to 20 recent sessions in memory
      if (this.sessions.size > 20) {
        const oldestId = this.sessions.keys().next().value;
        this.sessions.delete(oldestId);
      }
    });
    session.start();
    return session;
  }

  getSession(id) {
    return this.sessions.get(id) || null;
  }

  listSessions() {
    return Array.from(this.sessions.values()).map((s) => ({
      id: s.id,
      command: s.command,
      args: s.args,
      status: s.status,
      inputPromptType: s.inputPromptType,
      startedAt: s.startedAt,
      completedAt: s.completedAt,
      exitCode: s.exitCode,
    }));
  }

  writeInput(id, input, options) {
    const session = this.getSession(id);
    if (!session) return false;
    return session.writeInput(input, options);
  }

  killSession(id, signal) {
    const session = this.getSession(id);
    if (!session) return false;
    return session.kill(signal);
  }
}

module.exports = {
  CONFIRMATION_PATTERN,
  PtyManager,
  PtySession,
  SUDO_PROMPT_PATTERN,
};
