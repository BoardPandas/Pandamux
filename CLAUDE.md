# PandaMUX Development Guide

PandaMUX is a native, chat-first desktop for supervising local and remote AI coding agents. The current workspace uses GPUI through `gpui-kit`, an authenticated server-owned JSON-RPC control plane, `alacritty_terminal` for the desktop grid, `portable-pty` for server-owned local terminals, and `russh` for remote Linux composition.

The active implementation is not the removed Iced multiplexer. [`tasks/plan-repo.md`](tasks/plan-repo.md) is historical except for its retained Section 12 UI specification. [`tasks/plan-rewrite.md`](tasks/plan-rewrite.md) is the current implementation and acceptance contract.

## Authority and status

For current facts, use this order:

1. Implementation and manifests
2. `CHANGELOG.md`
3. This guide
4. The maintained [`docs/README.md`](docs/README.md) index

The only version source is `[workspace.package] version` in the root [`Cargo.toml`](Cargo.toml). Crates inherit it with `version.workspace = true`.

Phase 4A through 4F implementation is recorded after the R0 development gate,
but the Phase 4 exit matrix remains open. R0.1 preserves the unfinished usage
prototype; R0.2 completes production proxy routing; R0.3 enforces exact
provider-instance execution; R0.4 secures the local control boundary. R0.5 has
exact live evidence for Galahad and native Windows IPC. The two physical rows
deferred for development sequencing on 2026-09-29 later passed on exact 0.60.64
commit `a41c5bd40861fb37de7a53252d96dcb055828362`: a real provider turn survived
more than 30 minutes of physical Wi-Fi disconnection, and a remote full-screen
terminal survived 39 minutes of Connected Modern Standby. Exact 0.60.15 commit
`3a77382` passes P4-C1, P4-C2, and
P4-C4. Exact 0.60.41 commit
`c6a20ff4f768747d0dadd59b88b40d3e20f7674d` passes the unchanged P4-C3
terminal, accessibility, visual, scaling, and IPC rows on one clean native
artifact set, including controlled successful-login interaction, 100 and 200
percent scaling, High Contrast, disabled animations, visual review, genuine
Narrator with exact READY, ECHO, and reopened-output phrases, and the complete
local plus independent-peer named-pipe security matrix. Exact 0.60.43 commit
`372f2ae83e67a62cd3b382e5db52af96d00a85ce` passes the complete local gate,
schema migration, live driver model discovery, and a real Codex turn through
the exact configured instance and selected model. Its exact limited-user
Windows artifacts also pass native build and headless smoke, visible no-route
fallback into Provider Settings, and an authenticated picker-created thread
whose `codex-default` and `gpt-6.1-sol` route matches the exact-build CLI. P4-C3 is
Pass again. A separate supporting live run later killed the exact remote proxy
after visible text and one successful file-writing shell tool. Reconnect kept
one protected attempt and the one-line marker without replay. Exact 0.60.47
then repaired unresolved-tool false success and passed a clean supporting run
with one approval plus one loopback HTTP POST across abrupt proxy loss.
Reconnect kept one request, one terminal tool ledger row, one protected
attempt, one settled turn, and one matching managed usage observation without
replay. Exact 0.60.49 then made terminal tool-result persistence atomic and
passed a controlled Galahad restart while one real approval result remained
durably Pending. Two identical post-restart submissions refused redispatch and
the loopback request count remained one. The same exact server then passed an
isolated process crash while a real terminal tool-result transaction held the
SQLite writer lock. Restart retained the complete pre-result state and one
external request without inventing or replaying a result. Exact 0.60.52 then
repaired continuation-route persistence and passed a controlled-source,
real-Codex continuation restart. The target route survived persistence, one
loopback request occurred before the complete five-process descendant tree was
killed, restart failed the continuation and attempt without replay, and two
duplicate continuation calls refused redispatch. The unchanged exact server
later passed a real external-service interruption. Organization policy selected
real `claude-default` plus
`claude-3-7-sonnet-latest`; exactly one disposable HTTPS request committed
before the full provider process tree was killed, and restart did not replay
the request or invent a terminal result. The endpoint was revoked and the
isolated credential was restored during verified cleanup. Subsequent capacity
diagnostics are retained and rejected: Codex remained serviceable, completed
the requested turns, and emitted no capacity failure or failover. They exposed
first a false hard limit from a 100 percent window, then an overbroad reading of
`ordinaryUsageAllowed=false` while paid credits remained available. Version
0.60.56 treats included usage and credits as separate serviceability paths and
handles structured terminal Codex errors while allowing the app server to own
errors it will retry. Its exact Galahad artifact passed a live
credit-serviceability route proof with capacity-aware routing enabled, one
completed Codex attempt, and no retry or Claude fallback. Real total provider
capacity exhaustion is still open.
A supporting Hyper-V rehearsal on exact 0.60.43 first exercised the
provider-network and terminal recovery paths. Exact 0.60.64 then passed the two
required physical P4-C6 rows with event, usage, offset, replay,
abandoned-proxy, SleepStudy, and cleanup evidence. Exact 0.60.17 commit
`2f56a27` partially passes P4-C5 with live scanner and Hub durability,
conservative capacity projections and route explanations, and independent
rollback controls. Real pre-output provider capacity exhaustion remains open.
The exact results are recorded in
[`docs/operations/PHASE4_ACCEPTANCE.md`](docs/operations/PHASE4_ACCEPTANCE.md).
The remaining capacity-failover row and final post-P4-C5 documentation archive
keep Phase 4 open.

