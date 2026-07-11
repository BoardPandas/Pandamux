# PandaMUX — Agent Guide

PandaMUX is a native Windows terminal multiplexer for AI agents, built as a Rust workspace (Iced + alacritty_terminal + portable-pty + russh). The Electron/TypeScript prototype this repo started from has been removed.

**The authoritative development guide is [`CLAUDE.md`](CLAUDE.md).** Read it for build/dev/test commands, the crate architecture, the release process, and the repo conventions. This file is a thin pointer so agent tooling that looks for `AGENTS.md` lands in the right place; do not maintain a second copy of the guide here.

Quick pointers:
- Master plan and phase history: [`tasks/plan-repo.md`](tasks/plan-repo.md) (UI design spec in Section 12).
- Workflow rules (commits, changelog, version bump, knowledge-base checks, custom agents): `.claude/`.
- Version single source: `[workspace.package] version` in the root `Cargo.toml`.
