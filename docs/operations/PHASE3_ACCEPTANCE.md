# Phase 3 Production Acceptance Record

Date opened: 2026-09-28

Last updated: 2026-09-29

Status: **Open for production acceptance, closed as a Phase 4 development gate by
explicit user-approved rescope on 2026-09-29. The exact Galahad artifact, lifecycle,
managed install, transfer, Codex, Claude, and Antigravity provider rows, and native
Windows IPC behavior have live evidence. The required 30-minute physical laptop
disconnect and sleep checks remain open and still block release claims.**

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
  `f9aa2b20a72e008ac6a80fc7a4858f28b0635e37`, followed by
  `9c196e8d492f44ba1f0cfaacf82a0b71e06aca0a`,
  `8b5d104a294fda9389ab49f40980540327b1191e`, and
  `28614286b6ce2e9b14f1c4279bee4cd69709eb34`
- Exact live artifact source: `28614286b6ce2e9b14f1c4279bee4cd69709eb34`
- Exact live artifact version: `0.54.11`
- R0 node artifact manifest:
  `target/r0-node-artifacts/0.54.11/x86_64-unknown-linux-musl/r0-node-manifest.txt`
- Native Windows acceptance commit: `b314ae79f1f16d1eac3a5360eba340dda70884e8`
- Native Windows acceptance version: `0.54.12`
- Native Windows build target: `x86_64-pc-windows-gnu`
- Publication: disabled

## Source and fixture evidence

| Check | Result | Evidence |
|---|---|---|
| Preserve unfinished Phase 4 work | Pass | Exact baseline, binary patch, untracked archive, hashes, and byte-for-byte dry restore are recorded in `tasks/plan-rewrite.md` under R0.1 |
| Full workspace tests | Pass | `cargo test --workspace`: 302 passed, 0 failed, and 2 real-shell terminal tests intentionally ignored at `0.54.11` |
| Strict static analysis | Pass | `cargo clippy --workspace --all-targets -- -D warnings` |
| Formatting and repository guards | Pass | `cargo fmt --all --check`, `git diff --check`, `scripts/check-rust-boundaries.ps1`, and `scripts/check-claude-wiring.mjs` |
| Dependency advisory gate | Unavailable | `cargo-deny` is not installed on this host, so no advisory result is claimed |
| Production routing and provider lifecycle | Pass in source and fixtures | The Hub owns SSH composition, the proxy has bounded queues and durable replay, exact stored provider instances are resolved, lifetime permits are joined, and explicit cancellation remains authoritative |
| Antigravity source and fixture contract | Pass | Official Google ACP 1.1.1 pins, streaming verification, hardened extraction, live ACP process ownership, strict OAuth relay validation, workspace-confined file capabilities, opaque permission option IDs, bounded cancellation, and the two-process ceiling are implemented and have authenticated live evidence |
| R0 artifact exact-commit build | Pass | The artifact harness built and reverified both `0.54.11` musl binaries from clean commit `2861428`, recorded exact sizes and SHA-256 values, and kept publication disabled |
| Unix local control boundary | Pass | Authenticated CLI checks and tests cover private runtime modes, peer credentials, scoped capabilities, immutable request context, and redacted MCP projections |
| Windows security module MSVC type-check | Pass with scope limitation | The exact `windows_security.rs` module compiled to metadata for `x86_64-pc-windows-msvc` against `windows-sys 0.61.2` |
| Windows workspace cross-check | Pass with scope limitation | `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu` compiled the workspace and Windows-only tests without warnings |
| Native Windows IPC behavior | Pass | Exact `0.54.12` server and CLI artifacts from clean commit `b314ae7` passed SID-scoped naming, the user-only DACL, remote-client rejection, PID and image enforcement, and first-instance anti-squatting on Windows 11 |
| Best-effort provider disabled-state contract | Pass on exact Galahad artifact | Exact `0.54.7` health reports persisted Cursor, Grok, and OpenCode as disabled with typed code, message, and remediation fields |

## Native Windows IPC evidence

The exact Windows binaries were built from clean commit
`b314ae79f1f16d1eac3a5360eba340dda70884e8` and workspace version `0.54.12` with:

