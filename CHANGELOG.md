# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
