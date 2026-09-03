# Zodex Core Local Production — GPT 5.6 Sol High Work Transfer

**Prepared:** 2026-08-17 (Asia/Kuala_Lumpur)  
**Project:** Zodex  
**Objective:** Finish Core Local Production  
**Handoff state:** CLP-01, CLP-02, and CLP-03 all finished with `PASS`
recommendations. The implementation is installed and functionally accepted. Formal
lead review fields were still `pending` in the agent status at handoff.

## 1. Delegated gate results

### CLP-01 — Uninstalled GUI acceptance: PASS

- Tested clean source commit
  `058cc11f2fb5ceafbdac85adde3a10913d5d9da2` on
  `feature/zodex-foundation`.
- The uninstalled candidate launcher passed all five diagnostics: `ChatGPT`,
  `app.asar`, `codex`, `rg`, and `codex-code-mode-host`.
- The existing `/opt/codex-desktop` instance was detected and the candidate was
  correctly held until the owner closed it. Zodex then ran alone and shut down
  cleanly with zero orphan Electron/ChatGPT processes.
- Native ChatGPT login remained active without a login prompt. Existing projects,
  threads, profiles, preferences, plugins/skills, permissions, and MCP
  configuration remained available.
- All 18 routed models were visible: 11 `kilo-free` and 7 `opencode-free`.
  Descriptions retained the exact `[kiloFree]` and `[OpencodeFree]` prefixes.
- Existing provider selection remained `kilo-free, opencode-free`. The default
  model was restored to `gpt-5.6-luna` after selection testing.
- Before/after checks showed zero drift in the recognized Router/Codex files.
  File modes remained `0600`; managed blocks stayed intact.
- Router doctor passed with Node `22.23.2`, authenticated sign-in, 18 matching
  catalog/routes, active Router `0.4.0-beta.3`, and zero relevant `FAIL` lines.
- Mobile remote-control endpoint and pairing initialized normally.
- No package installation, OAuth, provider request, quota use, or `auth.json`
  access occurred.

Recorded CLP-01 deviations:

- A dismissible startup modal reported:
  `Zodex configuration is invalid: CODEX_LINUX_FEATURES_DIR is unavailable`.
  The disabled `zodex-modules` hook is evaluating an unconfigured optional module
  directory. Closing the modal exposes the otherwise functional GUI.
- Thinking-view behavior was not exercised in C1 because it is model/event
  dependent.

### CLP-02 — Reversible side-by-side Arch installation: PASS

- Installed exact package:
  `/home/akif/Projects/zodex/dist-next/zodex-pacman-release/zodex-26.810.52044+zodex.058cc11f2fb5-1-x86_64.pkg.tar.zst`
- Verified package SHA-256:
  `bdf6658b9c6c303076eb31cd604495d628bfa6afb57eddd080e3dd9ab034fde4`
- Installed version:
  `zodex 26.810.52044+zodex.058cc11f2fb5-1`.
- The owner ran `sudo pacman -U` locally and entered sudo authentication only in
  the local terminal. No password was requested or relayed through chat.
- Zodex and the existing
  `codex-desktop 2026.07.20.083133+3e5055c1-1` now coexist in pacman.
- Package ownership comparison found zero non-directory file collisions: 4,828
  Zodex files versus 15,795 existing `codex-desktop` files.
- Zodex owns its independent namespace, including `/opt/zodex`, `/usr/bin/zodex`,
  `/usr/share/applications/zodex.desktop`, its icon, and its AppArmor profile.
- Package metadata correctly identifies Zodex and `RadicalCrowd/zodex`; it neither
  conflicts with nor replaces `codex-desktop`.
- No standalone Zodex updater binary, updater bundle, or updater service was
  installed. The existing `/usr/bin/codex-update-manager` remains owned only by
  `codex-desktop`.
- `zodex-omniroute.service` remained disabled and inactive.
- `codex-router.service` remained enabled/active. Router doctor passed with 18
  routes and zero `FAIL` lines.
- SHA-256 values for the five recognized Codex/Router configuration files were
  identical before and after installation; their modes remained `0600`.