```bash
rustup target add x86_64-pc-windows-gnu
CARGO_TARGET_DIR=/tmp/pandamux-r0-windows-target cargo build --locked --release \
  --target x86_64-pc-windows-gnu -p pandamux-server -p pandamux-cli
```

| Artifact | Size | SHA-256 |
|---|---:|---|
| `pandamux-server.exe` | 24,646,772 | `3905612691373864a9a72b13cac4747131df4c5a1c5900cde9e3e1df601578b6` |
| `pandamux-cli.exe` | 4,088,486 | `816039e496e7a434438d9494df111249e7aa2ee314f40edb413faf699cb21e6e` |
| Transfer ZIP | 9,727,955 | `1a3659f76ef886b132305d7d22f351860f5b9a6108999cea0951ad04fb741f95` |

The execution host was Windows 11 Pro system `COLT-SHINE`. Tests ran inside its
isolated Hyper-V guest `SF-C4AD-VM`, Windows 11 Enterprise Evaluation build 26200,
as local user `SF-C4AD-VM\C4adUser`, SID
`S-1-5-21-3343393599-1391300870-1798795327-1001`. The server created pipe
`\\.\pipe\pandamux-hub-S-1-5-21-3343393599-1391300870-1798795327-1001`.
The exact CLI reported `pandamux v0.54.12`; authenticated `ping` passed at about
0.6 ms, and `identify` returned `platform=windows`, `role=hub`, and protocol 3.

The same-user PowerShell control probe reached the protected pipe but was rejected
before routing. Server stderr recorded `Rejected local pipe client: named-pipe client
image is not approved: powershell.exe`. A separate `SF-C4AD-VM\C4adAdmin` process was
denied with `Access to the path is denied.` A second PandaMUX server process exited 1
with Windows error code 5, `PermissionDenied`, and `Access is denied`, while the first
server remained healthy and answered another authenticated ping.

Remote-client rejection was verified with a controlled baseline. The guest temporarily
used Hyper-V's `Default Switch`, where the host was `172.25.80.1` and the guest was
`172.25.86.186`. A control pipe with the same user ACL but without remote rejection
accepted the authenticated host connection and returned `CONTROL_OK`. The corresponding
host connection to the PandaMUX pipe failed with `Access to the path is denied`. Local
PandaMUX pings passed immediately before and after the remote probe, and the integrated
result reported `Passed: true`.

Checkpoint `pre-pandamux-r0-b314ae7-20260929` protected the guest before mutation.
Attaching the test network triggered a Hyper-V save and restore that rolled the guest
disk back to that checkpoint, so the deciding run attached networking first, waited for
restore, then recopied and rehashed both binaries before testing. At acceptance cleanup,
the temporary credential mapping, firewall rule, guest network attachment, and test
processes were removed. A later audit found no PandaMUX guest staging root, firewall
rule, or process. The shared C4ad lab had subsequently acquired newer unrelated
checkpoints and a new network attachment, which this recovery run deliberately left
untouched. The host-side PandaMUX staging directory was removed, and the SupportForge
remote session was explicitly disconnected.

## R0.5 live matrix

Provider execution rows are decided on Galahad, the Phase 3 remote environment. The
Local Windows cells retain whether the same provider scenario was also repeated there;
the required Windows-specific R0.5 evidence is the native IPC row above.

