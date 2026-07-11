# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.35.1]

### Changed

- Updated pinned dependencies: `serde_json` `1.0.145` -> `1.0.150` and `uuid` `1.19.0` -> `1.23.4`. As part of the `serde_json` update, its float-formatting dependency switched from `ryu` to `zmij` in the lockfile.

## [0.35.0]

### Added

- **Phase 7 ship pipeline: signed installer + release automation.** Tagging `v*` now runs one GitHub Actions workflow (`.github/workflows/release.yml`, windows-latest) that builds the native app, Azure-signs `pandamux.exe` and `pandamux-cli.exe`, packages a single NSIS `Setup.exe` with `cargo-packager`, signs the installer, and publishes a GitHub Release whose only asset is that signed installer. The in-app updater (added in 0.34.0) discovers it via the Releases API, so there is no Velopack feed / `latest.yml`. Signing uses Azure Trusted Signing with credentials sourced from Doppler; an unsigned build is the fallback only if a signing run fails.
- **Windows icon + version metadata embedded in `pandamux.exe`** via a `winresource` `build.rs` (replaces the Electron rcedit step). Explorer, the taskbar, and the properties dialog show "PandaMUX Everywhere" and the app version, which is sourced from `CARGO_PKG_VERSION`. A dev box without the Windows SDK still builds (the embed is skipped with a warning); CI embeds for real.
- **cargo-packager NSIS configuration** (`[package.metadata.packager]` in `crates/pandamux-app/Cargo.toml`) that bundles the app resources (`themes`, `sounds`, `icons`, `shell-integration`, `pandamux-orchestrator`, `claude-instructions.md`) and the CLI alongside `pandamux.exe`, laid out where the runtime looks for them. Validated end-to-end locally (`cargo packager` produces `pandamux_0.35.0_x64-setup.exe`).

### Changed

