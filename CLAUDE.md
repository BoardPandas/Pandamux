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

The codebase is at the Phase 4 recovery gate. R0.1 preserves the unfinished usage prototype; R0.2 completes production proxy routing; R0.3 enforces exact provider-instance execution; R0.4 secures the local control boundary. R0.5 has exact live evidence for Galahad and native Windows IPC, while the required 30-minute physical laptop disconnect and sleep checks remain open. Do not call the rewrite production-accepted, released, or shipped before the matrix passes and its record is archived.

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

The MCP surface is intentionally read-only in Phase 4. `read_settings` and `list_schedules` return purpose-built projections. They must never expose profile paths, provider environment overrides, arbitrary provider JSON, secrets, schedule prompts, repository paths, notification targets, or FullAccess state.

## Provider execution

Every executable provider path starts from a stored `ProviderInstanceConfig` resolved for the thread's environment. The shared resolver applies the exact provider kind, binary override, profile directory, settings, environment overrides, enabled state, capability map, authentication probe, and concurrency limit. Missing or incompatible instances fail with structured errors. Production code never falls back to the Custom mock.

Per-instance permits are held for the complete provider-process lifetime, including approval waits, and are released on cancellation, crash, configuration replacement, or shutdown. The selected instance ID is carried through the thread, health and login paths, turn, process audit, and provider launch.

Phase 4A will extend that typed identity into normalized usage observations and subscription mappings. The recovered usage prototype is evidence only; its append-only buckets, caller-supplied provenance, floating-point pricing, and in-memory path cursors are not the implementation contract.

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