| Row | Local Windows | Galahad | Status and evidence |
|---|---|---|---|
| Codex configured-instance login, start, stream, approval, cancel, reconnect, access ceiling | Not run | Pass | Exact `0.54.7` exercised `codex-default`, model `gpt-6-luna`, basic output, approval, read-only denial, image receipt, cancellation, settlement, and replay after a short disconnect. Binary identity and event evidence are below |
| Claude configured-instance login, start, stream, approval, cancel, reconnect, access ceiling | Not run | Pass | Exact `0.54.11` exercised authenticated isolated instance `claude-default`, configured model selector `default`, current structured stream input and nested output, one Ask approval with exact pending input, ReadOnly denial, cancellation, settlement, and replay after a short disconnect |
| Antigravity configured-instance login, start, stream, approval, cancel, reconnect, access ceiling | Not run | Pass | Exact `0.54.11` exercised authenticated isolated instance `antigravity-default` against pinned Google ACP 1.1.1, including streamed output, opaque approval option IDs, pre-tool ReadOnly denial, cancellation, settlement, and replay after a short disconnect |
| Antigravity node-side managed download | Not applicable | Pass | A fresh isolated `0.54.1` node returned `source=node_download`; the official Google ACP 1.1.1 archive and extracted files matched pinned sizes and hashes, private modes, and cleanup rules |
| Antigravity blocked-node fallback download plus SFTP | Not applicable | Pass | Independent egress failed through closed proxy `127.0.0.1:9`; the Hub returned `source=hub_push`, transferred the verified private archive through SFTP, and the node reverified and extracted it without staging residue |
| Antigravity remote OAuth relay | Not applicable | Pass | The production relay completed the strictly validated Google callback for `antigravity-default`; the resulting isolated token file is private mode `0600`, and subsequent proxy health plus live ACP turns passed without exposing callback or token material |
| Thirty-minute laptop disconnect during remote turn | Not applicable | Not run | Twenty-second Hub disconnects passed for Codex on exact `0.54.7` and for Claude and Antigravity on exact `0.54.11`, preserving node work and replaying final events. These are supporting evidence, not substitutes for the required 30-minute physical laptop check |
| Thirty-minute laptop sleep during remote full-screen terminal | Not applicable | Not run | Requires coordinated physical laptop sleep with a live full-screen terminal and offset capture |
| Idle node upgrade using R0 artifacts | Not applicable | Pass through `0.54.11` | Exact artifacts, compatible discovery, idle checks, graceful stops, launcher ownership, authenticated ping and identify, served version, and executable hashes were captured. The last acceptance transition was `0.54.10` to `0.54.11` |
| Busy node upgrade refusal or defer | Not applicable | Pass | With PTY `term-a02635e5` active, the `0.54.2` request returned `ssh_daemon_upgrade_deferred` with `retryable=true`; the prior PID, served version, and PTY remained intact |
| Pasted image to remote provider | Not applicable | Pass | Exact `0.54.7` transferred the 970,142-byte PNG with matching SHA-256, and Codex identified its bear subject plus cyan and golden-yellow outlines |
| Unpushed worktree delta synchronization | Not applicable | Pass | The production proxy applied a 400-byte bundle for unpushed commit `25f0f6845f1524913c3b4f2bf644cb798e4399ba`; the remote branch and blob hash match without an upstream push |
| Cursor common fixture and live smoke | Not run | Pass as disabled | Exact `0.54.7` exposes stored code `live_acceptance_incomplete`, an actionable message, and remediation instead of routing work to the unaccepted driver |
| Grok common fixture and live smoke | Not run | Pass as disabled | Exact `0.54.7` exposes stored code `live_acceptance_incomplete`, an actionable message, and remediation instead of routing work to the unaccepted driver |
| OpenCode common fixture and live smoke | Not run | Pass as disabled | Exact `0.54.7` exposes stored code `live_acceptance_incomplete`, an actionable message, and remediation instead of routing work to the unaccepted driver |

## Exact `0.54.11` artifact and node evidence

| Binary | Size | SHA-256 |
|---|---:|---|
| `pandamux-server` | 14,207,968 | `808941358cf535af935bb41aadf60ab5c5c840f4ee1a63ef427285b61263f7a7` |
| `pandamux` | 1,913,288 | `33dad015cd4f22b9392222c09770d24a521d1b8a78029a31f8a5a5958f088a87` |

The manifest records commit `28614286b6ce2e9b14f1c4279bee4cd69709eb34`,
target `x86_64-unknown-linux-musl`, generation time `2026-09-29T02:50:52Z`,
and `publication=disabled`. Both outputs are static PIE x86-64 ELF files.

Galahad serves the exact artifact from
`/home/brandon/.pandamux/server/0.54.11/pandamux-server`. The final cleanup restart
created launcher-owned PID `1840415`; `/proc/1840415/exe` resolved to that path and
both deployed binaries matched the manifest hashes. The bootstrap returned environment
`env-galahad`, protocol 3, and version `0.54.11`. The daemon had no child processes
after cleanup. Earlier exact-artifact lifecycle and managed-install databases and
installed artifacts remain preserved.

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

