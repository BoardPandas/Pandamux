# Phase 3 Production Acceptance Record

Date opened: 2026-09-28

Last updated: 2026-09-28

Status: **Open. The exact Galahad artifact, lifecycle, managed install, transfer,
and Codex provider rows have live evidence. Claude, Antigravity OAuth and provider
interaction, the required laptop disconnect and sleep checks, and native Windows
acceptance remain open.**

This record separates source and fixture verification from production acceptance.
Passing unit, cross-compile, or proxy fixture tests does not satisfy a live Windows or
Galahad row.

## Deciding source

- Recovery baseline: `9511d0e22d44192f106e2f84a7c96b96d76734b6`
- Recovery implementation worktree: `/home/brandon/.codex/worktrees/recovery-r0/Pandamux`
- Recovery implementation branch: `codex/recovery-r0`
- Workspace version at recovery baseline: `0.53.44`
- Foundation commit: `b5afd00ef34e59182eeb7c286672df399b34753e`
- Galahad acceptance commits:
  `e562ae4f114ee63906700b9d4734153b83011b80`,
  `fbb5878`, `59cf023`, `ac8d0cd`, `495b6e6`, `10904b9`, and
  `f9aa2b20a72e008ac6a80fc7a4858f28b0635e37`
- Exact live artifact source: `f9aa2b20a72e008ac6a80fc7a4858f28b0635e37`
- Exact live artifact version: `0.54.7`
- R0 node artifact manifest:
  `target/r0-node-artifacts/0.54.7/x86_64-unknown-linux-musl/r0-node-manifest.txt`
- Publication: disabled

## Source and fixture evidence

| Check | Result | Evidence |
|---|---|---|
| Preserve unfinished Phase 4 work | Pass | Exact baseline, binary patch, untracked archive, hashes, and byte-for-byte dry restore are recorded in `tasks/plan-rewrite.md` under R0.1 |
| Full workspace tests | Pass | `cargo test --workspace`: 296 passed, 0 failed, and 2 real-shell terminal tests intentionally ignored at `0.54.7` |
| Strict static analysis | Pass | `cargo clippy --workspace --all-targets -- -D warnings` |
| Formatting and repository guards | Pass | `cargo fmt --all --check`, `git diff --check`, `scripts/check-rust-boundaries.ps1`, and `scripts/check-claude-wiring.mjs` |
| Dependency advisory gate | Unavailable | `cargo-deny` is not installed on this host, so no advisory result is claimed |
| Production routing and provider lifecycle | Pass in source and fixtures | The Hub owns SSH composition, the proxy has bounded queues and durable replay, exact stored provider instances are resolved, lifetime permits are joined, and explicit cancellation remains authoritative |
| Antigravity source and fixture contract | Pass | Official Google ACP 1.1.1 pins, streaming verification, hardened extraction, live ACP process ownership, strict OAuth relay validation, workspace-confined file capabilities, bounded cancellation, and the two-process ceiling are implemented; live authenticated provider rows remain open |
| R0 artifact exact-commit build | Pass | The artifact harness built and reverified both `0.54.7` musl binaries from clean commit `f9aa2b2`, recorded exact sizes and SHA-256 values, and kept publication disabled |
| Unix local control boundary | Pass | Authenticated CLI checks and tests cover private runtime modes, peer credentials, scoped capabilities, immutable request context, and redacted MCP projections |
| Windows security module MSVC type-check | Pass with scope limitation | The exact `windows_security.rs` module compiled to metadata for `x86_64-pc-windows-msvc` against `windows-sys 0.61.2` |
| Windows workspace cross-check | Pass with scope limitation | `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu` compiled the workspace and Windows-only tests without warnings |
| Native Windows IPC behavior | Not run | A Windows host is still required for SID, DACL, remote-client, PID and image, and anti-squatting behavior |
| Best-effort provider disabled-state contract | Pass on exact Galahad artifact | Exact `0.54.7` health reports persisted Cursor, Grok, and OpenCode as disabled with typed code, message, and remediation fields |

## R0.5 live matrix

