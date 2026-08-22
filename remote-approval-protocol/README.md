# Remote approval protocol contract (v1)

This directory is the normative, implementation-facing contract for the
disabled-by-default Zodex Remote Approvals public beta.  It defines messages
and safety properties; it intentionally does **not** implement cryptography,
networking, browser storage, or process execution.

## Scope and hard exclusions

The beta mirrors only upstream structured command, file-change, and permission
approval requests, plus boolean or closed-enum `requestUserInput` requests.
It never carries a password, API key, OAuth code, free-form text, terminal
stdin, an arbitrary terminal prompt, or an `other` response.  A decoder MUST
fail closed for unknown request kinds, unknown response schemas, an unexpected
field, a malformed or stale envelope, or a request whose exact bytes cannot be
bound to its action digest.

Remote sudo-password entry, arbitrary terminal interaction, prompt scraping,
and shell command parsing are separate research work and are not compatible
with this contract or public-beta packaging.

## Canonical data and cryptographic binding

All protocol JSON is canonicalized with RFC 8785 JCS (UTF-8, no whitespace).
`action_digest` is lowercase hexadecimal SHA-256 of the JCS `action` object.
`request_digest` is lowercase hexadecimal SHA-256 of the JCS request object
with `host_signature` omitted.  `response_digest` is defined analogously with
`device_signature` omitted.  Comparing a reserialized object or a display
string is forbidden: acceptance uses the transmitted canonical action bytes
and `action_digest` only.

The host signs `request_digest` using ECDSA P-256 with SHA-256.  Signatures are
the fixed-width 64-byte `r || s` form, encoded base64url without padding.
Responses are signed the same way by the paired device key and include a
WebAuthn assertion demonstrating user verification (UV).  Verifiers MUST
validate `clientDataJSON.challenge` as `base64url(SHA-256(response_digest))`,
the RP ID/origin, authenticator-data UV flag, credential-to-device binding, and
signature before considering the response valid.

Requests and responses are encrypted end-to-end using RFC 9180
`DHKEM(P-256, HKDF-SHA256) / HKDF-SHA256 / AES-256-GCM`.  Implementations MUST
use an audited HPKE library and the suite's standard framing; they MUST NOT
invent AES/ECDH framing.  The blind relay receives only `RelayFrame` objects
and cannot decrypt their `ciphertext`.  The schemas describe unencrypted inner
messages and relay wrappers separately for testability.

## Request lifecycle

1. A desktop host accepts one eligible upstream request and creates a fresh
   UUID request ID, at least 128-bit nonce, action digest, issue time, and an
   absolute `expires_at` exactly no later than five minutes after issue.
2. The host signs and HPKE-encrypts that request only to the currently active
   device's current encryption key.  Paired inactive devices never receive a
   decryptable request.
3. The relay stores opaque ciphertext under the request ID until acknowledged,
   cancelled, or expired.  URLs and push payloads contain only the opaque
   request ID; neither contains a bearer token, command, path, title, action
   digest, or ciphertext.
4. The PWA decrypts, rechecks the request signature and expiry, displays the
   exact action, and requests WebAuthn UV.  A tap or push receipt never
   approves anything.
5. The host accepts the first response that is syntactically valid, signed by
   the active device, WebAuthn-UV verified, action-bound, unexpired, and still
   pending.  It atomically marks the request consumed before resolving the
   upstream request.  All later responses, even identical ones, fail with
   `response_replayed`.
6. Cancellation, expiry, revocation, epoch mismatch, transport ambiguity, or
   upstream resolution by another client resolves the remote request as failed
   closed and deletes queued relay ciphertext.

Clock policy: the host is authoritative for expiry.  A receiver may allow at
most 30 seconds of local clock skew when deciding whether to display a request,
but the host never accepts a response at or after `expires_at`.

## Device pairing, activation, rotation, and revocation

Pairing begins locally on desktop with a single-use, high-entropy QR bootstrap
that expires in 60 seconds and is never logged or stored after completion. The
desktop and PWA show a short authentication string (SAS) derived from the
authenticated transcript; the owner must compare it on both devices. Pairing
records bind a stable `device_id`, signing public key, HPKE public key,
WebAuthn credential ID, allowed RP ID/origin, and lifecycle state.

Many devices may be paired, but exactly one is `active`. Only a local desktop
user-presence action may activate, deactivate, revoke, or rotate a device key.
Each activation/revocation/rotation increments the monotonically increasing
`active_device_epoch`. A response with a different epoch is invalid even if its
key was previously active. Rotation keeps no acceptance overlap: old keys are
revoked before the epoch is published. Lost-device recovery requires local
desktop revocation and a new QR+SAS pairing; it never accepts a remote recovery
message.

Private keys belong in the OS keyring on desktop and non-extractable WebCrypto
storage where the PWA platform supports it. Configuration stores only flags,
acknowledgement versions, compiled relay profile IDs, and public device IDs.
It stores no password, private key, relay bearer token, native ChatGPT auth, or
pairing bootstrap material.

## Relay, transport, limits, and audit

The transport is outbound TLS/WSS from desktop and PWA. There is no public
inbound listener. A relay implementation exposes exactly `connect`, `publish`,
`subscribe`, `ack`, `pushWakeup`, and `health`; all other operations are
protocol errors. A concrete relay profile is compiled, reviewed, and selected
by ID—not supplied as an arbitrary runtime URL.

The relay accepts only opaque frames up to 64 KiB ciphertext and 2 KiB routing
metadata, with one outstanding request per thread/action and at most 32 pending
requests per active device. It rate limits each authenticated connection to 60
frames/minute and uses a maximum ciphertext retention of five minutes. It
deletes ciphertext on acknowledgement, cancellation, or expiry. Push payloads
are generic, e.g. `"A Zodex approval is ready"`, with no sensitive metadata.

Audit records are metadata-only: event name, request/device IDs, epochs,
timestamps, outcome/error code, request kind, relay profile ID, and truncated
cryptographic key IDs. Never record action text, paths, command arguments,
ciphertext, URLs, WebAuthn blobs, passwords, or full hashes. Audit log export
is local-only and must preserve the same redaction.

## Browser residual risk

WebAuthn and WebCrypto reduce key exposure but cannot guarantee zero retention
of displayed action data or JavaScript strings. Browsers, keyboard IMEs,
accessibility services, memory garbage collection, notifications, screenshots,
and crash reporting remain residual risks. The PWA must disclose these risks,
use CSP with no third-party scripts, and avoid storing decrypted request data
in history, IndexedDB, service-worker caches, logs, analytics, or push data.

## State machines and errors

Host request states are `pending_approval`, `delivered`, `awaiting_response`,
`accepted`, `rejected`, `cancelled`, `expired`, `resolved_elsewhere`, and
`failed`. Only the first three are pending. Terminal states are immutable and
delete relay ciphertext. A device states `pairing`, `paired_inactive`,
`active`, `revoked`, or `rotated`; only `active` may respond.

`errors.md` names all wire-visible failure codes. Every validation failure is
terminal for that submitted response and is fail closed for upstream resolution
unless an independent valid response remains pending before expiry.

## Schema and fixture usage

`schemas/` uses JSON Schema Draft 2020-12. Schema validation is necessary but
not sufficient: implementations must also enforce cryptographic verification,
JCS digests, state transitions, timestamp arithmetic, and local-user-presence
requirements described here. `fixtures/` are deterministic non-secret shape
examples; placeholder keys/signatures/ciphertexts are not cryptographically
valid test vectors and MUST never be accepted by production verification.
