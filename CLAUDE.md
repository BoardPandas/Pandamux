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

Phase 4A through 4F implementation is recorded after the R0 development gate, but the Phase 4 exit matrix remains open. R0.1 preserves the unfinished usage prototype; R0.2 completes production proxy routing; R0.3 enforces exact provider-instance execution; R0.4 secures the local control boundary. R0.5 has exact live evidence for Galahad and native Windows IPC. On 2026-09-29, the user approved deferring the required 30-minute physical laptop disconnect and sleep checks so development could continue. Those rows still block any production-accepted, released, or shipped claim until the matrix passes and its record is archived. Exact 0.60.15 commit `3a77382` passes the P4-C1 Claude accounting repair, P4-C2 local candidate, and P4-C4 cloned Galahad reconciliation gates. Exact 0.60.22 commit `fee9e02` passed the P4-C3 rows that were executed, but P4-C3 was reopened on 2026-10-07 after successful provider-login launch left its Terminal surface undiscoverable and noninteractive. Exact 0.60.24 commit `581113e` proved the repaired panel, terminal binding, and automatic focus on native Windows, but its injected PowerShell command remained at a continuation prompt because ConPTY received a line feed instead of carriage return. Exact 0.60.25 commit `8f0ec20` proved native command execution visually, then failed because painted Terminal output was absent from UI Automation. Exact 0.60.26 commit `b696aad` exposed the output as a bounded semantic Log and passed the successful-login interaction, High Contrast, and disabled-animation rows, but genuine Narrator Speech Recap did not announce READY or ECHO. Exact 0.60.27 commit `5e00715` passed successful-login interaction, 200 percent scaling, High Contrast, and disabled-animation rows, but direct UI Automation focus did not reach the Log and its advertised Tab path remained PTY input. Exact 0.60.28 commit `c0200a9` added an explicit F6 input/output focus toggle and passed successful-login interaction, 200 percent scaling, High Contrast, and disabled-animation rows. Narrator displaced focus to the PandaMUX window, where the terminal-scoped shortcut could not run, so 0.60.28 is rejected. Exact 0.60.29 commit `2b3b806` moved the handler to the root visual element and passed native build, smoke, remote lifecycle, successful-login, 200 percent, High Contrast, and disabled-animation rows. Narrator again left no GPUI child focus path, so that root element did not receive F6 and 0.60.29 is rejected. The 0.60.30 source candidate intercepts F6 at the application level before focus-path dispatch and still requires exact native acceptance. Exact 0.60.17 commit `2f56a27` partially passes P4-C5 with live scanner and Hub durability, conservative capacity projections and route explanations, and independent rollback controls. Real external quota exhaustion and post-side-effect capacity interruption remain open even though their diagnostic fixtures pass. Earlier 0.60.0 through 0.60.8 evidence remains scoped to its named provider, Hub, IPC, and native UI rows. The exact results are recorded in [`docs/operations/PHASE4_ACCEPTANCE.md`](docs/operations/PHASE4_ACCEPTANCE.md); the reopened login-terminal row, remaining external failover, final documentation archive, and physical rows keep Phase 4 open.

## Build and development

Use Rust stable. Windows builds require the MSVC build tools. The application has no Node runtime, though Node 24 runs the repository wiring guard.

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
pwsh -File scripts/check-rust-boundaries.ps1
node scripts/check-claude-wiring.mjs
```

Useful scoped checks:

```bash
cargo test -p pandamux-core
cargo test -p pandamux-protocol
cargo test -p pandamux-providers
cargo test -p pandamux-remote
cargo test -p pandamux-server
cargo test -p pandamux-cli
cargo test -p pandamux-term
cargo test -p pandamux-desktop
cargo check -p pandamux-protocol -p pandamux-client -p pandamux-cli -p pandamux-server -p pandamux-desktop
```

The CI workflow in `.github/workflows/rust.yml` checks Windows formatting, Clippy, crate boundaries, advisories, tests, and release binaries; it also checks the core binaries on Apple Silicon and builds the x86_64 Linux musl node artifacts.

Before any long or potentially silent command, explain what will run, estimate its duration, and state whether it is safe to interrupt.

## Workspace architecture

```text
crates/
  pandamux-core/       Framework-free domain types, IDs, settings, threads, turns, events, policy
  pandamux-protocol/   Versioned RPC, subscription, terminal, settings, and proxy contracts
  pandamux-providers/  Provider drivers, profiles, parser fixtures, and process supervision
  pandamux-remote/     SSH auth and pooling, exec, SFTP, bootstrap, ProxyJump, proxy tunnels
  pandamux-server/     SQLite single writer, router, provider resolver, PTYs, Hub and node runtime
  pandamux-client/     Client event projections and rendering-oriented state
  pandamux-desktop/    GPUI chat shell, timeline, composer, settings, surfaces, terminal grid
  pandamux-cli/        Authenticated CLI and MCP stdio adapter
  pandamux-term/       Optional `grid` and `pty` terminal features
