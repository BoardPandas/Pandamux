# Phase 4 Acceptance Record

Date: 2026-09-29
Updated: 2026-10-07

Status: **Accounting accepted, Phase 4 live acceptance incomplete.** Exact 0.60.15
commit `3a77382` passed the P4-C1 implementation, P4-C2 local candidate, and
P4-C4 cloned-store reconciliation gates. Fresh simple and tool-using Claude turns
matched transcript usage by exact message ID, repeated scan and restart remained
stable, and the historical seven-day query matched direct SQL. Independent
provider reports were unavailable, so each external comparison is explicitly
Unknown. Earlier exact candidates retain their named Windows UI and live-provider
evidence only. The remaining exact-candidate native interaction, durability,
failover, current IPC, and physical matrices keep Phase 4 open. No push,
deployment, production activation, or release occurred.

## Source and preservation boundary

- Exact executable source: clean `codex/recovery-r0` commit
  `96aad1d16f357f1bc70d46b9889ad403550a4d1a`, version `0.60.0`.
- Exact native Usage accessibility follow-up source: clean `codex/recovery-r0`
  commit `2be212f9a290637add9a4f5c662ca3155da98a0c`, version `0.60.3`.
- Exact accounting source: clean `codex/recovery-r0` commit `3a77382`, version
  `0.60.15`. This documentation-only follow-up uses workspace version `0.60.16`;
  no 0.60.16 native or live result is claimed.
- The intermediate 0.60.2 commit `8a52d2a21da1d5d0f69540864a6162d2b30375bc`
  repaired direct UI Automation activation for Save and Cancel. The 0.60.3
  follow-up exposed the editor text fields on semantic nodes tied to their real
  editing focus. This documentation follow-up uses workspace version `0.60.4`.
  No 0.60.4 Windows or Galahad executable is claimed as tested here.
- Original checkout: `/home/brandon/Repos/Pandamux` at recovery baseline
  `9511d0e22d44` with the unfinished Phase 4 prototype preserved. R0.1 archived
  a binary-safe patch and untracked-file tar, recorded their hashes, and verified
  every captured path by byte comparison in a detached restore worktree before
  source editing. See `tasks/plan-rewrite.md` R0.1 for the archive location and
  exact hashes. The original dirty checkout was not reset, cleaned, or staged.
- The Galahad acceptance root was
  `/home/brandon/.pandamux/acceptance/phase4-96aad1d`; its node runtime,
  database, profiles, and test repository were separate from the existing
  `/home/brandon/.pandamux/run` production runtime. The production 0.54.11
  daemon remained running throughout. The test node was stopped after acceptance,
  and its private database and artifacts remain for inspection.
- The Windows host built from an isolated clean source clone under
  `C:\ProgramData\PandaMUX\acceptance\phase4-96aad1d\source`. The host's
  existing `C:\Github\Pandamux` checkout was not edited. The guest
  `SF-C4AD-VM` used `C:\ProgramData\PandaMUX\acceptance\phase4-96aad1d` and
  `C:\Users\C4adUser\AppData\Local\PandaMUX-Phase4-96aad1d` only.

## Exact artifacts and startup

The Linux artifacts were built from the clean commit with publication disabled:

```bash
scripts/r0-node-artifacts.sh build
```

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Linux musl `pandamux-server` | 17,497,312 | `5e1471fad2af3de5f95e72f21ac09b764b052a4d976c90469c2cac3165a58c2b` |
| Linux musl `pandamux` CLI | 2,020,336 | `32a80cdc6253ebee945a969397065158ae673372a07a17b0496fe93529970072` |
| Windows GNU `pandamux-server.exe` | 28,433,717 | `06479f8bd295f3a4cb7eefaf3bfc48f4b2a4970fd690254c7432db8124931688` |
| Windows GNU `pandamux-cli.exe` | 4,562,000 | `60acc30f7a50aaa120e51186190a7f536ff91b5a34de6f8cc81013520ca11160` |
| Windows MSVC `pandamux.exe` | 27,208,704 | `a2762195c586a7042b61bbd53c0199e2b6170431c8ad9f9c0c17968dfbfce2c9` |

