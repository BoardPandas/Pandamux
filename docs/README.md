# PandaMUX Documentation

> **Latest Updates (July 2026):**
> - **v0.53 (Unreleased):** Check-for-updates button and one-click install in Settings, plus an update banner on launch that downloads and runs the signed installer.
> - **v0.52.0:** SSH connections are now shared per host (spec 1.6), so a second session to a connected host starts near-instantly; closing an SSH tab keeps the host connection pooled.
> - **v0.52.0:** Project and tab shortcuts (Ctrl+1..9, Ctrl+Tab), a keyboard-shortcut cheat sheet (Ctrl+/ or F1), and custom keybindings via `keyboard.overrides`.
> - **v0.52.0:** Fresh bare terminals offer a skippable tool chooser (Claude / Codex / Gemini / custom); the Home dashboard is live across projects (spec 2.5).

## Quick Start

| Goal | Start Here |
|------|------------|
| **Understand the system** | [ARCHITECTURE.md](core/ARCHITECTURE.md) |
| **Build and run locally** | [GETTING_STARTED.md](GETTING_STARTED.md) |
| **What PandaMUX is** | [OVERVIEW.md](OVERVIEW.md) |
| **Drive it from the CLI** | [CLI_REFERENCE.md](api/CLI_REFERENCE.md) |
| **Cut a release** | [RELEASE.md](operations/RELEASE.md) |
| **Look up a term** | [GLOSSARY.md](GLOSSARY.md) |

---

## Core

Foundational architecture and per-crate documentation.

| Document | Description |
|----------|-------------|
| [ARCHITECTURE.md](core/ARCHITECTURE.md) | Crate-isolation invariant, backend-owned state, immutable split tree, PTY=Surface keep-alive |
| [CORE_DOMAIN.md](core/CORE_DOMAIN.md) | `pandamux-core`: canonical state model, split tree, projects, agents, branded IDs |
| [TERMINAL_ENGINE.md](core/TERMINAL_ENGINE.md) | `pandamux-term`: grid, local PTY, shell lifecycle, search/links, clipboard, cwd |
| [UI_SHELL.md](core/UI_SHELL.md) | `pandamux-ui`: Iced shell, chrome, panels, overlays, theming, read-projection |
| [APP_RUNTIME.md](core/APP_RUNTIME.md) | `pandamux-app`: composition root, intent dispatcher, pollers, persistence, updater |
| [CONFIGURATION.md](core/CONFIGURATION.md) | config.toml schema, settings, keymap, themes, and environment variables |

---

## API

| Document | Description |
|----------|-------------|
| [CLI_REFERENCE.md](api/CLI_REFERENCE.md) | The `pandamux` CLI: every command group and its mapping to V2 JSON-RPC pipe methods |

---

## Features

Feature and integration documentation.

| Document | Description |
|----------|-------------|
| [NAMED_PIPE_IPC.md](features/NAMED_PIPE_IPC.md) | The `\\.\pipe\pandamux` control plane: V1 hooks, V2 JSON-RPC, shared dispatcher, method catalog |
| [SSH_REMOTE.md](features/SSH_REMOTE.md) | russh remote PTYs and SFTP: connection model, per-host pool, OSC 52, image paste, launcher UI |
| [AGENT_ORCHESTRATION.md](features/AGENT_ORCHESTRATION.md) | Agent surfaces in visible panes, agent pipe methods, orchestrator plugin, Claude context startup |
| [SHELL_INTEGRATION.md](features/SHELL_INTEGRATION.md) | Shell hook scripts, OSC 7 / OSC 9;9 cwd tracking, report_pwd, git and port pollers |

---

## Operations

Release, packaging, and distribution procedures.

| Document | Description |
|----------|-------------|
| [RELEASE.md](operations/RELEASE.md) | Tag-driven GitHub Actions release: build, Azure Trusted Signing, NSIS installer, updater, winget |

---

## Glossary

| Document | Description |
|----------|-------------|
| [GLOSSARY.md](GLOSSARY.md) | Domain terms, branded ID types, and architecture vocabulary |

---

## Plans and Specs

Manually maintained design and planning documents (historical; not owned by doc-sync).

- [superpowers/plans/](superpowers/plans/) — implementation plans from the Rust rewrite
- [superpowers/specs/](superpowers/specs/) — design specs referenced by the changelog (spec 1.x / 2.x)

---

## Archive

Historical reference documents (superseded by the Rust rewrite).

- [archive/](archive/) — the prior Electron-era pages (main process, renderer, browser/CDP, AI integration)

---

## Related Resources

| Resource | Location |
|----------|----------|
| Repo README | [../README.md](../README.md) |
| Agent Registry | [../AGENTS.md](../AGENTS.md) |
| Claude Code Config | [../CLAUDE.md](../CLAUDE.md) |
| config.toml note | [config.md](config.md) |

---

**Last Updated:** July 2026
