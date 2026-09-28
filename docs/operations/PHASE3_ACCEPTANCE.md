# Phase 3 Production Acceptance Record

Date opened: 2026-09-28

Status: **Open. R0.5 live acceptance has not been run.**

This record separates source and fixture verification from production acceptance. Passing unit or proxy fixture tests does not satisfy a live Windows or Galahad row.

## Deciding source

- Recovery baseline: `9511d0e22d44192f106e2f84a7c96b96d76734b6`
- Recovery implementation worktree: `/home/brandon/.codex/worktrees/recovery-r0/Pandamux`
- Recovery implementation branch: `codex/recovery-r0`
- Workspace version at recovery baseline: `0.53.44`
- Deciding implementation commit: the clean recovery implementation commit containing this
  record; its exact SHA is copied from the R0 artifact manifest after generation
- R0 node artifact manifest: not generated; the harness correctly refuses a dirty tree

## Source and fixture evidence

| Check | Result | Evidence |
|---|---|---|
| Preserve unfinished Phase 4 work | Pass | Exact baseline, binary patch, untracked archive, hashes, and byte-for-byte dry restore are recorded in `tasks/plan-rewrite.md` under R0.1 |
| Server, request-context, proxy, and provider lifecycle tests | Pass | `cargo test -p pandamux-server`: 51 library, 2 server binary, 6 proxy transport, and 7 thread lifecycle tests passed |
| Antigravity source and fixture contract | Pass | Official Google ACP 1.1.1 pins and object-valued capabilities, streaming archive verification, hardened two-entry extraction, real ACP and OAuth ownership, workspace-confined file handlers, opaque permission IDs, bounded cancellation, environment-wide two-process enforcement, and verified Hub-to-node SFTP fallback are implemented; live rows remain open |
| Full workspace tests | Pass | `cargo test --workspace`: 286 passed, 0 failed, and 2 real-shell terminal tests intentionally ignored |
| Strict static analysis | Pass | `cargo clippy --workspace --all-targets -- -D warnings` |
| Protocol, client, CLI, server, and desktop compile check | Pass | `cargo check -p pandamux-protocol -p pandamux-client -p pandamux-cli -p pandamux-server -p pandamux-desktop` |
| Server dependency boundary | Pass | `cargo tree` showed `pandamux-remote` composed only by `pandamux-server`; the server terminal graph contains `portable-pty` and excludes `alacritty_terminal` |
| R0 artifact dirty-tree guard | Pass | `scripts/r0-node-artifacts.sh build` refused the intentionally dirty implementation tree before compilation or publication |
| Unix same-user CLI over production socket | Pass | A temporary Hub accepted authenticated `ping` and `identify`; runtime directory, record, and socket modes were `0700`, `0600`, and `0600`; generated state was moved to recoverable trash |
| Formatting, whitespace, crate boundaries, and wiring guards | Pass | `cargo fmt --all --check`, `git diff --check`, `scripts/check-rust-boundaries.ps1`, and `scripts/check-claude-wiring.mjs` |
| Windows security module MSVC type-check | Pass with scope limitation | The exact `windows_security.rs` module compiled to metadata for `x86_64-pc-windows-msvc` against `windows-sys 0.61.2` |
| Windows workspace cross-check | Pass with scope limitation | `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu` compiled the full workspace and Windows-only tests without warnings; the temporary target was removed afterward |
| Full Windows server cross-check from Linux | Blocked by verification host | Dependency build scripts stopped before PandaMUX code because MSVC `lib.exe` is absent; `ring` and bundled SQLite could not build |
| Native Windows IPC behavior | Not run | Requires a Windows host to run SID, DACL, remote-client, PID/image, and anti-squatting tests |
| Galahad artifact build and transfer | Not run | Requires a clean deciding commit and fresh R0 manifest |

## R0.5 live matrix

| Row | Local Windows | Galahad | Status and required evidence |
|---|---|---|---|
| Codex configured-instance login, start, stream, approval, cancel, reconnect, access ceiling | Not run | Not run | Capture instance ID, model, exact binary SHA, event log, approval decision, cancellation, reconnect cursor, and ceiling denial |
| Claude configured-instance login, start, stream, approval, cancel, reconnect, access ceiling | Not run | Not run | Capture the same evidence as Codex |
| Antigravity configured-instance login, start, stream, approval, cancel, reconnect, access ceiling | Not run | Not run | Also capture managed bundle version and scratch-volume preflight |
| Antigravity node-side managed download | Not applicable | Not run | Capture pinned URL, expected size and SHA-256, node download log, install record, and resolved binary |
| Antigravity blocked-node fallback download plus SFTP | Not applicable | Not run | Capture outbound block conditions, Hub verification, SFTP transfer, node re-verification, and restoration of network state |
| Antigravity remote OAuth relay | Not applicable | Not run | Capture validated origin, redirect port and state handling, one-shot relay, and final authenticated health without recording secrets |
| Thirty-minute laptop disconnect during remote turn | Not applicable | Not run | Capture pre-disconnect event cursor, node continuity, reconnect cursor, and recovered final events |
| Thirty-minute laptop sleep during remote full-screen terminal | Not applicable | Not run | Capture byte offset before sleep, node terminal continuity, reconnect offset, replay, and final screen |
| Idle node upgrade using R0 artifacts | Not applicable | Not run | Record source commit, manifest, both local and remote hashes, prior version, activation, health, and served version |
| Busy node upgrade refusal or defer | Not applicable | Not run | Capture owned work, refusal or deferred state, unchanged served version, and later safe activation |
| Pasted image to remote provider | Not applicable | Not run | Capture source size/hash, attachment chunks, remote file size/hash, and provider receipt |
| Unpushed worktree delta synchronization | Not applicable | Not run | Capture source commit or bundle base, dirty delta inventory, remote application, and build/test evidence without publishing upstream |
| Cursor common fixture and live smoke | Not run | Not run | Pass both or store a structured disabled reason exposed by protocol |
| Grok common fixture and live smoke | Not run | Not run | Pass both or store a structured disabled reason exposed by protocol |
| OpenCode common fixture and live smoke | Not run | Not run | Pass both or store a structured disabled reason exposed by protocol |

## Authorization and coordination

On 2026-09-28, the user granted full authorization to run the live matrix, including provider quota, OAuth or CLI login, binary and file transfer, Galahad daemon changes, temporary node egress blocking, and coordinated disconnect or sleep checks. Public push and release publication remain outside this recovery run. Interactive browser login and the coordinated thirty-minute laptop sleep still require the user to participate when those rows are reached.

## Gate decision

R0 remains open. Do not continue into Phase 4A as accepted implementation work until every required row passes or the plan records a user-approved rescope.
