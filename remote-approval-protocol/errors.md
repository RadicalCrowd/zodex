# Wire-visible errors

| Code | Required behavior |
| --- | --- |
| `unsupported_protocol` | Reject without forwarding or retrying with a different version. |
| `unsupported_request_kind` | Fail closed; never downgrade to free-form input. |
| `invalid_schema` | Reject the malformed message and audit only the code. |
| `invalid_canonicalization` | Reject; do not hash a locally reconstructed action. |
| `invalid_action_digest` | Reject and retain no display/action copy. |
| `invalid_signature` | Reject the envelope and rate-limit the sender. |
| `invalid_webauthn` | Reject; no user verification means no response. |
| `request_expired` | Delete queued ciphertext and resolve pending upstream approval as unavailable. |
| `request_cancelled` | Delete queued ciphertext; do not permit resurrection. |
| `response_replayed` | Reject after atomic first-response consumption. |
| `inactive_device` | Reject a response from a paired but non-active device. |
| `stale_device_epoch` | Reject after activation, rotation, or revocation changes the epoch. |
| `device_revoked` | Reject and delete pending ciphertext encrypted for that device. |
| `relay_limit_exceeded` | Fail closed for the affected request; never silently drop an approval. |
| `relay_unavailable` | Keep pending only until absolute expiry; do not extend the TTL. |
| `transport_ambiguous` | Do not retry a resolution action unless its receipt state is proven. |
| `resolved_elsewhere` | Stop remote processing when upstream ownership resolves the request. |
| `key_rotation_required` | Require local desktop rotation; do not accept remote key replacement. |
| `local_presence_required` | Reject remote activation, revocation, recovery, and profile changes. |
