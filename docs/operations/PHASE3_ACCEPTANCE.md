# Phase 3 Production Acceptance Record

Date opened: 2026-09-28

Status: **Open. Galahad artifact, bootstrap, managed-install, and transfer rows have
live evidence; provider interaction, OAuth, long disconnect or sleep, busy upgrade,
and native Windows rows remain open.**

This record separates source and fixture verification from production acceptance. Passing unit or proxy fixture tests does not satisfy a live Windows or Galahad row.

## Deciding source

- Recovery baseline: `9511d0e22d44192f106e2f84a7c96b96d76734b6`
- Recovery implementation worktree: `/home/brandon/.codex/worktrees/recovery-r0/Pandamux`
- Recovery implementation branch: `codex/recovery-r0`
- Workspace version at recovery baseline: `0.53.44`
- Foundation commit: `b5afd00ef34e59182eeb7c286672df399b34753e`
- Galahad acceptance-fix commit: `e562ae4f114ee63906700b9d4734153b83011b80`
- Exact live artifact version: `0.54.1`
- R0 node artifact manifest:
  `target/r0-node-artifacts/x86_64-unknown-linux-musl/r0-node-manifest.txt`
- Publication: disabled

## Source and fixture evidence

| Check | Result | Evidence |
|---|---|---|
| Preserve unfinished Phase 4 work | Pass | Exact baseline, binary patch, untracked archive, hashes, and byte-for-byte dry restore are recorded in `tasks/plan-rewrite.md` under R0.1 |
| Server, request-context, proxy, and provider lifecycle tests | Pass | `cargo test -p pandamux-server`: 53 library, 2 server binary, 6 proxy transport, and 7 thread lifecycle tests passed |
| Antigravity source and fixture contract | Pass | Official Google ACP 1.1.1 pins and object-valued capabilities, streaming archive verification, hardened two-entry extraction, real ACP and OAuth ownership, workspace-confined file handlers, opaque permission IDs, bounded cancellation, environment-wide two-process enforcement, and verified Hub-to-node SFTP fallback are implemented; live rows remain open |
| Full workspace tests | Pass | `cargo test --workspace`: 294 passed, 0 failed, and 2 real-shell terminal tests intentionally ignored for the `0.54.2` candidate; the preceding exact `e562ae4` artifact source passed 291 with the same two ignores |
| Strict static analysis | Pass | `cargo clippy --workspace --all-targets -- -D warnings` |
| Protocol, client, CLI, server, and desktop compile check | Pass | `cargo check -p pandamux-protocol -p pandamux-client -p pandamux-cli -p pandamux-server -p pandamux-desktop` |
| Server dependency boundary | Pass | `cargo tree` showed `pandamux-remote` composed only by `pandamux-server`; the server terminal graph contains `portable-pty` and excludes `alacritty_terminal` |
| R0 artifact exact-commit build | Pass | `scripts/r0-node-artifacts.sh build` refused a dirty tree, then built both `0.54.1` musl binaries from clean commit `e562ae4`, generated and re-verified the manifest, and kept publication disabled |
| Unix same-user CLI over production socket | Pass | A temporary Hub accepted authenticated `ping` and `identify`; runtime directory, record, and socket modes were `0700`, `0600`, and `0600`; generated state was moved to recoverable trash |
| Formatting, whitespace, crate boundaries, and wiring guards | Pass | `cargo fmt --all --check`, `git diff --check`, `scripts/check-rust-boundaries.ps1`, and `scripts/check-claude-wiring.mjs` |
| Windows security module MSVC type-check | Pass with scope limitation | The exact `windows_security.rs` module compiled to metadata for `x86_64-pc-windows-msvc` against `windows-sys 0.61.2` |
| Windows workspace cross-check | Pass with scope limitation | `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu` compiled the full workspace and Windows-only tests without warnings; the temporary target was removed afterward |
| Full Windows server cross-check from Linux | Blocked by verification host | Dependency build scripts stopped before PandaMUX code because MSVC `lib.exe` is absent; `ring` and bundled SQLite could not build |
| Native Windows IPC behavior | Not run | Requires a Windows host to run SID, DACL, remote-client, PID/image, and anti-squatting tests |
| Galahad artifact build and transfer | Pass | Both remote files match the local manifest size and SHA-256. Bootstrap first preserved a compatible discovered `0.54.0` daemon, then activated exact launcher-owned `0.54.1` only after an authorized idle graceful stop |
| Best-effort provider disabled-state contract | Pass in source, live persistence pending `0.54.2` | Settings schema v3 persists typed disabled reasons for Cursor, Grok, and OpenCode; focused core, protocol, resolver, and health tests pass |

