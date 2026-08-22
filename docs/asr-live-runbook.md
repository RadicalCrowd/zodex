# Zodex Task ASR-LIVE: Multi-Client Remote Approvals Compatibility Runbook

## 1. Overview
Task ASR-LIVE prepares and validates multi-client compatibility for Zodex Remote Approvals. Specifically, it tests whether a second stock app-server proxy client attached to Desktop's shared socket (shared-app-server-socket) can observe and resolve structured approval requests while native ChatGPT Remote (remote-mobile-control) remains connected and active.

During this non-live preparation task, execution is strictly constrained to offline dry-run simulation using isolated temporary paths, synthetic requests, and zero native auth access.

## 2. Architecture & Invariants
- Shared Socket Authority: Desktop owns the app-server Unix socket (shared-app-server-socket).
- Native ChatGPT Remote: Connected host session via remote-mobile-control.
- Second Proxy Client: Additional client attached via codex app-server proxy --sock socket_path.
- Dry-Run Default: The harness defaults to offline dry-run mode.
- Live Execution Gate Refusal: Live execution requires ALLOW_LIVE_ASR_EXECUTION=1 or ZODEX_ASR_LIVE_GATE=1. In this preparation task, any live gate execution is refused and throws LIVE_EXECUTION_REFUSED.
- Isolated Runtime Paths: Harness instances use temporary CODEX_HOME and XDG_RUNTIME_DIR paths (mode 0700) created via fs.mkdtempSync.
- Transcript Sanitization: All logged transcripts and JSON outputs automatically redact tokens, JWTs, private keys, secrets, and auth headers.
- Zero Native Auth Inspection: The harness never reads or accesses native ChatGPT authentication files (~/.config/Codex/auth.json).

## 3. Scenario Test Matrix
1. delivery: Request delivery to all connected clients (both native-remote and proxy-client-2).
2. response_ownership: Response arbitration by responder identity (delivered client succeeds, undelivered client fails).
3. duplicate_response: First valid response resolves request; second response fails with already-resolved.
4. serverRequest/resolved: Resolution notification broadcast to all observing clients.
5. disconnect: Disconnecting one client leaves request pending for remaining clients; request cancels if 0 clients remain.
6. expiry: 5-minute TTL boundary handling (requests expire after 5 minutes; late responses return expired).
7. remote_continuity: Native Remote connection remains active and unaffected throughout proxy client lifecycle.

## 4. Execution Instructions
Run offline test suite:
node --test linux-features/remote-approvals/test.js

Run harness in dry-run mode:
node linux-features/remote-approvals/asr-live-harness.js

Verify live gate protection:
ALLOW_LIVE_ASR_EXECUTION=1 node linux-features/remote-approvals/asr-live-harness.js
# Refuses live execution with LIVE_EXECUTION_REFUSED.

## 5. Controlled Future Live Verification Procedure
When live testing is authorized in a controlled environment outside this preparation task:
1. Ensure a live Desktop instance is running with shared-app-server-socket enabled.
2. Verify native ChatGPT Remote is connected (remote-mobile-control).
3. Connect codex app-server proxy --sock socket_path from a second terminal.
4. Trigger an upstream approval request and confirm dual delivery, first response resolution, and Remote stream continuity.