| Row | Local Windows | Galahad | Status and evidence |
|---|---|---|---|
| Codex configured-instance login, start, stream, approval, cancel, reconnect, access ceiling | Not run | Pass | Exact `0.54.7` exercised `codex-default`, model `gpt-6-luna`, basic output, approval, read-only denial, image receipt, cancellation, settlement, and replay after a short disconnect. Binary identity and event evidence are below |
| Claude configured-instance login, start, stream, approval, cancel, reconnect, access ceiling | Not run | Blocked on interactive login | Claude Code 2.1.284 is installed on Galahad, but its isolated `claude-default` profile reports unauthenticated. `claude auth login` requires the account owner to complete the browser flow |
| Antigravity configured-instance login, start, stream, approval, cancel, reconnect, access ceiling | Not run | Blocked on interactive login | Google ACP 1.1.1 is installed and healthy enough to begin OAuth, but the isolated `antigravity-default` profile is not authenticated |
| Antigravity node-side managed download | Not applicable | Pass | A fresh isolated `0.54.1` node returned `source=node_download`; the official Google ACP 1.1.1 archive and extracted files matched pinned sizes and hashes, private modes, and cleanup rules |
| Antigravity blocked-node fallback download plus SFTP | Not applicable | Pass | Independent egress failed through closed proxy `127.0.0.1:9`; the Hub returned `source=hub_push`, transferred the verified private archive through SFTP, and the node reverified and extracted it without staging residue |
| Antigravity remote OAuth relay | Not applicable | Blocked on interactive login | The production start path emits a strictly validated Google URL and owns a five-minute flow. Completion requires the account owner to open the URL and return the browser callback while the flow is active |
| Thirty-minute laptop disconnect during remote turn | Not applicable | Not run | A 20-second Hub disconnect passed on exact `0.54.7`, preserving node work and replaying final events after cursor 294. It is supporting evidence, not a substitute for the required 30-minute laptop check |
| Thirty-minute laptop sleep during remote full-screen terminal | Not applicable | Not run | Requires coordinated physical laptop sleep with a live full-screen terminal and offset capture |
| Idle node upgrade using R0 artifacts | Not applicable | Pass through `0.54.7` | Exact artifacts, compatible discovery, idle checks, graceful stops, launcher ownership, authenticated ping and identify, served version, and executable hash were captured. The last acceptance transition was `0.54.6` to `0.54.7` |
| Busy node upgrade refusal or defer | Not applicable | Pass | With PTY `term-a02635e5` active, the `0.54.2` request returned `ssh_daemon_upgrade_deferred` with `retryable=true`; the prior PID, served version, and PTY remained intact |
| Pasted image to remote provider | Not applicable | Pass | Exact `0.54.7` transferred the 970,142-byte PNG with matching SHA-256, and Codex identified its bear subject plus cyan and golden-yellow outlines |
| Unpushed worktree delta synchronization | Not applicable | Pass | The production proxy applied a 400-byte bundle for unpushed commit `25f0f6845f1524913c3b4f2bf644cb798e4399ba`; the remote branch and blob hash match without an upstream push |
| Cursor common fixture and live smoke | Not run | Pass as disabled | Exact `0.54.7` exposes stored code `live_acceptance_incomplete`, an actionable message, and remediation instead of routing work to the unaccepted driver |
| Grok common fixture and live smoke | Not run | Pass as disabled | Exact `0.54.7` exposes stored code `live_acceptance_incomplete`, an actionable message, and remediation instead of routing work to the unaccepted driver |
| OpenCode common fixture and live smoke | Not run | Pass as disabled | Exact `0.54.7` exposes stored code `live_acceptance_incomplete`, an actionable message, and remediation instead of routing work to the unaccepted driver |

## Exact `0.54.7` artifact and node evidence

| Binary | Size | SHA-256 |
|---|---:|---|
| `pandamux-server` | 14,180,816 | `abdca6ac5f2c596b8130354765f5602663d60144981affeed547daaccd9c3b1c` |
| `pandamux` | 1,911,200 | `9494d7351e0dbf7a68cb8761e29bd7b40fd1cb6ee737b3c73707fd8e81c2e40b` |

