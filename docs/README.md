# PandaMUX Documentation

This index routes contributors to current sources for the GPUI chat-first
workspace. Read the root [`Cargo.toml`](../Cargo.toml) for the version;
documentation does not duplicate it.

> **Latest Updates (October 2026):**
> - **v0.60.96:** Checked Phase 5C budget reservations, grants, usage reconciliation, and conservation diagnostics.
> - **v0.60.95:** Durable shared execution admission, multi-scope caps, interactive reserve, and lease recovery.
> - **v0.60.94:** Crash-safe Phase 5C attempt worktrees and private checkpoints.
> - **v0.60.93:** Replay-safe Phase 5C run transitions and effect outbox.
> - **v0.60.92:** Migration 14 and the durable Phase 5C execution store.
> - **v0.60.89:** Final Phase 5B local and native Windows acceptance reconciliation.
> - **v0.60.88:** Native terminal confirmation controls now publish their disabled UI Automation state.
> - **v0.60.87:** Physical keyboard traversal restored for both confirmation actions.
> - **v0.60.85:** Native Windows trusted storage, ACL, reparse, Git durability, and long-path hardening.
> - **v0.60.84:** Phase 5B maintained documentation and exact local acceptance record.

## Authority order

When sources disagree, use this order and correct the lower-authority source:

1. implementation and manifests;
2. [`CHANGELOG.md`](../CHANGELOG.md);
3. [`CLAUDE.md`](../CLAUDE.md);
4. this maintained wiki;
5. archived plans, acceptance history, and research.

[`AGENTS.md`](../AGENTS.md) is the routing page for agent tooling.
[`tasks/plan-rewrite.md`](../tasks/plan-rewrite.md) is the active implementation
and acceptance contract. [`tasks/plan-repo.md`](../tasks/plan-repo.md) retains
rewrite history and the Section 12 UI specification, but it is not the current
roadmap.

## Start here

| Goal | Page |
|---|---|
| Understand the product and current architecture | [Overview](OVERVIEW.md) |
| Build, run, and verify the workspace | [Getting Started](GETTING_STARTED.md) |
| Understand process and state ownership | [Architecture](core/ARCHITECTURE.md) |
| Change Rust dependencies or toolchains safely | [Toolchain and Dependency Policy](operations/DEPENDENCIES.md) |
| Find runtime configuration and profile rules | [Configuration](core/CONFIGURATION.md) |
| Understand trusted sources, activation, and review | [Trusted Configuration](features/TRUSTED_CONFIGURATION.md) |
| Use the command-line client | [CLI Reference](api/CLI_REFERENCE.md) |
| Understand release gates without implying publication | [Release and Packaging](operations/RELEASE.md) |
| Decode project terminology | [Glossary](GLOSSARY.md) |

## Current generated wiki

The generated pages below are maintained from [`_toc.yaml`](_toc.yaml). Every
page names its source files, uses bounded line citations, and marks generated
sections with stable AUTOGEN identifiers.

### Core

- [Architecture](core/ARCHITECTURE.md)
- [Core Domain](core/CORE_DOMAIN.md)
- [Terminal Engine](core/TERMINAL_ENGINE.md)
- [GPUI Desktop](core/UI_SHELL.md)
- [Application Runtime](core/APP_RUNTIME.md)
- [Configuration](core/CONFIGURATION.md)

### Features and interfaces

- [CLI Reference](api/CLI_REFERENCE.md)
- [Local IPC and Security](features/NAMED_PIPE_IPC.md)
- [SSH Remote Nodes](features/SSH_REMOTE.md)
- [Agent Orchestration](features/AGENT_ORCHESTRATION.md)
- [Trusted Configuration](features/TRUSTED_CONFIGURATION.md)
- [Provider Routing and Capacity](features/PROVIDER_ROUTING.md)

### Operations and reference

- [Toolchain and Dependency Policy](operations/DEPENDENCIES.md)
- [Release and Packaging](operations/RELEASE.md)
- [Glossary](GLOSSARY.md)
- [Generation Metadata](_meta/GENERATION.md)
- [Generation Summary](_meta/SUMMARY.md)

## Current architecture map

| Area | Source of truth | Responsibility |
|---|---|---|
| Domain | [`crates/pandamux-core`](../crates/pandamux-core) | Framework-free IDs, threads, events, trusted-configuration identities, mutation records, policy, routing, and usage types |
| Orchestration | [`crates/pandamux-orchestrator`](../crates/pandamux-orchestrator) | Pure schema, candidate inspection, bundle, DAG, routing, approval, model-parse, and replay engine |
| Wire protocol | [`crates/pandamux-protocol`](../crates/pandamux-protocol) | Versioned JSON-RPC, trusted-configuration, subscriptions, capabilities, and Hub-to-node contracts |
| Providers | [`crates/pandamux-providers`](../crates/pandamux-providers) | Provider drivers, profiles, discovery, parsing, and process supervision |
| SSH and bootstrap | [`crates/pandamux-remote`](../crates/pandamux-remote) | SSH pool, exec, SFTP, bootstrap, worktree synchronization, and proxy tunnels |
| Server | [`crates/pandamux-server`](../crates/pandamux-server) | Durable store, managed configuration sources, activation, distribution, mutation review, request contexts, provider execution, PTYs, Hub, and node runtime |
| Client projections | [`crates/pandamux-client`](../crates/pandamux-client) | Event-derived state, trusted-configuration review state, transport, usage projection, and token smoothing |
| Desktop | [`crates/pandamux-desktop`](../crates/pandamux-desktop) | GPUI chat shell, trusted configuration, timeline, composer, settings, surfaces, terminal grid, and usage views |
| CLI | [`crates/pandamux-cli`](../crates/pandamux-cli) | Authenticated commands and restricted MCP stdio adapter |
| Terminal primitives | [`crates/pandamux-term`](../crates/pandamux-term) | Separately selectable `grid` and `pty` features |
| Dependency validation | [`tools/pandamux-boundary-check`](../tools/pandamux-boundary-check) | Target-resolved ownership, feature, snapshot, embedding, and native-stack checks |