```

### Hard boundaries

- `pandamux-core` remains independent of UI, storage, terminal, and SSH frameworks.
- `pandamux-protocol` contains wire contracts and must not depend on implementation crates.
- `pandamux-server` is the production composition owner of `pandamux-remote`.
- `pandamux-server` depends on `pandamux-term` with `default-features = false, features = ["pty"]`.
- `pandamux-desktop` depends on `pandamux-term` with `default-features = false, features = ["grid"]`.
- Provider and durable state live behind the server. The desktop maintains projections and submits intents.
- Run `scripts/check-rust-boundaries.ps1` after dependency changes.

## Server and local IPC

`pandamux-server` runs as a Hub on the desktop machine and as a node daemon on remote Linux hosts. It owns SQLite, provider processes, local PTYs, attachments, event ordering, and environment routing.

The runtime record is stored under `%LOCALAPPDATA%\pandamux\run\server.json` on Windows or `~/.pandamux/run/server.json` on Unix. It contains separate control and MCP capabilities. Connections must authenticate with `system.hello` before routing.

`RequestContext` is assigned by the accepting transport and contains the peer role, authenticated environment, client identity, and capability scope. Request JSON cannot select or upgrade those fields.

- Local desktop and CLI: control scope, with MCP methods excluded
- MCP adapter: `system.hello`, `system.ping`, and the read-only MCP tool surface
- Provider child: expiring token, exact thread, exact notification or memory methods
- Node proxy: server-assigned proxy role for the node's authenticated environment
- Internal calls: explicit in-process context only

Windows uses a SID-scoped pipe name, a protected user-only DACL, remote-client rejection, first-instance anti-squatting, and connecting PID and image validation. Unix uses a `0700` runtime directory, `0600` runtime records and sockets, safe owner and symlink checks, and peer credential validation.

The v1 boundary is per OS user, not hostile same-user process isolation. FullAccess providers intentionally have host-user authority. Never describe a FullAccess session as process-isolated.

The MCP surface is intentionally read-only in Phase 4. `read_settings` and `list_schedules` return purpose-built projections. `validate_thread_proposal` checks one named project, environment, and provider route without creating a thread or granting approval. These tools must never expose profile paths, provider environment overrides, arbitrary provider JSON, secrets, schedule prompts, repository paths, notification targets, FullAccess state, or opaque subscription keys.

## Provider execution

Every executable provider path starts from a stored `ProviderInstanceConfig` resolved for the thread's environment. The shared resolver applies the exact provider kind, binary override, profile directory, settings, environment overrides, enabled state, capability map, authentication probe, and concurrency limit. Missing or incompatible instances fail with structured errors. Production code never falls back to the Custom mock.

Per-instance permits are held for the complete provider-process lifetime, including approval waits, and are released on cancellation, crash, configuration replacement, or shutdown. The selected instance ID is carried through the thread, health and login paths, turn, process audit, and provider launch.

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
idempotent across restart.

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

The current GPUI rewrite has not passed R0.5. `.github/workflows/release.yml` still contains historical packaging references and must not be used as evidence that the current workspace is releasable. Phase 7 owns the final Windows packaging, signing, publication, and update pipeline.

## Repository conventions

- Preserve unrelated and user-owned work in dirty checkouts. Never reset, clean, broadly stage, or absorb it.
- Treat approved plans and acceptance matrices as contracts. Record deliberate deviations and their reasons.
- Update `CHANGELOG.md` and the root version when preparing a commit according to `.claude/rules/commit-changelog.md`.
- Write commit messages through a file with `git commit -F`; do not use an inline message.
- Consult the LL-G and BP knowledge bases using the repository rules before code or configuration work.
- Use the custom agents in `.claude/agents/` only when delegation is explicitly authorized.
- Do not use em dashes or doubled hyphens as prose punctuation in files, comments, or messages. Command-line flags remain unchanged.
- Do not claim a test, build, artifact, commit, push, deployment, or live acceptance result without direct evidence.