The manifest records commit `f9aa2b20a72e008ac6a80fc7a4858f28b0635e37`,
target `x86_64-unknown-linux-musl`, generation time `2026-09-29T01:49:45Z`,
and `publication=disabled`. Both outputs are static PIE x86-64 ELF files.

Galahad serves the exact artifact from
`/home/brandon/.pandamux/server/0.54.7/pandamux-server`. The final cleanup restart
created launcher-owned PID `1594516`; `/proc/1594516/exe` resolved to that path and
matched the manifest hash. Authenticated `ping` and `identify` returned role `node`,
environment `env-galahad`, protocol 3, and version `0.54.7`. The daemon had no child
processes after cleanup. Three isolated managed-install acceptance daemons were stopped
gracefully, while their databases and installed artifacts were preserved.

## Codex live evidence

- Provider instance: `codex-default`; model: `gpt-6-luna`; account badge: `ChatGPT`.
- Official Codex CLI: version `0.158.0`, size 286,594,376 bytes, SHA-256
  `167c0148a849d2444f1b5a7fb5f8bb2de1de5ae13a2a504b833fc765980f5cd9`.
- Exact `0.54.7` basic stream settled successfully with only
  `R0_PANDAMUX_CODEX_OK`.
- Ask mode emitted tool execution at sequence 3, `ApprovalRequested` at 4,
  `ApprovalResolved` at 5, tool success at 6, `TurnCompleted` at 37, and
  `TurnSettled` at 38. The marker contained exactly 14 bytes, SHA-256
  `6f5736c74d5aa98a2cb59c06783603f631047285f3c12c6abff4775249255954`,
  and was removed after verification.
- Read-only mode produced no approval, the shell reported a read-only filesystem,
  and the unique marker was independently absent.
- Cancellation interrupted a running `sleep 60`, persisted `TurnCompleted` with
  `Interrupted` at sequence 12 and `TurnSettled` at 13, and left no remote sleep
  process. Durable status readback was `interrupted`.
- A short reconnect test disconnected after proxy cursor 294 while `sleep 15` ran,
  reconnected at cursor 294, replayed the thread events, and settled with exact text
  `R0_RECONNECT_OK`.
- The exact image file and remote attachment were both 970,142 bytes with SHA-256
  `743b7c51f618a7097a2b5192cda4f08a66701043e84d3e9b694901a8292cbcd4`.
  Codex described a bear with cyan and golden-yellow neon outlines.

## Antigravity managed-install evidence

Pinned Google ACP input:

- Version: `agy_acp_server_1.1.1`
- URL: `https://dl.google.com/agy-extensions/releases/linux/agy-acp-server-agy_acp_server_1.1.1-linux-x86_64.zip`
- Archive size: 681,969,407 bytes
- Archive SHA-256: `38f62d01b32deb0907b3d39a71ec301fd36369f6ffd1cf262d4af385177f79df`
- `agy_acp_server.par`: 1,880,360,328 bytes, mode `0755`
- `localharness_external`: 128,966,920 bytes, mode `0755`
- `active.json`: mode `0600`

The direct-download and blocked-egress fallback proofs ran on isolated exact `0.54.1`
nodes because those rows predate the later provider fixes. The installed bundle is the
same pinned Google ACP 1.1.1 payload used by the current `0.54.7` node. These rows prove
installation and transfer only, not Google authentication or provider interaction.

## Remote transfer evidence

The production SSH proxy synchronized local unpushed commit
`25f0f6845f1524913c3b4f2bf644cb798e4399ba` into Galahad branch
`r0-live-e562ae4`. The remote `r0-delta.txt` hash is
`5367e3aceb0084187252080e99b5bec5d7ebaaa74a123a1b7b85503d26cc5386`.
No upstream push occurred.

## Authorization and coordination

On 2026-09-28, the user granted full authorization to run the live matrix, including
provider quota, OAuth or CLI login, binary and file transfer, Galahad daemon changes,
temporary node egress blocking, and coordinated disconnect or sleep checks. Public push
and release publication remain outside this recovery run. The two browser logins, the
physical laptop sleep checks, and the native Windows run require interactive or external
host participation when those rows are scheduled.

## Gate decision

R0 remains open. Do not continue into Phase 4A as accepted implementation work until
every required row passes or the plan records a user-approved rescope.