- **Single version source is now the root `Cargo.toml`** (`[workspace.package] version = "0.35.0"`); every crate inherits it via `version.workspace = true`. The commit/changelog rule and its pre-commit hooks now bump `Cargo.toml` instead of the removed `package.json`.
- The GUI binary installs as `pandamux.exe` (declared via `[[bin]] name = "pandamux"`), while the cargo package stays `pandamux-app`.
- Rewrote `CLAUDE.md`, `AGENTS.md` (now a pointer to `CLAUDE.md`), the README, and the auto-injected Claude Code instructions to describe the native Rust app and to drop the retired `pandamux browser` guidance (agents use Claude Code's own browser tooling; `system.capabilities` reports `browser: false`).
- winget publishing (`winget.yml` + `winget/*.yaml`) now targets the signed NSIS installer instead of the old portable zip. Because the installer type changed, the winget-pkgs package needs a one-time manual re-bootstrap PR before auto-updates resume.

### Removed

- **Deleted the never-shipped Electron/TypeScript prototype**: `src/`, `package.json`, `pnpm-lock.yaml`, `pnpm-workspace.yaml`, `electron-builder.json`, the TypeScript/Vite/ESLint config, the Node version pins, the Vitest suite, the Electron CLI (`resources/cli/`), the stale release-zip mirror (`zip/`), and the ASAR release doc. The repo is now a pure Rust workspace plus shared `resources/` and `site/`. This was a fork pull with no user base, so it is a plain deletion (no migration).

## [0.34.2]

### Added

- Restored the repository-local `.agents/` skill catalog and `.codex/` agent, hook, and configuration definitions so Codex tooling remains available alongside the existing `.claude/` configuration.

### Changed

- Bumped the app version to `0.34.2`.

## [0.34.1]

### Removed

- Deleted the legacy `.agents/` and `.codex/` tooling-mirror directories (duplicate agent/skill/hook definitions for other tool frontends), superseded by `.claude/` in the bootstrap template sync. `.claude/` is now the single source of truth. No application code or runtime behavior is affected.

### Changed

- Bumped the app version to `0.34.1`.

## [0.34.0]

### Added

- **In-app update checker (`pandamux-app::updater`, Phase 7 groundwork).** On launch (kicked from the first status poll) and every 6 hours, the GUI calls the GitHub Releases API (`/repos/BoardPandas/Pandamux/releases/latest`), strips the leading `v`, semver-compares the tag to the running version, and (once the release is past a 6-hour quarantine window) raises an "Update available" toast through the existing notification model. Drafts and prereleases are never offered, and the same version is toasted only once. This means a published GitHub Release only needs the single signed `Setup.exe` asset for the app to discover and (once packaging lands) install updates: no Velopack update feed or `.nupkg` is required. The decision logic (API parse, `.exe` asset selection, semver compare, RFC 3339 timestamp parse, quarantine gate) is pure and hermetically unit-tested; the network fetch (`reqwest`, OS-TLS/schannel) is gated behind the `iced-runtime` feature so the headless build and unit tests never touch the network. The download-and-run-installer step is wired when packaging exists and there is a real release to point at.

### Changed

- Bumped the app version to `0.34.0`.
- Refined the Phase 7 ship plan in `tasks/plan-repo.md`: the release is one GitHub Actions workflow (build -> Azure-sign `pandamux.exe` -> wrap into `Setup.exe` -> sign the installer -> publish) whose GitHub Release carries exactly one asset, the signed `.exe`; updates flow through the in-app checker above (so no Velopack feed). Dropped the Electron session-import migration (the repo's Electron build was a never-shipped fork pull, so there is no user base to migrate); its removal is a plain file deletion.

## [0.33.0]

### Added

- **Phase 6 (F1): Copy/paste over SSH with OSC 52 + bracketed paste.** The terminal engine now captures a program's OSC 52 clipboard-store escape (`pandamux-term::clipboard`, via a capturing `EventListener` on the grid, secure `Osc52::OnlyCopy` policy) and the app forwards it to the OS clipboard through `arboard` (`pandamux-app::clipboard_os`), on every UI refresh and after each headless pipe command. This works identically for local shells and SSH-backed surfaces because both feed the same grid. Clipboard reads by a remote (OSC 52 load) are denied by default with a per-host opt-in (`ClipboardConfig`). Outgoing pastes are wrapped in bracketed-paste markers when the target has requested DECSET 2004. New pipe methods `clipboard.copy` / `clipboard.get` / `clipboard.policy` and `surface.paste`, plus CLI `pandamux clipboard copy|get|policy` and `pandamux paste`.
- **Phase 6 (F2): SSH remote surfaces with tmux durability and reconnect.** A new `pandamux-term::ssh` (`RemoteSessionManager`, promoted from the proven Phase 2 russh spike) gives a terminal surface an SSH channel as its byte source instead of a local PTY, with a synchronous API mirroring `PtySessionManager` so the backend treats remote and local surfaces uniformly. The remote command wraps the login shell in `tmux new-session -A -s pandamux-<surface>` (falling back to a plain login shell when tmux is absent, degraded/no durability), and the driver reconnects with exponential backoff and resets the grid on re-attach so the server repaint reconciles cleanly (plan Section 5). Resize forwards as an SSH `window-change`. Auth covers the Windows OpenSSH-compatible agent pipe (default; covers 1Password), a key file, or a password. New pipe methods `ssh.connect` / `ssh.disconnect` / `ssh.list` / `ssh.profiles` / `ssh.save_profile` / `ssh.remove_profile` / `ssh.import_config`, plus CLI `pandamux ssh connect|disconnect|list|profiles|save-profile|import`. Terminal I/O (`send_text` / `send_key` / `read_text` / `resize` / `kill` / `paste`) routes to the SSH session for remote surfaces. A slim SSH context chip names the host on a remote pane.
- **Phase 6 (F3): Paste/drop images into a remote session over SFTP.** `RemoteSessionManager::upload_image` transfers a local file to `/tmp/pandamux-paste-<uuid>` on the remote via `russh-sftp` and injects the remote path into the terminal; a local surface injects the local path. New pipe method `surface.paste_image` and CLI `pandamux paste-image <path> [--surface <id>]`.
- **SSH host profiles + `~/.ssh/config` import** (`pandamux-core::ssh`): `SshHostProfile` / `SshProfiles` and a `parse_ssh_config` parser (Host/HostName/User/Port/IdentityFile/ProxyJump, wildcard hosts skipped). Passwords are never stored; a password profile only records that a prompt is needed.
- **Copy-mode yank primitive + region extraction** (`TerminalGrid::region_text`) over scrollback coordinates, and the `clipboard.copy` path, deliver the load-bearing half of the Phase 4 copy-mode-yank deferral.
- Opt-in live SSH validation: `pandamux-app --ssh-smoke` connects to a real host (configured via `PANDAMUX_SSH_SMOKE_*` env vars), runs a durable marker command, reads it back, and prints `PANDAMUX_SSH_SMOKE_OK`. Flag-gated; never runs in CI.

### Changed

- Bumped the app version to `0.33.0`.

### Notes

- The full interactive vi-navigation copy mode and canvas link-click hit-testing (the remaining halves of the paired Phase 4 deferrals) still need GUI interaction work and are tracked. Known-hosts pinning for SSH is a tracked follow-up (the connection currently accepts the server key, matching the Phase 2 spike); ProxyJump is parsed and stored but not yet dialed. The remote-session and SFTP paths are unit-tested hermetically and exercised end-to-end via `--ssh-smoke` against a live host, since they cannot be verified in headless CI.

## [0.32.1]

### Changed

- Marked Phase 5 COMPLETE in `tasks/plan-repo.md`: checked off the nine remaining Phase 5 to-dos (delivered across 0.24.0-0.32.0), recorded the tracked deferrals (JetBrains Mono TTF asset, radial glows + glass blur, the settings.json hook wiring + live activity observer, copy-mode yank/link-open, animation polish), and added a Phase 5 completion status line. Next is Phase 6 (SSH / OSC 52 / SFTP).
- Bumped the app version to `0.32.1`.

## [0.32.0]

### Added

- Claude Code startup integration (`pandamux-app::claude_context`), ported from the Electron `claude-context.ts`. On the real GUI launch the native app now, best-effort: (1) injects a marker-delimited PandaMUX block into the user's `~/.claude/CLAUDE.md` (idempotent, never touching content outside the `<!-- pandamux:start ... -->` / `<!-- pandamux:end -->` markers), and (2) installs the pandamux-orchestrator plugin into `~/.claude/plugins/cache/pandamux-orchestrator/{version}/`, registering it in `installed_plugins.json` and enabling it in `settings.json`. Every step is idempotent and unit-tested against temp directories; a failure logs and never aborts launch.

### Changed

- Bumped the app version to `0.32.0`.

### Notes

- The Claude Code hook wiring in `settings.json` and the live activity observer (the observability half of the Electron integration) remain deferred to the Phase 7 ship boundary; the busy-agent status dot is already fed by the agent registry as the interim signal.

## [0.31.0]

### Added

- Line icons: a canvas-drawn icon set (`pandamux-ui::icons`, 15 stroked line glyphs at ~1.3px) now replaces the unicode-glyph placeholders across the titlebar (search, bell, settings, window controls), the icon rail (sessions, palette, new, notifications, settings), the pane tab bars (add, split-right, split-down, zoom), and the status-bar git segment, matching the design's line-icon style.
- The window background now uses the design's vertical gradient (`bg-top -> bg-bottom` per chrome theme) instead of a flat fill.

### Changed

- Bumped the app version to `0.31.0`.

### Notes

- The JetBrains Mono TTF is not bundled in the repo, so the mono face still resolves via the named-font fallback (`theme::MONO_FONT`) until the font file is dropped into `resources/fonts` and registered; the radial teal/gold ambience glows and true backdrop blur have no Iced primitive and remain the documented layered-translucency approximation.

## [0.30.0]

### Added

- Per-surface terminal color scheme: `surface.set_color_scheme` / `surface.clear_color_scheme` pipe methods and `pandamux set-color-scheme <surfaceId> <scheme>` / `clear-color-scheme <surfaceId>` CLI commands. A surface can override the global theme with any loaded theme, applied to just that pane's viewport; overrides are pruned when the surface closes.
- Multi-window parity: `window.list` / `window.focus` pipe methods and `pandamux list-windows` / `windows` / `focus-window <id>` CLI commands (single-window native build; spawning additional OS windows needs the Iced multi-window runtime and stays out of scope).
- The app sets the Windows AppUserModelID (`com.pandamux.app`) at startup so the taskbar groups it under a stable identity, matching the Electron build.

### Changed

- Bumped the app version to `0.30.0`.

## [0.29.0]

### Added

- Per-session working-directory tracking. The terminal layer now parses OSC 9;9 (cmd / Windows Terminal) and OSC 7 (`file://` URIs) from each session's PTY byte stream (`pandamux-term::cwd::CwdScanner`, incremental so a split sequence is handled), and the dispatcher accepts the V1 `report_pwd <surfaceId> <path>` line that bash/pwsh send over the pipe. The git status-bar poller is now scoped to the focused session's reported cwd (falling back to the process cwd), so the branch/ahead reflects the session you are actually in.

### Changed

- Bumped the app version to `0.29.0`.

## [0.28.0]

### Added

- Wired the rest of the designed keyboard shortcuts: Ctrl+D / Ctrl+Shift+D split the focused pane right / down, Ctrl+W closes the focused pane, Ctrl+Enter zooms it, and Ctrl+Shift+P opens the command palette (alongside the existing Ctrl+K). The keystroke variants resolve the focused pane in the runtime.
- Command-palette keyboard navigation: Up/Down move the highlighted item and Enter activates it, both gated to when the palette is open (so Enter/arrows are inert elsewhere).

### Changed

- Bumped the app version to `0.28.0`.

## [0.27.0]

### Added

- Terminal themes: the native shell now loads the bundled `.theme` files (Ghostty color format) at startup and applies the selected theme's colors (background, foreground, cursor, ANSI palette) to the terminal viewport, independent of the light/dark chrome theme. New `theme.list` / `theme.select` / `theme.get` pipe methods and `pandamux list-themes` / `themes` / `select-theme <name>` CLI commands.
- Config import: `config.import_windows_terminal` (parses a Windows Terminal settings.json `schemes[]` into themes) and `config.import_ghostty` pipe methods, plus `pandamux config import-windows-terminal <file>` / `config import-ghostty <name> <file>` (files read client-side), and `config show` / `config path` / `config reload` (and `reload-config`).
- Internationalization scaffolding: a `pandamux-core::i18n` locale catalog (English + French, partial Arabic/Japanese) with `i18n.set_locale` / `i18n.translate` pipe methods and `pandamux set-locale <en|fr|ar|ja>`.
- `pandamux-core::config` (Theme model, Ghostty/Windows-Terminal parsers, `ThemeStore`) and `pandamux-ui::theme::TermScheme`, both unit-tested.

### Changed

- Threaded a `ThemeStore` and `Localizer` through the shared single-writer dispatcher (via `DispatchCtx`), so CLI/orchestrator theme and locale changes reach the running UI.
- Bumped the app version to `0.27.0`.

## [0.26.0]

### Added

- Drag-and-drop pane splitting (plan Section 12.3). Press a tab and drag it: the pane shows five drop zones (left/right as full-height 25% strips, and a central column of top/center/bottom), the source tab dims, and releasing over a zone moves the tab. Center appends it as a tab in the target pane; a directional zone creates a new pane beside/above/below the target holding the moved surface. A movement gate distinguishes a real drag from a plain click (which just focuses the tab), and Esc cancels an in-flight drag. Empty source panes are pruned, and UI-initiated splits stay 2-level so the column projection round-trips.
- New `pandamux-core::move_surface` split-tree operation and `DropZone` type, a `SurfaceIntent::Move` intent, and a `surface.move` pipe method (`{surfaceId, targetPaneId, zone}`), all unit-tested (center append + prune, directional split, no-op on a pane's only tab, missing surface/target).

### Changed

- Bumped the app version to `0.26.0`.

## [0.25.0]

### Added

- Markdown and diff surfaces now render their content in the pane instead of a placeholder: a line-based markdown renderer (headings, bullets, blockquotes, fenced code, rules, paragraphs) and a per-line-colored unified-diff view, both on the fixed-dark pane scheme.
- New pipe methods `markdown.set_content`, `markdown.load_file`, `diff.set_content`, and `diff.refresh`, backed by a `pandamux-core::SurfaceContents` store threaded through the single-writer dispatcher (so CLI/orchestrator content updates reach the live UI). Content for a closed surface is pruned automatically.
- New CLI commands `pandamux markdown set <surfaceId> [--file <path>] [--content <text>]` and `pandamux diff set <surfaceId> ...`. `--file` is read client-side, so the pipe server never touches the filesystem. This completes the pandamux-orchestrator dashboard path (`pandamux markdown set <sid> --file dashboard.md`).

### Changed

- Folded the shared backend dispatcher's growing `&mut` parameter list into a single `DispatchCtx` struct (app/ptys/notifications/agents/sidebar/contents/clock/spawn flag), so `handle_line` and the sub-dispatchers stay readable as more surfaces gain state.
- Bumped the app version to `0.25.0`.

## [0.24.0]

### Added

- Spawned terminal shells and agents now carry the `PANDAMUX_*` environment (`PANDAMUX=1`, `PANDAMUX_SURFACE_ID`, `PANDAMUX_PIPE`, and `PANDAMUX_AGENT_ID` for agent surfaces), so shell-integration scripts, the CLI, and the pandamux-orchestrator hooks can find the pipe and identify their surface/agent. The orchestrator's `on-agent-stop`/`on-tool-use` hooks key their per-agent state on `PANDAMUX_AGENT_ID`. Closes the Phase 4-deferred `PANDAMUX_*` env plumbing on session spawn.
- Added a `with_env` builder to `PtyCommand` (extra child environment variables).

### Fixed

- `agent spawn` and `agent list` now include an `agentId` field (aliasing the agent `id`), which the orchestrator's `spawn-agents.sh` reads from `agent spawn` output and the monitoring loop reads from `agent list`. Previously only `id` was returned, so scripted orchestration read a null `agentId`.

### Changed

- Added an orchestrator wire-compatibility regression test that replays the exact pipe-method sequence the pandamux-orchestrator scripts invoke (`ping`, `layout.grid`, `agent.spawn`/`list`/`kill`, `notification.raise`, `sidebar.*`) and asserts every response field its `json-tool.js` parser depends on, so orchestration cannot silently break on a shape drift.
- Bumped the app version to `0.24.0`.

## [0.23.1]

### Changed

- Updated `tasks/plan-repo.md` Phase 5 status to mark the agent manager and sidebar status/progress/log surface as delivered, and refined the remaining to-do list (orchestrator-script verification, drag-split, Claude-context startup injection, markdown/diff surfaces, themes/i18n, visual polish).
- Bumped the app version to `0.23.1`.

## [0.23.0]

### Added

- Added the native sidebar status/progress/log surface: `set-status <key> <value>`, `set-progress <value> [--label]`, `log <level> <message>`, and `sidebar-state` CLI commands (with `sidebar.set_status`/`set_progress`/`log`/`get_state` pipe methods), backed by a `pandamux-core::sidebar` store (key/value statuses, a 0-100 progress bar, and a 200-line capped log). This is the other half of what the orchestrator drives to report wave progress.
- The status bar now shows the sidebar progress (`label NN%`) in the accent color when set.

### Changed

- Threaded the sidebar store through the shared single-writer dispatcher so CLI/orchestrator progress reports reach the running UI.
- Bumped the app version to `0.23.0`.

## [0.22.0]

### Added

- Added the native agent manager: `agent spawn` / `agent spawn-batch` / `agent status` / `agent list` / `agent kill` CLI commands (and the matching `agent.*` pipe methods). An agent is a terminal surface running a given command; spawning creates the surface, starts a PTY with the command in the target cwd, and registers it. `spawn-batch` distributes across panes round-robin, stacks as tabs, or splits a pane per agent. This is what the pandamux-orchestrator plugin drives (together with the already-present `layout grid`).
- The status-bar activity dot now turns gold (busy-agent) while agents are registered.
- Added an optional working directory to `PtyCommand` (agents launch in their `cwd`).

### Changed

- The shared backend dispatcher now owns an `AgentRegistry` (in `pandamux-core::agent`), threaded through the single-writer path so CLI/orchestrator-spawned agents appear live in the running UI.
- Bumped the app version to `0.22.0`.

## [0.21.2]

### Changed

- Updated `tasks/plan-repo.md` to record Phase 5 as in progress: marked the delivered slices (pipe/UI unification, session panel, command palette + quick-launch + settings, git/port pollers, browser-method rejection), enumerated the remaining Phase 5 work as explicit tracked to-dos, and added the Phase 5 implementation gotchas (Iced subscription pipe bridge, `run_with` Hash trick, boxed-stream coercion, poller async-only rule, overlay/scrim composition, session-projection model) to Section 10.
- Bumped the app version to `0.21.2`.

## [0.21.1]

### Changed

- The native pipe server and CLI now reject `browser.*` methods and the `browser` command with a clear "not supported in the native build; use Claude Code's browser tooling" message instead of a generic error, matching the parity contract's dropped-browser decision (plan Section 4.1). The Rust CLI already exposes no browser subcommands and `system.capabilities` already reports `browser: false`; rewriting the shared Electron-facing docs and injected instructions is deferred to the Phase 7 Electron-deprecation boundary since the Electron build still ships the browser pane.
- Bumped the app version to `0.21.1`.

## [0.21.0]

### Added

- Added background status-bar pollers to the native shell: a git poller (branch + ahead-count, run in the process working directory) and a localhost dev-port scanner, both driven off the Iced timer via async `tokio` I/O and surfaced in the status bar's git and ports segments. Per-session cwd scoping arrives with shell-integration OSC reporting.

### Changed

- Bumped the app version to `0.21.0`.

## [0.20.0]

### Added

- Added the command palette (Ctrl+K): a 560px centered overlay with a live substring filter over commands, pane actions, and "switch to session" entries; each row carries its action and shows a shortcut chip. Enter runs the selection.
- Added the quick-launch popover: a 300px picker of local shell profiles (PowerShell 7, Windows PowerShell, Command Prompt, WSL) that create a new session; opened by the rail "+", the session-panel footer, and Ctrl+T (SSH host import lands with the SSH manager).
- Added the settings modal (640x440 on a scrim) with a left nav (General / Terminal / Keyboard / Notifications / Quick launch); the General tab toggles UI theme, selects the accent, and toggles the status bar live, and the Keyboard tab lists the bound shortcuts.
- Added overlay management: one centered overlay at a time over a dismiss-on-backdrop scrim, Escape to close, and Ctrl+, to open settings.

### Changed

- The native runtime now owns command-palette and settings-section state, rebuilds the palette item list (with session switches) each refresh, and routes overlay open/dismiss, palette query/activate, quick-launch, settings-section, and accent-select messages.
- Bumped the app version to `0.20.0`.

## [0.19.0]

### Added

- Added the 264px session panel to the native shell: sessions are shell contexts projected across every workspace (each terminal surface is one session), grouped by a live Project / Type / Host segment switcher, with shell-badge/name+meta/status-dot rows, a pinned-first group holding the active session, a session count, and a "+ New session" footer.
- Selecting a session focuses its pane and activates its workspace without swapping the on-screen layout (owner-confirmed session model, plan Section 12.1 #2). The Sessions rail button toggles the panel; the rail "+" and the panel footer create a new session.

### Changed

- The native runtime now builds a session projection into its view model each refresh and routes session select / regroup / new-session messages through core intents.
- Bumped the app version to `0.19.0`.

## [0.18.0]

### Added

- Began Phase 5 of the native Rust rewrite by unifying the named-pipe server and the live Iced UI onto a single-writer state path (plan Section 6.2). When the native shell is running it now embeds the pipe server as an Iced subscription, so a CLI-, agent-, or orchestrator-driven command (`split`, `notify`, `read-screen`, workspace/pane/surface changes) applies to the same canonical state the UI owns and shows up live in the window.
- Added `notify`, `list-notifications`, and `clear-notifications` CLI commands (backed by new `notification.raise`/`notification.list`/`notification.clear` pipe methods), so notifications raised from the command line reach the running UI's bell and panel.

### Changed

- Extracted the pipe dispatcher into a single shared, synchronous `pandamux-app::backend` module used by both the standalone pipe server and the live runtime, replacing the previously duplicated dispatch logic so a CLI-driven and a UI-driven mutation are indistinguishable at the state layer.
- Bumped the app version to `0.18.0`.

## [0.17.0]

### Added

- Completed the Phase 4 backend and terminal-adjacent UI for the native Rust rewrite.
- Added a hand-built terminal engine in `pandamux-term`: case-insensitive/whole-word search over scrollback + visible buffer, full serialization (scrollback plus visible screen), and URL link detection, all with character-offset spans and headless grid-harness tests.
- Ported the Electron PTY lifecycle semantics into `pandamux-term`: shell resolution (pwsh > powershell > cmd), ConPTY-friendly write chunking, DA1 and CPR query interception, same-size resize dropping, Windows process-tree kill (`taskkill /T /F`), and POSIX/WSL working-directory translation.
- Added a backend notification store (`pandamux-core::notification`) holding up to 200 notifications with read-first eviction, plus a 320px notifications slide-over panel with source-dotted cards, a "Clear all" action, and a titlebar bell unread dot.
- Added session persistence (`pandamux-app::persistence`): atomic auto-save of the session every ~30s, named sessions (save/load/list/delete), and version-change handling that clears only the volatile auto-session while preserving named sessions.
- Added a find-in-terminal bar (query input, match count, next/prev, case toggle) with the current match highlighted in the terminal viewport, accent underlines for detected links, a copy-mode indicator, and a session-activity status dot (running/busy/idle).
- Added keyboard shortcuts for find (Ctrl+F) and the notifications panel (Ctrl+N).

### Changed

- The native runtime now owns find state, the notification store, and copy-mode state, recomputing find matches against the focused terminal on each refresh and autosaving the session on a timer.
- Bumped the app version to `0.17.0`.

## [0.16.1]

### Changed

- Promoted the four visual-polish/asset sub-items deferred during the Phase 3 UI build-out (JetBrains Mono TTF bundling, line-icon artwork, background gradient + radial glows, and backdrop-filter blur) from prose notes into explicit tracked `[ ]` to-do items under the Phase 5 design deliverables, so they cannot get lost.
- Added a Section 9 risk-table row for the Iced 0.14 backdrop-filter-blur limitation, and a tracked Phase 6 checkbox for the optional Pageant SSH agent bridge.
- Bumped the app version to `0.16.1`.

## [0.16.0]

### Added

- Completed the Phase 3 native Rust UI design build-out, replicating the design handoff's chrome in Iced: a central theme module (`pandamux-ui::theme`) encoding every design token (dark and light chrome palettes, configurable accent set, typography scale, radii, shadows, spacing, and the fixed-dark terminal scheme).
- Added a frameless window with a 40px custom titlebar (logo mark, session-switcher pill, notification bell with unread dot, settings, and min/max/close controls) with the whole bar as a drag region.
- Added the 52px icon rail (Sessions, command palette, new session, notifications, and bottom-pinned settings) with active and hover states.
- Added a toggleable 26px status bar showing the shell indicator, git branch and ahead-count, ports, session and pane counts, encoding, and version.
- Styled the pane workspace to spec: column layout with 8px gaps and 10px padding, a 36px per-pane tab bar (shell glyph, label, close, split-right/split-down, and zoom controls), focus ring, pane shadow, and radii.
- Wired the terminal pane's fixed-dark color scheme to the canvas grid widget with a block cursor that blinks on a ~1.1s cadence for the focused pane.
- Added a column-view projection that renders the binary split tree as the design's 2-level column layout, with a graceful fallback that never drops panes for arbitrary-depth (CLI/orchestrator) trees.
- Added keyboard shortcuts for toggling the status bar (Ctrl+B), switching the chrome theme (Ctrl+Shift+T), cycling the accent (Ctrl+Shift+A), and requesting the command palette (Ctrl+K).

### Changed

- The native Iced runtime now composes the full chrome via `app_view`, owns chrome view state derived from canonical state (session/pane counts, active shell/session), selects the Iced theme from the chosen chrome theme, and routes window controls through window tasks.
- Bumped the app version to `0.16.0`.

## [0.15.17]

### Added

- Adopted the high-fidelity Iced UI design handoff (`design_handoff_pandamux_ui/`) as the authoritative visual spec for the native Rust rewrite, to be replicated across Phases 3-5.

### Changed

- Updated `tasks/plan-repo.md` with a new UI Design Replication section (exact design tokens, drag-drop interaction spec, and the build decisions to keep the binary split tree with a column-view projection and to keep both workspace and session concepts), plus per-phase design deliverables for Phases 3, 4, and 5.
- Bumped the app version to `0.15.17`.

## [0.15.16]

### Added

- Added the Phase 3 native Rust workspace scaffold with `pandamux-core`, `pandamux-term`, `pandamux-ui`, `pandamux-app`, and `pandamux-cli`.
- Added backend-owned workspace, pane, surface, split tree, zoom, and layout grid state with Rust tests.
- Added a native Windows named-pipe server and Rust CLI parity for the foundational system, workspace, pane, surface, layout, and terminal I/O commands.
- Added portable-pty session management, Alacritty-backed screen text capture, live PTY smoke tests, and live Iced terminal snapshots.
- Added feature-gated Iced shell rendering, periodic refresh, shell controls for splits and tabs, noninteractive shell smoke coverage, and an interactive shell launch path.
- Added Windows Rust CI for formatting, crate boundary checks, tests, shell smoke, and native binary builds.

### Changed

- Updated `tasks/plan-repo.md` with Phase 3 implementation progress, validation evidence, and the interactive Iced smoke result.
- Bumped the app version to `0.15.16`.

## [0.15.15]

### Added

- Completed the Phase 2 native Rust de-risk spike in `spikes/phase2-native-terminal`: Iced terminal viewport, Alacritty grid and PTY smokes, glyphon/wgpu visual QA artifact, Galahad russh PTY/tmux/Claude validation, and direct key, Windows OpenSSH agent, 1Password-compatible agent, and password auth paths.
- Added repo-local Codex and agent configuration under `.codex/`, `.agents/`, and `AGENTS.md` for the PandaMUX workflow.
- Added `tasks/prd-doppler-secrets.md` documenting the Doppler-backed release secret plan.

### Changed

- Wired the release workflow to fetch signing secrets from Doppler and sign `pandamux.exe` with Azure Trusted Signing after rcedit metadata embedding.
- Updated `tasks/plan-repo.md` to mark Phase 2 complete and move the Rust rewrite to Phase 3 foundation work.
- Bumped the app version to `0.15.15`.

### Changed

- **Replaced the app icon and logo across the board with the new BoardPandas bear badge.** Regenerated every icon/logo asset from the new artwork: the Windows app icon (`resources/icons/icon.ico`, multi-size), the runtime window icon (`resources/icon.png`), the titlebar logo (`src/renderer/assets/logo.png`), the marketing-site favicon and logo, and the docs logos. The placeholder `icon.svg` (previously a blue "w") now carries the bear badge.

- **Synced `.claude/references/hooks-and-settings.md` to Claude Code 2.1.201** from the claude-code-bootstrap template: hook structured output (`updatedToolOutput`, `additionalContext`, `reloadSkills`/`sessionTitle`), `Tool(param:value)` parameter matching, HTTP hook custom headers with env-var interpolation, the `PermissionRequest` auto-approval pattern, new settings (`defaultMode` rename, `fallbackModel`, `enforceAvailableModels`, `disableBundledSkills`, `requiresMinimumVersion`), the full six-tier settings precedence chain, and the `ENABLE_PROMPT_CACHING_1H` cache lever.

- **Renamed the product from wmux to PandaMUX Everywhere.** The display/brand name is now "PandaMUX Everywhere" and the technical short-name (CLI command, package, executable, named pipe, environment variables) is `pandamux`. This is a breaking change: the CLI command is now `pandamux` (was `wmux`), environment variables use the `PANDAMUX_*` prefix (was `WMUX_*`), the named pipe is `\\.\pipe\pandamux` (was `\\.\pipe\wmux`), the Windows AppUserModelId/appId is `com.pandamux.app` (was `com.wmux.app`, so auto-update treats this as a new app), the user config path is `~/.pandamux/config.toml` (was `~/.wmux/config.toml`), and the winget package is `BoardPandas.PandaMUX` (was `BoardPandas.wmux`). The bundled Claude Code plugin is now `pandamux-orchestrator` with the `/pandamux:orchestrate` command. Release artifacts are named `pandamux-<version>-win-x64.zip`. Site references point at `pandamux.boardpandas.ai`.

### Added

- Comprehensive project documentation wiki under `docs/` (14 pages, generated by `/doc-sync`): overview, getting started, architecture, main-process modules, renderer and state, configuration, CLI reference, named-pipe control plane, agent orchestration, browser/CDP, AI integration, shell integration, release/packaging, and a glossary. Includes a `_toc.yaml` source of truth, a `README.md` index, Mermaid diagrams, and evidence-based citations pinned to the current commit. The existing `docs/superpowers/` plans and specs are left as manually maintained.
- Full repo review and native Rust rewrite master plan (`tasks/plan-repo.md`): approved direction to rebuild PandaMUX Everywhere as a fully native Rust app (Iced + alacritty_terminal + portable-pty + russh), drop the browser pane, add SSH copy/paste, remote Claude Code, and image-paste-over-SSH features, migrate the interim Electron app from npm to pnpm, and package with Velopack + Azure Artifact Signing.
- Claude Code developer tooling under `.claude/` (agents, skills, rules, hooks, references, scripts).
- `.gitattributes` enforcing LF line endings on shell scripts so shebangs work on Git Bash/macOS/Linux.

### Changed

- Rewrote `README.md` and `CLAUDE.md` to document the native Rust rewrite direction (browser pane retired, SSH features planned, Electron app frozen), repoint the project owner and all GitHub references from the upstream fork (`amirlehmam/wmux`) to `BoardPandas/Pandamux`, and point CLAUDE.md's workflow conventions at `.claude/` as the source of truth. Reduced the cmux attribution to a light protocol-lineage credit.
- Repointed all remaining old-fork (`amirlehmam`) references to `BoardPandas`/`Pandamux` across the marketing site (`site/**` HTML + i18n in every language), release/publishing config (`electron-builder.json`, `.github/workflows/winget.yml`), the orchestrator plugin manifests, and source constants (`update-checker.ts`, `HelpSettings.tsx`, `BrowserPane.tsx`). Renamed the winget manifests to `BoardPandas.PandaMUX.*.yaml`.
- Corrected the marketing site's license label from AGPL-3.0 to MIT to match the actual `LICENSE` file.
- Marked Phase 1 (pnpm migration) complete in the rewrite plan (`tasks/plan-repo.md`) and reconciled its steps to the as-built implementation.
- Expanded `.gitignore` with language, IDE, OS, and secret-file patterns plus Claude Code local files.
- Migrated the build toolchain from npm to pnpm (Phase 1 of the Rust rewrite plan). Pinned pnpm 11.10.0 + Node 24.18.0 (24 LTS) via the `packageManager` and `engines` fields; added `pnpm-workspace.yaml` (hoisted `node_modules` for node-pty/ASAR, plus `allowBuilds` approvals for node-pty/electron/esbuild since pnpm 11 blocks dependency build scripts by default, and a `packages: [.]` entry so `pnpm run` works at the repo root without `-w`), `.nvmrc`/`.node-version`, and converted the lockfile to `pnpm-lock.yaml`. Updated the Release CI workflow and the documented release process to use pnpm. Developers should run `corepack enable pnpm` and use `pnpm` commands (e.g. `pnpm install`, `pnpm run dev`).
- Updated runtime dependencies to their current releases: `react`/`react-dom` 19.2.7, `zustand` 5.0.14, `electron-updater` 6.8.9, `marked` 18.0.5, and dev tooling `@typescript-eslint/*` 8.62.1, `@types/react` 19.2.17.
- Replaced the `uuid` package with the platform-native `crypto.randomUUID()` via a small `src/shared/id.ts` helper (used in both the main process and the renderer). `uuid` went ESM-only in v12, which would break the CommonJS main-process build; the built-in generator produces the same RFC 4122 v4 IDs with no dependency. Dropped `uuid` and `@types/uuid`; moved `@types/ws` from `dependencies` to `devDependencies` where it belongs.
- Upgraded xterm.js to 6.0 and its addons to their matching releases (`@xterm/addon-image` 0.9.0, `@xterm/addon-search` 0.16.0, `@xterm/addon-serialize` 0.14.0, `@xterm/addon-unicode11` 0.9.0, `@xterm/addon-web-links` 0.12.0, `@xterm/addon-webgl` 0.19.0). The terminal renderer now falls back WebGL → DOM.
- Upgraded the build toolchain to current majors: TypeScript 6.0, Vite 8.1 (Rolldown bundler), Vitest 4.1, `@vitejs/plugin-react` 6.0, ESLint 10.6 (added `@eslint/js` as an explicit devDependency), `concurrently` 10, `wait-on` 9, `@types/node` 24. Added `"types": ["node"]` to both tsconfigs (TypeScript 6 no longer auto-includes every `@types/*` package), replaced the deprecated `baseUrl` with relative `paths` in the renderer config, and set `ignoreDeprecations: "6.0"` on the CommonJS main config (its `node` module resolution is deprecated but still functions through TypeScript 6). Attached the original error as `cause` when re-throwing PTY-create failures (ESLint 10's `preserve-caught-error` rule).
- Upgraded Electron from 33 to 43 (Chromium 150 / Node.js 24.17 / N-API 10) and `electron-builder` from 25 to 26. node-pty's N-API prebuilds load and spawn PTYs unchanged on Electron 43 (verified end-to-end: the app boots, the renderer mounts terminals, and PTYs stream data), so the source-free prebuild approach (`npmRebuild: false`, `asarUnpack`) still holds. Marked `electron-winstaller`'s install build script as intentionally skipped in `pnpm-workspace.yaml` (electron-builder pulls it in transitively for the Squirrel/NSIS installer, which this project never builds), which also settles pnpm's pre-run dependency check.

### Removed

- Dropped the `@xterm/addon-canvas` renderer. The Canvas addon was never republished for the xterm 6.0 API (it stayed pinned to xterm 5) and was already deprecated for mispainting rows/wide characters under load (issues #23/#30). Visible terminals use the WebGL renderer, falling back to xterm's built-in DOM renderer when WebGL is unavailable, over the per-process context budget, or after a WebGL context loss.

### Security

- Updated `ws` to 8.21.0, closing a memory-exhaustion denial-of-service advisory (GHSA, high) and an uninitialized-memory disclosure advisory (moderate) that affected the shipped renderer/CDP WebSocket usage.

### Fixed

- Stopped rebuilding node-pty from source on install. node-pty ships N-API prebuilds that are ABI-stable across Node and Electron, so the `electron-builder install-app-deps` postinstall (and electron-builder's packaging rebuild, now disabled via `"npmRebuild": false`) was unnecessary and failed on some Windows toolchains inside node-pty's legacy winpty gyp target. A normal `pnpm install` no longer requires a Python/Visual Studio build toolchain.

## [0.15.1]

- Baseline prior to changelog tracking. See git history for earlier changes.