The server owns durable state, provider processes, and local PTYs. The desktop
and CLI are clients. Remote Linux nodes run `pandamux-server daemon` and are
reached through a versioned SSH exec proxy; PandaMUX does not open a remote TCP
control listener.

## Acceptance records and current boundary

Four manually maintained pages separate the ongoing Phase 4 closeout from the
Phase 5A pure-domain checkpoint and Phase 5B local implementation evidence:

- [Usage Observability and Transcript Privacy](features/USAGE_OBSERVABILITY.md)
  defines the current data, privacy, UI, capacity, and failover contracts.
- [Phase 4 Acceptance Record](operations/PHASE4_ACCEPTANCE.md) records exact
  commits, versions, artifact hashes, commands, results, failures, limitations,
  cleanup, and preserved evidence.
- [Phase 5A Deterministic Orchestrator Acceptance](operations/PHASE5A_ACCEPTANCE.md)
  records the exact pure-domain checkpoint chain, local command matrix,
  rejected 0.60.75 candidate, accepted 0.60.76 repair, and absent runtime or
  native evidence.
- [Phase 5B Trusted Configuration Acceptance](operations/PHASE5B_ACCEPTANCE.md)
  maps every AC-B01 through AC-B18 row to retained evidence, records the exact
  checkpoint and native-repair chain, and keeps external and production
  limitations explicit.

The current recovery branch includes exact evidence for accounting,
reconciliation, provider and model discovery, native Windows accessibility and
local IPC, restart and side-effect boundaries, external-service interruption,
and Codex credit serviceability. Exact 0.60.62 also closes the affected
provider-login and Surfaces native matrix on the physical MSI laptop. Exact
0.60.64 commit `a41c5bd` closes both physical P4-C6 rows with one real provider
turn across a 30-minute Wi-Fi outage and one remote terminal across 39 minutes
of Connected Modern Standby. Those accepted rows do not establish complete
Phase 4 production acceptance.

The following gates remain open:

- a real pre-output provider capacity failure followed by the specified safe
  failover behavior;
- final P4-C7 synchronization after the remaining P4-C5 row stabilizes.

Phase 5A pure-domain work passed its exact local acceptance matrix under the
explicit sequencing exception recorded in the active plan. Its versioned
schemas, canonical bundle, DAG and routing policy, approvals, bounded model
parsing, and replay state do not
convert either open Phase 4 row into a pass. They also do not establish a
database, effect interpreter, provider dispatch, worktree, RPC, CLI, desktop,
or live-provider path. Phase 5B now supplies trusted configuration persistence,
managed Git, source inspection, exact project trust, immutable activation,
node-distribution contracts, mutation review, and desktop controls. Exact
0.60.88 passes its complete local matrix and the required native Windows
storage, hidden-Git-child, atomic-recovery, keyboard, and UI Automation checks.
No external Git/SFTP transfer or production activation was exercised. Phase 5
evidence remains separate from exact 0.60.64 Phase 4 runtime evidence and the
retained Galahad databases. This generated-wiki refresh remains an interim P4-C7
synchronization; it does not claim that Phase 4 is complete, released,
deployed, signed, published, or served.

## Security boundary

Local requests authenticate before routing and receive a server-owned request
context. Desktop and CLI control, MCP, provider-child, node-proxy, and internal
calls use distinct capabilities. Windows uses a SID-scoped protected named
pipe. Unix uses a private runtime directory and socket plus peer credentials.

This boundary does not isolate a hostile process already running as the same OS
user. A FullAccess provider session intentionally has host-user authority. See
[`CLAUDE.md`](../CLAUDE.md#server-and-local-ipc) and
[Local IPC and Security](features/NAMED_PIPE_IPC.md).

## Historical material

Removed Electron and Iced implementations remain under [`archive/`](archive/)
and [`superpowers/`](superpowers/) for history. The former shell-integration
page is retained at
[`archive/iced-multiplexer/SHELL_INTEGRATION.md`](archive/iced-multiplexer/SHELL_INTEGRATION.md).
These files are not current implementation guidance.

The [Phase 3 Acceptance Record](operations/PHASE3_ACCEPTANCE.md) remains a
historical evidence record. Decision-oriented product research remains under
[`research/`](research/); verify implementation and external facts again
before relying on an older comparison.

---

**Last Updated:** October 2026
