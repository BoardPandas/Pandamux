# PandaMUX — Agent Guide

PandaMUX is a native Windows terminal multiplexer for AI agents, built as a Rust workspace (Iced + alacritty_terminal + portable-pty + russh). The Electron/TypeScript prototype this repo started from has been removed.

**The authoritative development guide is [`CLAUDE.md`](CLAUDE.md).** Read it for build/dev/test commands, the crate architecture, the release process, and the repo conventions. This file is a routing page so agent tooling that looks for `AGENTS.md` lands on the right sources without duplicating them.

## AI fast path

1. Read [`README.md`](README.md) for the product purpose, user problem, and shipped feature set.
2. Read [`CLAUDE.md`](CLAUDE.md) for architecture, invariants, commands, release mechanics, and workflow rules.
3. Use [`docs/README.md`](docs/README.md) to open the relevant crate, feature, API, or operations page.
4. Use [`tasks/plan-repo.md`](tasks/plan-repo.md) only for rewrite history, design rationale, and the UI specification in Section 12. It is not the current roadmap.

Authority order for current facts: implementation and manifests, then `CHANGELOG.md`, then `CLAUDE.md`, then the maintained `docs/` wiki. Files under `docs/archive/` and `docs/superpowers/` are historical. If sources conflict, verify the implementation and correct the documentation as part of the task.

Quick pointers:
- Historical rewrite plan and phase record: [`tasks/plan-repo.md`](tasks/plan-repo.md) (UI design spec in Section 12).
- Maintained documentation index: [`docs/README.md`](docs/README.md).
- Workflow rules (commits, changelog, version bump, knowledge-base checks, custom agents): `.claude/`.
- Version single source: `[workspace.package] version` in the root `Cargo.toml`.
