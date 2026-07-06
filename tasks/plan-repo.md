# PandaMUX Everywhere: Full Repo Review and Native Rust Rewrite Plan

Date: 2026-07-05
Status: Approved direction (fully native Rust, Warp/Zed-style). This document is the master plan. **Phase 1 (pnpm migration) is COMPLETE; Phase 2 de-risk spike is COMPLETE; start Phase 3 foundation next.**
Updated: 2026-07-05 (review pass): refined terminal-engine and UI-framework rationale; added the crate-isolation invariant and a state-ownership design note; added a testing strategy; expanded the risk table; restructured the phases with the pnpm migration promoted to Phase 1 (gates all Rust work).
Updated: 2026-07-05 (Phase 1 complete): marked Phase 1 DONE and reconciled its steps to the as-built implementation; recorded the node-pty N-API / npmRebuild and pnpm workspace-mode gotchas in Section 10.
Updated: 2026-07-06 (Phase 2 start): clarified the non-browser V2/CLI parity contract, added `layout.grid` and other current CLI verbs to the parity checklist, and made browser-instruction cleanup an explicit migration task before Rust replaces Electron.
Updated: 2026-07-06 (Phase 2 local spike): added `spikes/phase2-native-terminal` with exact Rust pins, PTY/grid/text-stack smokes, a burst test, and a spike report. Later 2026-07-06 updates closed the renderer, SSH reattach, auth, signing, visual QA, and maintenance-tax gates.
Updated: 2026-07-06 (Phase 2 remote host): validated `ssh galahad`, installed tmux on Galahad, proved tmux detach/reattach by reconnecting and capturing the same session, and launched Claude Code 2.1.200 inside tmux to the workspace trust prompt. This used system OpenSSH; native russh request_pty and Windows agent-pipe auth remain.
Updated: 2026-07-06 (Phase 2 native russh): added `--russh-galahad-smoke`; validated native russh public-key auth, `request_pty`, tmux disconnect/reconnect, Claude Code launch in tmux, and cleanup against Galahad. This used direct key-file auth; Windows OpenSSH agent-pipe auth remains.
Updated: 2026-07-06 (Phase 2 renderer and agent path): added `--russh-galahad-agent-smoke` using russh's Windows named-pipe `AgentClient`. Added `--gpu-render-smoke`, which renders Unicode through glyphon/wgpu to an offscreen texture, reads pixels back, and reports static and dynamic input frame timings.
Updated: 2026-07-06 (Azure Artifact Signing enrolled): closed the Phase 2 signing-eligibility gate. Eligibility was cleared without new identity/business validation by reusing the existing `SupportForge` certificate profile on the Wellforce `HDBtrustedsigning` account. Created a `pandamux-ci-signing` service principal with the Artifact Signing Certificate Profile Signer role, stored the six `AZURE_*` values plus a read-only Doppler service token in the `pandamux` Doppler project (`prd`), and wired signing into the Electron `release.yml`.
Updated: 2026-07-06 (Phase 2 auth matrix): added `--russh-galahad-1password-smoke` and `--russh-galahad-password-smoke`. Direct key-file auth, Windows OpenSSH agent auth, 1Password-compatible agent auth, and password auth all pass the native russh PTY, tmux reattach, Claude Code launch, and cleanup path. The Windows OpenSSH service path was validated with `ssh-agent` running as `Automatic`, `\\.\pipe\openssh-ssh-agent` present, and a loaded `chaz-windows` ED25519 certificate identity. The 1Password-compatible provider path was validated with `PANDAMUX_RUSSH_SMOKE_OK`, `setup_bytes=124`, `reattach_bytes=125`, and `claude_bytes=718`.
Updated: 2026-07-06 (Phase 2 complete): added the canvas-backed Iced `TerminalViewport`, `--iced-widget-smoke`, and `--visual-qa-smoke`, which writes `phase2-visual-qa.bmp` and checks row-level glyph rendering for box drawing, CJK/wide glyphs, emoji, combining marks, powerline, RTL, and ligature text. `cargo info` reports `iced 0.14.0` and `alacritty_terminal 0.26.0` as current, so no one-minor maintenance bump exists to test today. Phase 2 is closed; the next work is Phase 3 production workspace scaffolding.

---

## 1. Decision Summary