The Linux hashes were reverified on Galahad after transfer. The Windows hashes
were reverified on `SF-C4AD-VM` after transfer. The Windows desktop was built
natively on `COLT-SHINE` with MSVC, the Windows SDK shader compiler, and
`cargo build --locked --release -p pandamux-desktop`. Windows server and CLI
were cross-built with
`cargo build --locked --release --target x86_64-pc-windows-gnu -p pandamux-server -p pandamux-cli`.
The Linux-to-Windows desktop cross-build did not complete because the pinned
GPUI Windows build expects its generated shader file from a native Windows
build script. Native MSVC desktop build succeeded instead.

The clean guest lacked `VCRUNTIME140.dll`, so the first desktop launch failed
with Windows `0xC0000135`. An x64 Microsoft VC142 runtime DLL was copied
app-locally alongside the test executable, not installed globally; both copies
hashed to `270ba17bf89cc2eb6e6ce536eb78e4614ab5499f653ab7e07fe9e3d33bf3b6e3`.
With that DLL, `pandamux.exe --headless` exited 0. A scheduled task in the
interactive `SF-C4AD-VM\C4adUser` session ran `pandamux.exe --smoke`; its
`smoke.exit` is `0`, and `smoke.log` records that the window initialized and
rendered cleanly. The exact guest server and CLI also passed authenticated
`hello`, `ping`, and `identify` under the SupportForge SYSTEM session. That
SYSTEM test is not a substitute for the 0.60.0 normal-user named-pipe security
matrix. The earlier exact 0.54.12 normal-user matrix is recorded in
`docs/operations/PHASE3_ACCEPTANCE.md`.

## Exact 0.60.15 accounting and reconciliation acceptance

The clean accounting candidate is commit `3a77382`, version `0.60.15`. Its
isolated Galahad root is
`/home/brandon/.pandamux/acceptance/phase4-3a77382-0.60.15-p4-c4`.
The retained source at
`/home/brandon/.pandamux/acceptance/phase4-96aad1d` was copied before migration
or scan. The exact candidate artifacts used on Galahad were:

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Linux `pandamux-server` | 18,204,088 | `d4f9b3d04eb5f8218b109c0ec24a0bb15f9b176d485a5e53e0f016f5edfbea91` |
| Linux `pandamux-cli` | 2,034,000 | `aebaf76070d0f2fe0320ba6f31a78a33d4e526a2dd73b82bdcdfaf21c409108f` |

The same clean commit also produced local release artifacts and passed the
desktop and control-plane floor:

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Linux desktop `pandamux` | 60,149,672 | `14ee6a5ee522ac816fa3a03e2b8c13cbc0c22f11dcc22575050ca59ec628512a` |
| Linux `pandamux-server` | 18,800,232 | `c33dffcdda711be81dea814e04720bb9e12c7211f8e160cd5bb90bd88fb437f2` |
| Linux `pandamux-cli` | 2,174,664 | `68cc5bea36456d7879948dccc456cec99ade3d586d5da01ffa63b2eacb5ea344` |

`pandamux --headless` returned 0 with `ALL CHECKS PASSED`. An isolated exact
server and CLI completed authenticated `system.hello`, `system.ping`, and
`system.identify` with protocol 3, node role, version 0.60.15, and the expected
test environment. Formatting, strict all-target and all-feature Clippy, 423
workspace tests with zero failures and two intentional real-shell ignores, the
Rust boundary guard, the Claude wiring guard, and `cargo deny` all passed.
Dependency policy retained non-failing warnings for duplicate dependency
versions, one unmatched license allowance, and yanked transitive `chacha20
0.10.1` and `yoke-derive 0.8.3`.

### Clone, migration, and source fitness

Before migration, the clone matched the retained source database, WAL, SHM,
Claude corpus, and Codex corpus by SHA-256. It held migration 11, 23 usage
observations, 23 outbox rows, 3 quarantine rows, and 24 scanner cursors. The
source roots contained 15 Claude JSONL files totaling 2,527,996 bytes and 19
Codex JSONL files totaling 4,707,708 bytes. Their stable content digests were
`ee02fa6bfcc7cef98dfb7fdfea07970ece2337508640777e6c64ad2cdab2ba09`
and `eeaa3ac29f326609bcf845bfb2a89377c3d8c500c34ba6e02068e527857addb1`.