On 2026-10-09 the user explicitly authorized the Phase 5 dependency and
boundary gate to proceed while that provider-controlled P4-C5 row remains
unavailable. This sequencing exception does not mark P4-C5 or P4-C7 complete.
Phase 5A now implements the pure, versioned orchestration domain: authored
schema validation, canonical bundle compilation, DAG validation, deterministic
two-step routing, approval reconciliation, bounded model-output parsing, and
replayable command/event state. Phase 5A itself remains pure and does not own
SQLite, filesystem effects, network transport, provider dispatch, worktrees,
RPC, CLI, or desktop orchestration.

Phase 5B now implements the trusted configuration control plane around that
pure compiler. Exact 0.60.88 commit
`a0e6defb859ee0809737b464e90e730ff8373d05` owns private app-data paths,
migration 13, revisioned settings, managed personal Git, bounded instructions
mirrors, exact project-layer trust, immutable bundle activation, authenticated
node distribution, durable mutation proposals, and local confirmation and
history surfaces. Its complete local Linux matrix passes. Native Windows
verification covers protected ownership and DACLs, reparse rejection, managed
Git durability and hidden child processes, atomic personal-repository recovery,
and physical limited-user keyboard and UI Automation confirmation behavior.
External Git and SFTP distribution, CI, and production activation remain
unexercised, so this is not production or release evidence and does not close
P4-C5 or P4-C7.

Phase 5C now implements the private server-owned execution runtime on exact
0.60.117 commit `e53406bbd211e701f89a25d63a8cd672033a22d9`. Migration 14,
replay-verified runs, the effect outbox, attempt-owned worktrees, shared
admission, integer budgets, local and authenticated node execution, typed
artifacts, verification evidence, watchdogs, cancellation, joined shutdown,
and startup recovery pass the complete portable matrix. Exact native Linux
tests pass process-group descendant cleanup and concurrent Git worktree
isolation. Native run `38040781118` passes all eight target jobs, including the
retained Windows rooted-path, junction, locked-worktree, real-child-death, and
IPC-smoke tests and the Apple Silicon worktree and real-child tests. The
required Windows Job Object descendant and macOS process-group descendant rows
remain unverified, so complete local/native acceptance is not established. A
physical remote node, live providers, and the controlled external-effect row
also remain unrun. Phase 5C adds no public run RPC, CLI, or desktop
orchestration surface and does not close P4-C5 or P4-C7.

