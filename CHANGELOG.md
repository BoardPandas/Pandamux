# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
