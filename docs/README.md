# PandaMUX Everywhere Documentation

> **Latest Updates (July 2026):**
> - **Rename:** the product is now "PandaMUX Everywhere"; the CLI, package, pipe, and env vars use the `pandamux` short-name (`\\.\pipe\pandamux`, `PANDAMUX_*`).
> - **Direction:** approved master plan to rebuild as a fully native Rust app (Iced + alacritty_terminal + portable-pty); the Electron app is frozen to bug fixes.
> - **Toolchain:** migrated from npm to pnpm (pinned pnpm 11.10.0 + Node 24 LTS); upgraded Electron 33 to 43 and the TS6/Vite8/Vitest4/ESLint10 build toolchain.
> - **Terminal:** upgraded xterm.js to 6.0 and dropped the abandoned Canvas addon; the renderer now falls back WebGL to DOM.
> - **Security:** updated `ws` to 8.21.0, closing a high-severity DoS and a moderate memory-disclosure advisory.

## Quick Start

| Goal | Start Here |
|------|------------|
| **Understand the system** | [ARCHITECTURE.md](core/ARCHITECTURE.md) |
| **Run the project locally** | [GETTING_STARTED.md](GETTING_STARTED.md) |
| **What is PandaMUX Everywhere** | [OVERVIEW.md](OVERVIEW.md) |
| **Drive it from the CLI** | [CLI_REFERENCE.md](api/CLI_REFERENCE.md) |
| **Cut a release** | [RELEASE.md](operations/RELEASE.md) |
| **Look up a term** | [GLOSSARY.md](GLOSSARY.md) |

---

## Overview

| Document | Description |
|----------|-------------|
| [OVERVIEW.md](OVERVIEW.md) | What PandaMUX Everywhere is, its stack, repo layout, and where to start reading. |
| [GETTING_STARTED.md](GETTING_STARTED.md) | Prerequisites, pnpm install, dev workflow, build scripts, and tests. |

---

## Core

Foundational platform documentation.

| Document | Description |
|----------|-------------|
| [ARCHITECTURE.md](core/ARCHITECTURE.md) | Electron process model, IPC, preload bridge, split tree, and keep-alive surfaces. |
| [MAIN_PROCESS.md](core/MAIN_PROCESS.md) | The Electron main-process modules: entry point, PTY lifecycle, windows, IPC handlers, and helpers. |
| [RENDERER_AND_STATE.md](core/RENDERER_AND_STATE.md) | React UI structure, terminal rendering, keyboard shortcuts, and the Zustand store. |
| [CONFIGURATION.md](core/CONFIGURATION.md) | Settings store, user config, theme loading, terminal-config import, and environment variables. |

---

## API

| Document | Description |
|----------|-------------|
| [CLI_REFERENCE.md](api/CLI_REFERENCE.md) | The pandamux CLI: every command group, arguments, and how it maps to pipe methods. |

---

## Features

Feature-specific documentation.

| Document | Description |
|----------|-------------|
| [NAMED_PIPE_IPC.md](features/NAMED_PIPE_IPC.md) | The `\\.\pipe\pandamux` server: V1 text hooks, V2 JSON-RPC, the renderer bridge, and Electron IPC channels. |
| [AGENT_ORCHESTRATION.md](features/AGENT_ORCHESTRATION.md) | Spawning agent PTYs across panes, the orchestration watcher, store slices, and the orchestrator plugin. |
| [BROWSER_CDP.md](features/BROWSER_CDP.md) | The browser pane driven through the Chrome DevTools Protocol: bridge, proxy, and CLI surface. |
| [AI_INTEGRATION.md](features/AI_INTEGRATION.md) | Claude Code context injection, hook configuration, plugin install, activity observation, and OpenCode support. |
| [SHELL_INTEGRATION.md](features/SHELL_INTEGRATION.md) | Shell hook scripts, OSC sequences, and the git, PR, port, and shell-detection pollers. |

---

## Operations

Deployment, packaging, and release procedures.

| Document | Description |
|----------|-------------|
| [RELEASE.md](operations/RELEASE.md) | The ASAR-based portable-zip release flow, native-module unpacking, rcedit, auto-update, and CI. |

---

## Glossary

| Document | Description |
|----------|-------------|
| [GLOSSARY.md](GLOSSARY.md) | Domain terms and branded ID types used across PandaMUX Everywhere. |

---

## Plans and Specs

Active design and planning documents (manually maintained, outside the doc-sync TOC).

| Location | Description |
|----------|-------------|
| [superpowers/plans/](superpowers/plans/) | Implementation plans (wmux v2 features, saved sessions, drag-and-drop, orchestrator plugin, OpenCode compatibility). |
| [superpowers/specs/](superpowers/specs/) | Design specs paired with the plans above. |

---

## Related Resources

| Resource | Location |
|----------|----------|
| Repo README | [../README.md](../README.md) |
| Claude Code Config | [../CLAUDE.md](../CLAUDE.md) |
| Changelog | [../CHANGELOG.md](../CHANGELOG.md) |

---

**Last Updated:** July 2026