Only clone-local absolute paths were remapped. The first exact startup applied
migration 12 and rescanned the copied corpus. It stabilized at 43 observations,
43 outbox rows, 6 quarantine rows, 58 file cursors, 4 retained exact-ID Codex
model mismatches, and 1 retained Claude legacy-debt row. A second startup left
all counts and normalized observation digest
`85ad392b70959681dbacbac1826fed2f289a76d3fb5d0518da5b6733813ef305`
unchanged. Product `usage.query` coverage remained Partial for Claude with
`legacy_reconciliation_debt` and for Codex with
`managed_transcript_mismatch`. A no-op scanner pass never upgraded those
persistent debts to Complete in the product projection.

### Fresh exact-ID Claude turns

The simple acceptance turn
`turn-9facf3ef-bb3b-4d67-96aa-52fef7b6400f` completed with one exact component,
message `msg_011CfnFf39xkCP2qMfGKvk4N`, actual model `claude-opus-5-5`, 2 input,
11,698 cache-read, 9,115 cache-write, and 16 output tokens. Its separate terminal
cost fact was 75,588 provider-reported micro-USD.

The tool acceptance turn
`turn-a36b0236-cb11-4be1-a671-01107a0439fe` used Bash exactly once for `pwd`.
The durable ledger recorded success and the isolated project directory. It
retained two exact components:

| Message ID | Model | Input | Cache read | Cache write | Output |
|-|-|-:|-:|-:|-:|
| `msg_011CfnFfEMfF8Mjztzp1o2dx` | `claude-opus-5-5` | 2 | 20,813 | 19,898 | 74 |
| `msg_011CfnFfR3bTaU9q76NijVpt` | `claude-opus-5-5` | 2 | 40,711 | 3,294 | 14 |

Its separate terminal cost fact was 275,204 provider-reported micro-USD. A
sanitized extraction from the copied Claude transcript contained exactly those
three message IDs. Session, message, model, input, output, cache-read,
cache-write, and reasoning fields matched all three managed components exactly.
No new mismatch row appeared. This directly verifies that final
`message_delta.usage` values replace smaller preliminary stream values on the
same message component.

After live ingestion, repeated scan and restart remained at 48 observations,
48 outbox rows, 6 quarantine rows, 59 file cursors, 4 retained mismatches, and 1
retained legacy-debt row. The normalized digest remained
`906f31c0167bff1453e9788403654d1a878b94edacacbd46e6211eeafdbc2f7b`.
An authenticated `usage.query` returned all 48 facts in one page. Its aggregate
of 304,523 input, 667,885 cache-read, 297,219 cache-write, 4,110 output, and 0
reasoning tokens, plus 422,740 provider-reported micro-USD across 3 cost facts,
matched direct SQL exactly.

### Historical seven-day result

The retained historical window is the inclusive 604,800,000 millisecond span
from `2026-09-22T22:22:42.754Z` through the last pre-rerun observation at
`2026-09-29T22:22:42.754Z`. The UTC boundary avoids host timezone ambiguity.
The Oct 7 acceptance turns are intentionally outside this historical window.

`usage.query` returned 43 retained facts totaling 304,517 input, 594,663
cache-read, 264,912 cache-write, 4,006 output, and 0 reasoning tokens. One
legacy cost-bearing fact contributed 71,948 provider-reported micro-USD. The
query aggregate matched direct SQL exactly. Old ambiguous result UUID and
assistant message-ID facts were not merged or repaired. They remain visible in
the raw query and are excluded from any claim of a reconciled external total.

No independent provider statement was retained for Claude, Codex, or
Antigravity. The same-event managed cost is not an independent Claude report,
and an API-equivalent estimate is not provider-reported spend. Claude, Codex,
and Antigravity external reconciliation therefore each report **Unknown**. The
two percent comparison threshold is not applicable. Antigravity's absence from
the normalized window is not translated into zero usage.

### Preservation and cleanup

Final hashes of the retained `node.db`, `node.db-shm`, and empty `node.db-wal`
were respectively
`8392e444508c6c598312e97adf1c761dafc8674dcc704c2f7cdca6aa97de806f`,
`fd4c9fda9cd3f9ae7c962b0ddf37232294d55580e1aa165aa06129b8549389eb`,
and `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`,
identical to the pre-clone values. The two corpus digests also remained
identical, and no process held the retained database open. The original dirty
checkout remained unchanged.

