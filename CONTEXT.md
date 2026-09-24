# PandaMUX Quick Context

This is a compact vocabulary aid for tools that discover `CONTEXT.md`. It is not a second development guide. Start with [`AGENTS.md`](AGENTS.md), then use [`CLAUDE.md`](CLAUDE.md) and the maintained [`docs/`](docs/README.md) wiki.

PandaMUX is a native Windows terminal multiplexer and visibility layer for running AI CLI agents in parallel. The Rust backend owns canonical state; the Iced UI is a read-projection; local PTYs, SSH PTYs, the CLI, and the bundled orchestrator meet at the named-pipe dispatcher.

## Product language

**Surface**: A terminal, markdown, or diff instance shown as a tab inside a pane. The retired Electron browser surface is not part of the native product.

**Pane**: A rectangular region in a workspace split tree that contains one or more surfaces.

**Workspace**: One visible split layout. It is not the same thing as a project or an individual shell session.

**Session**: A shell context indexed across workspaces and grouped by project, type, or host. Selecting a session focuses its existing pane rather than replacing the workspace layout.

**Project**: Stable identity for related local or SSH locations and sessions, independent of the editable workspace title.

**Live layout preview**: The temporary layout shown while dragging a surface tab, matching the layout that would result from dropping it at the current target.

## Current boundaries

- The browser/CDP pane was intentionally removed. Agents use their own browser tooling.
- PandaMUX does not implement an MCP server.
- The bundled `pandamux-orchestrator` plugin is installed manually; PandaMUX does not write to `~/.claude` on launch.
- The current settings file is JSON under the per-user PandaMUX data directory, not `~/.pandamux/config.toml`.
- Historical plans live under `tasks/plan-repo.md`, `docs/superpowers/`, and `docs/archive/`; verify current status against code, `Cargo.toml`, `CHANGELOG.md`, and `CLAUDE.md`.
