# Remote Approvals (Synthetic Fixture & ASR-LIVE Harness)

This disabled-by-default feature contains a pure, offline fixture for the
request shapes used by the planned Remote Approvals beta. It normalizes
command, file-change, permissions, and enum/boolean tool-input requests and
models deterministic first-valid-response arbitration, resolution, expiry,
cancellation, and disconnect handling.

The fixture is deliberately not a Desktop patch, transport, relay, broker, or
secret-input implementation. It has no live ChatGPT, Remote, authentication,
network, quota, or generated-bundle dependency. Free-form input, passwords,
malformed requests, stale requests, duplicates, cross-thread responses, and
unknown request methods fail closed.

## Task ASR-LIVE Non-Live Preparation

For Task **ASR-LIVE**, this feature provides a disposable compatibility harness
(`asr-live-harness.js`) and documented runbook ([docs/asr-live-runbook.md](../../docs/asr-live-runbook.md)).

The harness uses a synthetic `native-remote` client alongside a second
`proxy-client-2` to model the live compatibility scenarios that Task ASR-LIVE
needs to prove. It does not enable, launch, or exercise the real live gate.

### Invariants & Protection

- **Default Dry-Run**: Runs in dry-run mode using synthetic requests.
- **Live Gate Protection**: Environment gate (`ALLOW_LIVE_ASR_EXECUTION=1` or `ZODEX_ASR_LIVE_GATE=1`) is strictly **refused** during this non-live preparation task (`LIVE_EXECUTION_REFUSED`).
- **Isolated Paths**: Isolates `CODEX_HOME` and runtime sockets under temporary directories (`0700`).
- **Sanitized Output**: Redacts tokens, JWTs, keys, and credentials from transcripts.
- **Zero Native Auth**: Never inspects or reads native auth files.

Run tests with:

```bash
node --test linux-features/remote-approvals/test.js
node linux-features/remote-approvals/asr-live-harness.js
```

Use the runbook for task context, safety rules, and the exact unexecuted live
command:

```bash
cat docs/asr-live-runbook.md
```