An unsupported `pandamux-server --version` probe briefly launched the exact
candidate against Galahad's default hub path because the server ignores unknown
flags. Immutable inspection proved that `hub.db` was newly created by the probe,
contained zero usage observations, contained two scanner diagnostics, and passed
SQLite integrity checking. No prior database was overwritten. The exact file
was moved recoverably into the isolated acceptance root at
`accidental-default-probe/hub.db`, with SHA-256
`e07863853d6b6a9ff27972c1171d3bfff9a66d4ae81307e12aaadf6db462fa29`.
The default hub path and runtime files were restored to their prior absent state,
and no acceptance process remained running. Version 0.60.17 adds side-effect-free
server help and version actions and rejects unknown, incomplete, conflicting, or
invalid startup arguments so the same probe cannot launch a default Hub again.

### Exact 0.60.3 native accessibility repair artifacts

The follow-up source moved from Linux to COLT-SHINE as a prerequisite-bound Git
bundle with SHA-256
`401089dc7c06f3097e2292f20262582c93a4131dcee12e707c1ef0afb35a18b5`.
The host decoded and verified the bundle, cloned the already accepted isolated
source, fetched only the repair commit, checked out exact commit `2be212f`, and
proved a clean worktree before and after building. The host's existing
`C:\Github\Pandamux` checkout was not edited.

Native MSVC release builds used Visual Studio 2019 Build Tools, Rust 1.98.1,
the Windows SDK shader compiler, four Cargo jobs, and locked dependencies:

```powershell
cargo build --locked --release -p pandamux-desktop
cargo build --locked --release -p pandamux-server -p pandamux-cli
```

| Artifact | Bytes | SHA-256 |
|-|-:|-|
| Windows MSVC `pandamux.exe` | 27,189,760 | `2f9fb97a6bc6a9c9633fe5987ce75931f58556ca6439e5a563344dfd2f51cb9e` |
| Windows MSVC `pandamux-server.exe` | 13,798,400 | `21b1aa84aae621610d5e8b9d7bd43651f9ebcee591b25e20197bddbfa41adef6` |
| Windows MSVC `pandamux-cli.exe` | 1,323,520 | `342834e38bfccd54177b094ff45fe6adb4b9aeee19ba9921b3cf4e18c03a7302` |

The exact `pandamux.exe --headless` run exited 0 and reported all checks passed.
Build scripts, full logs, exit files, source, bundle, probe scripts, results, and
the assembled `run-bin` remain under
`C:\ProgramData\PandaMUX\acceptance\phase4-2be212f`.

## Galahad live provider and Hub evidence

The isolated 0.60.0 node answered authenticated CLI `ping` and `identify`.
Its test project was a separate Git repository under the acceptance root.
Subscription keys were explicitly mapped to named instances, and an isolated
organization and compatible pool allowed Codex and Claude. No production
subscription mapping, policy, or node database was changed.

| Scenario | Result | Deciding record |
|-|-|-|
| Codex direct pool turn | Pass | `turn-5dc7d424-4012-479a-9906-03f85f64dd16` completed with `P4F_LIVE_OK`; attempt `attempt-c7f2ae5c-8010-484a-a00f-d0e4edc5bc76` retained provider kind `codex`, version `0.158.0`, exact selected instance, and managed usage of 14,256 input, 10 output, and 12,288 cache-read tokens |
| Claude selected after test pool preference change | Pass | `turn-aca21df8-cf70-4a00-a7d3-d85bdb843dfd` completed with `P4F_CLAUDE_PREFERRED_OK`; attempt `attempt-a3028860-f259-45e8-b73b-d20b2093593c` retained provider kind `claude` and model `claude-3-7-sonnet-latest`. The route gave Claude score 1,750 and rejected the saturated Codex instance for `concurrency_limit_reached` |
| Antigravity exact instance | Pass | `turn-f4798eea-8b15-4abe-930e-acc737a91156` completed with `P4F_ANTIGRAVITY_OK`; attempt `attempt-6b9eaac2-c69b-4c1d-b951-d4f967566fe8` retained provider kind `antigravity` and pinned ACP version `agy_acp_server_1.1.1` |
| Best-effort providers | Disabled by design | Cursor, Grok, and OpenCode health reports retained their configured disabled reasons; no live turn was claimed |

The offline Antigravity status probe reported `authenticated=false` even though
the isolated profile held a private token and a real ACP turn succeeded. The
probe only reports detected credentials, not a verified live login. The real
turn, not that status field, decides the live row.

