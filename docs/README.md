# PandaMUX Documentation

This index routes contributors to current sources for the GPUI chat-first rewrite. Read the root [`Cargo.toml`](../Cargo.toml) for the version; documentation does not duplicate it.

## Start here

| Goal | Current source |
|---|---|
| Understand the product and current status | [`README.md`](../README.md) |
| Build, test, and follow repository conventions | [`CLAUDE.md`](../CLAUDE.md) |
| Orient an agent and resolve source authority | [`AGENTS.md`](../AGENTS.md) |
| Implement the active rewrite | [`tasks/plan-rewrite.md`](../tasks/plan-rewrite.md) |
| Inspect CI truth | [`.github/workflows/rust.yml`](../.github/workflows/rust.yml) |
| Inspect current release notes | [`CHANGELOG.md`](../CHANGELOG.md) |

## Current architecture map

| Area | Source of truth | Responsibility |
|---|---|---|
| Domain | [`crates/pandamux-core`](../crates/pandamux-core) | Framework-free IDs, settings, threads, turns, events, schedules, policy data |
| Wire protocol | [`crates/pandamux-protocol`](../crates/pandamux-protocol) | Versioned JSON-RPC and Hub-to-node proxy contracts |
| Providers | [`crates/pandamux-providers`](../crates/pandamux-providers) | Provider drivers, profiles, parsing, and process supervision |
| SSH and bootstrap | [`crates/pandamux-remote`](../crates/pandamux-remote) | SSH pool, exec, SFTP, host bootstrap, and proxy tunnels |
| Server | [`crates/pandamux-server`](../crates/pandamux-server) | Single-writer store, request contexts, provider execution, PTYs, Hub and node runtime |
| Client projections | [`crates/pandamux-client`](../crates/pandamux-client) | Event-derived view state and token smoothing |
| Desktop | [`crates/pandamux-desktop`](../crates/pandamux-desktop) | GPUI chat shell, timeline, composer, settings, surfaces, and terminal grid |
| CLI | [`crates/pandamux-cli`](../crates/pandamux-cli) | Authenticated commands and restricted MCP stdio adapter |
| Terminal primitives | [`crates/pandamux-term`](../crates/pandamux-term) | Separately selectable `grid` and `pty` features |

The server is the production owner of remote composition and durable state. The desktop is a GPUI client. Remote Linux nodes run the server daemon and are reached through the versioned SSH exec proxy; no remote TCP control port is opened.

## Active Phase 4 status

The unfinished Phase 4 prototype was preserved under Recovery Gate R0.1. R0.2 through R0.4 establish production remote routing, exact provider-instance resolution, and authenticated local control. R0.5 has exact live evidence for Galahad and native Windows IPC. The user approved deferring its remaining physical laptop checks on 2026-09-29, which closes the development gate but does not satisfy production acceptance or release. Phase 4A supplies the normalized usage, policy, migration, and authenticated bounded-protocol foundation. Phase 4B adds node-owned scanning and reconciliation. Phase 4C adds durable Hub synchronization, indexed aggregation, effective-dated pricing, and cross-node coverage. Phase 4D adds conservative capacity revisions, bounded provider health, and explicit preview-before-launch login actions. Phase 4E adds the top-level Usage surface, named provider and subscription presentation, canonical policy enforcement, and source-level accessibility work, with native Windows interaction acceptance still open. The complete current contract is in [`features/USAGE_OBSERVABILITY.md`](features/USAGE_OBSERVABILITY.md).

The authoritative completion records, unresolved rows, and exact acceptance criteria are in [`tasks/plan-rewrite.md`](../tasks/plan-rewrite.md).

## Security boundary

Local requests authenticate before routing and receive a server-owned `RequestContext`. Desktop and CLI control, MCP, provider-child, node-proxy, and internal calls have distinct scopes. Windows uses a SID-scoped protected named pipe; Unix uses a private runtime directory and socket plus peer credentials.

This boundary does not isolate a hostile process already running as the same OS user. FullAccess provider sessions intentionally have host-user authority. See [`CLAUDE.md`](../CLAUDE.md#server-and-local-ipc) for the exact contract.

## Historical wiki

The pages below document the removed Iced multiplexer and remain available for implementation history. They are not authoritative for the active GPUI workspace, current commands, crate names, IPC security, or release readiness.

- [`OVERVIEW.md`](OVERVIEW.md)
- [`GETTING_STARTED.md`](GETTING_STARTED.md)
- [`core/`](core/)
- [`api/CLI_REFERENCE.md`](api/CLI_REFERENCE.md)
- [`features/`](features/)
- [`operations/RELEASE.md`](operations/RELEASE.md)
- [`GLOSSARY.md`](GLOSSARY.md)

Older Electron material is under [`archive/`](archive/). Superseded implementation plans and specs are under [`superpowers/`](superpowers/). [`tasks/plan-repo.md`](../tasks/plan-repo.md) retains the native rewrite history and Section 12 UI specification, but it is not the current roadmap.

## Research

Decision-oriented research remains under [`research/`](research/). Verify code and external facts again before relying on an older comparison.
