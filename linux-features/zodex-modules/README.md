# Zodex Modules

This optional feature adds the Zodex module boundary and config-file-only OAuth
broker acknowledgements. It is disabled by default and never stores an API key,
OAuth token, password, or other credential.

Enable the build-time feature in the git-ignored `linux-features/features.json`:

```json
{
  "enabled": ["zodex-modules"]
}
```

Then copy the staged example to `~/.config/zodex/config.json`. The runtime also
accepts an explicit `ZODEX_CONFIG_FILE` for isolated testing. Credentials do not
belong in this JSON; secret-like fields are rejected.

Keep the file private:

```bash
chmod 600 ~/.config/zodex/config.json
```

The file must be a regular file owned by the current user. Unknown brokers and
providers fail closed; this phase allows OmniRoute only for `anthropic` and
`google`, while `opencode-community` remains an empty audited allowlist.

OAuth brokers and provider connections remain off by default. To request one,
set both its broker and connection `enabled` fields to `true`. On the next app
refresh Zodex shows warning 1. Add `zodex-oauth-risk-v1` to that connection's
`acknowledgements` array after accepting it. Refresh again for warning 2, then
add `zodex-oauth-effects-v1` to activate the connection on the following
refresh. The app has no button that bypasses these file-based steps.

The remote-control extension contract is intentionally narrow. JSON can select
a packaged module but cannot load an arbitrary script. Every module manifest
must use API version 1, be disabled by default, and request only declared
remote-control capabilities. This phase ships the host contract and tests, not
an additional remote-control module.

Test with:

```bash
node --test linux-features/zodex-modules/test.js
```

Known risks:

- A third-party OAuth broker can see account tokens and routed prompt data.
- Provider policy can differ from technical OAuth compatibility.
- Renderer bundle names can drift when the upstream DMG changes; the patch is
  optional and fails without modifying an unmatched asset.
