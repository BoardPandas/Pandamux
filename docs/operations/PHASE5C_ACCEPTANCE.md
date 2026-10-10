# Phase 5C Execution Runtime Acceptance Record

Date: 2026-10-10

Status: **Implemented, locally verified, and accepted for the local/native
scope; not accepted for production scope.** Exact version 0.60.119 commit
`ce9c48f0a95c1847abe72c1b3f49398ab362425f` is the deciding local/native
acceptance source. All eight jobs passed in exact
[native run 38054202454](https://github.com/BoardPandas/Pandamux/actions/runs/38054202454),
including the named Windows Job Object descendant and macOS process-group
descendant tests. Together with the retained portable, native Windows
path/reparse/worktree, and Unix process-group/worktree rows, that closes the
binding automated local/native matrix. Version 0.60.120 is a documentation-only
evidence reconciliation. A physical remote Linux node, live
Codex/Claude/Antigravity, and the controlled external-effect row remain unrun.

This record is the C-K documentation checkpoint. It does not close P4-C5 or
P4-C7 and does not claim deployment, activation, packaging, signing,
publication, release, health, or served-version evidence. The original
implementation authorization prohibited push and CI; later user authorization
permitted pushing this branch and manually dispatching and monitoring the
native workflow only.

## Deciding Source and Scope

- Worktree: `/home/brandon/.codex/worktrees/recovery-r0/Pandamux`
- Branch: `codex/recovery-r0`
- Binding contract:
  [`tasks/2026-10-09-phase-5c-execution-owner.md`](../../tasks/2026-10-09-phase-5c-execution-owner.md)
- Deciding local/native acceptance source: version 0.60.119, commit
  `ce9c48f0a95c1847abe72c1b3f49398ab362425f`
- Schema: migration 14, with a verified pre-v14 schema-13 backup before a
  persistent store migrates
- Runtime boundary: private server-owned execution APIs only. Public run RPC,
  CLI, desktop orchestration, and a user-facing recovery-review resolver remain
  Phase 5D work.

`pandamux-orchestrator` remains a pure crate. `RunExecutionService` interprets
durable run effects, while `ThreadManager` remains the sole provider-session
owner and every manual or orchestrated provider start uses shared durable
admission. The Phase 4 side-effect ledger and `AttemptBoundary` remain the
fallback and retry authority.

## Checkpoint Chain

| Checkpoint | Version | Exact commit | Outcome |
|---|---:|---|---|
| C-A pure runtime contracts | 0.60.91 | `b5ea2155da046f4dc6a3410fea2cffd32ef6b90f` | Committed and focused verification passed |
| C-B migration 14 and store | 0.60.92 | `3ac78afe7fe56130befd55885fc7a4291359b9d5` | Committed and focused verification passed |
| C-C engine adapter and outbox | 0.60.93 | `9f8cccab9cbdd41e9525b256ccaf8733285a10e3` | Committed and focused verification passed |
| C-D worktrees and checkpoints | 0.60.94 | `f56e0db82c2908efe8d816eb246c59f32b88730b` | Committed and focused verification passed |
| C-E admission and leases | 0.60.95 | `ffcbf9e5f4cde27c5c0abcb8c4e995a5e1789b4e` | Committed and focused verification passed |
| C-F budgets and reconciliation | 0.60.96 | `7d2cde64c830f31550ebb774cf0c1de725f657fe` | Committed and focused verification passed |
| C-G local provider effects | 0.60.97 | `c4157d24215beedd45e32fbc62d746652241eecb` | Committed and focused verification passed |
| C-H authenticated node execution | 0.60.98 | `1ef172a1935fb228e35d778e5c83d4bb7075fec8` | Committed and focused verification passed |
| C-I blackboard and evidence | 0.60.99 | `afa878b51181a77b7c10b8418f212aa47bc14db8` | Committed and focused verification passed |
| C-J lifecycle and recovery | 0.60.100 | `79ce5a0db9a8de01a9044738cd4eeacc65c40726` | Committed; initial complete portable matrix and exact Linux native rows passed; a later matrix replay exposed the shutdown race below |
| C-J verification repair | 0.60.101 | `0b59964d46b44be67dc662d1d9483a9be00a5101` | Committed; provider progress now survives a concurrent cancellation clock advance without weakening strict store callers |
| C-K documentation and acceptance | 0.60.102 | `6fb9a8ff248b81a59d6ae2f7ee105ac8e8dbc5e4` | Committed; its pre-commit matrix passed, while exact post-commit replay exposed the concurrent cancellation-report join defect below |
| C-K verification repair | 0.60.103 | `d78a22c0cffae2454373b8527b11c821da20d627` | Committed; every in-process waiter now receives the winning owner's exact cancellation report; deterministic concurrency, pressured stress, the complete matrix, and exact Linux rows pass |
| C-K initial acceptance reconciliation | 0.60.104 | `d5b238188938ed1d20232bed967a4518fa7cee7f` | Committed; documentation and version metadata only; its exact native run exposed target-portability defects |
| Native fixture repair | 0.60.105 | `3ff587c2a527be47e35d859f9e5198d9655186d0` | Restored target-specific Phase 5C verification after the first native run |
| Target-portability repair | 0.60.106 | `5302748bd0f7ab8aee2dab887dd27dd7a2a324bb` | Closed additional macOS and Windows fixture failures |
| Target-portability repair | 0.60.107 | `d5746bb07b4a47d3b199c42f6422230401ca0c1b` | Closed remaining target portability gaps found by native CI |
| Host-line-ending repair | 0.60.108 | `bc9dc6dd07d42ad47dc8947d31ab8c269a677573` | Made the CRLF fixture host-independent |
| Native-fixture portability repair | 0.60.109 | `d2bbb60984f6ab3fb4136bb829a702fc431f93cb` | Reconciled platform-specific fixture behavior without weakening runtime checks |
| Native runtime repair | 0.60.110 | `e21fd57ed5b979b9a408efd6dc8ac6a2ca9b39cb` | Fixed the local-acceptance monotonic-clock race and initial Windows Git path normalization |
| Windows assertion repair | 0.60.111 | `5201bda0f006bb9e9645d3e1b230a445a1674ded` | Removed Windows-only assertion type ambiguity |
| Windows Git-path repair | 0.60.112 | `b2e6be8dab5afe790e9385647e1789d4a9917757` | Normalized attempt-worktree Git subprocess paths and remote fixture arguments |
| Private-leaf repair | 0.60.113 | `361a1d862020fdb1fb50613c24edf3db4c4225c4` | Precreated private attempt-worktree leaves before Git materialization |
| Windows diagnostic checkpoint | 0.60.114 | `f75ca28c5b4af3d4a7f8a3d85ff3a2f2909b7383` | Added targeted diagnostics for the remaining Windows path rejection |
| Windows lint repair | 0.60.115 | `9493bfcbbf639e46833f521cf984c1a94321082c` | Kept the native diagnostic path strict-Clippy clean |
| Windows prefix repair | 0.60.116 | `086e9bcec1140c75925f99e9a0a7f9f3fe2bc502` | Stopped querying metadata for the non-queryable absolute Windows prefix while retaining rooted-path validation |
| Artifact-smoke repair | 0.60.117 | `e53406bbd211e701f89a25d63a8cd672033a22d9` | Replaced the obsolete server `--headless` smoke invocation; all eight exact native-CI jobs pass |
| Native-CI evidence reconciliation | 0.60.118 | `e00a507c247cdbf6e89db289e32c4d69a0a3c259` | Documentation and version metadata only; records exact target-CI success and retained native/live gaps |
| Native descendant acceptance | 0.60.119 | `ce9c48f0a95c1847abe72c1b3f49398ab362425f` | Adds exact Windows Job Object and native Linux/macOS process-group descendant tests; all three pass on their native platforms |
| Local/native acceptance reconciliation | 0.60.120 | This documentation commit | Documentation and version metadata only; records the now-complete automated local/native matrix and retains production-readiness gaps |

Every checkpoint and each verification or native-CI repair was staged by
explicit path, inspected, and committed before the final reconciliation. No
commit includes a protected bundle or the unrelated security-scan report.

## Acceptance Criteria and Retained Evidence

| Criterion | Result | Retained evidence |
|---|---|---|
| AC-C01 replay equivalence | Pass | `state_replay` prefix/suffix, duplicate, sequence, and idempotency tests plus verified SQLite replay tests |
| AC-C02 crash boundaries | Pass, portable | CRASH-01 through CRASH-12 matrix below |
| AC-C03 isolated attempt ownership | Pass | `attempt_worktrees` WT-01 through WT-06, including native Windows and macOS execution |
| AC-C04 user checkout unchanged | Pass | WT-07 and WT-08 byte, status, index, HEAD, and ref comparison |
| AC-C05 retain unsafe worktrees | Pass | WT-09 through WT-14 dirty, locked, foreign, link, collision, and interrupted-creation cases, including native Windows and macOS execution |
| AC-C06 all concurrency caps | Pass | `execution_admission` QUE-01 through QUE-08, including independent connections |
| AC-C07 reserve interactive capacity | Pass | QUE-09 through QUE-14 ordering, cap-one, blocked-head, starvation, and shared manual races |
| AC-C08 budget reservation and conservation | Pass | execution-budget/store BUD-01 through BUD-10 |
| AC-C09 unknown and excess usage | Pass | BUD-11 through BUD-15 retained reservation, overage pause, and mismatch diagnostics |
| AC-C10 artifact bounds and authority | Pass | `blackboard_dag` ART-01 through ART-12, SEC-01, and link-substitution test |
| AC-C11 evidence-gated reviewer DAG | Pass | DAG-01 through DAG-06 three-task and failure-propagation tests |
| AC-C12 bounded fallback | Pass | `run_execution_service_tests` EFX-01 through EFX-08 and typed cleanup-verification cases |
| AC-C13 joined cancellation | Pass, portable and native | LIFE-01 through LIFE-09 plus exact Linux/macOS process-group and Windows Job Object descendant cleanup on version 0.60.119 |
| AC-C14 ownership and clock recovery | Pass, portable | REC-01 through REC-08 and CLK-01 through CLK-03 fixtures |
| AC-C15 deduplicated human review | Pass, internal | REC-09 through REC-13 ambiguity, reattach, response-loss, deduplication, and audit tests; no public resolver exists |
| AC-C16 authenticated remote execution | Pass, loopback only | REM-01 through REM-07 environment binding, replay, conflict, disconnect, status, and cancellation tests |
| AC-C17 migration 14 safety | Pass | schema-13 preservation, independent verified backup, failure rollback, retry, constraints, and malformed-row rejection tests |
| AC-C18 architecture and toolchains | Pass, local and target CI | pure-boundary fixtures, exact pins, Rust 1.89 Windows and headless graphs, locked all-target checks, dependency policy, and all eight exact native-CI jobs |

The named evidence resides in:

- [`crates/pandamux-orchestrator/tests/state_replay.rs`](../../crates/pandamux-orchestrator/tests/state_replay.rs)
- [`crates/pandamux-server/src/store/orchestration_phase5c.rs`](../../crates/pandamux-server/src/store/orchestration_phase5c.rs)
- [`crates/pandamux-server/tests/attempt_worktrees.rs`](../../crates/pandamux-server/tests/attempt_worktrees.rs)
- [`crates/pandamux-server/tests/execution_admission.rs`](../../crates/pandamux-server/tests/execution_admission.rs)
- [`crates/pandamux-server/src/run_execution_service_tests.rs`](../../crates/pandamux-server/src/run_execution_service_tests.rs)
- [`crates/pandamux-server/src/remote_execution_tests.rs`](../../crates/pandamux-server/src/remote_execution_tests.rs)
- [`crates/pandamux-server/tests/blackboard_dag.rs`](../../crates/pandamux-server/tests/blackboard_dag.rs)
- [`crates/pandamux-server/tests/run_shutdown.rs`](../../crates/pandamux-server/tests/run_shutdown.rs)
- [`crates/pandamux-server/tests/run_recovery.rs`](../../crates/pandamux-server/tests/run_recovery.rs)

## Crash and Failpoint Matrix

| Boundary | Result | Retained test evidence |
|---|---|---|
| CRASH-01 event/snapshot/effect commit | Pass | `run_transition_crash_replays_exactly_rejects_changed_bytes_and_broadcasts_after_commit` |
| CRASH-02 effect claim and lease | Pass | `effect_failpoints_resume_database_only_and_never_redispatch` |
| CRASH-03 capacity plus budget claim | Pass | `atomic_capacity_budget_dispatch_rolls_back_and_replays_exactly` |
| CRASH-04 worktree intent before Git | Pass | `crash_04_and_05_recover_from_durable_intent_without_duplicate_ownership` |
| CRASH-05 Git completion before receipt | Pass | the same worktree test plus `checkpoint_intent_and_git_crashes_replay_once_and_changed_bytes_conflict` |
| CRASH-06 dispatch marker before provider call | Pass | EFX-01 and the store before/after dispatch-marker failpoints |
| CRASH-07 provider acceptance before outcome | Pass | EFX-03, response-loss suppression, and exact remote reattachment tests |
| CRASH-08 outcome before acknowledgement | Pass | store before/after outcome and acknowledgement failpoints |
| CRASH-09 artifact plus task settlement | Pass | `art_11_art_12_atomic_completion_replays_after_crash_and_rolls_back_on_failure` |
| CRASH-10 cancel intent before interrupt | Pass | `life_04_05_and_crash_10_persist_intent_before_cooperative_drain` |
| CRASH-11 node prepare before Hub receipt | Pass | deterministic attempt-prepare and prepare-receipt repair tests |
| CRASH-12 Hub/node dispatch markers | Pass | two-store crash reconciliation, marker replay/status, and post-provider reattachment tests |

All failpoint fixtures use test-owned temporary repositories and databases.
They assert durable state or explicit review, not a timing sleep, and do not
contact an external provider or repository.

## Local Verification

The complete Section 11 matrix initially passed on exact C-J commit
`79ce5a0db9a8de01a9044738cd4eeacc65c40726`. A later C-K matrix replay exposed
an intermittent shutdown race: cancellation could advance the durable attempt
clock after the provider writer loaded it, causing valid approval progress and
the following terminal usage receipt to be dropped. Exact repair commit
`0b59964d46b44be67dc662d1d9483a9be00a5101` adds an owner-internal atomic clamp
while retaining strict regression rejection for ordinary store callers. Its
deterministic regression and 30 shutdown stress iterations passed.

The exact 0.60.102 documentation commit replay then exposed a second concurrent
join defect: a cancellation waiter that did not own cleanup reconstructed a
generic report from durable state and could lose the winning owner's lifecycle
flags and precise quarantine reason. Exact 0.60.103 repair commit
`d78a22c0cffae2454373b8527b11c821da20d627` publishes the owner's report to all
in-process waiters while retaining bounded durable-state observation as the
crash/restart fallback. Its deterministic two-caller regression passes, as do
20 pressured 237-test server-unit-suite repetitions (4,740 tests), the complete
Section 11 matrix, and the explicit exact Linux native rows.

Native CI then exposed only target-specific fixture, path, lint, and artifact
smoke defects. Each repair was isolated, locally verified, versioned, committed,
pushed, and rerun without weakening the persistence, ownership, admission,
budget, recovery, cleanup, or acceptance contract. Exact 0.60.117 commit
`e53406bbd211e701f89a25d63a8cd672033a22d9` passes focused local parser,
architecture, MSRV, dependency-policy, formatting, and diff gates and the full
workspace matrix on Windows and macOS in native CI. The documentation-only
0.60.118 reconciliation passed the complete local Section 11 matrix before
commit and all eight target jobs after commit.

Exact 0.60.119 commit
`ce9c48f0a95c1847abe72c1b3f49398ab362425f` adds only retained native
descendant-cleanup tests. Its exact Linux process-group case passes locally;
the complete ten-case `run_shutdown` integration, provider suites, locked
all-target check, strict Clippy, boundary fixtures and runner, formatting, and
diff hygiene also pass. Native run
[38054202454](https://github.com/BoardPandas/Pandamux/actions/runs/38054202454)
then passes all eight jobs and names both
`lifecycle_supervised_windows_job_object_kills_its_descendant` and
`lifecycle_supervised_unix_process_group_kills_its_descendant` as passing on
Windows and macOS respectively. This is the exact committed evidence that
closes the automated local/native matrix; it does not exercise the separate
production-readiness rows.

The documentation-only 0.60.120 working tree then passed the complete Section
11 matrix before commit: every focused crate suite, the five-target boundary
runner and its negative fixtures, the full locked workspace with two
intentional real-shell ignores, all-target check, warning-denied Clippy,
dependency policy, Rust 1.89 non-desktop product graph, Claude wiring,
formatting, and diff hygiene. Exact post-commit verification and native CI are
reported separately rather than inferred from this pre-commit run.

The first full-workspace replay stopped during compilation when the disposable
`/dev/shm` target reached its storage quota. After removing only task-owned
build caches and disabling incremental/debug-info storage, one replay observed
a transient `git_command_start_failed` from the managed-Git timeout fixture.
That exact fixture then passed 20 of 20 stress replays, and the unchanged full
locked-workspace command passed with two intentional real-shell tests ignored.
No repository or protected evidence file was involved in either environment
failure.

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | Pass, exit 0 |
| `cargo test --locked -p pandamux-core` | Pass, exit 0 |
| `cargo test --locked -p pandamux-orchestrator` | Pass, exit 0 |
| `cargo test --locked -p pandamux-protocol` | Pass, exit 0 |
| `cargo test --locked -p pandamux-providers` | Pass, exit 0 |
| `cargo test --locked -p pandamux-server` | Pass, exit 0 |
| `cargo test --locked -p pandamux-remote` | Pass, exit 0 |
| `cargo test --locked -p pandamux-client` | Pass, exit 0 |
| `cargo test --locked -p pandamux-desktop` | Pass, exit 0 |
| `cargo test --locked -p pandamux-boundary-check` | Pass, exit 0 |
| `cargo run --locked -p pandamux-boundary-check --quiet` | Pass for all five declared target graphs, exit 0 |
| `cargo test --locked --workspace` | Pass, exit 0; two intentional real-shell tests ignored |
| `cargo check --locked --workspace --all-targets` | Pass, exit 0 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | Pass, exit 0 |
| `cargo deny check advisories licenses bans sources --hide-inclusion-graph` | Pass, exit 0; only configured unmatched-license, duplicate, and yanked-transitive warnings |
| `cargo +1.89.0 check --locked --workspace --all-targets --exclude pandamux-desktop` | Pass, exit 0 |
| `node scripts/check-claude-wiring.mjs` | Pass, exit 0 |
| `git diff --check` | Pass, exit 0 |

This establishes portable local verification, including controlled real local
children and an authenticated loopback node. The native workflow establishes
target-CI evidence separately; neither result is physical-remote,
live-provider, deployment, or release evidence.

## Native CI Repair History

| Run | Exact source | Jobs | Deciding outcome |
|---|---|---:|---|
| [38024474000](https://github.com/BoardPandas/Pandamux/actions/runs/38024474000) | 0.60.104 `d5b2381` | 4 pass / 3 fail | macOS tests, Windows MSRV, and Windows Clippy failed |
| [38025563970](https://github.com/BoardPandas/Pandamux/actions/runs/38025563970) | 0.60.105 `3ff587c` | 4 pass / 3 fail | macOS tests, Windows Clippy, and Windows MSRV failed |
| [38027296242](https://github.com/BoardPandas/Pandamux/actions/runs/38027296242) | 0.60.106 `5302748` | 4 pass / 3 fail | Windows dependency policy, Windows MSRV, and macOS tests failed |
| [38028306900](https://github.com/BoardPandas/Pandamux/actions/runs/38028306900) | 0.60.107 `d5746bb` | 7 pass / 1 fail | Windows workspace tests failed |
| [38029186760](https://github.com/BoardPandas/Pandamux/actions/runs/38029186760) | 0.60.108 `bc9dc6d` | 6 pass / 2 fail | macOS and Windows workspace tests failed |
| [38030670980](https://github.com/BoardPandas/Pandamux/actions/runs/38030670980) | 0.60.109 `d2bbb60` | 6 pass / 2 fail | macOS and Windows workspace tests failed |
| [38032504473](https://github.com/BoardPandas/Pandamux/actions/runs/38032504473) | 0.60.110 `e21fd57` | 6 pass / 2 fail | Windows Clippy and Windows MSRV failed |
| [38033217466](https://github.com/BoardPandas/Pandamux/actions/runs/38033217466) | 0.60.111 `5201bda` | 7 pass / 1 fail | Windows workspace tests failed |
| [38034603821](https://github.com/BoardPandas/Pandamux/actions/runs/38034603821) | 0.60.112 `b2e6be8` | 7 pass / 1 fail | Windows workspace tests failed |
| [38036093752](https://github.com/BoardPandas/Pandamux/actions/runs/38036093752) | 0.60.113 `361a1d8` | 7 pass / 1 fail | Windows workspace tests failed |
| [38037425683](https://github.com/BoardPandas/Pandamux/actions/runs/38037425683) | 0.60.114 `f75ca28` | 7 pass / 1 fail | Windows Clippy failed |
| [38038014667](https://github.com/BoardPandas/Pandamux/actions/runs/38038014667) | 0.60.115 `9493bfc` | 7 pass / 1 fail | Windows workspace tests failed |
| [38038855449](https://github.com/BoardPandas/Pandamux/actions/runs/38038855449) | 0.60.116 `086e9bc` | 7 pass / 1 fail | Windows artifact evaluator used the obsolete `--headless` argument |
| [38040781118](https://github.com/BoardPandas/Pandamux/actions/runs/38040781118) | 0.60.117 `e53406b` | 8 pass / 0 fail | All exact native-CI jobs passed |
| [38044556752](https://github.com/BoardPandas/Pandamux/actions/runs/38044556752) | 0.60.118 `e00a507` | 8 pass / 0 fail | Documentation reconciliation remained green on its exact committed head |
| [38054202454](https://github.com/BoardPandas/Pandamux/actions/runs/38054202454) | 0.60.119 `ce9c48f` | 8 pass / 0 fail | Exact Windows Job Object and macOS process-group descendant tests passed; automated local/native acceptance closes |

## Native and Live Acceptance

| Row | Status | Exact evidence or gap |
|---|---|---|
| Linux process-group descendant cleanup | Pass | On exact 0.60.119 commit, `cargo test --locked -p pandamux-server --test run_shutdown lifecycle_supervised_unix_process_group_kills_its_descendant -- --exact`: 1 passed |
| Linux real concurrent Git worktrees and ownership checks | Pass | On exact 0.60.103 repair commit, `cargo test --locked -p pandamux-server --test attempt_worktrees`: 11 passed |
| Windows native automation | Pass | Exact 0.60.119 native CI passes workspace tests, release build, artifact smoke, rooted-path validation, junction substitution rejection, locked-worktree retention, replayable cleanup, and `lifecycle_supervised_windows_job_object_kills_its_descendant` |
| macOS native automation | Pass | Exact 0.60.119 native CI passes the full `attempt_worktrees` integration suite, link-substitution rejection, locked-worktree retention, release build, and `lifecycle_supervised_unix_process_group_kills_its_descendant` |
| Authenticated loopback node | Pass, portable | Private environment-bound RPC, disconnect, status, cancel, and replay tests passed inside `pandamux-server` |
| Physical remote Linux node | Unrun | No exact physical-node baseline, lease, proxy reconnect, cancellation, or provider-turn evidence |
| Live Codex | Unrun | No live provider was contacted |
| Live Claude | Unrun | No live provider was contacted |
| Live Antigravity | Unrun | No live provider was contacted |
| Controlled external HTTP/file/command effect | Unrun | No disposable external service or repository was mutated |
| Target-specific CI | Pass | All eight jobs passed for exact 0.60.119 commit `ce9c48f0a95c1847abe72c1b3f49398ab362425f` in [run 38054202454](https://github.com/BoardPandas/Pandamux/actions/runs/38054202454) |

The implemented/local, target-specific CI, and complete automated local/native
classifications pass. The physical-remote, live-provider, and controlled
external-effect rows remain explicitly unrun and prevent any production-ready
or production-accepted claim.

## Retained Debt and Preservation

No production run database, app-data execution root, external repository, or
live provider was used, so this work created no real retained worktree or
recovery-review debt. Tests intentionally exercise dirty, locked, foreign,
ambiguous, and cleanup-pending records in test-owned temporary directories and
prove they are retained while classified; the fixtures are removed only by
their temporary-directory owners after assertions complete.

The original checkout was not modified. Eleven root `.pandamux-*.bundle`
files and `tasks/security-scan-2026-10-09.md` remain untracked, byte-identical,
and outside every checkpoint commit. No reset, clean, forced worktree removal,
or broad staging command was used.

## Status Separation

| Dimension | Status |
|---|---|
| Implementation | Complete for Phase 5C's private runtime, with final native acceptance coverage at exact 0.60.119 commit |
| Portable local verification | Pass |
| Native automation | Pass for the binding Windows, Linux, and macOS rows on exact committed evidence |
| Complete local/native acceptance | Pass |
| Physical remote/live acceptance | Not established |
| Checkpoint commits | C-A through C-K, the native-CI repairs, and exact 0.60.119 native acceptance coverage are committed; the 0.60.120 reconciliation is this documentation commit |
| Push | Checkpoint chain through exact 0.60.119 pushed; this documentation checkpoint is reported separately after commit |
| CI | Exact 0.60.119 native run 38054202454 passed all eight jobs; this documentation checkpoint is reported separately |
| Deployment | Not performed |
| Activation | Not performed |
| Packaging/signing/publication | Not performed |
| Release/served version | Not performed or established |
| Phase 4 P4-C5 | Remains Partial |
| Phase 4 P4-C7 | Remains open |

## Decisions, Deviations, and Limitations

- No Phase 5C persistence, ownership, budget, recovery, cleanup, or acceptance
  requirement was deliberately weakened.
- C-K paused when its complete-matrix replay exposed the load-dependent C-J
  shutdown race. The 0.60.101 repair is a separate commit: only the
  `ThreadManager` owner-internal provider-progress path clamps to the newer
  durable clock, while the public store path continues to reject regressions.
- Exact 0.60.102 post-commit replay exposed a concurrent cancellation-report
  join defect, so final C-K reconciliation paused again. The 0.60.103 repair is
  a separate commit: the cleanup owner publishes one exact report to every
  in-process waiter, while durable state remains the bounded recovery fallback.
- `fs4` is a direct server dependency using the already workspace-pinned
  version; Phase 5C introduced no new third-party version.
- Native CI repairs were limited to fixture portability, platform path
  validation, lint cleanliness, the monotonic local-acceptance clock, private
  worktree materialization, and the supported server smoke invocation; none
  relaxed a runtime or acceptance requirement.
- Windows and macOS automation moved from Partial to Pass only after exact
  retained tests named and proved their platform-specific descendant cleanup.
  Physical-remote, live-provider, and controlled-external evidence remains
  Unrun.
- Later user authorization permitted branch push and manual CI dispatch and
  monitoring only. Deployment, activation, packaging, signing, publication,
  release, provider calls, and external-repository mutation remained
  prohibited.
- Local 0.60.118 and 0.60.119 verification used task-owned `/dev/shm` targets
  because the repository filesystem had insufficient free space. Incremental
  compilation and debug-info storage were disabled after a disposable-cache
  quota failure; the locked dependency graph, test commands, warning policy,
  and assertions were unchanged.
- The internal recovery-review resolution method is tested and audited, but a
  human-facing resolver is intentionally unavailable until Phase 5D.
- Migration 14 has no destructive down migration. Rollback requires stopping
  the server, verifying and restoring the complete pre-v14 schema-13 backup,
  preserving the v14 database, and retaining all ambiguous worktrees,
  artifacts, ledgers, and reviews.