## R0.5 live matrix

| Row | Local Windows | Galahad | Status and required evidence |
|---|---|---|---|
| Codex configured-instance login, start, stream, approval, cancel, reconnect, access ceiling | Not run | Not run | Capture instance ID, model, exact binary SHA, event log, approval decision, cancellation, reconnect cursor, and ceiling denial |
| Claude configured-instance login, start, stream, approval, cancel, reconnect, access ceiling | Not run | Not run | Capture the same evidence as Codex |
| Antigravity configured-instance login, start, stream, approval, cancel, reconnect, access ceiling | Not run | Not run | Also capture managed bundle version and scratch-volume preflight |
| Antigravity node-side managed download | Not applicable | Pass | Fresh isolated exact `0.54.1` node returned `source=node_download`; Google ACP 1.1.1 extracted to the pinned hash directory with exact sizes, executable modes, private active record, and no staging residue |
| Antigravity blocked-node fallback download plus SFTP | Not applicable | Pass | Independent curl failed through closed proxy `127.0.0.1:9`; the production Hub path returned `source=hub_push`, cached the exact private archive, transferred it over SFTP, re-verified and extracted it on the node, and removed staged files |
| Antigravity remote OAuth relay | Not applicable | Not run | Capture validated origin, redirect port and state handling, one-shot relay, and final authenticated health without recording secrets |
| Thirty-minute laptop disconnect during remote turn | Not applicable | Not run | Capture pre-disconnect event cursor, node continuity, reconnect cursor, and recovered final events |
| Thirty-minute laptop sleep during remote full-screen terminal | Not applicable | Not run | Capture byte offset before sleep, node terminal continuity, reconnect offset, replay, and final screen |
| Idle node upgrade using R0 artifacts | Not applicable | Pass for `0.54.0` to `0.54.1` | Exact hashes, compatible-discovery behavior, idle state, graceful stop, launcher ownership, authenticated ping and identify, and served version were captured |
| Busy node upgrade refusal or defer | Not applicable | Not run | Capture owned work, refusal or deferred state, unchanged served version, and later safe activation |
| Pasted image to remote provider | Not applicable | Partial | Production Hub chunk transfer passed for a 970,142-byte PNG and the node SHA-256 matched. Provider receipt remains blocked on authenticated provider acceptance |
| Unpushed worktree delta synchronization | Not applicable | Pass | Production Hub created and routed a 400-byte bundle for unpushed commit `25f0f6845f1524913c3b4f2bf644cb798e4399ba`; the remote branch and blob hash match without an upstream push |
| Cursor common fixture and live smoke | Not run | Pending disabled-state deployment | Source defaults it disabled with code `live_acceptance_incomplete`, message, and remediation; exact `0.54.2` live protocol proof remains |
| Grok common fixture and live smoke | Not run | Pending disabled-state deployment | Source defaults it disabled with code `live_acceptance_incomplete`, message, and remediation; exact `0.54.2` live protocol proof remains |
| OpenCode common fixture and live smoke | Not run | Pending disabled-state deployment | Source defaults it disabled with code `live_acceptance_incomplete`, message, and remediation; exact `0.54.2` live protocol proof remains |

## Exact `0.54.1` artifact evidence