| Decision | Choice | Rationale |
|---|---|---|
| Overall direction | **Full native Rust rewrite** (no webview, GPU-rendered) | Terminal rendering performance is a top-3 product priority; effort is explicitly not a constraint; best long-term foundation for modern, resilient, secure, maintainable, cross-platform |
| UI framework | **Iced** (MIT), version-pinned | Chosen for three reasons the rewrite depends on: (1) cosmic-term is a shipping reference implementation of almost exactly PandaMUX's architecture; (2) System76 depends on Iced for the entire COSMIC desktop, so it has a committed institutional maintainer and will not be abandoned; (3) clean MIT license. Pre-1.0 churn is an accepted, managed cost (not a stability guarantee), contained by version pinning plus the crate-isolation rule in Section 6.1. GPUI rejected: open GPL-contamination defect (zed#55470), not built for external consumption, weak accessibility. egui is the designated native fallback if Iced specifically fails (more stable API, but immediate mode fights IME, polished chrome, and the backend-owned-state model) |
| Terminal grid model | **alacritty_terminal**, pinned to an exact version | No Rust terminal engine offers both a full grid model and a stable semver public API; every option is an internal crate (wezterm-term, termwiz) or reinvents the grid (vte is parser-only). alacritty_terminal wins on battle-tested VT handling, a live production consumer (cosmic-term) that surfaces breakage before we hit it, crates.io publication, and the `Handler` trait needed for OSC 52. Inline image rendering is NOT required (see Feature 3), removing the only argument for the wezterm-term fork. API instability is contained by isolating it behind pandamux-term's own grid types (Section 6.1), never leaking alacritty_terminal types past that crate |
| GPU text pipeline | **cosmic-text** (HarfRust shaping) + **swash** + **glyphon** on **wgpu** | 2026-standard stack; gives ligatures, emoji, font fallback, correct wide chars. Must verify double-width rendering in our own renderer (Alacritty's clipping bug is a rendering-layer issue we must not inherit) |
| PTY | **portable-pty** | ConPTY (Windows) + Unix pty behind one trait API; WezTerm's own production PTY layer; direct node-pty replacement |
| SSH | **russh** (v0.62.2, 2026-07) + **russh-sftp** | Actively maintained pure-Rust SSH; remote PTY channel feeds the same grid path as a local PTY; SFTP for the image-paste feature |
| Packaging/updates | **Velopack** (GithubSource) + **winresource** (build.rs) | Replaces electron-builder + electron-updater + latest.yml + rcedit in one tool; the entire 12-step manual ASAR release process and the OneDrive path-with-spaces pain disappear |
| Code signing | **Azure Artifact Signing** (~$10/mo, GA April 2026), **eligibility confirmed 2026-07-06** | Cheap enough that shipping unsigned is no longer the right trade; OV-tier reputation (SmartScreen warnings fade with downloads). Eligibility was the real friction point and is now cleared by reusing the existing `SupportForge` certificate profile (Wellforce `HDBtrustedsigning` account), so no fresh identity/business validation was needed. Signing is wired into the Electron `release.yml` now (see the 2026-07-06 signing note and Section 10). Unsigned portable zip stays as the fallback if a signing run fails |
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
- Clarification added 2026-07-06: "preserved as-is" means all **non-browser** V2 methods and CLI commands that remain product features stay wire-compatible. The browser/CDP methods are intentionally removed, and must either be absent from the Rust CLI or return a clear unsupported response while `system.capabilities` reports that browser automation is not available.

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
- Windows SSH auth: use russh's `AgentClient<NamedPipeClient>` against the Windows OpenSSH-compatible agent named pipe (`\\.\pipe\openssh-ssh-agent`) first. This covers the Windows OpenSSH service and optional compatible providers such as 1Password when the user has one installed and enabled. 1Password is not required for the platform. Direct key-file auth and password auth remain separate paths. Optional Pageant fallback can come later. ProxyJump remains separate channel-dial glue work; no turnkey ProxyJump path exists in the current spike.

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

### 4.1 Non-browser CLI and V2 parity contract

Before Phase 3 implements the Rust pipe server, keep this table as the contract. Any command marked "preserve" must have a matching V2 handler and a CLI regression check.

| Surface | Current command or method | Rust rewrite decision | Phase |
|---|---|---|---|
| System | `ping`, `identify`, `capabilities`, `tree` | Preserve, with browser capability reported false | Phase 3 |
| Windows | `list-windows`, `focus-window` | Preserve for multi-window parity | Phase 5 |
| Workspaces | `new-workspace`, `close-workspace`, `select-workspace`, `rename-workspace`, `list-workspaces` | Preserve | Phase 3 |
| Panes | `split`, `pane split`, `close-pane`, `focus-pane`, `zoom-pane`, `list-panes` | Preserve | Phase 3 |
| Layout | `layout grid` / `layout.grid` | Preserve. Required by pandamux-orchestrator visible-agent spawning | Phase 3 |
| Surfaces | `new-surface`, `close-surface`, `focus-surface`, `list-surfaces` | Preserve for terminal, markdown, and diff. Reject browser clearly | Phase 3 |
| Surface color | `set-color-scheme`, `clear-color-scheme`, `surface.set_color_scheme` | Preserve | Phase 5 |
| Themes | `list-themes`, `themes`, `theme.list` | Preserve | Phase 5 |
| Config | `config show`, `config reload`, `config path`, `reload-config` | Preserve | Phase 5 |
| Terminal I/O | `send`, `send-key`, `read-screen`, `trigger-flash` | Preserve. `read-screen` depends on native serialize support | Phase 4 |
| Agents | `agent spawn`, `agent spawn-batch`, `agent status`, `agent list`, `agent kill` | Preserve | Phase 5 |
| Markdown | `markdown <file>`, `markdown set ...`, `markdown.load_file`, `markdown.set_content` | Preserve | Phase 5 |
| Diff | `diff`, `diff.refresh` | Preserve | Phase 5 |
| Notifications | `notify`, `list-notifications`, `clear-notifications` | Preserve | Phase 4 |
| Sidebar | `set-status`, `set-progress`, `log`, `sidebar-state` | Preserve | Phase 5 |
| Hooks | `hook.event` | Preserve | Phase 5 |
| Browser | `pandamux browser *`, `browser.*`, CDP IPC | Drop. Remove from CLI help, capabilities, injected instructions, docs, and shell comments | Phase 5 |

Migration note: `resources/claude-instructions/claude-instructions.md` currently tells agents to use `pandamux browser`. That file, the generated user instruction block, README CLI examples, docs, and shell-integration comments must be updated before the Rust app becomes the default build. The new instruction should point agents to Claude Code's own browser tooling while keeping `pandamux markdown` guidance.

---

## 5. New features (target design)

### F1: Copy/paste over SSH (OSC 52 + bracketed paste)
- Wire `clipboard_store` events to `arboard` (Rust clipboard crate); allow set unconditionally with a size cap.
- `clipboard_load` (remote reads local clipboard): denied by default, per-host opt-in setting with prompt.
- Track DECSET 2004; wrap pastes in `ESC[200~ ... ESC[201~` when enabled.
- Works identically for local PTYs and SSH-backed surfaces since both feed the same grid.

### F2: Remote Claude Code over SSH
- Connection manager: host profiles (host, user, auth, jump host), known-hosts verification, keepalive.
- Auth: direct key files, password fallback, and Windows OpenSSH-compatible agent named pipe via russh's named-pipe agent client. This covers optional providers such as 1Password when installed and enabled; Pageant bridge optional later.
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

### Phase 1: pnpm migration (COMPLETE 2026-07-05)
Status: **DONE**. Commits `fb6a6ac` (migration) + `dc78668` (native-rebuild + workspace-mode fix) on master, pushed. Exit criteria met: clean-tree `pnpm install --frozen-lockfile` exits 0 with no Python/VS toolchain; `pnpm run build:main` (tsc) and `pnpm run build:renderer` (vite) succeed; node-pty verified loading under both Node 24 and Electron 33. Tests 153/158 (the 5 are the conpty console-list agent in a headless shell, an environment artifact, not a regression). CI packaging is validated on the next release run.

Repo strategy settled: Rust builds on master alongside the frozen Electron app (see Section 6 "Repo strategy"); path-filtered/non-required Rust CI lands with the workspace scaffold in Phase 3.

As implemented (a few corrections to the original plan, kept here for accuracy):
1. npm to pnpm migration:
   - Settings live in `pnpm-workspace.yaml`, not `.npmrc` (pnpm 11 treats `.npmrc` as registry/auth only): `nodeLinker: hoisted` (mandatory for node-pty + ASAR), `allowBuilds` for node-pty/electron/esbuild (pnpm 11 blocks dependency build scripts by default; `onlyBuiltDependencies` is deprecated in favor of `allowBuilds`), and `packages: [.]` so `pnpm run` works at the root without `-w`.
   - Pinned pnpm 11.10.0 + Node 24.18.0 via the `packageManager` and `engines` fields; added `.nvmrc`/`.node-version`; converted package-lock.json to `pnpm-lock.yaml` via `pnpm import`.
   - No native rebuild: removed the `electron-builder install-app-deps` postinstall and set `"npmRebuild": false` in electron-builder.json. node-pty is N-API, so its prebuild is ABI-portable and no per-runtime rebuild is needed (see the Phase 1 gotchas in Section 10).
   - CI (release.yml): `pnpm/action-setup@v4` before `setup-node` (Node 24.18.0, `cache: pnpm`), `pnpm install --frozen-lockfile`, `pnpm exec` for the build/package steps.
   - Release-process doc updated to pnpm (staging uses `pnpm install --prod --ignore-scripts --config.node-linker=hoisted` to stay junction-free).
2. Feature freeze the Electron app: bug fixes only; all new work goes to the Rust track.
3. Browser pane removal on the Electron side: still optional/deferred.

### Phase 2: De-risk spike (exit criteria gated)
Status: **DONE 2026-07-06.** The disposable spike lives at `spikes/phase2-native-terminal` with a detailed report in `PHASE2_REPORT.md`. It proves dependency pins, local PTY capture, Alacritty grid parsing, text shaping, a 2,000-line burst smoke, canvas-backed Iced terminal viewport integration, native russh direct key-file auth, Windows OpenSSH agent auth, 1Password-compatible agent auth, native russh password auth, native `request_pty`, tmux reconnect, Claude Code launch on Galahad, a real offscreen glyphon/wgpu render path, and a visual QA artifact for the hard glyph families. Azure Artifact Signing eligibility is confirmed and signing is wired into the Electron release pipeline; see the 2026-07-06 signing note. The full production widget now moves to Phase 3 implementation, not Phase 2 de-risking.

0. Lock the spike scope before code lands: record the current non-browser CLI/V2 parity contract from Section 4.1; browser methods are excluded on purpose, and capability reporting must make that explicit.
1. Iced window with a custom wgpu terminal-grid widget (cosmic-text + swash + glyphon) fed by portable-pty running pwsh; study cosmic-term's TerminalBox implementation.
2. Validate rendering: throughput (large `cat`/build output), input latency feel, resize correctness, unicode (CJK/emoji/combining), powerline/box-drawing, ligatures, double-width glyph rendering (no Alacritty-style clipping). These are the rendering acceptance criteria referenced by Section 6.3.
3. russh spike: SSH to a Linux box, `request_pty` + shell, stream into the same widget; run Claude Code interactively; kill the connection and re-attach via tmux. Include direct key-file auth, password auth, and the Windows OpenSSH-compatible agent named-pipe auth path, since optional providers such as 1Password should work when present without becoming a requirement for every user.
4. Maintenance-tax check: **DONE 2026-07-06 as far as crates.io allows.** `cargo info iced` reports `0.14.0` as current and `cargo info alacritty_terminal` reports `0.26.0` as current, so no one-minor bump exists to test today. Re-run this at the first Phase 3 dependency refresh.
5. Azure Artifact Signing eligibility check: **DONE 2026-07-06.** Eligibility confirmed by reusing the existing `SupportForge` certificate profile on the Wellforce `HDBtrustedsigning` account (no new identity/business validation required). CI signing identity (`pandamux-ci-signing` service principal) created and granted the Artifact Signing Certificate Profile Signer role on that profile; the six `AZURE_*` secrets stored in Doppler (`pandamux`/`prd`); signing wired into the Electron `release.yml`. Unsigned-zip remains the fallback if a run fails.
6. Exit criteria: 60fps+ under heavy output on the dev machine; typing feels instant; remote Claude Code session survives a disconnect and reattaches cleanly. If the spike fails, fallback is hardened Electron (the pnpm-migrated app from Phase 1); if Iced specifically is the failure, egui is the native fallback.
7. Spike report: append a Phase 2 gotcha/lesson entry to Section 10, including exact crate pins tested, commands run, rendering results, and any decision changes.

### Phase 3: Foundation
- Cargo workspace scaffold; enforce the 6.1 crate-isolation boundary in CI from day one.
- pandamux-core split tree (port split-utils.ts semantics + its tests first, they are the best-specified logic in the repo).
- Implement the backend-owned-state model per Section 6.2 (intent-in, delta-out; single writer; Iced read-projection). This is the load-bearing architectural work; do it before building UI breadth on top.
- Stand up the testing harnesses from Section 6.3 (core unit tests, headless grid harness, pipe wire-compat suite).
- Named-pipe server in tokio, wire-compatible V1 + non-browser V2; port pipe-server tests.
- pandamux-cli against it (`ping`, `identify`, `tree`, workspace/pane/surface commands, and `layout grid` because the orchestrator depends on it).
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
- Remove browser/CDP assumptions from injected Claude instructions, CLI help, README examples, docs, capability output, and shell-integration comments. Point browser work to Claude Code's native browser tooling instead.
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
- Crates (initial pins at Phase 3; re-verify current release state before locking, see Section 11): iced 0.14+, alacritty_terminal (exact pin), cosmic-text, swash, glyphon 0.11+, wgpu, portable-pty, russh 0.62+, russh-sftp, arboard, linkify, tokio, serde/serde_json, pulldown-cmark, velopack, winresource, notify-rust (or Windows toast crate), keyring (if needed for SSH profiles). CI boundary check: cargo-deny or equivalent to enforce Section 6.1. Phase 2 local spike pins are recorded in `spikes/phase2-native-terminal/PHASE2_REPORT.md`.
- CI: GitHub Actions windows-latest primary; Azure Artifact Signing account (~$10/mo, eligibility confirmed in Phase 2).
- Phase 1 (interim, immediate): pnpm 11.10.0 + Node 24.18.0 (24 LTS).

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Custom grid widget quality (the whole bet) | Phase 2 spike with hard exit criteria; cosmic-term as reference; fallback = hardened Electron (the pnpm-migrated app is the fallback baseline); egui if Iced specifically fails |
| alacritty_terminal has no stable-API guarantee | Pin an exact version (not a caret range); isolate behind pandamux-term's own grid types per Section 6.1 so churn/swap is contained to one crate; cosmic-term is a live consumer that surfaces breakage first; Phase 2 could not perform a one-minor bump because the pinned versions are current, so re-run the bump check at the first Phase 3 dependency refresh |
| Backend/UI state-split complexity (Elm vs backend ownership) | Design settled in Section 6.2 (intent-in, delta-out, single writer, read-projection) and implemented in Phase 3 before UI breadth; cosmic-term subscription pattern as reference |
| Iced pre-1.0 churn | Version pinning; System76's institutional backing keeps it alive; cosmic-term/libcosmic as pattern source; pandamux-core/pandamux-term carry logic so the UI layer stays thin and swappable |
| Search/serialize/links rebuilt by hand | Scoped in Phase 4; port xterm addon behaviors as specs and cover them with the headless grid harness; linkify for URLs |
| Rendering correctness hard to test automatically | Headless grid harness covers logic (6.3); rendering validated as explicit Phase 2 acceptance criteria + golden-image where feasible; honestly scoped, not assumed |
| Azure Artifact Signing eligibility | Verify enrollment (identity/business validation) in the Phase 2 spike, not at ship time; unsigned portable zip (today's posture) stays as the fallback |
| Windows OpenSSH-compatible agent pipe is provider-dependent | The spike validates russh's named-pipe agent client against the Windows OpenSSH service with a loaded Galahad identity and against a 1Password-compatible provider. Direct key-file auth and password auth stay as first-class alternatives |
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
- (Phase 1) FIX for the winpty/native-rebuild blocker: node-pty 1.1.0 uses node-addon-api (N-API), so its prebuild is ABI-stable and loads under both Node 24 (ABI 137) and Electron 33 (ABI 130, N-API 9), verified. The from-source rebuild was therefore pure waste that happened to fail. Removed the `electron-builder install-app-deps` postinstall and set `"npmRebuild": false` in electron-builder.json (packaging trusts the prebuilds, already `asarUnpack`ed). Result: `pnpm install` needs no Python/VS toolchain and exits 0. General rule: N-API native deps do not need per-runtime rebuilds; only re-add a rebuild step if a non-N-API (nan) native dep is introduced.
- (Phase 1) A `pnpm-workspace.yaml` (required in pnpm 11 to hold `nodeLinker`/`allowBuilds`) puts even a single-package repo into workspace mode, and with no `packages:` entry `pnpm run <script>` at the root fails with ERR_PNPM_NO_SCRIPT and suggests `-w`. Fix: add `packages: [.]` so the root is the sole workspace package and `pnpm run` works normally.
- (Phase 2) 2026-07-06 kickoff review: the orchestrator's visible-agent path depends on `pandamux layout grid`; do not leave it out of Phase 3 pipe/CLI parity. Dropping browser/CDP also requires rewriting injected Claude instructions and CLI/docs so agents stop trying `pandamux browser`.
- (Phase 2) 2026-07-06 spike scaffold: `spikes/phase2-native-terminal` pins `iced = 0.14.0` and `portable-pty = 0.9.0`; `cargo check` passes; `cargo run -- --pty-smoke` prints `PANDAMUX_PHASE2_PTY_OK`. The first PTY read implementation hung waiting for EOF, and the first bounded implementation saw only `ESC[6n`; the smoke harness now answers CPR with `ESC[1;1R`. Carry this forward with the existing DA1 handling when porting PTY lifecycle semantics.
- (Phase 2) 2026-07-06 local stack result: exact pins tested in `spikes/phase2-native-terminal`: `iced = 0.14.0`, `alacritty_terminal = 0.26.0`, `portable-pty = 0.9.0`, `glyphon = 0.11.0`, `cosmic-text = 0.19.0`, `swash = 0.2.9`, `wgpu = 30.0.0`, `russh = 0.62.2` with `default-features = false` and `ring`. Passing commands: `cargo check`, `cargo fmt --check`, `cargo run -- --grid-smoke`, `cargo run -- --pty-smoke`, `cargo run -- --burst-smoke 2000`, and `cargo run -- --text-stack-smoke`. Burst result on the dev machine: `pty_ms=498`, `total_ms=503`, `bytes=40965`, final marker reached the Alacritty grid.
- (Phase 2) 2026-07-06 dependency gotcha: the current exact stack compiles but resolves three `cosmic-text` versions: `0.15.0` via Iced/cryoglyph, `0.18.2` via glyphon, and `0.19.0` as the direct plan pin. Phase 3 should either align on the glyphon re-export or intentionally isolate text shaping so the duplicate stack does not become production debt.
- (Phase 2) 2026-07-06 remote host validation: `ssh galahad` connects to Fedora Linux as `chaz`; Claude Code is present (`2.1.200`); tmux was installed with `dnf`; a `pandamux-phase2-smoke` tmux session survived SSH disconnect and was captured after reconnect; Claude Code launched inside tmux to the `/home/chaz` workspace trust prompt; smoke sessions were cleaned up. This validates the host and durability shape through system OpenSSH, not native russh.
- (Phase 2) 2026-07-06 native russh validation: `cargo run -- --russh-galahad-smoke` connects to `10.55.88.48:22` as `chaz` using `~/.ssh/galahad`, authenticates via russh public-key auth, requests a PTY, creates a tmux session, disconnects, reconnects with a fresh russh session, captures and cleans the same tmux session, then launches Claude Code inside a bounded tmux smoke and cleans it up. Result: `PANDAMUX_RUSSH_SMOKE_OK`, `setup_bytes=124`, `reattach_bytes=125`, `claude_bytes=718`.
- (Phase 2) 2026-07-06 russh gotcha: Galahad's SSH server did not send an exit-status message for the PTY exec before closing the channel. The smoke accepts marker-plus-clean-close. Keep marker-based remote-session tests and avoid relying solely on exit status for PTY-backed interactive commands.
- (Phase 2) 2026-07-06 auth matrix: direct key-file auth, Windows OpenSSH agent auth, 1Password-compatible agent auth, and password auth pass the full native russh PTY, tmux reattach, Claude Code launch, and cleanup path against Galahad. `--russh-galahad-agent-smoke` uses russh's `AgentClient<NamedPipeClient>` against `\\.\pipe\openssh-ssh-agent`, then tries agent identities with `authenticate_publickey_with` or `authenticate_certificate_with`. Windows OpenSSH live validation passed after enabling the Windows OpenSSH `ssh-agent` service, setting it to `Automatic`, and loading a `chaz-windows` ED25519 certificate identity. `--russh-galahad-1password-smoke` also passed through the same smoke path with `setup_bytes=124`, `reattach_bytes=125`, and `claude_bytes=718`.
- (Phase 2) 2026-07-06 renderer/perf/Unicode validation: `--gpu-render-smoke` passed on `NVIDIA GeForce RTX 5090` via Vulkan, rendering Unicode through glyphon/wgpu to an offscreen texture and reading pixels back. Result: `layout_lines=9`, `glyphs=256`, `nonblack_pixels=12526`, `first_frame_ms=16`, `avg_frame_ms=0.516`, `avg_dynamic_frame_ms=1.584`, `max_frame_ms=16`, `fps_estimate=1936.7`.
- (Phase 2) 2026-07-06 renderer gotchas: `glyphon 0.11.0` depends on `wgpu 29.0.4`, while the plan's direct GPU probe uses `wgpu 30.0.0`; the spike uses a `wgpu_glyphon` alias pinned to 29.0.4 for the render path. Also, reshaping the whole terminal buffer for every simulated keystroke averaged about 27.9ms per frame; the passing dynamic path uses a separate input-line buffer and averages 1.584ms. Production must avoid scrollback-wide reshaping on input.
- (Phase 2) 2026-07-06 Iced widget and visual QA closure: added a reusable canvas-backed `TerminalViewport`, wired the Iced shell to use it, added `--iced-widget-smoke`, and added `--visual-qa-smoke phase2-visual-qa.bmp`. Visual QA passed and wrote an inspectable BMP artifact. Latest render result: `first_frame_ms=14`, `avg_frame_ms=0.452`, `avg_dynamic_frame_ms=1.287`, `fps_estimate=2212.6`; row checks passed for box drawing (`1716`), wide/CJK (`971`), emoji (`820`), combining marks (`938`), powerline (`807`), RTL (`423`), and ligature text (`1129`). Manual inspection in Codex confirmed the artifact is readable and nonblank.
- (Phase 2) 2026-07-06 maintenance-tax check: no one-minor bump exists to test today. `cargo info iced` reports `0.14.0` as current; `cargo info alacritty_terminal` reports `0.26.0` as current. Carry the deliberate bump test to Phase 3's first dependency refresh.
- (Phase 2) 2026-07-06 Azure Artifact Signing enrollment: the flagged eligibility risk was cleared with NO new identity/business validation by reusing an existing certificate profile. Setup: reused the `SupportForge` profile on the `HDBtrustedsigning` account (Wellforce tenant `cea21578-…-528c`, resource group `WF-Platform`, endpoint `https://eus.codesigning.azure.net/`); created a `pandamux-ci-signing` Entra app registration + service principal; granted it the **Artifact Signing Certificate Profile Signer** role scoped to that profile. Gotchas to carry to Phase 7 and LL-G: (1) the built-in role was RENAMED from "Trusted Signing Certificate Profile Signer" to **"Artifact Signing Certificate Profile Signer"** (role GUID `2837e146-70d7-4cfd-ad55-7efa6464f958`); `az role assignment create` errors on the old name. (2) The role assignment is Azure RBAC (ARM), not Microsoft Graph, so it cannot be done through Graph app-registration tooling; it needs `az`/portal. (3) Signing MUST run after any exe modification (rcedit icon/version embed) or the Authenticode signature is invalidated; `release.yml` orders sign after rcedit and before zip. (4) Secrets live in Doppler `pandamux`/`prd` (six `AZURE_*` vars) and reach CI via one `DOPPLER_TOKEN` repo secret + `dopplerhq/secrets-fetch-action`; the client secret expires 2027-07-06. The same account/profile/identity is reusable for the Rust rewrite's Velopack signing in Phase 7.
- (Phase 3) ...

Route durable discoveries to the LL-G knowledge base per repo rules.

## 11. Open items to verify before locking pins

The version specifics in this plan (iced 0.14, alacritty_terminal API shape, russh 0.62.2, glyphon 0.11, Azure Artifact Signing GA/eligibility, whether the GPUI GPL-contamination issue zed#55470 has resolved) should be re-checked against current release state at Phase 2/Phase 3 start. The architectural decisions above are version-independent; the exact pins are not.
