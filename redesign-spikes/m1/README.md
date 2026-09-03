# M1 disposable feasibility spikes

These tests use temporary state, synthetic content, inert processes, and the owner-supplied native Codex binary. They must not read or copy native authentication state, start a model turn, modify live catalogs/configuration, install packages, publish artifacts, or restart Desktop.

Run the App Server spike with:

```sh
node --test redesign-spikes/m1/app-server.test.js
node --test redesign-spikes/m1/tmux.test.js
node --test redesign-spikes/m1/storage.test.js
```

`ZODEX_NATIVE_CODEX` may point to another owner-supplied compatible binary. The test always replaces `CODEX_HOME` with a private temporary directory.
