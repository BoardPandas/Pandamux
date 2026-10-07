<h1 align="center">PandaMUX</h1>
<p align="center">A native, chat-first desktop for supervising local and remote AI coding agents.</p>

PandaMUX is a Rust workspace built around a GPUI desktop, an authenticated JSON-RPC server, provider adapters, and SSH-connected Linux nodes. The server owns durable thread, turn, terminal, settings, and event state. The desktop and CLI are clients of that same service.

This repository began as an Electron prototype, then had an Iced multiplexer implementation. Both have been removed from the active workspace. Historical rationale remains in [`tasks/plan-repo.md`](tasks/plan-repo.md), while [`tasks/plan-rewrite.md`](tasks/plan-rewrite.md) is the current implementation contract.

## Current development status

The workspace version comes only from `[workspace.package] version` in [`Cargo.toml`](Cargo.toml). Do not infer the current version from documentation prose.

The codebase is implementing Phase 4 provider observability work after completing the
R0 development gate:

- R0.1 preserves and verifies the unfinished Phase 4 usage prototype without treating it as accepted production code.
- R0.2 implements Hub-to-node routing through a versioned SSH exec proxy, reconnect semantics, event replay, and joined cleanup.
- R0.3 resolves every provider turn from the exact stored provider instance and holds per-instance capacity for the provider process lifetime.
- R0.4 secures local IPC with server-assigned request contexts, distinct capabilities, OS-user boundaries, and redacted MCP reads.
- R0.5 has exact live evidence for Galahad and native Windows IPC. On 2026-09-29, the user approved deferring the required 30-minute physical laptop disconnect and sleep checks so Phase 4 development could continue. Those checks still block production-accepted, released, or shipped claims.
- Phase 4A provides normalized usage observations, durable transactional cursors and outbox state, canonical organization and failover policy, migration-4 reconciliation through additive migration 6, and bounded authenticated usage, capacity, and health protocol pages.
- Phase 4B adds node-owned Claude and Codex transcript scanners with durable platform file identity, bounded lifecycle and retention, explicit coverage diagnostics, transactional cursor advancement, and managed-turn reconciliation. Claude token usage is keyed to exact assistant message identity, while terminal result cost remains a separate turn-level fact. Raw transcript text and paths remain node-local.
- Phase 4C adds bounded Hub pulls, transactional per-node high-water marks, acknowledged outbox compaction, explicit retention-gap backfill, indexed SQL aggregates, effective-dated integer pricing, and cross-node coverage metadata. Provider-reported spend remains separate from API-equivalent estimates.
- Phase 4D adds schema-versioned capacity revisions and conservative projections,
  authenticated Hub-to-node capacity exchange, bounded concurrent provider health with
  durable freshness, and explicit preview-before-launch login actions. Codex quota
  evidence can be Reported; unverified provider capacity remains Unknown.
- Phase 4E adds the top-level Usage and capacity surface, pure client reducers for every
  data and failure state, named subscription and provider presentation, canonical
  organization-policy editing and enforcement, truthful health controls, and the
  source-level accessibility gate. Exact 0.60.22 native Windows evidence passes
  the executed 100 and 200 percent keyboard paths, semantic table and editor
  focus, empty and populated table semantics, dark and light review, High
  Contrast, disabled animations, actionable Offline behavior, and genuine
  Narrator status, error, table, field-label, and focus-return checks.