- Zodex was intentionally not launched during this gate.
- Reversible uninstall command: `sudo pacman -Rns zodex`. It has not been run.

Recorded CLP-02 deviations:

- The first delegated background sudo attempt could not obtain an interactive
  TTY. It was stopped, and the owner executed the same command locally.
- CLP-01 still had `lead_review: pending` when CLP-02 started. The owner’s direct
  delegation was treated as authorization. This procedural exception must be
  acknowledged when formalizing the lead reviews.

### CLP-03 — Installed-app acceptance and rollback proof: PASS

- `/usr/bin/zodex` correctly launches `/opt/zodex/start.sh` and the packaged
  `/opt/zodex/ChatGPT` binary.
- Installed diagnostics again passed all five core assets.
- Two complete installed Zodex launch/close cycles passed. Both ended with zero
  orphan Electron/ChatGPT processes.
- Zodex branding is present in the system launcher, desktop entry, WMClass, dock
  icon, and application identity.
- Native ChatGPT authentication, projects, threads, profiles, settings,
  skills/plugins, MCP configuration, and model state persisted across both
  launches.
- All 18 routed models and their 11 `[kiloFree]` / 7 `[OpencodeFree]`
  descriptions remained intact.
- Mobile remote control initialized normally. Thinking/reasoning display rendered
  conditionally when supported provider events were available.
- Final Router doctor used Node `v22.23.2` and passed: authenticated sign-in,
  only `kilo-free` and `opencode-free` enabled, 18 exact routes, active Router
  `0.4.0-beta.3`, and zero `FAIL` lines.
- Default model `gpt-5.6-luna`, active profile `None`, and all 18 top-level config
  sections remained intact. Recognized config files remained mode `0600`.
- Rollback proof passed: after Zodex was fully closed, the owner launched
  `/usr/bin/codex-desktop`; its native login, projects, settings, profiles, and
  model catalog loaded correctly. It then closed cleanly with zero orphan
  processes.
- Both GUI applications were left closed for the owner’s final launch choice.
- No OAuth activation, live provider quota request, or `auth.json` access occurred.

Recorded CLP-03 deviations:

- The same dismissible `CODEX_LINUX_FEATURES_DIR is unavailable` modal appeared
  on installed Zodex startup.
- Internal proprietary web-bundle strings still show `Codex` in the top-left UI
  and `ChatGPT` as a dock/taskbar subtitle. This was an intentional preservation
  boundary: the project did not deep-patch proprietary upstream bundles merely to
  replace internal labels.

## 2. Current Core Local Production state

### Built and installed

- Public source fork: `RadicalCrowd/zodex`.
- Local source: `/home/akif/Projects/zodex`.
- Branch: `feature/zodex-foundation`.
- Accepted source commit:
  `058cc11f2fb5ceafbdac85adde3a10913d5d9da2`.
- Zodex is built from OpenAI’s signed official Linux package
  `chatgpt 26.810.52044`; the recorded upstream input SHA-256 is
  `708a15a1bb76e2bb7f0e376e5145391fa277ad3a64057c1d32537bdc2a1b4e6e`.
- Zodex has a distinct `zodex` app/package/desktop identity and a reviewed custom
  icon.
- The accepted Arch package is installed side-by-side with the existing community
  `codex-desktop`; neither package replaces the other.
- The first Zodex package is updater-free by design.

### Tested

- Earlier source validation included the full desktop Node suite, focused module
  tests, repository smoke tests, Rust updater tests, shell syntax, package
  inspection, and launcher diagnostics.
- The final metadata/package changes passed 12/12 focused packaging tests and
  51/51 repository smoke tests.
- C1–C3 added real acceptance evidence: uninstalled launch, exact package install,
  two installed launches, Router doctor, state preservation, process exclusivity,
  mobile remote control, and rollback launch of the old app.
- ChatGPT sign-in remained available throughout without reading authentication
  material.

### What remains

- Formal lead review has not yet changed CLP-01/02/03 from `pending` to
  `accepted`, even though every task finished with recommendation `pass`.