Exact 0.60.38 commit `d328ad8` supersedes the preceding source-candidate note:
its complete local gate, native build, host smoke, remote lifecycle, and
standard successful-login row passed, but genuine Narrator still did not speak
the controlled output. Diagnostics proved the native PandaMUX HWND retained
foreground, active, and keyboard-focus ownership, ordinary UI Automation
honored the active descendant, and synchronized Narrator navigation reached
the terminal header controls before skipping the semantic Log group. The
0.60.39 source candidate therefore uses AccessKit's dedicated Terminal role,
which Windows maps to a UI Automation Document. At that point exact native
acceptance was still required.

Exact 0.60.39 commit `8b7b880` supersedes the preceding 0.60.39 candidate
note. Its complete local gate, native build, host headless and limited-user
smoke, remote lifecycle, and standard successful-login row passed. Windows
reported `terminal-output-log` as a focusable UI Automation Document, but
genuine Narrator visited that Document without speaking its parent-only
accessible name and then left PandaMUX. Exact 0.60.40 commit `d4c9dd8` retained
the Document, but its label-only semantic child was absent from the exact
Windows UI Automation tree despite passing the local, native-build, and host
gates. Exact 0.60.41 commit `c6a20ff` provides that child through an AccessKit
value backed by a synthetic text run, enabling UIA TextPattern content. Its
complete local, native, interaction, scaling, display-mode, Narrator, visual,
and named-pipe matrices passed. The 0.60.43 routing correction also passed.
Exact 0.60.62 commit `757caf0` passed the affected provider-login and Surfaces
matrix on the physical MSI at 100 and 200 percent, so P4-C3 remains Pass.

## Build and development

Use the Rust 1.98.1 development/evidence toolchain pinned by
`rust-toolchain.toml`. The workspace declares Rust 1.89 as its MSRV, proven in
CI for the Windows product graph and the Linux headless graph. Windows builds
require the MSVC build tools. The application has no Node runtime, though Node
24 runs the repository wiring guard.

```bash
# GPUI desktop, binary name pandamux
cargo run -p pandamux-desktop
cargo run -p pandamux-desktop -- --headless
cargo run -p pandamux-desktop -- --smoke

# Server composition root
cargo run -p pandamux-server
cargo run -p pandamux-server -- daemon --role node
cargo run -p pandamux-server -- proxy

# CLI
cargo run -p pandamux-cli -- --help
cargo run -p pandamux-cli -- ping
cargo run -p pandamux-cli -- identify
cargo run -p pandamux-cli -- thread list
cargo run -p pandamux-cli -- thread send <thread-id> "prompt"
cargo run -p pandamux-cli -- notify "Build complete"
cargo run -p pandamux-cli -- mcp
```

The desktop discovers or starts a sibling `pandamux-server` and authenticates through the private runtime record. For manual development, start the server first if the binaries do not share a target directory.

### Validation

Run Cargo commands serially in one checkout.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --locked -p pandamux-boundary-check --quiet
node scripts/check-claude-wiring.mjs
```

Useful scoped checks:

```bash
cargo test -p pandamux-core
cargo test -p pandamux-orchestrator
cargo test -p pandamux-protocol
cargo test -p pandamux-providers
cargo test -p pandamux-remote
cargo test -p pandamux-server
cargo test -p pandamux-cli
cargo test -p pandamux-term
cargo test -p pandamux-desktop
cargo test -p pandamux-boundary-check
cargo check -p pandamux-protocol -p pandamux-client -p pandamux-cli -p pandamux-server -p pandamux-desktop
```

The CI workflow in `.github/workflows/rust.yml` checks Windows formatting,
Clippy, resolved crate boundaries, the complete cargo-deny policy, tests, and
release binaries on Rust 1.98.1. Separate jobs compile the Windows product
graph and Linux headless graph at Rust 1.89; Apple Silicon explicitly tests the
pure orchestrator, and compile-only all-target checks cover its musl x64 and
arm64 graphs in addition to the existing x64 remote-binary build.

Before any long or potentially silent command, explain what will run, estimate its duration, and state whether it is safe to interrupt.

## Workspace architecture

```text
crates/
  pandamux-core/       Framework-free domain types, IDs, settings, configuration, mutation, and policy
  pandamux-orchestrator/ Pure schema, bundle, DAG, routing, policy, and run-state engine
  pandamux-protocol/   Versioned RPC, trusted-configuration, terminal, settings, and proxy contracts
  pandamux-providers/  Provider drivers, profiles, parser fixtures, and process supervision
  pandamux-remote/     SSH auth and pooling, exec, SFTP, bootstrap, ProxyJump, proxy tunnels
  pandamux-server/     SQLite owner, configuration services, router, PTYs, Hub and node runtime
  pandamux-client/     Client event and trusted-configuration projections
  pandamux-desktop/    GPUI chat shell, trusted settings, surfaces, and terminal grid
  pandamux-cli/        Authenticated CLI and MCP stdio adapter
  pandamux-term/       Optional `grid` and `pty` terminal features
