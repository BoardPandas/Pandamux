# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Renamed the product from wmux to PandaMUX Everywhere.** The display/brand name is now "PandaMUX Everywhere" and the technical short-name (CLI command, package, executable, named pipe, environment variables) is `pandamux`. This is a breaking change: the CLI command is now `pandamux` (was `wmux`), environment variables use the `PANDAMUX_*` prefix (was `WMUX_*`), the named pipe is `\\.\pipe\pandamux` (was `\\.\pipe\wmux`), the Windows AppUserModelId/appId is `com.pandamux.app` (was `com.wmux.app`, so auto-update treats this as a new app), the user config path is `~/.pandamux/config.toml` (was `~/.wmux/config.toml`), and the winget package is `BoardPandas.PandaMUX` (was `BoardPandas.wmux`). The bundled Claude Code plugin is now `pandamux-orchestrator` with the `/pandamux:orchestrate` command. Release artifacts are named `pandamux-<version>-win-x64.zip`. Site references point at `pandamux.boardpandas.ai`.

### Added

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

### Fixed

- Stopped rebuilding node-pty from source on install. node-pty ships N-API prebuilds that are ABI-stable across Node and Electron, so the `electron-builder install-app-deps` postinstall (and electron-builder's packaging rebuild, now disabled via `"npmRebuild": false`) was unnecessary and failed on some Windows toolchains inside node-pty's legacy winpty gyp target. A normal `pnpm install` no longer requires a Python/Visual Studio build toolchain.

## [0.15.1]

- Baseline prior to changelog tracking. See git history for earlier changes.
