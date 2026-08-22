# Codex Remote Approvals Linux protocol core

This crate is the native, fail-closed implementation of the locked
`remote-approval-protocol` v1 contract.  It deliberately contains no relay,
socket, keyring, browser, WebAuthn assertion-verification, or process-execution
logic.  Those components must call its typed validators before acting on a
request or response.

## Cryptographic dependencies

- [`serde_jcs`](https://crates.io/crates/serde_jcs) performs RFC 8785 JSON
  Canonicalization Scheme serialization.  Protocol digests are SHA-256 over
  those canonical UTF-8 bytes.
- [`p256`](https://crates.io/crates/p256) provides audited RustCrypto P-256
  ECDSA keys and fixed-width IEEE P1363 `r || s` signatures.  The crate uses
  prehash signing/verification so the protocol's SHA-256 digest is the ECDSA
  prehash rather than implementing ECDSA itself.
- [`hpke`](https://crates.io/crates/hpke) implements RFC 9180 base mode using
  `DHKEM(P-256, HKDF-SHA256) / HKDF-SHA256 / AES-256-GCM`.  This crate uses its
  standard encapsulation framing and does not construct AES, ECDH, HKDF, or
  nonce framing itself.

## Residual assumptions

Cryptographic acceptance requires trusted caller-provided public keys and a
separate WebAuthn verifier.  `validate_response` checks the response's typed
binding, device identity, epoch, state, and device ECDSA signature; callers
must verify the WebAuthn assertion, its RP ID/origin, UV flag, and credential
binding before resolving an upstream action.  System key storage, transport
authentication, relay limits, and atomic persistence of `RequestLifecycle`
are broker responsibilities.

## Broker Responsibilities

Higher-level broker implementations must fulfill seven non-delegable security responsibilities:

1. **WebAuthn Verification**: Validate WebAuthn `clientDataJSON.challenge` (matching `base64url(SHA-256(response_digest))`), RP ID/origin, User Verification (UV) flag, and credential-to-device binding before accepting an upstream response.
2. **Keyring Storage**: Secure private signing and decryption keys in the OS keyring on desktop or non-extractable WebCrypto storage in PWAs.
3. **TLS/WSS Transport**: Manage outbound TLS/WSS network connections framing without listening on public inbound ports.
4. **Relay Frame & Rate Limits**: Enforce 64 KiB maximum ciphertext size, 2 KiB metadata size, max 32 pending requests per active device (1 per action/thread), 60 frames/min rate limit per connection, and 5 minute max ciphertext retention.
5. **Atomic Persistence**: Track single-use lifecycle state transitions (`RequestLifecycle`) and persist state atomically to prevent replay or race conditions.
6. **Fail-Closed Upstream Resolution & Ciphertext Deletion**: Immediately resolve upstream requests as failed and delete queued relay ciphertext on error, expiry, cancellation, or revocation.
7. **Metadata-Only Audit Logging**: Log only safe metadata (event names, request/device IDs, epochs, timestamps, outcome/error codes, request kinds, profile IDs, truncated key IDs) and never raw action text, file paths, command arguments, ciphertexts, URLs, WebAuthn blobs, passwords, or full hashes.

The selected `hpke` release publishes RFC 9180 known-answer tests, but its
upstream documentation notes that it has not received a formal independent
audit.  This is a tracked library-assurance assumption: upgrades require a
security review, pinned-version test run, and review of the upstream advisory
history.  The crate does not claim the dependency is formally audited.

The JSON schemas are normative for wire shape.  The checked-in fixtures are
deliberately non-secret shape examples and contain placeholder signatures and
digests; they are useful only for parsing and must never verify.