tools/
  pandamux-boundary-check/ Target-resolved Cargo graph and feature-boundary validator
```

### Hard boundaries

- `pandamux-core` remains independent of UI, storage, terminal, and SSH frameworks.
- `pandamux-orchestrator` remains pure: no Tokio, filesystem, network, SQLite,
  GPUI, provider process, keyring, or embedding dependency.
- `pandamux-protocol` contains wire contracts and must not depend on implementation crates.
- `pandamux-server` is the production composition owner of `pandamux-remote`.
- `pandamux-server` depends on `pandamux-term` with `default-features = false, features = ["pty"]`.
- `pandamux-desktop` depends on `pandamux-term` with `default-features = false, features = ["grid"]`.
- Provider and durable state live behind the server. The desktop maintains projections and submits intents.
- Run `cargo test --locked -p pandamux-boundary-check` and then
  `cargo run --locked -p pandamux-boundary-check --quiet` after dependency
  changes. The checker evaluates target-resolved Cargo metadata and deliberately
  failing fixture workspaces.

## Server and local IPC

`pandamux-server` runs as a Hub on the desktop machine and as a node daemon on remote Linux hosts. It owns SQLite, provider processes, local PTYs, attachments, event ordering, and environment routing.

The runtime record is stored under `%LOCALAPPDATA%\pandamux\run\server.json` on Windows or `~/.pandamux/run/server.json` on Unix. It contains separate control and MCP capabilities. Connections must authenticate with `system.hello` before routing.

`RequestContext` is assigned by the accepting transport and contains the peer role, authenticated environment, client identity, and capability scope. Request JSON cannot select or upgrade those fields.

- Local desktop and CLI: control scope, with MCP methods excluded
- MCP adapter: `system.hello`, `system.ping`, read-only tools, and proposal-only trusted access changes
- Provider child: expiring token, exact thread, exact notification or memory methods
- Node proxy: server-assigned proxy role for the node's authenticated environment
- Internal calls: explicit in-process context only

Windows uses a SID-scoped pipe name, a protected user-only DACL, remote-client rejection, first-instance anti-squatting, and connecting PID and image validation. Unix uses a `0700` runtime directory, `0600` runtime records and sockets, safe owner and symlink checks, and peer credential validation.

The v1 boundary is per OS user, not hostile same-user process isolation. FullAccess providers intentionally have host-user authority. Never describe a FullAccess session as process-isolated.

The MCP surface cannot mutate configuration directly. `read_settings` and
`list_schedules` return purpose-built projections, while
`validate_thread_proposal` checks one named project, environment, and provider
route without creating a thread or granting approval. Phase 5B adds only
`propose_agent_access`: it uses the server-owned request context, persists one
revision-bound proposal, returns promptly, and requires a separate local-control
approval. These tools must never expose profile paths, provider environment
overrides, arbitrary provider JSON, secrets, schedule prompts, repository
paths, notification targets, FullAccess state, or opaque subscription keys.

## Trusted configuration control plane

`DataPaths` is the injected owner of the private database, backup, runtime,
personal, instructions, candidate, bundle, staging, recovery, and attachment
roots. Migration 13 is additive and runs only after a verified private database
backup. It adds revisioned settings plus candidate, trust, bundle, distribution,
proposal, and append-only audit records without renumbering earlier migrations.

The personal layer is one private plumbing-only Git repository. It validates
paths, sizes, case collisions, runtime data, and secret patterns before object
creation, updates the active ref through compare-and-swap, and uses retained
commits for forward-only reverts. Instructions use private bare mirrors,
`ls-remote` discovery before bounded fetch, immutable candidate refs, no-checkout
inspection, optional signature verification, and explicit last-good retention.
Project trust binds organization, project, credential-free remote identity,
canonical repository root, and the exact strict `.pandamux/` content digest.

Compilation combines the active instructions, managed personal layer, trusted
project layer, and revisioned local settings through the pure orchestrator.
Local activation verifies and materializes an immutable content-addressed
archive, then commits the active pointer, audit, candidate transition, and node
outbox together. Authenticated node proxies receive the private archive over
SFTP, verify its archive and source identities independently, and atomically
activate only the expected prior bundle. Existing run records keep their pinned
bundle.

Mutation proposals key idempotency by server-stamped caller, tool, and request
identity. Canonical old and new digests, expected revision, expiry, impact, and
resolution are durable before notification. Only authenticated local control
can list, approve once, or reject. Database-backed targets apply their exact
compare-and-swap mutation, immutable audit, and terminal resolution in one
transaction. The personal Git target uses an idempotent ref update followed by
reconcilable audit persistence because Git and SQLite cannot share one native
transaction.

The desktop Trusted Configuration category loads source and candidate status,
safe diffs, exact project trust, pending and recent proposals, activation
history, and personal history. It uses redacted projections, non-color-only
status text, named actions, exact revision and digest values, pessimistic
loading, and forward-only revert operations. Exact 0.60.88 physical Windows
evidence covers Tab traversal, Enter approval, UI Automation names and states,
and terminal controls that are disabled, non-focusable, and non-invokable.
High Contrast, scaling, reduced-motion, and Narrator were not rerun specifically
for this Phase 5B surface and must not be inferred from that result.

## Provider execution

Every executable provider path starts from a stored `ProviderInstanceConfig` resolved for the thread's environment. The shared resolver applies the exact provider kind, binary override, profile directory, settings, environment overrides, enabled state, capability map, authentication probe, and concurrency limit. Missing or incompatible instances fail with structured errors. Production code never falls back to the Custom mock.

Before `ThreadManager` acquires that final provider permit, the shared durable
`ExecutionAdmissionService` must claim capacity. Manual and orchestrated work
share provider, environment, organization, and global accounting;
orchestrated work also checks run and agent limits. Queue order, interactive
reserve, owner, TTL, and claim revision are persisted in SQLite. Per-instance
permits and admission claims are held for the complete provider-process
lifetime, including approval waits, and are released on cancellation, crash,
configuration replacement, or shutdown. The selected instance ID is carried
through the queue, thread, health and login paths, turn, process audit, and
provider launch.

Orchestrated admission also consumes an exact plan from
`ExecutionBudgetService`. The successful claim atomically checks and reserves
the task, run, compiled-agent, and organization UTC-day integer ceilings before
creating a dispatch effect. Supplemental budget changes require a connected
local approval bound to exact request bytes. Terminal reconciliation uses the
attempt's durable managed-turn facts, prefers provider-reported micro-USD, then
effective-dated integer pricing, and never treats legacy floating-point or
unknown cost as zero. Unknown dimensions retain their reservation; actual
overage is recorded before the run pauses. Conservation diagnostics compare
the durable ledgers without claiming to replace or certify source facts.

`RunExecutionService` is the sole server adapter between pure run transitions
and effect delivery. It persists effect intent and the dispatch marker before
calling `ThreadManager`; `ThreadManager` remains the only provider-session
owner and returns an exact prepared-turn owner rather than transferring the
session. The run service retains explicit handles for completion, watchdog,
lease-renewal, cancellation, and observer tasks. Typed not-started outcomes may
release admission and budget only after the responsible cleanup owner verifies
the worktree and provider boundary. Ambiguous or externally committed attempts
retain the Phase 4 tool ledger and `AttemptBoundary` authority and cannot use
ordinary fallback.

Every orchestration attempt binds one app-owned worktree, full branch ref,
baseline, private checkpoint index, capacity claim, budget reservation, and
pinned bundle before provider dispatch. Authenticated remote execution uses
separate repository prepare, attempt prepare, dispatch, status, cancel, event,
and acknowledgement receipts on Hub and node stores. Blackboard artifacts are
canonical, content-addressed, immutable, typed, bounded, and unable to grant
authority; successful task settlement publishes required artifacts and
server-derived evidence atomically.

Server startup holds one private incarnation lock while recovery verifies
replay, effects, leases, budgets, worktrees, checkpoints, cleanup state, and
remote receipts. It reclaims only exact expired local ownership, reattaches
only the exact live attempt, and deduplicates every ambiguity or unverifiable
cleanup into retained human review. Shutdown closes intake, stops new claims,
cancels active attempts concurrently through the durable cancellation path,
joins every owner, checkpoints SQLite, and reports failure when any live owner
or cleanup debt remains.

The desktop composer never converts a provider kind into an instance ID. After bounded
health checks, it builds its routes from enabled, authenticated stored instances and
`provider.models` results. Model discovery uses each provider driver's current contract,
is bounded by the same pagination, deadline, and concurrency limits as health, and
returns explicit per-instance failures instead of fabricated fallback models. Codex
queries its authenticated app-server model list; drivers without a live model-list API
may return their declared metadata. If no configured route has both accepted health
evidence and a nonempty model result, New Thread opens Provider Settings with an error.

Phase 4A extends that typed identity into normalized usage observations and subscription mappings. It uses additive migration 6 because R0.3 already applied migration 5 for provider execution audit. The recovered usage prototype remains evidence only; its append-only buckets, caller-supplied provenance, floating-point pricing, and in-memory path cursors are not the implementation contract.

Phase 4B puts transcript scanning under `pandamux-server`. Provider parsing in
`pandamux-providers::usage_transcript` is pure and fixture-tested. One bounded blocking
worker owns filesystem enumeration, startup and turn-completion triggers, periodic
jitter, durable file-identity cursors, parser state, quarantine, diagnostics, and joined
shutdown. Managed turn usage is written before transcript backfill and remains
authoritative during reconciliation. Scanner RPC projections never include transcript
text, profile paths, canonical source paths, or unmasked account labels. Additive
migration 7 carries the scanner state and reconciliation indexes; migrations 4 through
6 remain immutable.

Additive migration 12 corrects Claude usage identity without rewriting migrations 1
through 11. A stateful per-turn parser deduplicates repeated stream chunks by exact
assistant message id and retains every distinct assistant message. Managed usage stores
one token component per message with the reported model and cache categories, plus one
separate terminal result cost fact. Reconciliation uses only exact provider message
identity. Timestamp skew and one missing optional cost can converge, while conflicting
provider, instance, subscription, model, or token measurements remain separate and keep
source coverage Partial with a durable diagnostic. Existing ambiguous result UUID and
message id history is preserved rather than guessed from timestamps.

Claude partial-message streaming is enabled so the terminal `message_delta.usage`
updates the component first seen at `message_start`. Root and subagent lanes keep their
active message identities separate. The terminal delta, not the preliminary start
value, decides each component's final token and cache totals.

Phase 4C keeps node outboxes and Hub ingestion bounded. The Hub owns one asynchronous
synchronization service, persists its database at `data/hub.db` by default, and commits
each authenticated environment's observations with its node-store identity and
high-water mark. Reconnect resumes incrementally. Store replacement or acknowledged
retention loss enters durable bounded backfill before old facts are removed. Migration 8
adds synchronization state, retention accounting, query indexes, and normalized
effective-dated pricing rules without rewriting prior migrations. `usage.query` pages
facts but computes complete filtered totals in SQLite. Provider-reported spend and
API-equivalent estimates remain separate, and any nonzero token category without an
exact effective rule keeps the estimate unknown.

Phase 4D keeps quota evidence separate from provider availability. Additive migration 9
persists immutable capacity revisions and provider health snapshots without rewriting
earlier migrations. Capacity snapshots carry a schema version, authenticated
environment and provider identity, quota window, exact integer fractions, reset,
source, confidence, provider version, evidence scope, and age policy. Hub
synchronization sends only bounded referenced subscription metadata before its
revisions. Projection selects the safer conflicting value, decays old evidence, marks
offline and clock-skewed sources, and never lets stale Hub evidence raise an active
local hard limit before its verified reset.

Only Codex rate-limit fixtures currently justify Reported quota evidence. Grok live
collection remains disabled, and Claude and Antigravity capacity remains Unknown unless
a defensible provider source is added. Provider health probes are side-effect-free,
deadline-bounded, concurrency-capped, and persist last success separately from current
installation, executable, version, authentication confidence, runtime, check time, and
expiry. Unknown authentication is never Healthy. Claude and Codex login requires a
short-lived preview of the exact environment, instance, profile, and safely quoted
command before the server opens a Terminal session. Antigravity retains its dedicated
OAuth flow.

Phase 4E adds Usage as a top-level GPUI rail destination. `pandamux-client::usage` owns
the pure filter and status reducer, exact integer formatters, and presentation-safe
capacity cards. The desktop renders token totals first, labels modeled cost as an
API-equivalent estimate, keeps provider-reported spend separate, exposes source coverage
and freshness, and provides text or table equivalents for all capacity and usage data.
Subscription editing accepts a user-defined name and optional masked account label;
opaque subscription and provider-instance keys are join values only.

Project-scoped thread creation and turns, authenticated remote forwarding, schedule
validation and execution, read-only MCP proposals, failover filtering, and Broadside
authorization use fail-closed policy primitives before provider selection. Missing or
ambiguous project assignment fails closed on each implemented project-bound path. The
organization UI edits named provider and subscription boundaries, budget, concurrency,
tier defaults, Broadside permissions, and explicit failover pools. Project assignment
stays disabled until a canonical project catalog exists. There is no runtime Broadside
share action. Remote schedules, orchestrator schedules, and continue-existing-thread
schedules report explicit unavailable states.

Phase 4F adds migration 10 for durable turn attempts, tool ledgers, audited
continuations, and idempotent approval or tool-result deliveries. Additive migration 11
attaches the exact provider kind, profile, binary override, capabilities, and probed
version to each attempt so a safe retry preserves both provider executions and managed
usage follows the completed attempt. Pool members bind one
provider instance, named subscription, model, declared capability set, and user
preference. Runtime selection rechecks those declarations against the registered driver,
then applies organization policy, required capabilities and access, model compatibility,
installation, authentication, health, capacity hard blocks, and concurrency before it
scores any eligible member. Every decision stores an explanation for every candidate.
For remote project turns and continuations, the Hub replaces caller input with its
canonical organization and referenced-pool snapshot. The authenticated node validates
that snapshot and each member authorization again before routing.

Provider adapters emit a monotonic attempt boundary before visible output, tool activity,
or approval interaction is forwarded. Capacity exhaustion retries once only from the
safe boundary and excludes the exhausted provider. Any later exhaustion records exact
provider evidence, partial output, tool ledger state, and a worktree checkpoint. The
user-visible continuation starts a fresh provider session with a bounded handoff, verifies
that checkpoint has not changed, and never carries a native resume token across provider
instances. The continuation target is linked durably before its handoff reaches the
provider, and duplicate continuation, approval, and completed tool-result delivery is
idempotent across restart. A completed tool result commits its immutable ledger
transition, timeline event, and delivered marker in one transaction, so identical
provider replay creates no second projection. A provider cannot turn an unresolved
started tool into a successful PandaMUX turn: completion checks the durable ledger,
emits a nonrecoverable `unresolved_tool_result` error, preserves the ambiguous row, and
fails without retry.

Advanced settings expose independent rollback switches for usage scanning, Hub usage
synchronization, capacity-aware routing, and automatic failover. Scanner and Hub service
switches take effect at process start. Capacity routing applies on the next turn, while
automatic failover applies at the next capacity decision. Disabling them does not remove
observations or rewrite legacy tables.

The source-level accessibility gate names icon controls, retains visible non-color
status text, restores focus after the subscription editor, keeps errors visible, wraps
controls, scrolls the Usage surface, and presents a full usage data table. Native Windows
keyboard-only, 200 percent scaling, reduced-motion, forced-colors, and screen-reader
acceptance are not established by Linux compilation or unit tests.

## Remote composition

`pandamux-remote` owns SSH mechanics, but `pandamux-server` owns their production composition.

1. The Hub authenticates with `russh`, detects the Linux platform, and reads the node runtime record.
2. Bootstrap verifies exact server and CLI artifact size and SHA-256 before atomic placement.
3. A compatible daemon is discovered or started with `pandamux-server daemon`.
4. An SSH exec channel runs `pandamux-server proxy`, which bridges to the daemon's private Unix socket.
5. The versioned proxy carries bounded RPC, cancellation, durable event replay, EOF state, and joined cleanup.
6. A dropped tunnel marks the environment stale or unreachable. It never silently reroutes remote work to the local machine.

The non-release R0 artifact harness is `scripts/r0-node-artifacts.sh`. Its build mode refuses a dirty tree, requires both binaries, records commit, version, size, and SHA-256, re-verifies the directory, and leaves publication disabled.

## Release state

Release authorization is not implied by implementation work. Keep implementation, verification, commit, push, deployment, and live acceptance as separate evidence boundaries.

The current GPUI rewrite has passed R0.5, including both physical P4-C6 rows,
but it has not passed the complete Phase 4 exit matrix. The real pre-output
provider-capacity row and final post-P4-C5 documentation archive remain open.
Phase 5B has complete evidence for its authorized local and required native
Windows scope. It has no external Git/SFTP or production-distribution evidence;
those absent claims do not reopen its authorized acceptance scope.
Phase 5C is implemented and locally verified, with exact native Linux process
and worktree evidence plus one all-green eight-job Windows, Apple Silicon,
Linux, musl, MSRV, and dependency-policy CI run. Windows and macOS automation
remain partial against the binding native matrix because the Windows Job Object
descendant and macOS process-group descendant rows are not independently
tested. Physical-remote, live-provider, and controlled external-effect rows
remain unrun, so complete local/native or production acceptance is not
established.
`.github/workflows/release.yml` still contains historical packaging references
and must not be used as evidence that the current workspace is releasable.
Phase 7 owns the final Windows packaging, signing, publication, and update
pipeline.

## Repository conventions

- Preserve unrelated and user-owned work in dirty checkouts. Never reset, clean, broadly stage, or absorb it.
- Treat approved plans and acceptance matrices as contracts. Record deliberate deviations and their reasons.
- Update `CHANGELOG.md` and the root version when preparing a commit according to `.claude/rules/commit-changelog.md`.
- Write commit messages through a file with `git commit -F`; do not use an inline message.
- Consult the LL-G and BP knowledge bases using the repository rules before code or configuration work.
- Use the custom agents in `.claude/agents/` only when delegation is explicitly authorized.
- Do not use em dashes or doubled hyphens as prose punctuation in files, comments, or messages. Command-line flags remain unchanged.
- Do not claim a test, build, artifact, commit, push, deployment, or live acceptance result without direct evidence.
