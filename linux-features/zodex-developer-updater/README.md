# Zodex Developer Updater

This Arch-only feature is disabled by default. It provides the namespaced
`zodex-update-manager` used by the Zodex config watcher after the owner has
explicitly enabled and acknowledged developer updates in `config.json`.

The manager snapshots the configured local checkout, including intentional
uncommitted edits, into its private cache. It never fetches, pulls, resets, or
edits that checkout. A candidate is built only when the snapshot fingerprint
changes; installation waits for Zodex to exit and then requests Polkit through
the normal `pacman -U` path. It never stops or relaunches Zodex.

Provision an installed feature with:

```sh
zodex-update-manager developer configure --checkout /absolute/path/to/zodex
```

State is kept under the `zodex-update-manager` XDG namespaces. The manager
never reads native ChatGPT authentication material.