## Claude live evidence

- Provider instance: `claude-default`; configured model selector: `default`; isolated
  Claude Code version: `2.1.284`.
- Exact `0.54.11` basic streaming produced only `R0_PANDAMUX_CODEX_OK`, then
  `TurnCompleted` and `TurnSettled`.
- Ask mode emitted Bash execution at sequence 3, `ApprovalRequested` at 4,
  `ApprovalResolved` at 5, tool success at 6, completion at 8, and settlement at 9.
  PandaMUX returned Claude's exact pending tool input in the current nested control
  response. The marker contained exactly 14 bytes with SHA-256
  `6f5736c74d5aa98a2cb59c06783603f631047285f3c12c6abff4775249255954`
  and was removed after independent verification.
- ReadOnly initially exposed Claude's `ExitPlanMode` approval escape. Commit `2861428`
  added that tool to the ReadOnly deny list without replacing configured restrictions.
  The exact rebuilt artifact then produced no approval, completed and settled normally,
  and left the unique marker independently absent.
- Cancellation interrupted a running `sleep 60`, persisted `TurnCompleted` with
  `Interrupted` at sequence 4 and `TurnSettled` at 5, recorded durable status
  `interrupted`, and left no matching process.
- The short reconnect test disconnected after proxy cursor 494, reconnected at 496,
  replayed the completed tool event, returned exact text `R0_RECONNECT_OK`, and settled.
- Live discovery also corrected the current Claude stream input, nested assistant and
  tool-result output, approval response, interrupt request, error result, full-access
  opt-in, and permission-mode contracts in commits `9c196e8`, `8b5d104`, and `2861428`.

## Antigravity live evidence

- Provider instance: `antigravity-default`; pinned provider version:
  `agy_acp_server_1.1.1`. The production OAuth relay completed for the isolated profile,
  and the resulting token file is mode `0600`.
- Exact `0.54.11` basic output arrived as two deltas that reconstructed to
  `R0_PANDAMUX_CODEX_OK`, followed by successful completion and settlement.
- Ask mode exercised three live ACP permission requests, including the requested write.
  PandaMUX selected the exact opaque `allow` option offered by the agent and preserved
  the high-risk metadata on `allow_always`. The requested marker contained exactly
  14 bytes with the expected SHA-256 and was removed after independent verification.
- ReadOnly denied the requested shell write in the pre-tool policy without surfacing an
  approval. The turn completed and settled, and the exact marker path was absent.
- Cancellation interrupted a running `sleep 60`, persisted `TurnCompleted` with
  `Interrupted` at sequence 4 and `TurnSettled` at 5, recorded durable status
  `interrupted`, and left no matching process.
- The definitive short reconnect test disconnected after proxy cursor 569 and
  reconnected at 570. Replayed events stayed contiguous, the accumulated stream ended
  with `R0_RECONNECT_OK`, and completion and settlement arrived at sequences 15 and 16.
- A first ceiling observation overran the sequential local acceptance receiver while
  Antigravity emitted more than 512 rapid deltas. The ignored harness gained a dedicated
  event pump and explicit lag failure before the definitive rerun. No production source
  or deployed artifact changed for that harness correction.

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
same pinned Google ACP 1.1.1 payload used by the current `0.54.11` node. Those earlier
rows prove installation and transfer; the separate `0.54.11` live evidence above proves
Google authentication and provider interaction.

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
and release publication remain outside this recovery run. Both browser login flows and
the native Windows run are complete. The physical laptop disconnect and sleep checks
still require interactive participation when those rows are scheduled.

## Gate decision

The user approved the documented rescope on 2026-09-29, so R0 is closed as a Phase 4
development gate and implementation may continue. The 30-minute physical laptop
disconnect and full-screen-terminal sleep rows were deferred, not waived. This record
remains open, and PandaMUX must not be described as production-accepted, released, or
shipped until those rows pass and their evidence is archived.
