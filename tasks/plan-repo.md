# wmux: Full Repo Review and Native Rust Rewrite Plan

Date: 2026-07-05
Status: Approved direction (fully native Rust, Warp/Zed-style). This document is the master plan.

---

## 1. Decision Summary

| Decision | Choice | Rationale |
|---|---|---|
| Overall direction | **Full native Rust rewrite** (no webview, GPU-rendered) | Terminal rendering performance is a top-3 product priority; effort is explicitly not a constraint; best long-term foundation for modern, resilient, secure, maintainable, cross-platform |
| UI framework | **Iced** (MIT) | Mature, actively stewarded (0.14, 2026); COSMIC Terminal (cosmic-term) is a shipping app with almost exactly wmux's architecture (alacritty_terminal + custom wgpu widget + PaneGrid splits + per-pane tabs) and serves as a reference implementation. GPUI rejected for now: pre-1.0 churn, open GPL-contamination licensing defect (zed#55470), weak accessibility |
| Terminal grid model | **alacritty_terminal** v0.26+ | First-party, stable, crates.io-published, proven under Iced by cosmic-term. Inline image rendering is NOT required (see Feature 3 below), which removes the only argument for the wezterm-term fork |
| GPU text pipeline | **cosmic-text** (HarfRust shaping) + **swash** + **glyphon** on **wgpu** | 2026-standard stack; gives ligatures, emoji, font fallback, correct wide chars. Must verify double-width rendering in our own renderer (Alacritty's clipping bug is a rendering-layer issue we must not inherit) |
| PTY | **portable-pty** | ConPTY (Windows) + Unix pty behind one trait API; WezTerm's own production PTY layer; direct node-pty replacement |
| SSH | **russh** (v0.62.1, 2026-07) + **russh-sftp** | Actively maintained pure-Rust SSH; remote PTY channel feeds the same grid path as a local PTY; SFTP for the image-paste feature |
| Packaging/updates | **Velopack** (GithubSource) + **winresource** (build.rs) | Replaces electron-builder + electron-updater + latest.yml + rcedit in one tool; the entire 12-step manual ASAR release process and the OneDrive path-with-spaces pain disappear |
| Code signing | **Azure Artifact Signing** (~$10/mo, GA April 2026) | Cheap enough that shipping unsigned is no longer the right trade; OV-tier reputation (SmartScreen warnings fade with downloads) |
| Browser pane / CDP | **Dropped** | Only used for viewing/automating a browser in-app; not used by the orchestrator; Claude Code has its own browser tooling. Dropping it removes ~900 LOC of the hardest-to-port subsystem |
| Interim Electron app | **Migrate npm to pnpm now; feature-freeze; deprecate when Rust app ships** | User decision: Electron is dropped entirely as soon as the Rust app is built |

### Infrastructure note

The plan-repo skill's locked web infrastructure (Northflank, Better Auth, Postgres, Redis, R2, Resend) does not apply: wmux is a desktop application with no server backend, no auth, no database. The only "infrastructure" is GitHub Releases (distribution), Azure Artifact Signing (signing), and Netlify (the static wmux.org site, unchanged).

---

## 2. Requirements (user's answers)

- Keep every feature currently in the repo (except the browser pane, explicitly approved for removal).
- New features: (1) copy/paste over SSH sessions; (2) run Claude Code on remote Linux machines over SSH; (3) paste/drop local images into a remote Claude Code session (transfer, not render).
- Qualities: modern, resilient, good security, easy to maintain.
- Windows-first; Linux/macOS ports should be easy later.
- Amount of migration effort is explicitly not a constraint; being on the best foundation is.
- Terminal rendering performance/feel is a top-3 product priority (compete on terminal experience like Warp).
- Migrate npm to pnpm for the interim Electron app.

---

## 3. What the research established (key data points)

### Current codebase shape (measured)
- ~17,700 LOC TypeScript (main ~5,900; renderer ~10,500 + ~3,200 CSS; preload 202; cli ~535; shared ~478), ~2,400 LOC tests (24 unit spec files, main-process-heavy, zero renderer component tests).
- Renderer is cleanly isolated: the only Electron surface is `window.wmux` (74 refs in 14 files); zero direct `require('electron')`/Node imports in renderer code.
- The main process reaches into the renderer via `webContents.executeJavaScript` (17+ call sites through `pipe-bridge.ts`): the renderer's Zustand store is the source of truth for workspace/pane/surface state. This inversion is Electron-only and is corrected in the rewrite: **the Rust backend owns all state; the UI is a view**.
- Contract surface to reimplement: ~68 IPC channels + ~50 preload methods + ~30 named-pipe V2 JSON-RPC methods (heavy overlap). The V2 pipe protocol (newline-delimited JSON-RPC over `\\.\pipe\wmux`, token-authenticated) is Electron-independent and is **preserved as-is** so the CLI, shell integration, and wmux-orchestrator plugin keep working.

### Framework landscape (July 2026)
- Electron: mature but 300-500MB RSS, live context-isolation CVEs (e.g. CVE-2026-34780), permissive-by-default security.
- Tauri v2: credible, but webview divergence (xterm.js WKWebView bug #3575, WebKitGTK WebGL quirks) and CDP-needs-CEF made it a compromise; superseded by the native decision.
- Wails v3: still alpha; ruled out.
- Native Rust (Zed/Warp model): Zed 1.0 shipped April 2026 proving viability; Warp documents the bespoke-framework cost ("building the architecture of a browser"), which is why we adopt Iced rather than rolling our own chrome.
- cosmic-term is the load-bearing precedent: alacritty_terminal + cosmic-text/glyphon/wgpu custom widget + PaneGrid + segmented-button tabs, shipping in production.

### Known gaps we own (no crate exists)
- xterm.js addon equivalents for **search**, **serialize**, and **web-links** must be hand-built against the grid cell-iteration API (`linkify` crate helps for URL detection). Real but bounded work.
- Accessibility: Iced's AccessKit integration is in progress, not done. Budget explicit a11y work; do not assume web-platform parity.
- cosmic-text's HarfRust shaping switch is recent (2026); verify complex-script/RTL edge cases early.

### SSH / clipboard mechanics
- OSC 52 is the standard for copy-over-SSH; alacritty_terminal surfaces it as `clipboard_store`/`clipboard_load` Handler events (base64 already decoded). Policy: allow set; gate/deny query (iTerm2/kitty posture). Bracketed paste is a mode we track and wrap outgoing pastes with.
- Remote PTY over SSH is architecturally identical to a local PTY: a russh channel with `request_pty` is just another byte stream into `Term::advance_bytes`. Resize forwards as `window-change`.
- Durable remote agent sessions: attach through remote tmux (`tmux new -A -s <name>`) + reconnect-with-backoff. Do not reimplement mosh.
- Windows SSH auth: speak the Windows OpenSSH agent named pipe (`\\.\pipe\openssh-ssh-agent`) first (1Password registers as it); optional Pageant fallback. No turnkey ProxyJump in any ecosystem; ~100 lines of channel-dial glue.

---

## 4. Feature inventory to preserve (parity checklist)

From the current app (CLAUDE.md, changelog, code):

- [ ] Workspaces (create/close/select/rename/list), sidebar with workspace/session rows
- [ ] Split panes (immutable binary split tree; split/close/focus/zoom; grid layout), drag-drop pane reordering with live preview
- [ ] Surfaces (tabs) per pane: terminal, markdown, diff (browser type dropped); keep-alive semantics (grid state persists across tab switches; in native land this is free since the grid model is retained)
- [ ] PTY lifecycle: shell resolution (pwsh > powershell > cmd), chunked writes, DA1 interception rationale, WSL path translation, process-tree kill on close (port the semantics from pty-manager.ts, not the code)
- [ ] Named pipe `\\.\pipe\wmux`: V1 text protocol (shell hooks) + V2 JSON-RPC with auth token and public-method allowlist. Wire-compatible.
- [ ] CLI (`wmux` binary): all commands except `wmux browser *` (dropped)
- [ ] Agent manager: spawn/spawn-batch/status/list/kill, round-robin distribution
- [ ] wmux-orchestrator plugin: unchanged (talks CLI/pipe); verify script compatibility
- [ ] Claude Code integration: claude-context injection, hooks config, activity observer, sidebar status/progress/log
- [ ] Notifications (OS toast, flash, bell, panel, max 200), notification ring
- [ ] Settings window (all category panels), 51+ keyboard shortcuts, command palette
- [ ] Markdown pane (pulldown-cmark + sanitization replaces marked + dompurify), diff pane (auto-diff tab with opt-out)
- [ ] Themes (light/dark, theme JSON loading; WT/Ghostty config import), i18n
- [ ] Session persistence (auto-save 30s, named sessions, version-change handling)
- [ ] Git branch/dirty polling, GitHub PR polling, port scanner
- [ ] Shell integration scripts (ps1/sh/cmd): unchanged; same env vars (`WMUX`, `WMUX_SURFACE_ID`, `WMUX_PIPE`, `WMUX_CLI`)
- [ ] Find-in-terminal, copy mode, links: rebuilt natively (search/serialize/web-links gap noted above)
- [ ] Auto-update with quarantine window (port updater.ts semantics onto Velopack)
- [ ] Custom titlebar, AppUserModelId taskbar grouping, MOTW handling docs
- [ ] Terminal image *rendering* (xterm addon-image): NOT carried over (see Decision Summary); revisit as Kitty-graphics Handler extension only if demand appears

Explicitly dropped: BrowserPane/AddressBar, cdp-bridge.ts, cdp-proxy.ts, `wmux browser *` CLI, `browser` surface type, CDP IPC channels.

---

## 5. New features (target design)

### F1: Copy/paste over SSH (OSC 52 + bracketed paste)
- Wire `clipboard_store` events to `arboard` (Rust clipboard crate); allow set unconditionally with a size cap.
- `clipboard_load` (remote reads local clipboard): denied by default, per-host opt-in setting with prompt.
- Track DECSET 2004; wrap pastes in `ESC[200~ ... ESC[201~` when enabled.
- Works identically for local PTYs and SSH-backed surfaces since both feed the same grid.

### F2: Remote Claude Code over SSH
- Connection manager: host profiles (host, user, auth, jump host), known-hosts verification, keepalive.
- Auth: Windows OpenSSH agent named pipe (covers 1Password), key files; Pageant bridge optional later.
- A "remote surface" is a surface whose byte source is a russh PTY channel instead of portable-pty; the grid, rendering, input, and pipe protocol see no difference.
- Durability: default remote command wraps in `tmux new -A -s wmux-<surface>`; on channel EOF/error, reconnect with backoff and re-attach. Claude Code keeps running server-side across disconnects.
- Resize events forward as SSH `window-change`.

### F3: Paste/drop images into a remote Claude Code session
- This is file transfer, not terminal graphics: on paste (clipboard image) or drag-drop onto a remote surface, wmux uploads the image via SFTP (russh-sftp) to a remote temp path (e.g. `/tmp/wmux-paste-<uuid>.png`), then injects the remote path into the terminal input (Claude Code accepts image file paths in prompts).
- Local surfaces keep the existing behavior (write temp file locally, inject local path), matching today's `clipboard.pasteImage`.
- Cleanup: best-effort deletion of transferred temp files on session close; document the residue caveat.

---

## 6. Planned repository structure (Rust workspace)

```
wmux/
  Cargo.toml                 # workspace
  crates/
    wmux-core/               # shared types, split tree, session model, pipe protocol (JSON-RPC types)
    wmux-term/               # terminal engine: alacritty_terminal integration, PTY (portable-pty),
                             #   SSH remote PTY (russh), OSC 52 policy, search/serialize/links
    wmux-ui/                 # Iced app: grid widget (wgpu/cosmic-text/glyphon), panes/splits/tabs,
                             #   sidebar, settings, command palette, markdown/diff panes, theming
    wmux-app/                # binary: composition root, named-pipe server, agent manager,
                             #   pollers (git/pr/port), session persistence, updater (Velopack)
    wmux-cli/                # binary: `wmux` CLI, pipe client (wire-compatible with today)
  resources/                 # icons, themes, sounds, shell-integration, wmux-orchestrator (carried over)
  site/                      # unchanged (wmux.org)
  tasks/                     # planning docs (this file)
```

Planned CLAUDE.md hierarchy (create when folders exist):
- Root `CLAUDE.md`: rewrite conventions, workspace layout, release process (Velopack), parity checklist pointer.
- `crates/wmux-term/CLAUDE.md`: grid/PTY/SSH invariants (PTY ID = surface ID survives; byte-source abstraction).
- `crates/wmux-ui/CLAUDE.md`: Iced patterns, theming, widget conventions.

Repo strategy: build the Rust workspace in this repo on a long-lived branch or `rust/` subtree while master keeps the frozen Electron app; flip default when parity ships. (Owner's call at Phase 1 start.)

---

## 7. Phased plan

### Phase 0: De-risk spike (exit criteria gated)
1. Iced window with a custom wgpu terminal-grid widget (cosmic-text + swash + glyphon) fed by portable-pty running pwsh; study cosmic-term's TerminalBox implementation.
2. Validate: throughput (large `cat`/build output), input latency feel, resize correctness, unicode (CJK/emoji/combining), powerline/box-drawing, ligatures, double-width glyph rendering (no Alacritty-style clipping).
3. russh spike: SSH to a Linux box, `request_pty` + shell, stream into the same widget; run Claude Code interactively; kill the connection and re-attach via tmux.
4. Exit criteria: 60fps+ under heavy output on the dev machine; typing feels instant; remote Claude Code session survives a disconnect. If the spike fails, fallback is hardened Electron (documented in section 9).

### Phase 1: Foundation
- Cargo workspace scaffold; wmux-core split tree (port split-utils.ts semantics + its tests first, they are the best-specified logic in the repo).
- Backend-owned state model (the inversion of today's executeJavaScript pattern).
- Named-pipe server in tokio, wire-compatible V1 + V2; port pipe-server tests.
- wmux-cli against it (`ping`, `identify`, `tree`, workspace/pane/surface commands).
- Multi-pane/multi-tab terminal shell in Iced: splits, tabs, focus, zoom.

### Phase 2: Feature parity (the long middle)
- Sidebar, settings panels, command palette, keyboard shortcuts, themes + config import, i18n.
- Markdown + diff surfaces; notifications; session persistence; agent manager + orchestrator verification; git/PR/port pollers; Claude Code context/hooks/observer.
- Hand-built terminal search, serialize (read-screen), link detection.
- Track against the Section 4 checklist; parity = every unchecked box checked or explicitly waived.

### Phase 3: New features
- F1 OSC 52 copy/paste + bracketed paste.
- F2 SSH connection manager + remote surfaces + tmux durability + reconnect.
- F3 SFTP image paste/drop.

### Phase 4: Ship
- Velopack packaging + winresource metadata + Azure Artifact Signing; GitHub Actions release pipeline (evaluate dist-generated workflow vs hand-rolled; verify Velopack/dist coexistence in a spike first).
- Session import from the Electron app's saved sessions (best effort).
- Update wmux.org, README, docs; publish migration notes.
- Deprecate and remove the Electron app (user decision: drop everything Electron once the Rust app is built).
- macOS/Linux: deferred; rcodesign (mac notarization from any OS) and AppImage/Flatpak noted for later.

### Interim track (Electron app, immediately)
1. npm to pnpm migration (consult BP knowledge base before touching config, per repo rule):
   - `.npmrc`: `node-linker=hoisted` (mandatory for node-pty + ASAR packing).
   - `pnpm-workspace.yaml`: `allowBuilds: { node-pty: true }` (pnpm 11 blocks build scripts by default; pnpm 11 requires Node 22+).
   - `pnpm import` from package-lock.json; delete package-lock.json + node_modules; `packageManager` field via corepack (`corepack use pnpm@11`).
   - CI: `pnpm/action-setup@v4` BEFORE `setup-node` (`cache: pnpm`), Node 22, `pnpm install --frozen-lockfile`, `npx` to `pnpm exec`.
   - Release-process note: staging installs can keep `npm install --omit=dev --ignore-scripts` OR use `pnpm install` with hoisted linker; verify `.asar-staging/node_modules` contains zero junctions before any `rm -rf` (pnpm junction-deletion hazard, pnpm#10707).
2. Feature freeze: bug fixes only; all new work goes to the Rust track.
3. Optional: remove the browser pane early on the Electron side too (it is dropped from the product either way).

---

## 8. Environment/tooling required

- Rust stable toolchain (rustup), cargo; MSVC build tools (Windows target).
- Crates (initial pins at Phase 1): iced 0.14+, alacritty_terminal 0.26+, cosmic-text, swash, glyphon 0.11+, wgpu, portable-pty, russh 0.62+, russh-sftp, arboard, linkify, tokio, serde/serde_json, pulldown-cmark, velopack, winresource, notify-rust (or Windows toast crate), keyring (if needed for SSH profiles).
- CI: GitHub Actions windows-latest primary; Azure Artifact Signing account (~$10/mo).
- Interim: pnpm 11 + Node 22.

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Custom grid widget quality (the whole bet) | Phase 0 spike with hard exit criteria; cosmic-term as reference; fallback = hardened Electron (pnpm-migrated app is the fallback baseline) |
| Search/serialize/links rebuilt by hand | Scoped in Phase 2; port xterm addon behaviors as specs; linkify for URLs |
| Iced learning curve (Elm model vs React/Zustand) | cosmic-term/libcosmic source as patterns; wmux-core keeps logic out of the UI layer |
| Accessibility regression vs web | Track Iced AccessKit progress; budget explicit a11y pass pre-1.0 |
| cosmic-text HarfRust recency | Exercise CJK/RTL/emoji in Phase 0 |
| Velopack + dist interplay unknown | Small packaging spike at Phase 4 start; Velopack alone is sufficient if they conflict |
| pnpm junction deletion on Windows | Hoisted linker; verify zero reparse points in staging dirs before recursive deletes |
| Orchestrator plugin breakage | Pipe protocol kept wire-compatible; regression-test the plugin's scripts against the Rust pipe server in Phase 2 |

## 10. Learning lessons / gotchas (fill during implementation)

- (Phase 0) ...
- (Phase 1) ...

Route durable discoveries to the LL-G knowledge base per repo rules.
