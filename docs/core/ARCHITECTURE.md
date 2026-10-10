<!-- PAGE_ID: pandamux_03_architecture -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [Cargo.toml:1-48](../../Cargo.toml#L1-L48)
- [CLAUDE.md:218-377](../../CLAUDE.md#L218-L377)
- [crates/pandamux-client/src/lib.rs:1-702](../../crates/pandamux-client/src/lib.rs#L1-L702)
- [crates/pandamux-desktop/src/lib.rs:1-558](../../crates/pandamux-desktop/src/lib.rs#L1-L558)
- [crates/pandamux-orchestrator/src/lib.rs:1-55](../../crates/pandamux-orchestrator/src/lib.rs#L1-L55)
- [crates/pandamux-orchestrator/src/compatibility.rs:1-501](../../crates/pandamux-orchestrator/src/compatibility.rs#L1-L501)
- [crates/pandamux-orchestrator/src/bundle.rs:1-1157](../../crates/pandamux-orchestrator/src/bundle.rs#L1-L1157)
- [crates/pandamux-orchestrator/src/state.rs:1-1300](../../crates/pandamux-orchestrator/src/state.rs#L1-L1300)
- [crates/pandamux-protocol/src/lib.rs:1-245](../../crates/pandamux-protocol/src/lib.rs#L1-L245)
- [crates/pandamux-server/src/lib.rs:1-159](../../crates/pandamux-server/src/lib.rs#L1-L159)
- [crates/pandamux-server/src/attempt_watchdog.rs:11-230](../../crates/pandamux-server/src/attempt_watchdog.rs#L11-L230)
- [crates/pandamux-server/src/attempt_worktree.rs:1280-1402](../../crates/pandamux-server/src/attempt_worktree.rs#L1280-L1402)
- [crates/pandamux-server/src/remote_execution.rs:101-851](../../crates/pandamux-server/src/remote_execution.rs#L101-L851)
- [crates/pandamux-server/src/run_execution_service.rs:123-330](../../crates/pandamux-server/src/run_execution_service.rs#L123-L330)
- [crates/pandamux-server/src/run_recovery.rs:37-225](../../crates/pandamux-server/src/run_recovery.rs#L37-L225)
- [crates/pandamux-server/src/store/orchestration_phase5c.rs:26-490](../../crates/pandamux-server/src/store/orchestration_phase5c.rs#L26-L490)
- [tools/pandamux-boundary-check/src/lib.rs:1-761](../../tools/pandamux-boundary-check/src/lib.rs#L1-L761)

</details>

# Architecture

> **Related Pages**: [Core Domain](CORE_DOMAIN.md), [Application Runtime](APP_RUNTIME.md), [Local IPC and Security](../features/NAMED_PIPE_IPC.md)

---

<!-- BEGIN:AUTOGEN pandamux_03_architecture_crates -->
## Workspace and Crate Boundaries

PandaMUX has ten product crates plus one workspace validation tool. Domain, orchestration, and wire contracts sit below server, client, CLI, and desktop composition; terminal and SSH implementation details remain in dedicated crates rather than leaking into those contracts ([Cargo.toml:1-14](../../Cargo.toml#L1-L14), [lib.rs:368-416](../../tools/pandamux-boundary-check/src/lib.rs#L368-L416)).

| Crate | Primary responsibility | Boundary evidence |
|---|---|---|
| `pandamux-core` | Framework-free IDs, threads, events, policy, settings, routing, and usage types | Foundation for pure domain consumers ([CLAUDE.md:222-222](../../CLAUDE.md#L222-L222)) |
| `pandamux-orchestrator` | Pure Phase 5A schema, bundle, DAG, routing, approval, proposal-parse, and run-state engine | Its public surface is value-to-value behavior, and forbidden transitive runtime implementations are rejected ([lib.rs:3-55](../../crates/pandamux-orchestrator/src/lib.rs#L3-L55), [lib.rs:614-647](../../tools/pandamux-boundary-check/src/lib.rs#L614-L647)) |
| `pandamux-protocol` | Versioned JSON-RPC, subscriptions, terminal, settings, usage, and proxy messages | Carries wire values without importing implementation crates ([CLAUDE.md:224-224](../../CLAUDE.md#L224-L224)) |
| `pandamux-providers` | Provider drivers and process supervision | Runtime implementation remains outside the pure orchestrator ([CLAUDE.md:225-240](../../CLAUDE.md#L225-L240)) |
| `pandamux-remote` | SSH pool, bootstrap, command execution, SFTP, proxy tunnels, and worktree bundles | Server remains the production composition owner ([CLAUDE.md:226-242](../../CLAUDE.md#L226-L242)) |
| `pandamux-term` | Feature-split grid and PTY primitives | Server resolves exactly `pty`; desktop resolves exactly `grid` ([lib.rs:524-612](../../tools/pandamux-boundary-check/src/lib.rs#L524-L612)) |
| `pandamux-server` | Store, orchestration effects, worktrees, admission, budgets, artifacts, recovery, providers, terminals, remote environments, and Hub/node runtime | Exposes the production composition surface and keeps runtime implementations out of the pure engine ([lib.rs:1-159](../../crates/pandamux-server/src/lib.rs#L1-L159)) |
| `pandamux-client` | Event projections, client state, usage presentation, and transport seam | Exports projections and a mockable transport without UI types ([lib.rs:1-19](../../crates/pandamux-client/src/lib.rs#L1-L19)) |
| `pandamux-desktop` | GPUI shell and the server bridge | Re-exports view state and bridge types from focused modules ([lib.rs:3-60](../../crates/pandamux-desktop/src/lib.rs#L3-L60)) |
| `pandamux-cli` | Authenticated command-line and MCP adapter | Client entry point without UI ownership ([CLAUDE.md:230-230](../../CLAUDE.md#L230-L230)) |
| `pandamux-boundary-check` | Target-resolved Cargo graph and feature validation | Validates the locked graph for five declared targets and reports stable violation codes ([lib.rs:9-15](../../tools/pandamux-boundary-check/src/lib.rs#L9-L15), [lib.rs:368-416](../../tools/pandamux-boundary-check/src/lib.rs#L368-L416)) |

```mermaid
graph TD
    Core["pandamux-core"] --> Protocol["pandamux-protocol"]
    Core --> Orchestrator["pandamux-orchestrator"]
    Core --> Providers["pandamux-providers"]
    Core --> Remote["pandamux-remote"]
    Core --> Term["pandamux-term"]
    Protocol --> Server["pandamux-server"]
    Providers --> Server
    Remote --> Server
    Term --> Server
    Protocol --> Client["pandamux-client"]
    Client --> Desktop["pandamux-desktop"]
    Server --> Desktop
    Protocol --> Cli["pandamux-cli"]
    Checker["pandamux-boundary-check"] -. validates resolved graphs .-> Orchestrator
    Checker -. validates resolved graphs .-> Server
    Checker -. validates resolved graphs .-> Desktop
```

Sources: [Cargo.toml:1-48](../../Cargo.toml#L1-L48), [CLAUDE.md:218-249](../../CLAUDE.md#L218-L249), [lib.rs:1-55](../../crates/pandamux-orchestrator/src/lib.rs#L1-L55), [lib.rs:1-159](../../crates/pandamux-server/src/lib.rs#L1-L159), [lib.rs:368-449](../../tools/pandamux-boundary-check/src/lib.rs#L368-L449), [lib.rs:524-647](../../tools/pandamux-boundary-check/src/lib.rs#L524-L647)
<!-- END:AUTOGEN pandamux_03_architecture_crates -->

---

<!-- BEGIN:AUTOGEN pandamux_03_architecture_ownership -->
## State and Process Ownership

The server owns durable and executable state; clients own projections and presentation state. This prevents the desktop from becoming a second authority for threads, provider processes, terminals, event ordering, or remote routing ([CLAUDE.md:242-245](../../CLAUDE.md#L242-L245), [CLAUDE.md:251-267](../../CLAUDE.md#L251-L267)).

| Owner | State or process | Client-facing consequence |
|---|---|---|
| Server | SQLite store, orchestration effect interpreter, admission and budget services, worktree and artifact owners, recovery, provider drivers, terminal manager, attachments, environment router, usage services | Mutations are submitted to server services; neither the desktop nor the pure engine writes executable state directly ([lib.rs:41-151](../../crates/pandamux-server/src/lib.rs#L41-L151)) |
| Pure orchestrator | Authored-schema parsing, canonical bundles, validated plans, deterministic routes, command decisions, event reduction, and replay | Produces serializable events and effects but does not persist or execute them ([lib.rs:9-55](../../crates/pandamux-orchestrator/src/lib.rs#L9-L55), [state.rs:55-356](../../crates/pandamux-orchestrator/src/state.rs#L55-L356)) |
| Client crate | Thread/run projections, token smoothing, usage view state | Rendering state is derived from server events and can be rebuilt ([lib.rs:7-19](../../crates/pandamux-client/src/lib.rs#L7-L19)) |
| Desktop | GPUI views, composer, picker, terminal view, tray, and bridge handle | UI modules consume projections and send commands through the bridge ([lib.rs:21-60](../../crates/pandamux-desktop/src/lib.rs#L21-L60)) |
| Protocol | Serializable request, response, event, and proxy contracts | Both sides share data types without importing the other's implementation ([lib.rs:1-22](../../crates/pandamux-protocol/src/lib.rs#L1-L22)) |

`RunExecutionService` is the single adapter between pure run transitions and
durable effect delivery. It composes the existing `ThreadManager` session
owner, shared admission, pinned bundle materialization, authenticated remote
transport, blackboard storage, watchdog, and recovery services without moving
provider-session ownership into the orchestrator
([run_execution_service.rs:253-330](../../crates/pandamux-server/src/run_execution_service.rs#L253-L330)).

The server's public composition surface makes these responsibilities explicit:
`Store`, `RunExecutionService`, `ThreadManager`, `AttemptWorktreeService`,
`NodeExecutionService`, `RunRecoveryService`, `TerminalServerManager`, and
`EnvironmentRouter` are server exports, while the client exports projection
and formatting types ([lib.rs:41-151](../../crates/pandamux-server/src/lib.rs#L41-L151), [lib.rs:7-19](../../crates/pandamux-client/src/lib.rs#L7-L19)).

Sources: [CLAUDE.md:218-377](../../CLAUDE.md#L218-L377), [lib.rs:9-55](../../crates/pandamux-orchestrator/src/lib.rs#L9-L55), [state.rs:55-356](../../crates/pandamux-orchestrator/src/state.rs#L55-L356), [lib.rs:41-151](../../crates/pandamux-server/src/lib.rs#L41-L151), [run_execution_service.rs:253-330](../../crates/pandamux-server/src/run_execution_service.rs#L253-L330), [lib.rs:7-19](../../crates/pandamux-client/src/lib.rs#L7-L19)
<!-- END:AUTOGEN pandamux_03_architecture_ownership -->

---

<!-- BEGIN:AUTOGEN pandamux_03_architecture_local-flow -->
## Local Request and Event Flow

Local desktop and CLI traffic uses versioned JSON-RPC. The protocol crate exports the request/response types, `system.hello` contracts, subscriptions, and event envelopes; the server authenticates and routes them, and the client reduces returned events into projections ([lib.rs:48-76](../../crates/pandamux-protocol/src/lib.rs#L48-L76), [lib.rs:7-13](../../crates/pandamux-client/src/lib.rs#L7-L13)).

```mermaid
sequenceDiagram
    participant D as Desktop or CLI
    participant S as Local server
    participant R as Router
    participant P as Client projection

    D->>S: system.hello with capability
    activate S
    S->>R: authenticated request context
    R-->>S: hello result
    S-->>D: JSON-RPC response
    deactivate S
    D->>S: system.subscribe with cursor
    S-->>D: event envelopes
    D->>P: apply ordered events
```

The shared wire surface includes thread, provider, terminal, settings, schedule, attachment, filesystem, Git, usage, and worktree contracts. `EventEnvelope` adds environment identity and sequence information around domain events, allowing projections to remain transport-agnostic ([lib.rs:24-96](../../crates/pandamux-protocol/src/lib.rs#L24-L96)).

Sources: [lib.rs:24-96](../../crates/pandamux-protocol/src/lib.rs#L24-L96), [lib.rs:7-19](../../crates/pandamux-client/src/lib.rs#L7-L19), [lib.rs:36-39](../../crates/pandamux-desktop/src/lib.rs#L36-L39)
<!-- END:AUTOGEN pandamux_03_architecture_local-flow -->

---

<!-- BEGIN:AUTOGEN pandamux_03_architecture_remote-flow -->
## Remote Node Flow

The same server binary composes a desktop-machine Hub and remote Linux nodes. The Hub owns environment routing and connects to a node through the SSH-backed proxy transport; protocol contracts remain identical on either side ([CLAUDE.md:505-514](../../CLAUDE.md#L505-L514), [lib.rs:31-47](../../crates/pandamux-server/src/lib.rs#L31-L47)).

```mermaid
sequenceDiagram
    participant U as Desktop client
    participant H as Hub server
    participant T as SSH proxy tunnel
    participant N as Node server

    U->>H: environment-scoped RPC
    activate H
    H->>T: proxy request frame
    activate T
    T->>N: forward JSON-RPC request
    activate N
    N-->>T: response and event frames
    deactivate N
    T-->>H: ordered proxy frames
    deactivate T
    H-->>U: response and event envelopes
    deactivate H
```

`pandamux-server` exposes both sides of this boundary: `serve_proxy_connection` runs on the node, while `ProxyEnvironmentTransport` and `EnvironmentRouter` run on the Hub ([lib.rs:31-47](../../crates/pandamux-server/src/lib.rs#L31-L47)). The protocol crate carries the proxy envelope and environment-scoped RPC types without importing the SSH implementation ([lib.rs:4-22](../../crates/pandamux-protocol/src/lib.rs#L4-L22)).

Phase 5C remote attempts add a private two-store handshake above that transport.
The authenticated environment comes from request context, while repository
prepare, attempt prepare, dispatch, status, cancel, ordered event, and
acknowledgement records bind the exact run, effect, baseline, bundle, route,
lease generation, and idempotency identity
([orchestration_execution.rs:9-184](../../crates/pandamux-protocol/src/orchestration_execution.rs#L9-L184), [remote_execution.rs:101-851](../../crates/pandamux-server/src/remote_execution.rs#L101-L851)).

Sources: [CLAUDE.md:505-516](../../CLAUDE.md#L505-L516), [lib.rs:71-118](../../crates/pandamux-server/src/lib.rs#L71-L118), [lib.rs:1-22](../../crates/pandamux-protocol/src/lib.rs#L1-L22), [orchestration_execution.rs:9-230](../../crates/pandamux-protocol/src/orchestration_execution.rs#L9-L230), [remote_execution.rs:101-851](../../crates/pandamux-server/src/remote_execution.rs#L101-L851)
<!-- END:AUTOGEN pandamux_03_architecture_remote-flow -->

---

<!-- BEGIN:AUTOGEN pandamux_03_architecture_persistence -->
## Durability and Replay

Durable state and event ordering belong to the server, while the client projection is deliberately reconstructable. This split lets local reconnects and remote proxy reconnects resume from sequence cursors instead of treating the desktop's in-memory view as canonical ([CLAUDE.md:245-253](../../CLAUDE.md#L245-L253)).

| Durable concern | Authority | Replay consumer |
|---|---|---|
| Threads, turns, events, settings, schedules, usage, and attachments | Server `Store` ([lib.rs:47-58](../../crates/pandamux-server/src/lib.rs#L47-L58)) | Desktop/client projections |
| Thread and run timeline state | Ordered protocol event envelopes ([lib.rs:67-95](../../crates/pandamux-protocol/src/lib.rs#L67-L95)) | `ThreadProjection` and `RunProjection` |
| Phase 5C runs, effects, attempts, queues, leases, worktrees, budgets, artifacts, progress, reviews, and node receipts | Migration 14 tables plus replay verification ([migrations.rs:1711-2084](../../crates/pandamux-server/src/store/migrations.rs#L1711-L2084), [orchestration_phase5c.rs:648-933](../../crates/pandamux-server/src/store/orchestration_phase5c.rs#L648-L933)) | `RunExecutionService`, recovery, and future Phase 5D projections |
| Remote event cursor | Hub-side environment transport ([lib.rs:31-47](../../crates/pandamux-server/src/lib.rs#L31-L47)) | Reconnected node proxy |
| Terminal attachment state | Server terminal manager and terminal RPC offsets ([lib.rs:73-77](../../crates/pandamux-protocol/src/lib.rs#L73-L77)) | Reattached desktop terminal grid |

The protocol test surface verifies that event envelopes retain their subscription id and sequence across serialization, and that terminal attach responses preserve replay offset and truncation state ([lib.rs:129-155](../../crates/pandamux-protocol/src/lib.rs#L129-L155), [lib.rs:213-243](../../crates/pandamux-protocol/src/lib.rs#L213-L243)).

Server startup keeps orchestration intake closed while one incarnation owner
replays and verifies nonterminal runs. Shutdown closes intake, stops new
claims, joins or quarantines every active attempt owner, checkpoints SQLite,
and refuses to report graceful completion while live owners or checkpoint debt
remain ([run_recovery.rs:37-225](../../crates/pandamux-server/src/run_recovery.rs#L37-L225), [run_execution_service.rs:1260-1490](../../crates/pandamux-server/src/run_execution_service.rs#L1260-L1490), [main.rs:86-146](../../crates/pandamux-server/src/main.rs#L86-L146)).

Sources: [CLAUDE.md:245-377](../../CLAUDE.md#L245-L377), [lib.rs:67-95](../../crates/pandamux-protocol/src/lib.rs#L67-L95), [lib.rs:129-155](../../crates/pandamux-protocol/src/lib.rs#L129-L155), [lib.rs:213-243](../../crates/pandamux-protocol/src/lib.rs#L213-L243), [migrations.rs:1711-2084](../../crates/pandamux-server/src/store/migrations.rs#L1711-L2084), [orchestration_phase5c.rs:648-933](../../crates/pandamux-server/src/store/orchestration_phase5c.rs#L648-L933), [run_recovery.rs:37-225](../../crates/pandamux-server/src/run_recovery.rs#L37-L225), [run_execution_service.rs:1260-1490](../../crates/pandamux-server/src/run_execution_service.rs#L1260-L1490), [main.rs:86-146](../../crates/pandamux-server/src/main.rs#L86-L146)
<!-- END:AUTOGEN pandamux_03_architecture_persistence -->

---

<!-- BEGIN:AUTOGEN pandamux_03_architecture_invariants -->
## Architectural Invariants

The repository encodes its most important boundaries as build-time checks and narrow public APIs rather than relying only on convention.

- `pandamux-core` stays independent of UI, terminal-grid, and database implementations ([CLAUDE.md:238-240](../../CLAUDE.md#L238-L240)).
- `pandamux-orchestrator` stays free of runtimes, filesystem, network, storage, UI, provider, keyring, and embedding implementations ([lib.rs:3-7](../../crates/pandamux-orchestrator/src/lib.rs#L3-L7), [lib.rs:614-647](../../tools/pandamux-boundary-check/src/lib.rs#L614-L647)).
- `pandamux-protocol` contains wire contracts and cannot import implementation crates ([CLAUDE.md:241-241](../../CLAUDE.md#L241-L241)).
- `pandamux-server` is the production composition owner for SSH remoting, PTYs, provider execution, persistence, and routing ([CLAUDE.md:242-245](../../CLAUDE.md#L242-L245), [CLAUDE.md:251-267](../../CLAUDE.md#L251-L267)).
- `RunExecutionService` owns Phase 5C effect interpretation and explicit lifecycle joins, while `ThreadManager` remains the sole provider-session owner ([run_execution_service.rs:253-330](../../crates/pandamux-server/src/run_execution_service.rs#L253-L330), [thread_manager.rs:174-385](../../crates/pandamux-server/src/thread_manager.rs#L174-L385)).
- Attempt worktree ownership is checked against the canonical app-data root, private descendant metadata, and every queryable path component; Windows drive, UNC, and verbatim prefixes are carried into the rooted path without treating the prefix alone as a filesystem object ([attempt_worktree.rs:1280-1402](../../crates/pandamux-server/src/attempt_worktree.rs#L1280-L1402)).
- Desktop code holds rendering-oriented state and sends intents; it is not a durable-state writer ([CLAUDE.md:245-245](../../CLAUDE.md#L245-L245)).
- Request identity and capability scope are assigned by the accepting server transport, not accepted from request JSON ([CLAUDE.md:257-263](../../CLAUDE.md#L257-L263)).
- Full-access providers have host-user authority; the local boundary is per OS user and is not hostile same-user process isolation ([CLAUDE.md:265-267](../../CLAUDE.md#L265-L267)).
- Direct registry dependencies must be exact-pinned; path dependencies remain exempt ([lib.rs:418-449](../../tools/pandamux-boundary-check/src/lib.rs#L418-L449)).
- The boundary check reconstructs each workspace root's resolved feature graph per declared target, so indirect dependencies and default-feature leakage are visible ([lib.rs:140-339](../../tools/pandamux-boundary-check/src/lib.rs#L140-L339), [lib.rs:341-416](../../tools/pandamux-boundary-check/src/lib.rs#L341-L416)).
- GPUI snapshot alignment, desktop-only GPUI ownership, PTY/grid separation, future embedding isolation, and native-stack uniqueness are executable invariants ([lib.rs:451-698](../../tools/pandamux-boundary-check/src/lib.rs#L451-L698)).

```bash
cargo test --locked -p pandamux-boundary-check
cargo run --locked -p pandamux-boundary-check --quiet
```

The fixture test makes the guard falsifiable before the real locked graph is accepted ([CLAUDE.md:246-249](../../CLAUDE.md#L246-L249)).

Sources: [CLAUDE.md:236-267](../../CLAUDE.md#L236-L267), [run_execution_service.rs:253-330](../../crates/pandamux-server/src/run_execution_service.rs#L253-L330), [thread_manager.rs:174-385](../../crates/pandamux-server/src/thread_manager.rs#L174-L385), [attempt_worktree.rs:1280-1402](../../crates/pandamux-server/src/attempt_worktree.rs#L1280-L1402), [lib.rs:140-449](../../tools/pandamux-boundary-check/src/lib.rs#L140-L449), [lib.rs:451-698](../../tools/pandamux-boundary-check/src/lib.rs#L451-L698)
<!-- END:AUTOGEN pandamux_03_architecture_invariants -->

---
