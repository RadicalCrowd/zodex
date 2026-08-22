# Privileged Exec Research

**Research-only feature. Disabled by default. Not a public beta capability.**

This feature reserves a separate staging boundary for the future
`zodex_privileged_exec` MCP runner. The runner is intended to own only
structured processes that it launched itself. It must not inject input into
arbitrary agent-owned processes, provide a terminal, or act as a general shell
bridge.

The feature is intentionally isolated from the public `remote-approvals`
feature, `zodex-modules`, and the retired `interactive-console` prototype. It
does not add a renderer, Electron console, raw stdin UI, prompt matching,
password handling, PATH shim, shell command string, or privileged process
implementation. Those surfaces remain out of scope until a separate security
review and implementation milestone authorizes them.

## Safety boundary

- Accepts `{executable, argv, cwd, reason}` structured input only.
- Strict limit bounds are placed on request payloads.
- Validates executable and cwd using `fs::metadata` and rejects ambiguous paths containing control characters, line breaks, or directory traversal characters.
- Executes only the fully canonicalized absolute representation of paths directly.
- Does not expose arbitrary shell environments or stdin processing. Inherited environment is fully cleared.
- Requires standard regular executable file and directory configurations.
- Execution operates within a segregated process group (`process_group(0)`) utilizing checked `pid_t` conversions.
- Cancellation and 5-minute timeout hard-kill the whole pgid and return currently-read bounded output safely rather than dropping data.
- Structured execution payload securely binds the advisory `reason` field internally inside the computed SHA-256 invocation digest. Serialization errors on this digest bubble up deterministically.
- Errors during parsing, digest, initialization and process lifetime always fail-closed without panics.
- MCP interface logic is completely feature-gated to the `process-control` Linux system requirement.
- The feature remains disabled unless `privileged-exec-research` is explicitly
  listed in the Linux features configuration.
- Its future daemon and native key material must use a separate namespace from
  all public approval and ChatGPT authentication state.
- Configuration may contain flags, acknowledgement versions, and identifiers;
  it must never contain passwords, private keys, bearer tokens, or native
  ChatGPT authentication artifacts.
- No feature build may compile Cargo as part of update staging. Enabled builds
  consume a prebuilt, audited `codex-privileged-exec-linux` artifact only.
- The feature must never be promoted into public packages or represented as a
  supported remote-terminal or password-entry workflow without a new owner
  approval and independent security assessment.

## Enable only in an isolated research build

Add the feature to a disposable build's `linux-features/features.json`:

```json
{
  "enabled": ["privileged-exec-research"]
}
```

The staging hook expects an executable prebuilt backend at
`target/release/codex-privileged-exec-linux`. A research build can point to a
different audited artifact with
`CODEX_PRIVILEGED_EXEC_RESEARCH_SOURCE`. Updater paths reuse the packaged
artifact and never invoke Cargo.

This feature is not enabled in committed configuration and is not a promise
that the future runner exists yet.

## Validate the scaffold

```bash
node linux-features/privileged-exec-research/test.js
node --test scripts/lib/linux-features.test.js
```