Read-only inspection of the retained node database found four Codex and one
Claude `managed_turn` observations. The accepted direct Codex turn recorded
14,256 input, 10 output, and 12,288 cache-read tokens in both the turn and
its exact managed observation. The accepted Claude turn recorded 2 input and
22 output tokens in both places. The Hub-routed Codex turn also matched at
15,201 input, 12 output, and 12,288 cache-read tokens. The accepted
Antigravity turn had no provider-reported token usage and no managed usage
observation; its usage is Unknown, not zero. The node also retained 13 Codex
and five Claude transcript observations and three Codex `record_too_large`
quarantines. These quarantines remain visible source-coverage debt, and the
database comparison is not an independent weekly provider report.

A temporary Rust harness used `Server::connect_remote_environment` and the
production `pandamux-server proxy` command over the real SSH connection:

```bash
cargo run --locked -p pandamux-server --example phase4_live_acceptance
```

The harness created an in-memory Hub with a project policy allowing only
`codex-default`, then used the isolated Galahad node. Its final run returned:

```text
REMOTE_IDENTIFY_OK role="node"
HUB_POLICY_DENY_OK
HUB_REMOTE_CREATE_OK thread="thread-3ea2bc31-e4d2-4b2f-af8c-2f2787724b1f"
HUB_REMOTE_GET_OK
HUB_USAGE_SYNC_OK items=4
HUB_REMOTE_SEND_OK turn="turn-f084a577-7777-423d-90af-9769663b9c74"
HUB_REMOTE_TURN_OK
HUB_REMOTE_DISCONNECT_OK
```

The first extended run reached the same identity, policy, thread, and four-item
usage checks but rejected the new Codex turn because three earlier acceptance
sessions occupied the configured three-session limit. Raising only the isolated
test instance limit to four allowed the final run. The harness source was
removed after testing; the accepted output and node database remain as the
record. This proves a bounded live Hub pull and a Hub-routed turn, not
long-duration reconnect, retention-gap backfill, or weekly reconciliation.

## Native Usage accessibility probe

An interactive `C4adUser` scheduled task launched the exact Windows desktop
from its isolated profile and queried Windows UI Automation. The first query
returned only the top-level window because GPUI activates its AccessKit tree
on demand. After allowing the requested refresh frame, a second query found
12 named, keyboard-focusable navigation controls, including `Usage and
capacity`. Invoking that control through UI Automation exposed 26 descendant
elements, including 24 keyboard-focusable controls. They included the Usage
retry action, eight named filter controls, and six named subscription-edit
controls. A subsequent native 20-step Tab sequence reached the retry action,
all eight filters, and all six subscription-edit buttons in a stable order.
Opening the Claude subscription editor exposed named `Subscription name` and
`Masked account label` Edit controls plus Save and Cancel buttons. A test
character changed the unsaved name field from `Claude Code` to `Claude CodeX`,
proving that keyboard input reached the field. No save was invoked.

The 0.60.0 keyboard Cancel path passed: Enter on the focused Cancel button
closed the editor and restored accessibility focus to `Edit subscription for
Claude Code`. Its screen-reader path remained **unverified** because UI
Automation reported the top-level `PandaMUX` window as focused, and its Invoke
action on Cancel only focused the button. Probe results `ui-probe.json` through
`ui-probe-9.json` remain under the guest acceptance LocalAppData directory.

The exact 0.60.3 follow-up ran in the active limited
`COLT-SHINE\B_StL` console session with a new isolated LocalAppData profile per
run. The target behavior was deliberately narrow: UIA focused the existing Edit
button and keyboard Enter opened the editor, then the application had to publish
the name field as focused without a corrective UIA focus call. A single UIA
Invoke on Cancel then had to close the editor and restore focus to the original
Edit button.

The first strict 0.60.3 run was a transient miss: keyboard input still changed
`Claude Code` to `Claude CodeX`, direct UIA Cancel closed the editor, and no
process leaked, but UIA reported the top-level window instead of the input. A
fresh diagnostic run then passed before its later explicit focus diagnostic,
and two additional fresh-profile strict repeats passed without any corrective
focus call. In all three passing runs:

- the focused element was named `Subscription name`, had type
  `ControlType.Edit`, AutomationId `usage.subscription.name`, and
  `HasKeyboardFocus=true`;