- Phase 4F adds explicit capability-compatible failover pools, deterministic hard-filter-
  first routing, durable per-attempt side-effect boundaries, one automatic retry only
  before output, and audited fresh-session continuation after unsafe capacity exhaustion.
  Authenticated remote turns carry a Hub-owned policy and pool snapshot that the node
  validates again before selecting a route. Independent rollback switches preserve
  observations while disabling scanning, Hub synchronization, capacity-aware
  routing, or automatic failover. Exact 0.60.15 passed accounting and cloned-store
  reconciliation, exact 0.60.17 partially passed the P4-C5 durability and control
  matrix. Exact 0.60.22 also passed the current normal-user IPC matrix, but
  P4-C3 is reopened because its successful provider-login launch path was not
  exercised and later left the Terminal surface undiscoverable and
  noninteractive. Exact 0.60.24 proved the repaired panel, terminal binding,
  and automatic focus natively, but PowerShell did not execute the injected
  login command because ConPTY received a line feed instead of carriage
  return. Exact 0.60.25 proved command execution visually, then exposed that
  painted Terminal output was absent from UI Automation. Exact 0.60.26 added
  a bounded semantic output log and passed the successful-login interaction,
  High Contrast, and disabled-animation rows, but genuine Narrator Speech
  Recap did not announce the READY or ECHO output. Exact 0.60.27 then passed
  the successful-login interaction, 200 percent scaling, High Contrast, and
  disabled-animation rows, but its output Log could not receive focus through
  UI Automation and the advertised Tab path remained PTY input. Exact 0.60.28
  added an explicit F6 input/output focus toggle and passed the successful-login,
  200 percent scaling, High Contrast, and disabled-animation rows. Narrator
  displaced focus to the PandaMUX window, where the terminal-scoped shortcut
  could not run, so 0.60.28 is rejected. Exact 0.60.29 commit `2b3b806`
  passed native build, smoke, remote lifecycle, successful-login, 200 percent,
  High Contrast, and disabled-animation rows, but its root visual handler still
  had no GPUI focus path under Narrator and did not receive F6. Exact 0.60.30
  commit `9dae237` passed native build, smoke, remote lifecycle, and standard
  successful-login interaction, but Narrator still consumed or withheld F6
  before PandaMUX received it. Exact 0.60.31 commit `0394eb3` passed native
  build, smoke, remote lifecycle, and standard successful-login interaction.
  Its explicit `Type here` and `Read output` controls were visible and
  invokable, but accessibility activation restored window-level focus after
  the click handler requested the output Log, so Narrator still could not read
  READY or ECHO. Exact 0.60.32 commit `952b280` passed native build, smoke,
  remote lifecycle, and standard successful-login interaction, but direct UI
  Automation Invoke still left focus on the PandaMUX window because the styled
  control fell back to synthesized pointer input under Narrator. Exact 0.60.33
  commit `58452e7` passed native build, smoke, remote lifecycle, and standard
  successful-login interaction, but its direct AccessKit handler deferred the
  focus update and UI Automation Invoke still left focus on the PandaMUX window.
  Exact 0.60.34 commit `1f1df21` passed native build, smoke, remote lifecycle,
  and standard successful-login interaction, but Windows UI Automation still
  restored window-level focus after the synchronous accessibility callback.
  Exact 0.60.35 commit `b54f8e3` repeated the requested focus after platform
  Invoke dispatch and passed the standard row, but GPUI elided the delayed
  same-handle request while UI Automation focus remained on the PandaMUX
  window. The 0.60.36 source candidate first focuses the invoked control, then
  moves to the requested terminal target and still requires exact native
  Windows acceptance.
  Real external quota exhaustion, real post-side-effect interruption, and the
  deferred physical laptop checks keep the Phase 4 exit matrix open.

See the [Phase 4 live acceptance record](docs/operations/PHASE4_ACCEPTANCE.md)
and the completion criteria in [`tasks/plan-rewrite.md`](tasks/plan-rewrite.md).

## Product shape

- Chat threads stream provider output into a virtualized timeline and support approval, cancel, reconnect, attachments, model selection, and effort selection.
- Provider instances are explicit stored configurations. Claude, Codex, Antigravity, Cursor, Grok, and OpenCode adapters exist, but live acceptance varies by provider and environment.
- The right-side surfaces include agents, diffs, confined files, pull-request views, terminal sessions, floor-plan data, and artifacts.
- The server owns local PTYs and durable byte replay. The desktop owns only terminal grid rendering.
- Linux nodes run `pandamux-server daemon`; the Hub reaches them through `pandamux-server proxy` over SSH. No remote TCP listener is opened.
- The CLI supports connectivity, identification, thread listing and sending, notifications, and a restricted read-only MCP adapter.

## Build and run

