# Overnight Execution Report

Date: 2026-08-16

## Completed Deliverables

- Updated `feature/zodex-foundation` from the obsolete macOS-DMG patch pipeline
  to upstream commit `e6b51d9`, which builds from OpenAI's signed official Linux
  package and leaves the official ASAR unchanged unless an optional feature is
  enabled.
- Revalidated the disabled-by-default `zodex-modules` feature against official
  `chatgpt` package `26.810.52044` (`amd64`, SHA-256
  `708a15a1bb76e2bb7f0e376e5145391fa277ad3a64057c1d32537bdc2a1b4e6e`).
  Both feature descriptors applied successfully.
- Built a side-by-side candidate with app ID `zodex`, display name `Zodex`, and
  only `zodex-modules` enabled. Nothing was installed or promoted to `/opt`.
- Built and inspected the Arch package
  `dist-next/zodex-pacman-safe/zodex-26.810.52044+zodex.2-1-x86_64.pkg.tar.zst`
  (SHA-256
  `962d28ec230867f870b03587b9cafdc59f2085994075e8e7aa212f1e6661dc31`).
  It uses `/opt/zodex`, `/usr/bin/zodex`, `zodex.desktop`, the `zodex` app ID,
  and contains no update-manager payload.
- Fixed a side-by-side packaging bug: a custom no-updater package now targets
  its own `<package>-update-manager.service` during transition cleanup instead
  of stopping `codex-update-manager.service`. The existing `codex-desktop`
  behavior remains unchanged.
- Verified 54 focused Node tests, 51 repository smoke tests, and 49 Rust updater
  tests with zero failures. Shell syntax and `git diff --check` also passed.
- Confirmed the installed package remains
  `codex-desktop 2026.07.20.083133+3e5055c1-1`. No Codex configuration,
  ChatGPT authentication artifact, provider selection, service, or installed
  desktop file was changed.

## Assumptions Made

- The current upstream official-Linux-package architecture supersedes repair of
  the rejected August macOS-DMG patch anchors and is the safest supported base.
- Zodex should use the distinct app/package ID `zodex` and display name `Zodex`
  for local acceptance builds while retaining upstream attribution.
- Until the update manager, systemd unit, policy, and update state are fully
  namespaced, a Zodex acceptance package must be built with
  `PACKAGE_WITH_UPDATER=0`.
- Generated candidates and packages are validation artifacts only; package
  installation, live OAuth, live provider calls, and the final app restart are
  still owner-controlled gates.

## Errors & Follow-ups

- The old DMG candidate was correctly rejected when eight required compatibility
  patches drifted. Upstream has since retired that patch pipeline in favor of
  signed official Linux packages; no enforcement bypass was used.
- The host lacked `dpkg-deb`. A signed Arch `dpkg` package was unpacked only
  under `/tmp` for the build; nothing was installed system-wide.
- Docker was unavailable because its daemon was not running, so it was not used.
- Restricted-sandbox subprocess and DNS failures were rerun in the permitted
  host environment. The locked Rust dependencies then built successfully.
- The first updater-enabled Zodex package was rejected because it exposed the
  shared `codex-update-manager` paths. The first no-updater package was also
  rejected after inspection found it would stop that shared service. Neither
  package was installed; the latter defect is fixed in the accepted build.
- `makepkg` emitted one non-fatal `libfakeroot internal error: payload not
  recognized!` warning while still completing package checks and producing the
  archive. Review this warning before promotion.
- Before installation, add a distinct reviewed Zodex icon and either implement
  a fully namespaced updater/update channel or continue without the updater.
  Re-run package inspection and a side-by-side launch smoke test. The owner must
  perform the final application restart.