- the same node was keyboard-focusable, a control and content element, and
  exposed the UIA Value pattern;
- keyboard input changed the unsaved value from `Claude Code` to
  `Claude CodeX`;
- one UIA Invoke on Cancel closed the editor; and
- focus returned to `Edit subscription for Claude Code` with
  `HasKeyboardFocus=true`.

The diagnostic task and both strict repeat tasks returned 0, were unregistered,
and left no exact acceptance process running. This closes the functional editor
focus and direct Cancel rows with repeat evidence. The initial transient miss
remains recorded and should be watched during the remaining screen-reader and
full-matrix runs rather than being treated as nonexistent.

This is native control exposure, keyboard focus, and invocation evidence. It
does not establish announcement of usage values or table text to a screen
reader, 200 percent scaling, reduced motion, forced colors, or visual non-color
state. Those rows remain open.

## Static verification and remaining gates

The source commit passed `cargo fmt --all --check`, strict workspace Clippy,
`cargo test --workspace --quiet` with 412 passed, 0 failed, and two intentionally
ignored real-shell terminal tests, Windows GNU all-target type checking,
Rust crate-boundary and Claude wiring guards, built server and CLI smoke,
and desktop headless smoke. These are separate from the live artifact results.
The 0.60.1 documentation and dependency-policy follow-up also passed workspace
formatting, strict Clippy, and the full 412-test workspace suite with the same
two intentional ignores.

The 0.60.2 and 0.60.3 accessibility repairs each passed local formatting,
strict workspace Clippy, the full workspace suite with 412 passed, 0 failed,
and the same two intentional real-shell ignores, the Rust boundary guard,
Claude wiring guard, dependency policy, and desktop headless startup. The exact
0.60.3 commit additionally passed the native MSVC builds and UIA follow-up above.

`cargo-deny` 0.20.2 could not parse the old `deny.toml` syntax. The follow-up
configuration change removed deprecated keys and scoped seven existing
non-allowlisted transitive licenses to their exact crates and versions. With
that change, the exact command
`cargo deny check advisories licenses bans sources --hide-inclusion-graph`
returned `advisories ok, bans ok, licenses ok, sources ok`. The current exact
0.60.15 run emitted non-failing yanked-version warnings for transitive
`chacha20 0.10.1` and `yoke-derive 0.8.3`. The
global license allowlist remains narrow and does not admit GPL, AGPL, or LGPL.
A deliberate configuration tradeoff is `unmaintained = "workspace"`: the current
checker errors for unmaintained direct workspace dependencies but does not
report transitive unmaintained crates, replacing the removed general `"warn"`
setting. A release still needs a separate shipped-license and notice review,
especially for the scoped MPL-2.0 exceptions.

Phase 4 cannot yet be marked exited under `tasks/plan-rewrite.md`:

1. Complete the native Usage keyboard matrix beyond the verified 20-step Tab,
   editor focus, and direct UIA Cancel paths. Archive 200 percent scaling,
   reduced-motion, forced-colors, non-color-status, and screen-reader results,
   while watching for recurrence of the recorded transient focus miss. Rerun
   affected rows on the final exact candidate.
2. Run the live local plus Galahad restart, rotation, reconnect, bounded
   backfill, capacity, safe-retry, and post-side-effect interruption matrix.
   The exact 0.60.15 accounting turns and restart scans do not prove the full
   durability or failure-injection matrix.
3. Re-run the normal-user Windows named-pipe security matrix on the current
   candidate if it is to inherit a current-version production IPC claim.
4. Synchronize the maintained project documentation after P4-C5 stabilizes.
5. Complete the explicitly deferred R0.5 30-minute physical laptop disconnect
   and sleep checks before production acceptance or release.

The acceptance VM is still a shared lab. Its existing network attachment and
unrelated Hyper-V checkpoints were left untouched during the 0.60.0 matrix. No
installer, updater, signed artifact, deployment, or production activation was
tested.

During the 0.60.3 follow-up, the guest adapter was temporarily attached to the
existing Default Switch only to assess guest availability. Final cleanup on
COLT-SHINE verified the adapter back in its original disconnected state with no
switch, no exact acceptance process, no acceptance scheduled task, and a clean
exact source clone. No checkpoint was created, applied, or changed.