- The recurring `CODEX_LINUX_FEATURES_DIR` warning modal is the only observed
  Zodex-owned startup defect. It is dismissible but should be resolved before
  calling the experience polished production.
- Project status documents and the machine-readable workflow status still need
  their final accepted/complete updates.
- The owner still controls the final daily-use application launch/restart.
- Package signing, public binary distribution, OAuth Broker Preview, and the
  full remote-control module runtime are separate tracks and do not block Core
  Local Production.

## 3. Key context and decisions

### Architecture

- Zodex is a thin Arch-first desktop fork of
  `ilysenko/codex-desktop-linux`, which now rebuilds from OpenAI’s signed official
  Linux package rather than the rejected historical macOS-DMG patch path.
- Zodex Router remains the sole owner of routed Codex configuration. The desktop
  fork does not become a second Router/config owner.
- The installed Codex Desktop and Zodex share upstream user profile/state,
  including `~/.config/Codex`. They may coexist on disk but must never run
  concurrently.
- Native ChatGPT authentication is an explicit preservation boundary.
  `~/.codex/auth.json` must never be read, copied, hashed, migrated, printed, or
  modified.
- Existing routed providers are `kilo-free` and `opencode-free`; all 18 models and
  their description prefixes must be preserved.

### Packaging decisions

- Zodex uses `/opt/zodex`, `/usr/bin/zodex`, `zodex.desktop`, and Zodex-owned
  icon/AppArmor paths so it can coexist with `/opt/codex-desktop`.
- The package is intentionally updater-free because the inherited updater binary,
  service, locks, cache, and state would collide with `codex-desktop`. Any future
  updater must be fully namespaced and separately reviewed.
- The local package is unsigned. That is acceptable for current private/local
  production but not sufficient for a public signed release.
- Internal upstream `Codex`/`ChatGPT` strings were preserved rather than applying
  fragile deep patches to proprietary generated web bundles. The external Linux
  identity is Zodex.

### Optional features and broker decisions

- `zodex-modules` is intended to be optional and disabled by default. Its current
  missing-directory validation is responsible for the startup modal and should
  fail silently when the optional module path was never configured.
- OmniRoute is not part of the Core Local Production gate. Its installed user
  service remains disabled/inactive, no OAuth was performed, and no live provider
  quota was consumed.
- OAuth activation remains config-only, manual, warning-gated, and outside native
  ChatGPT authentication. Never ask the user to paste credentials into chat.

### Runtime dependencies

- Router development/doctor requires Node `22.23.2`, specifically
  `/home/akif/.nvm/versions/node/v22.23.2/bin`.
- Authoritative Router checkout:
  `/home/akif/.local/share/codex-router`.
- Authoritative doctor command from that checkout:
  `env PATH=/home/akif/.nvm/versions/node/v22.23.2/bin:/usr/local/sbin:/usr/local/bin:/usr/bin ./bin/model-router codex doctor`

## 4. Exact next steps for the next AI agent

1. **Treat the three reports as completed evidence and perform formal lead
   synthesis.** Record that CLP-01, CLP-02, and CLP-03 all recommend `PASS`.
   Explicitly acknowledge the CLP-02 sequencing deviation: the owner authorized
   installation before CLP-01’s lead-review field was updated.

2. **Decide the startup-modal release policy.** Recommended decision: treat the
   recurring `CODEX_LINUX_FEATURES_DIR is unavailable` modal as a small P1 Core
   polish defect, not as evidence that installation/login/routing failed.
   Do not hide it in the final verdict.

3. **Fix the optional-module startup modal if the owner wants polished Core Local
   Production before final declaration.** Work only in the isolated Zodex source
   branch and follow its `AGENTS.md`. The desired behavior is:
   - if `zodex-modules` or its module directory was never explicitly enabled or
     configured, missing `CODEX_LINUX_FEATURES_DIR` must not show an invalid-config
     modal;
   - explicitly enabled/configured invalid paths must still fail closed with a
     useful warning;
   - no arbitrary scripts, URLs, shell access, or secret-bearing config may be
     introduced.