Rust stable is required. Windows development uses the MSVC build tools. The workspace has no Node runtime dependency.

```bash
# Run the authenticated local Hub.
cargo run -p pandamux-server

# Run the GPUI desktop. It discovers or starts the Hub automatically.
cargo run -p pandamux-desktop

# Noninteractive desktop checks.
cargo run -p pandamux-desktop -- --headless
cargo run -p pandamux-desktop -- --smoke

# Inspect the actual CLI surface.
cargo run -p pandamux-cli -- --help
cargo run -p pandamux-cli -- ping
cargo run -p pandamux-cli -- thread list
```

Common validation commands:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pwsh -File scripts/check-rust-boundaries.ps1
node scripts/check-claude-wiring.mjs
```

Cargo builds and tests should run serially in one checkout because they share the target directory and may contend on the package cache. See [`CLAUDE.md`](CLAUDE.md) for scoped checks and platform notes.

## Architecture

```text
crates/
  pandamux-core/       Framework-free domain types and policy data
  pandamux-protocol/   Versioned JSON-RPC, subscription, and proxy wire contracts
  pandamux-providers/  Provider drivers, profiles, process supervision, and parsing
  pandamux-remote/     SSH pooling, exec, SFTP, bootstrap, and proxy tunnels
  pandamux-server/     Single-writer store, router, provider execution, PTYs, Hub and node runtime
  pandamux-client/     Client-side event projections and smoothing
  pandamux-desktop/    GPUI chat-first desktop and terminal grid rendering
  pandamux-cli/        Authenticated CLI and restricted MCP stdio adapter
  pandamux-term/       Feature-split terminal grid and local PTY primitives
```

Important boundaries:

- `pandamux-server` is the production composition owner of `pandamux-remote` and depends on `pandamux-term` with only the `pty` feature.
- `pandamux-desktop` uses `pandamux-term` with only the `grid` feature and does not own provider processes or durable state.
- `pandamux-protocol` carries wire types but does not depend on the server, desktop, terminal, or SSH implementations.
- `pandamux-core` is framework-free.

## Local control security

Clients discover the endpoint through `server.json` under the private runtime directory. A connection must begin with `system.hello` and a server-issued capability. Desktop and interactive CLI clients receive the local control scope; MCP receives a distinct read-only adapter scope; provider children receive an expiring capability bound to one thread and a narrow notification or memory allowlist.

On Windows, the pipe name contains the current user SID, the pipe DACL grants that user access, remote pipe clients are rejected, and the connecting process ID and image are validated. On Unix, the runtime directory is mode `0700`, sockets and runtime records are mode `0600`, unsafe symlinks or owners are rejected, and peer credentials must match the socket owner.

This is not a hostile same-user sandbox. A malicious process already running as the same OS user can read user-owned files, and a FullAccess provider intentionally has host-user authority. Scoped capabilities protect normal sandboxed and accidental call paths; they do not make FullAccess process-isolated.

## Remote composition

The Hub uses `pandamux-remote` to authenticate over SSH, detect the Linux target, bootstrap verified server and CLI artifacts, and open an exec channel running `pandamux-server proxy`. The proxy connects to the node daemon's private Unix socket and carries versioned requests, responses, cancellation, and replayable events. Node work continues when the laptop disconnects; reconnect resumes from durable sequence cursors.

The narrow R0 artifact harness is [`scripts/r0-node-artifacts.sh`](scripts/r0-node-artifacts.sh). It requires a clean exact commit, builds both node binaries, records sizes and SHA-256 hashes, verifies the result, and keeps publication disabled. It is not the Phase 7 release pipeline.

## Documentation

- [`AGENTS.md`](AGENTS.md): agent routing and repository authority order
- [`CLAUDE.md`](CLAUDE.md): authoritative development guide
- [`docs/README.md`](docs/README.md): maintained documentation index
- [`tasks/plan-rewrite.md`](tasks/plan-rewrite.md): current implementation and acceptance contract
- [`tasks/plan-repo.md`](tasks/plan-repo.md): historical rewrite record and retained UI specification

## License

PandaMUX is licensed under the [MIT License](LICENSE).
