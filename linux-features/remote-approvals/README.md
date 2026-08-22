# Remote Approvals (Synthetic Fixture)

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

Multi-client request delivery and response ownership remain a live
compatibility gate. Passing this synthetic fixture does not establish that
native ChatGPT Remote and another client can safely observe or resolve the
same request.