4. **Add regression coverage for the modal fix.** At minimum cover disabled and
   unconfigured, explicitly enabled with a valid path, and explicitly enabled with
   a missing/invalid path. Run the adjacent `zodex-modules` tests, repository smoke
   tests, shell syntax checks, and `git diff --check`.

5. **If source changes, rebuild and replace the local candidate safely.** Use the
   existing dedicated updater-free Zodex Arch candidate builder, verify the new
   source commit/upstream input/package hash, inspect package identity and path
   isolation, then upgrade only `zodex`. Preserve `codex-desktop` and keep
   OmniRoute disabled/inactive.

6. **Repeat the minimum affected acceptance after any modal fix.** With all Codex
   Desktop processes closed, launch installed Zodex twice, verify no modal,
   preserved native login/projects/settings/models, 18 routed descriptions,
   remote control, and clean shutdown. Run Router doctor. Then close Zodex and
   prove `/usr/bin/codex-desktop` still launches as rollback. Leave both closed.

7. **If the modal is explicitly accepted as a known limitation instead of fixed,**
   do not rebuild. Record it prominently in the Core Local Production release
   notes and proceed using the already verified installed package/hash.

8. **Finalize workflow state and documentation.** Update the authoritative agent
   status so each `lead_review` is `accepted`, set the workflow/current gate to
   complete, check all Core C1–C3 items in the production roadmap, and update the
   progress log, README, MOC, validation/rollback document, artifact/version/hash,
   and known limitations. Do not claim public-release readiness.

9. **Run the final non-secret safety audit.** Confirm:
   - `zodex` and `codex-desktop` remain installed side-by-side;
   - only one may run at a time;
   - Router doctor has no relevant `FAIL`;
   - `kilo-free` and `opencode-free` remain enabled with 18 routes;
   - `[kiloFree]` and `[OpencodeFree]` descriptions remain unchanged;
   - `zodex-omniroute.service` is disabled/inactive;
   - no authentication artifact was accessed;
   - the rollback command remains `sudo pacman -Rns zodex`.

10. **Issue the final Core Local Production verdict.** If the modal was fixed and
    the regression passed, declare Zodex Core Local Production Ready. If the owner
    accepts the modal, declare it ready with that explicit known limitation. Tell
    the owner the normal launch command `/usr/bin/zodex`, but leave the final app
    launch/restart to the owner.

## 5. Blockers and open questions

1. **Startup modal:** Should Core Local Production be declared with the dismissible
   `CODEX_LINUX_FEATURES_DIR` warning as a documented limitation, or should it be
   fixed and the package rebuilt first? Recommendation: fix it before calling the
   experience polished production.
2. **Lead-review bookkeeping:** All three agents recommend pass, but their
   `lead_review` fields were still pending. The next lead must formally accept or
   reject each gate.
3. **CLP-02 sequencing deviation:** Installation occurred after direct owner
   delegation but before the CLP-01 lead-review field was accepted. Evidence shows
   no resulting technical or preservation problem; retain this as an audit note.
4. **Internal labels:** Upstream `Codex`/`ChatGPT` strings remain inside the web UI.
   Current architecture treats this as intentional and non-blocking. Changing it
   would require a new approval and drift-prone proprietary bundle work.
5. **Unsigned package:** Not a blocker for private/local production. It remains a
   blocker for a public signed binary release.
6. **Optional tracks:** OAuth Broker Preview, live provider compatibility, package
   updater design, public release, and the production remote-control module runtime
   remain out of Core Local Production scope.

## Final handoff assessment

The installed Zodex Core has passed functional C1–C3 acceptance, preserved the
owner’s native ChatGPT session and all existing Router/model configuration, and
proved side-by-side rollback to the old desktop app. It is technically usable for
local production now. The only Zodex-owned UX defect observed is the dismissible
optional-module directory warning at startup. Resolve or explicitly accept that
limitation, finish lead-review/documentation bookkeeping, run the final doctor and
preservation audit, and leave the final launch to the owner.
