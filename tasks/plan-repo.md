# PandaMUX Everywhere: Full Repo Review and Native Rust Rewrite Plan

Date: 2026-07-05
Status: Approved direction (fully native Rust, Warp/Zed-style). This document is the master plan.
Updated: 2026-07-05 (review pass): refined terminal-engine and UI-framework rationale; added the crate-isolation invariant and a state-ownership design note; added a testing strategy; expanded the risk table; restructured the phases with the pnpm migration promoted to Phase 1 (gates all Rust work).

---

## 1. Decision Summary

| Decision | Choice | Rationale |
|---|---|---|
| Overall direction | **Full native Rust rewrite** (no webview, GPU-rendered) | Terminal rendering performance is a top-3 product priority; effort is explicitly not a constraint; best long-term foundation for modern, resilient, secure, maintainable, cross-platform |
| UI framework | **Iced** (MIT), version-pinned | Chosen for three reasons the rewrite depends on: (1) cosmic-term is a shipping reference implementation of almost exactly PandaMUX's architecture; (2) System76 depends on Iced for the entire COSMIC desktop, so it has a committed institutional maintainer and will not be abandoned; (3) clean MIT license. Pre-1.0 churn is an accepted, managed cost (not a stability guarantee), contained by version pinning plus the crate-isolation rule in Section 6.1. GPUI rejected: open GPL-contamination defect (zed#55470), not built for external consumption, weak accessibility. egui is the designated native fallback if Iced specifically fails (more stable API, but immediate mode fights IME, polished chrome, and the backend-owned-state model) |
| Terminal grid model | **alacritty_terminal**, pinned to an exact version | No Rust terminal engine offers both a full grid model and a stable semver public API; every option is an internal crate (wezterm-term, termwiz) or reinvents the grid (vte is parser-only). alacritty_terminal wins on battle-tested VT handling, a live production consumer (cosmic-term) that surfaces breakage before we hit it, crates.io publication, and the `Handler` trait needed for OSC 52. Inline image rendering is NOT required (see Feature 3), removing the only argument for the wezterm-term fork. API instability is contained by isolating it behind pandamux-term's own grid types (Section 6.1), never leaking alacritty_terminal types past that crate |
| GPU text pipeline | **cosmic-text** (HarfRust shaping) + **swash** + **glyphon** on **wgpu** | 2026-standard stack; gives ligatures, emoji, font fallback, correct wide chars. Must verify double-width rendering in our own renderer (Alacritty's clipping bug is a rendering-layer issue we must not inherit) |
| PTY | **portable-pty** | ConPTY (Windows) + Unix pty behind one trait API; WezTerm's own production PTY layer; direct node-pty replacement |
| SSH | **russh** (v0.62.1, 2026-07) + **russh-sftp** | Actively maintained pure-Rust SSH; remote PTY channel feeds the same grid path as a local PTY; SFTP for the image-paste feature |
| Packaging/updates | **Velopack** (GithubSource) + **winresource** (build.rs) | Replaces electron-builder + electron-updater + latest.yml + rcedit in one tool; the entire 12-step manual ASAR release process and the OneDrive path-with-spaces pain disappear |
| Code signing | **Azure Artifact Signing** (~$10/mo, GA April 2026), eligibility to be confirmed | Cheap enough that shipping unsigned is no longer the right trade; OV-tier reputation (SmartScreen warnings fade with downloads). Caveat: Trusted/Artifact Signing gates on identity/business validation and new-org/individual eligibility has been a real friction point; verify eligibility in the Phase 2 spike, not at ship time. Shipping an unsigned portable zip (today's posture) remains the fallback |
| Browser pane / CDP | **Dropped** | Only used for viewing/automating a browser in-app; not used by the orchestrator; Claude Code has its own browser tooling. Dropping it removes ~900 LOC of the hardest-to-port subsystem |
| Interim Electron app | **Migrate npm to pnpm now (Phase 1); feature-freeze; deprecate when Rust app ships** | User decision: pnpm migration happens before any Rust work; Electron is dropped entirely as soon as the Rust app is built |

### Infrastructure note

The plan-repo skill's locked web infrastructure (Northflank, Better Auth, Postgres, Redis, R2, Resend) does not apply: PandaMUX Everywhere is a desktop application with no server backend, no auth, no database. The only "infrastructure" is GitHub Releases (distribution), Azure Artifact Signing (signing), and Netlify (the static pandamux.boardpandas.ai site, unchanged).

---

## 2. Requirements (user's answers)

- Keep every feature currently in the repo (except the browser pane, explicitly approved for removal).
- New features: (1) copy/paste over SSH sessions; (2) run Claude Code on remote Linux machines over SSH; (3) paste/drop local images into a remote Claude Code session (transfer, not render).
- Qualities: modern, resilient, good security, easy to maintain.
- Windows-first; Linux/macOS ports should be easy later.
- Amount of migration effort is explicitly not a constraint; being on the best foundation is.
- Terminal rendering performance/feel is a top-3 product priority (compete on terminal experience like Warp).
- Migrate npm to pnpm for the interim Electron app, and do it before starting the Rust work.

---

## 3. What the research established (key data points)

### Current codebase shape (measured)
- ~17,700 LOC TypeScript (main ~5,900; renderer ~10,500 + ~3,200 CSS; preload 202; cli ~535; shared ~478), ~2,400 LOC tests (24 unit spec files, main-process-heavy, zero renderer component tests).
- Renderer is cleanly isolated: the only Electron surface is `window.pandamux` (74 refs in 14 files); zero direct `require('electron')`/Node imports in renderer code.
- The main process reaches into the renderer via `webContents.executeJavaScript` (17+ call sites through `pipe-bridge.ts`): the renderer's Zustand store is the source of truth for workspace/pane/surface state. This inversion is Electron-only and is corrected in the rewrite: **the Rust backend owns all state; the UI is a view** (design specified in Section 6.2).
- Contract surface to reimplement: ~68 IPC channels + ~50 preload methods + ~30 named-pipe V2 JSON-RPC methods (heavy overlap). The V2 pipe protocol (newline-delimited JSON-RPC over `\\.\pipe\pandamux`, token-authenticated) is Electron-independent and is **preserved as-is** so the CLI, shell integration, and pandamux-orchestrator plugin keep working.

### Framework landscape (July 2026)
- Electron: mature but 300-500MB RSS, live context-isolation CVEs (e.g. CVE-2026-34780), permissive-by-default security.
- Tauri v2: credible, but webview divergence (xterm.js WKWebView bug #3575, WebKitGTK WebGL quirks) and CDP-needs-CEF made it a compromise; superseded by the native decision.
- Wails v3: still alpha; ruled out.
- Native Rust GUI contenders evaluated: Iced, GPUI, egui. Iced chosen (see Decision Summary for the full rationale). GPUI rejected (GPL-contamination defect, external-consumption immaturity, weak a11y). egui retained only as the fallback if Iced specifically fails: its API is more stable, but immediate mode is a poor fit for PandaMUX's polished chrome (settings, command palette), IME/text-input handling, and the backend-owned-state model. Rolling our own chrome (Warp model) rejected: Warp documents the bespoke-framework cost ("building the architecture of a browser"), which is why we adopt Iced rather than reinventing it.
- Zed 1.0 shipped April 2026 proving native-Rust desktop viability; cosmic-term is the load-bearing precedent for our exact stack (alacritty_terminal + cosmic-text/glyphon/wgpu custom widget + PaneGrid + segmented-button tabs, shipping in production).

### Terminal-engine landscape (why alacritty_terminal, restated)
- There is no Rust terminal engine that provides both a complete grid model and a stable semver public API. Every realistic option is an internal crate.
- `wezterm-term`: internal crate, no external API guarantee; its only advantage (inline images) is out of scope.
- `termwiz`: WezTerm's lower-level surface/cell library; same internal-crate caveat, smaller consumer base.
- `vte` (parser only): the escape-sequence state machine, not a grid; choosing it means hand-building scrollback, selection, damage tracking, and wide-char handling, i.e. reinventing the hard part alacritty_terminal already solved.
- Conclusion: the choice is not stable-vs-unstable but which internal engine and how well it is contained. alacritty_terminal is the most battle-tested, has a production consumer that catches breakage first, and is contained behind pandamux-term's own types.

### Known gaps we own (no crate exists)
- xterm.js addon equivalents for **search**, **serialize**, and **web-links** must be hand-built against the grid cell-iteration API (`linkify` crate helps for URL detection). Real but bounded work.
- Markdown rendering: Iced ships a `markdown` module (0.13+). Evaluate it before committing to a bespoke pulldown-cmark + custom-widget path; it may cover the markdown surface with far less work. Sanitization is still ours to enforce regardless.
- Accessibility: Iced's AccessKit integration is in progress, not done. Budget explicit a11y work; do not assume web-platform parity.
- cosmic-text's HarfRust shaping switch is recent (2026); verify complex-script/RTL edge cases early.

### SSH / clipboard mechanics
- OSC 52 is the standard for copy-over-SSH; alacritty_terminal surfaces it as `clipboard_store`/`clipboard_load` Handler events (base64 already decoded). Policy: allow set; gate/deny query (iTerm2/kitty posture). Bracketed paste is a mode we track and wrap outgoing pastes with.
- Remote PTY over SSH is architecturally identical to a local PTY: a russh channel with `request_pty` is just another byte stream into `Term::advance_bytes`. Resize forwards as `window-change`.
- Durable remote agent sessions: attach through remote tmux (`tmux new -A -s <name>`) + reconnect-with-backoff. Do not reimplement mosh. Note: this assumes tmux is present on the remote; make that an explicit documented prerequisite with a defined behavior when it is absent (fall back to a plain PTY without durability, and surface the degraded state).
- Windows SSH auth: speak the Windows OpenSSH agent named pipe (`\\.\pipe\openssh-ssh-agent`) first (1Password registers as it); optional Pageant fallback. russh does NOT natively speak the Windows agent pipe: this is implementing the SSH-agent protocol over that named pipe ourselves, separate work from and in addition to the ~100 lines of channel-dial glue for ProxyJump (no turnkey ProxyJump exists in any ecosystem). Budget both line items and validate in the Phase 2 russh spike.

---

## 4. Feature inventory to preserve (parity checklist)

From the current app (CLAUDE.md, changelog, code):

- [ ] Workspaces (create/close/select/rename/list), sidebar with workspace/session rows
- [ ] Split panes (immutable binary split tree; split/close/focus/zoom; grid layout), drag-drop pane reordering with live preview
- [ ] Surfaces (tabs) per pane: terminal, markdown, diff (browser type dropped); keep-alive semantics. State retention is inherent in native land, not a trick: we keep each `Term` instance alive in memory, so switching tabs never reconstructs grid state. This is a different mechanism from today's `visibility: hidden` DOM approach, not a free side effect of it
- [ ] PTY lifecycle: shell resolution (pwsh > powershell > cmd), chunked writes, DA1 interception rationale, WSL path translation, process-tree kill on close (port the semantics from pty-manager.ts, not the code)
- [ ] Named pipe `\\.\pipe\pandamux`: V1 text protocol (shell hooks) + V2 JSON-RPC with auth token and public-method allowlist. Wire-compatible.
- [ ] CLI (`pandamux` binary): all commands except `pandamux browser *` (dropped)
- [ ] Agent manager: spawn/spawn-batch/status/list/kill, round-robin distribution
- [ ] pandamux-orchestrator plugin: unchanged (talks CLI/pipe); verify script compatibility
- [ ] Claude Code integration: claude-context injection, hooks config, activity observer, sidebar status/progress/log
- [ ] Notifications (OS toast, flash, bell, panel, max 200), notification ring
- [ ] Settings window (all category panels), 51+ keyboard shortcuts, command palette
- [ ] Markdown pane (pulldown-cmark or Iced's markdown module + sanitization replaces marked + dompurify), diff pane (auto-diff tab with opt-out)
- [ ] Themes (light/dark, theme JSON loading; WT/Ghostty config import), i18n
- [ ] Session persistence (auto-save 30s, named sessions, version-change handling)
- [ ] Git branch/dirty polling, GitHub PR polling, port scanner
- [ ] Shell integration scripts (ps1/sh/cmd): unchanged; same env vars (`PANDAMUX`, `PANDAMUX_SURFACE_ID`, `PANDAMUX_PIPE`, `PANDAMUX_CLI`)
- [ ] Find-in-terminal, copy mode, links: rebuilt natively (search/serialize/web-links gap noted above)
- [ ] Auto-update with quarantine window (port updater.ts semantics onto Velopack)
- [ ] Custom titlebar, AppUserModelId taskbar grouping, MOTW handling docs
- [ ] Terminal image *rendering* (xterm addon-image): NOT carried over (see Decision Summary); revisit as Kitty-graphics Handler extension only if demand appears

Explicitly dropped: BrowserPane/AddressBar, cdp-bridge.ts, cdp-proxy.ts, `pandamux browser *` CLI, `browser` surface type, CDP IPC channels.

---

## 5. New features (target design)

### F1: Copy/paste over SSH (OSC 52 + bracketed paste)
- Wire `clipboard_store` events to `arboard` (Rust clipboard crate); allow set unconditionally with a size cap.
- `clipboard_load` (remote reads local clipboard): denied by default, per-host opt-in setting with prompt.
- Track DECSET 2004; wrap pastes in `ESC[200~ ... ESC[201~` when enabled.
- Works identically for local PTYs and SSH-backed surfaces since both feed the same grid.

### F2: Remote Claude Code over SSH
- Connection manager: host profiles (host, user, auth, jump host), known-hosts verification, keepalive.
- Auth: Windows OpenSSH agent named pipe (covers 1Password) via our own agent-protocol-over-pipe implementation, plus key files; Pageant bridge optional later.
- A "remote surface" is a surface whose byte source is a russh PTY channel instead of portable-pty; the grid, rendering, input, and pipe protocol see no difference.
- Durability: default remote command wraps in `tmux new -A -s pandamux-<surface>` (documented tmux prerequisite; defined degraded behavior when absent); on channel EOF/error, reconnect with backoff and re-attach. Claude Code keeps running server-side across disconnects.
- Reconnect reconciliation: re-attaching to tmux triggers a full repaint from the server; the local grid must reset/reconcile to that repaint rather than append to stale state. Specify the reset-on-reattach behavior explicitly.
- Resize events forward as SSH `window-change`.

### F3: Paste/drop images into a remote Claude Code session
- This is file transfer, not terminal graphics: on paste (clipboard image) or drag-drop onto a remote surface, PandaMUX Everywhere uploads the image via SFTP (russh-sftp) to a remote temp path (e.g. `/tmp/pandamux-paste-<uuid>.png`), then injects the remote path into the terminal input (Claude Code accepts image file paths in prompts).
- Local surfaces keep the existing behavior (write temp file locally, inject local path), matching today's `clipboard.pasteImage`.
- Cleanup: best-effort deletion of transferred temp files on session close; document the residue caveat.

---

## 6. Planned repository structure (Rust workspace)

```
pandamux/
  Cargo.toml                 # workspace
  crates/
    pandamux-core/           # shared types, split tree, session model, pipe protocol (JSON-RPC types)
                             #   CANONICAL domain state lives here. Zero Iced dependency.
    pandamux-term/           # terminal engine: alacritty_terminal integration, PTY (portable-pty),
                             #   SSH remote PTY (russh), OSC 52 policy, search/serialize/links.
                             #   Exposes pandamux's OWN grid/cell types; alacritty_terminal never leaks out.
    pandamux-ui/             # Iced app: grid widget (wgpu/cosmic-text/glyphon), panes/splits/tabs,
                             #   sidebar, settings, command palette, markdown/diff panes, theming.
                             #   The ONLY crate that imports Iced.
    pandamux-app/            # binary: composition root + tokio runtime; owns authoritative mutable
                             #   state; named-pipe server, agent manager, pollers (git/pr/port),
                             #   session persistence, updater (Velopack)
    pandamux-cli/            # binary: `pandamux` CLI, pipe client (wire-compatible with today)
  resources/                 # icons, themes, sounds, shell-integration, pandamux-orchestrator (carried over)
  site/                      # unchanged (pandamux.boardpandas.ai)
  tasks/                     # planning docs (this file)
```

### 6.1 Crate-isolation invariant (hard rule)

Both of the rewrite's hardest technical bets (custom grid widget quality; framework/engine churn) are neutralized by strict isolation, not by choosing different crates. Enforce these as non-negotiable boundaries:

- `pandamux-core` and `pandamux-term` have **zero** Iced dependency.
- `pandamux-ui` is the **only** crate that imports Iced.
- `pandamux-term`'s public API exposes pandamux's own `Grid`/`Cell`/`Selection` types; `alacritty_terminal` types never appear in `pandamux-core`, `pandamux-ui`, or `pandamux-app`.

Payoff: an alacritty_terminal upgrade (or a full engine swap) touches only pandamux-term; a framework swap (if Iced ever truly fails and we fall back to egui) rewrites only pandamux-ui against unchanged core/term/logic. This boundary is also exactly what the backend-owned-state design (6.2) requires, so it pays for itself twice. A dependency-graph check (e.g. cargo-deny or a CI grep) should fail the build if the boundary is violated.

### 6.2 State-ownership design (resolves the Elm-vs-backend tension)

Today's app is inverted: the renderer's Zustand store is the source of truth and the main process reaches in via `executeJavaScript`. The rewrite inverts it back, but Iced's Elm architecture (the application model *is* the state; `update` is the only mutator) creates a design tension that must be settled before Phase 3.

Decision:
- **Canonical state is owned by `pandamux-app`** (the tokio runtime side): the workspace/pane/surface split tree, session model, agent registry. Types are defined in `pandamux-core`.
- **`pandamux-ui` (Iced) holds a read-projection only**, keyed for cheap diffing. It never mutates canonical state directly.
- **Communication is intent-in, delta-out:**
  - UI actions (split, close, focus, zoom, tab switch) are serialized as *intents* and sent over an async channel to the backend.
  - The backend applies the intent to canonical state, then emits a *state delta*.
  - An Iced `subscription` bridges the delta channel into the Iced message loop; deltas become `Message`s that update the UI's read-projection.
- **Single writer:** the backend is the only writer. The named-pipe server (CLI, agents, orchestrator) and the UI are *both* clients that submit the same intents to the same code path, so a CLI-driven split and a UI-driven split are indistinguishable at the state layer. This is the whole point of the inversion.
- This mirrors cosmic-term's subscription pattern; use libcosmic/cosmic-term as the reference implementation.

Anti-pattern to avoid: double-owning the tree in both the Iced model and the backend. The Iced model holds a projection, not a second source of truth.

### 6.3 Testing strategy

The current suite's weakness (main-heavy, zero renderer/component tests) is exactly the trap the rewrite must not repeat. Layered approach:

- **Logic (`pandamux-core`):** unit tests. Port `split-utils.ts` and session-model tests first: they are the best-specified logic in the repo and the safest thing to build on.
- **Pipe protocol (`pandamux-app`):** port pipe-server tests; add a wire-compatibility regression suite that replays the orchestrator plugin's scripts against the Rust pipe server.
- **Terminal engine (`pandamux-term`):** a **headless grid harness** that feeds byte streams into `Term::advance_bytes` and asserts resulting cell state, with no GPU involved. This is where VT correctness, OSC 52 policy, bracketed-paste wrapping, and the hand-built search/serialize/link extraction get real automated coverage. It sidesteps the "how do you test a GPU widget" problem for everything that is logic rather than pixels.
- **Rendering (`pandamux-ui`):** the genuinely hard part. Where feasible, golden-image/snapshot tests of the grid widget at the wgpu layer (headless adapter). Where not, rendering correctness (double-width glyphs, ligatures, powerline, CJK/emoji) is validated as explicit Phase 2 spike acceptance criteria and documented as manually verified. Do not pretend full automated GPU-render coverage exists; scope it honestly.

### Planned CLAUDE.md hierarchy (create when folders exist)
- Root `CLAUDE.md`: rewrite conventions, workspace layout, release process (Velopack), parity checklist pointer, the 6.1 isolation rule.
- `crates/pandamux-term/CLAUDE.md`: grid/PTY/SSH invariants (PTY ID = surface ID survives; byte-source abstraction; alacritty_terminal never leaks).
- `crates/pandamux-ui/CLAUDE.md`: Iced patterns, theming, widget conventions, the read-projection rule from 6.2.

### Repo strategy (decided): build on master directly
The Rust workspace lives on master alongside the frozen Electron app. The two build systems do not collide: Rust occupies `Cargo.toml` + `crates/` + `target/`; Electron occupies `package.json` + `src/` + `dist/`; `resources/` and `site/` are shared and carried over. A half-built `crates/` never affects the Electron ASAR build or release, so master stays shippable for Electron throughout the transition. This is simpler than a long-lived branch (no months-long drift/merge overhead against shared files, one source of truth, matches the owner's pragmatic preference); branch-isolation collaboration benefits are marginal for this repo.

CI hygiene (the one thing to manage, since Rust will be red for long stretches mid-development): keep the Rust and Electron pipelines independent and path-filtered so WIP Rust never blocks an Electron hotfix.
- Rust workflow triggers only on `crates/**` and `Cargo.toml`; mark it non-required (or `continue-on-error`) until parity.
- Electron release workflow stays as-is, triggered on its own paths/tags; it must not depend on Rust building.
- `.gitignore` gains `target/`; keep `node_modules/` and `dist/`.
Deprecation (Phase 7) is then just deleting the Electron files when the Rust app ships: no default-branch flip ceremony.

---

## 7. Phased plan

Phases are sequential. Phase 1 (pnpm) gates all Rust work per the user's direction. The old "long middle" is split into Phases 4 and 5 so parity progress is measurable.

### Phase 1: pnpm migration (do this first; blocks everything else)
Consult the BP knowledge base before touching config, per repo rule. Repo strategy is settled: Rust builds on master alongside the frozen Electron app (see Section 6 "Repo strategy"); no branch/subtree decision is needed. Path-filtered/non-required Rust CI gets set up when the workspace scaffold lands in Phase 3.
1. npm to pnpm migration:
   - `.npmrc`: `node-linker=hoisted` (mandatory for node-pty + ASAR packing).
   - `pnpm-workspace.yaml`: `allowBuilds: { node-pty: true }` (pnpm 11 blocks build scripts by default).
   - Pin pnpm **11.10.0** and Node **24.18.0** (24 LTS). `pnpm import` from package-lock.json; delete package-lock.json + node_modules; set the `packageManager` field via corepack (`corepack use pnpm@11.10.0`); set `engines` in package.json (`node >=24.18.0`, `pnpm >=11.10.0`) and optionally `.nvmrc`/`.node-version` to `24.18.0`.
   - CI: `pnpm/action-setup@v4` (version `11.10.0`) BEFORE `setup-node` (`node-version: 24.18.0`, `cache: pnpm`), `pnpm install --frozen-lockfile`, `npx` to `pnpm exec`.
   - Release-process note: staging installs can keep `npm install --omit=dev --ignore-scripts` OR use `pnpm install` with the hoisted linker; verify `.asar-staging/node_modules` contains zero junctions before any `rm -rf` (pnpm junction-deletion hazard, pnpm#10707).
2. Feature freeze the Electron app: bug fixes only; all new work goes to the Rust track.
3. Optional: remove the browser pane early on the Electron side too (it is dropped from the product either way).
Exit criteria: `pnpm install --frozen-lockfile` reproduces a working build and release from a clean tree; CI green; a release dry-run packs a correct ASAR.

### Phase 2: De-risk spike (exit criteria gated)
1. Iced window with a custom wgpu terminal-grid widget (cosmic-text + swash + glyphon) fed by portable-pty running pwsh; study cosmic-term's TerminalBox implementation.
2. Validate rendering: throughput (large `cat`/build output), input latency feel, resize correctness, unicode (CJK/emoji/combining), powerline/box-drawing, ligatures, double-width glyph rendering (no Alacritty-style clipping). These are the rendering acceptance criteria referenced by Section 6.3.
3. russh spike: SSH to a Linux box, `request_pty` + shell, stream into the same widget; run Claude Code interactively; kill the connection and re-attach via tmux. Include the Windows OpenSSH agent named-pipe auth path (our agent-protocol-over-pipe code), since it is on the critical path for the "1Password just works" story.
4. Maintenance-tax check: pin alacritty_terminal and Iced, then deliberately bump each by one minor version and fix the resulting breakage. An afternoon of this measures the real recurring upgrade cost and confirms the 6.1 isolation boundary actually holds.
5. Azure Artifact Signing eligibility check: confirm the org/individual can actually enroll and sign (identity/business validation), well before Phase 7. If ineligible, the unsigned-zip fallback becomes the ship posture.
6. Exit criteria: 60fps+ under heavy output on the dev machine; typing feels instant; remote Claude Code session survives a disconnect and reattaches cleanly. If the spike fails, fallback is hardened Electron (the pnpm-migrated app from Phase 1); if Iced specifically is the failure, egui is the native fallback.

### Phase 3: Foundation
- Cargo workspace scaffold; enforce the 6.1 crate-isolation boundary in CI from day one.
- pandamux-core split tree (port split-utils.ts semantics + its tests first, they are the best-specified logic in the repo).
- Implement the backend-owned-state model per Section 6.2 (intent-in, delta-out; single writer; Iced read-projection). This is the load-bearing architectural work; do it before building UI breadth on top.
- Stand up the testing harnesses from Section 6.3 (core unit tests, headless grid harness, pipe wire-compat suite).
- Named-pipe server in tokio, wire-compatible V1 + V2; port pipe-server tests.
- pandamux-cli against it (`ping`, `identify`, `tree`, workspace/pane/surface commands).
- Multi-pane/multi-tab terminal shell in Iced: splits, tabs, focus, zoom.

### Phase 4: Terminal-adjacent parity
The terminal engine and everything that lives close to the grid.
- Hand-built terminal search, serialize (read-screen), link detection, exercised through the headless grid harness.
- Find-in-terminal, copy mode, links UI.
- PTY lifecycle semantics ported from pty-manager.ts (shell resolution, chunked writes, DA1 interception, WSL path translation, process-tree kill).
- Session persistence (auto-save 30s, named sessions, version-change handling).
- Notifications (OS toast, flash, bell, panel, max 200), notification ring.
- Exit criteria: a keyboard-driven multi-pane terminal is fully usable for daily work with search, copy mode, and persistence.

### Phase 5: Peripheral UI and integrations parity
The rest of the Section 4 checklist.
- Sidebar, settings panels, command palette, 51+ keyboard shortcuts, themes + WT/Ghostty config import, i18n.
- Markdown surface (evaluate Iced's markdown module first) + diff surface.
- Agent manager (spawn/spawn-batch/status/list/kill, round-robin) + orchestrator plugin verification (regression-test its scripts against the Rust pipe server).
- Git/PR/port pollers; Claude Code context injection, hooks config, activity observer, sidebar status/progress/log.
- Custom titlebar, AppUserModelId grouping.
- Exit criteria: every Section 4 box checked or explicitly waived; parity declared.

### Phase 6: New features
- F1 OSC 52 copy/paste + bracketed paste.
- F2 SSH connection manager + remote surfaces + tmux durability + reconnect (with the reset-on-reattach reconciliation from Section 5).
- F3 SFTP image paste/drop.

### Phase 7: Ship
- Velopack packaging + winresource metadata + Azure Artifact Signing (or unsigned-zip fallback if eligibility failed in Phase 2); GitHub Actions release pipeline (evaluate dist-generated workflow vs hand-rolled; verify Velopack/dist coexistence in a spike first).
- Session import from the Electron app's saved sessions (best effort).
- Update pandamux.boardpandas.ai, README, docs; publish migration notes.
- Deprecate and remove the Electron app (user decision: drop everything Electron once the Rust app is built).
- macOS/Linux: deferred; rcodesign (mac notarization from any OS) and AppImage/Flatpak noted for later.

---

## 8. Environment/tooling required

- Rust stable toolchain (rustup), cargo; MSVC build tools (Windows target).
- Crates (initial pins at Phase 3; re-verify current release state before locking, see Section 11): iced 0.14+, alacritty_terminal (exact pin), cosmic-text, swash, glyphon 0.11+, wgpu, portable-pty, russh 0.62+, russh-sftp, arboard, linkify, tokio, serde/serde_json, pulldown-cmark, velopack, winresource, notify-rust (or Windows toast crate), keyring (if needed for SSH profiles). CI boundary check: cargo-deny or equivalent to enforce Section 6.1.
- CI: GitHub Actions windows-latest primary; Azure Artifact Signing account (~$10/mo, eligibility confirmed in Phase 2).
- Phase 1 (interim, immediate): pnpm 11.10.0 + Node 24.18.0 (24 LTS).

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Custom grid widget quality (the whole bet) | Phase 2 spike with hard exit criteria; cosmic-term as reference; fallback = hardened Electron (the pnpm-migrated app is the fallback baseline); egui if Iced specifically fails |
| alacritty_terminal has no stable-API guarantee | Pin an exact version (not a caret range); isolate behind pandamux-term's own grid types per Section 6.1 so churn/swap is contained to one crate; cosmic-term is a live consumer that surfaces breakage first; measure the upgrade tax in the Phase 2 spike |
| Backend/UI state-split complexity (Elm vs backend ownership) | Design settled in Section 6.2 (intent-in, delta-out, single writer, read-projection) and implemented in Phase 3 before UI breadth; cosmic-term subscription pattern as reference |
| Iced pre-1.0 churn | Version pinning; System76's institutional backing keeps it alive; cosmic-term/libcosmic as pattern source; pandamux-core/pandamux-term carry logic so the UI layer stays thin and swappable |
| Search/serialize/links rebuilt by hand | Scoped in Phase 4; port xterm addon behaviors as specs and cover them with the headless grid harness; linkify for URLs |
| Rendering correctness hard to test automatically | Headless grid harness covers logic (6.3); rendering validated as explicit Phase 2 acceptance criteria + golden-image where feasible; honestly scoped, not assumed |
| Azure Artifact Signing eligibility | Verify enrollment (identity/business validation) in the Phase 2 spike, not at ship time; unsigned portable zip (today's posture) stays as the fallback |
| Windows OpenSSH agent pipe is custom protocol work | russh does not speak it natively; budget agent-protocol-over-pipe implementation separately from the ProxyJump glue; validate both in the Phase 2 russh spike |
| tmux assumed present on remote for durability | Documented prerequisite; defined degraded behavior (plain PTY, no durability, surfaced to the user) when tmux is absent; specify reset-on-reattach reconciliation |
| Accessibility regression vs web | Track Iced AccessKit progress; budget explicit a11y pass pre-1.0 |
| cosmic-text HarfRust recency | Exercise CJK/RTL/emoji in Phase 2 |
| Velopack + dist interplay unknown | Small packaging spike at Phase 7 start; Velopack alone is sufficient if they conflict |
| pnpm junction deletion on Windows | Hoisted linker; verify zero reparse points in staging dirs before recursive deletes |
| Orchestrator plugin breakage | Pipe protocol kept wire-compatible; regression-test the plugin's scripts against the Rust pipe server in Phase 5 |

## 10. Learning lessons / gotchas (fill during implementation)

- (Phase 1) DONE 2026-07-05. pnpm 11 config moved out of `.npmrc`: `nodeLinker: hoisted` and `allowBuilds` live in `pnpm-workspace.yaml` (`.npmrc` is registry/auth only now). `onlyBuiltDependencies` is deprecated in pnpm 11, replaced by `allowBuilds` (a map, e.g. `node-pty: true`). Native/binary packages needing approval here: node-pty, electron, esbuild.
- (Phase 1) A globally npm-installed pnpm (`AppData\Roaming\npm\pnpm.ps1`) shadows the `packageManager` pin; run `corepack enable pnpm` so the corepack shim (in the Node dir) wins and the pinned 11.10.0 is used. Verify with `(Get-Command pnpm -All).Source`.
- (Phase 1) pnpm's `verify-deps-before-run` means a FAILING root `postinstall` makes the install incomplete, which then makes every `pnpm run <script>` re-trigger install and fail too. Bypass for verification by invoking binaries directly (`node_modules\.bin\tsc.cmd`, etc.); the real fix is making install exit 0.
- (Phase 1) The `postinstall` (`electron-builder install-app-deps`) rebuilds node-pty from source (node-gyp). Two local-toolchain gotchas, both pre-existing and identical under npm, neither caused by pnpm: (a) Python 3.12+ removed `distutils` -> `pip install setuptools` restores it (CI already does this); (b) node-pty's bundled winpty from-source build can fail (`GetCommitHash.bat` / gyp) on some Windows toolchains. Not on the release critical path: node-pty ships win32-x64 prebuilds (present and runtime-verified: `pty.node` loaded, spawn returned a pid), and the release flow deletes `node-pty/build` to force the prebuild path.
- (Phase 1) Verified under pnpm: `tsc` clean; 153/158 unit tests pass. The 5 failures are node-pty conpty console-list (`AttachConsole failed`) in a headless/no-console shell, an environment artifact, not a pnpm regression.
- (Phase 2) ...
- (Phase 3) ...

Route durable discoveries to the LL-G knowledge base per repo rules.

## 11. Open items to verify before locking pins

The version specifics in this plan (iced 0.14, alacritty_terminal API shape, russh 0.62.1, glyphon 0.11, Azure Artifact Signing GA/eligibility, whether the GPUI GPL-contamination issue zed#55470 has resolved) should be re-checked against current release state at Phase 2/Phase 3 start. The architectural decisions above are version-independent; the exact pins are not.