| Binary | Size | SHA-256 |
|---|---:|---|
| `pandamux-server` | 13,900,000 | `26e0604cc7284742ae295ad4f8276c60b1a0136b0c78d8d4942c34f9339174da` |
| `pandamux` | 1,914,376 | `766b774160b341995f0e9e04ddfa32e18799b543f88506b6900ffda6dc868cd6` |

The manifest records commit `e562ae4f114ee63906700b9d4734153b83011b80`,
target `x86_64-unknown-linux-musl`, generation time `2026-09-29T00:25:29Z`,
and `publication=disabled`. Both outputs are static PIE x86-64 ELF files.

Galahad first ran `0.54.0` as PID `899752`. Production bootstrap transferred and
verified `0.54.1` but returned `ownership=Discovered`, `runningVersion=0.54.0`,
and `launcherOwned=false`, so it did not replace compatible manually owned work. The
authenticated idle check returned zero terminals, zero threads, and zero active
Antigravity processes. After the authorized `SIGINT`, the old process exited and its
record and both sockets disappeared. A second bootstrap started PID `1088271` with
`ownership=LauncherOwned`, `runningVersion=0.54.1`, and `launcherOwned=true`.
`/proc/1088271/exe` matched the server manifest hash. The runtime record and both
sockets were mode `0600`, and authenticated `ping` and `identify` returned the expected
node, environment `env-galahad`, server `0.54.1`, and protocol 3.

## Antigravity managed-install evidence

Pinned Google ACP input:

- Version: `agy_acp_server_1.1.1`
- URL: `https://dl.google.com/agy-extensions/releases/linux/agy-acp-server-agy_acp_server_1.1.1-linux-x86_64.zip`
- Archive size: 681,969,407 bytes
- Archive SHA-256: `38f62d01b32deb0907b3d39a71ec301fd36369f6ffd1cf262d4af385177f79df`
- `agy_acp_server.par`: 1,880,360,328 bytes, mode `0755`
- `localharness_external`: 128,966,920 bytes, mode `0755`
- `active.json`: mode `0600`

The fresh direct-download root
`/home/brandon/.pandamux-r0-node-download-e562ae4` returned
`source=node_download`, reported installed version 1.1.1 and process capacity 0 of 2,
and left zero `.install-*` directories. The separate blocked-egress root
`/home/brandon/.pandamux-r0-fallback-e562ae4` inherited HTTP, HTTPS, and ALL proxy
settings pointing at closed port 9. Curl independently confirmed that Google access was
refused. The Hub cache under `/tmp/pandamux-r0-fallback-exact-e562ae4` matched the
pinned archive size and hash at mode `0600`; production fallback returned
`source=hub_push`, the node output sizes and modes matched, and staged upload count was
zero. Both isolated nodes ran the exact `0.54.1` server hash.

## Remote transfer evidence

The production SSH proxy synchronized local unpushed commit
`25f0f6845f1524913c3b4f2bf644cb798e4399ba` into Galahad branch
`r0-live-e562ae4`. The response reported one applied commit and a 400-byte bundle. The
remote branch points to the exact commit, and `r0-delta.txt` has matching SHA-256
`5367e3aceb0084187252080e99b5bec5d7ebaaa74a123a1b7b85503d26cc5386`.
No upstream push occurred.

The same proxy imported `resources/icon.png` through the Hub, split it into protocol
chunks, and assembled it in the remote thread attachment directory. Local and remote
size is 970,142 bytes and SHA-256 is
`743b7c51f618a7097a2b5192cda4f08a66701043e84d3e9b694901a8292cbcd4`.
This proves transport and storage, not provider receipt.

## Authorization and coordination

On 2026-09-28, the user granted full authorization to run the live matrix, including provider quota, OAuth or CLI login, binary and file transfer, Galahad daemon changes, temporary node egress blocking, and coordinated disconnect or sleep checks. Public push and release publication remain outside this recovery run. Interactive browser login and the coordinated thirty-minute laptop sleep still require the user to participate when those rows are reached.

## Gate decision

R0 remains open. Do not continue into Phase 4A as accepted implementation work until every required row passes or the plan records a user-approved rescope.
