# Phase 5C Execution Runtime Acceptance Record

Date: 2026-10-09

Status: **Implemented and locally verified; not accepted for the complete
local/native or production scope.** Exact version 0.60.103 commit
`d78a22cb69711d411ee857add7aed09d654452b0` is the deciding implementation
source and passes the complete portable matrix plus the exact Linux
process-group and real Git worktree rows. Version 0.60.104 is a
documentation-only reconciliation. Windows and macOS native rows, a physical
remote Linux node, live Codex/Claude/Antigravity, and the controlled
external-effect row remain unrun.

This record is the C-K documentation checkpoint. It does not close P4-C5 or
P4-C7 and does not claim push, CI, deployment, activation, packaging, signing,
publication, release, health, or served-version evidence.

## Deciding Source and Scope

- Worktree: `/home/brandon/.codex/worktrees/recovery-r0/Pandamux`
- Branch: `codex/recovery-r0`
- Binding contract:
  [`tasks/2026-10-09-phase-5c-execution-owner.md`](../../tasks/2026-10-09-phase-5c-execution-owner.md)
- Deciding implementation: version 0.60.103, commit
  `d78a22cb69711d411ee857add7aed09d654452b0`
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
| C-K verification repair | 0.60.103 | `d78a22cb69711d411ee857add7aed09d654452b0` | Committed; every in-process waiter now receives the winning owner's exact cancellation report; deterministic concurrency, pressured stress, the complete matrix, and exact Linux rows pass |
| C-K final acceptance reconciliation | 0.60.104 | This documentation commit | Documentation and version metadata only; records the exact 0.60.103 deciding source and retained acceptance gaps |

Every checkpoint and both verification repairs were staged by explicit path,
inspected, and committed before the final reconciliation. No commit includes a
protected bundle or the unrelated security-scan report.

## Acceptance Criteria and Retained Evidence

| Criterion | Result | Retained evidence |
|---|---|---|
| AC-C01 replay equivalence | Pass | `state_replay` prefix/suffix, duplicate, sequence, and idempotency tests plus verified SQLite replay tests |
| AC-C02 crash boundaries | Pass, portable | CRASH-01 through CRASH-12 matrix below |
| AC-C03 isolated attempt ownership | Pass | `attempt_worktrees` WT-01 through WT-06 |
| AC-C04 user checkout unchanged | Pass | WT-07 and WT-08 byte, status, index, HEAD, and ref comparison |
| AC-C05 retain unsafe worktrees | Pass | WT-09 through WT-14 dirty, locked, foreign, link, collision, and interrupted-creation cases |
| AC-C06 all concurrency caps | Pass | `execution_admission` QUE-01 through QUE-08, including independent connections |
| AC-C07 reserve interactive capacity | Pass | QUE-09 through QUE-14 ordering, cap-one, blocked-head, starvation, and shared manual races |
| AC-C08 budget reservation and conservation | Pass | execution-budget/store BUD-01 through BUD-10 |
| AC-C09 unknown and excess usage | Pass | BUD-11 through BUD-15 retained reservation, overage pause, and mismatch diagnostics |
| AC-C10 artifact bounds and authority | Pass | `blackboard_dag` ART-01 through ART-12, SEC-01, and link-substitution test |
| AC-C11 evidence-gated reviewer DAG | Pass | DAG-01 through DAG-06 three-task and failure-propagation tests |
| AC-C12 bounded fallback | Pass | `run_execution_service_tests` EFX-01 through EFX-08 and typed cleanup-verification cases |
| AC-C13 joined cancellation | Pass, portable; native incomplete | LIFE-01 through LIFE-09 and exact Linux descendant cleanup; Windows Job Object and macOS rows unrun |
| AC-C14 ownership and clock recovery | Pass, portable | REC-01 through REC-08 and CLK-01 through CLK-03 fixtures |
| AC-C15 deduplicated human review | Pass, internal | REC-09 through REC-13 ambiguity, reattach, response-loss, deduplication, and audit tests; no public resolver exists |
| AC-C16 authenticated remote execution | Pass, loopback only | REM-01 through REM-07 environment binding, replay, conflict, disconnect, status, and cancellation tests |
| AC-C17 migration 14 safety | Pass | schema-13 preservation, independent verified backup, failure rollback, retry, constraints, and malformed-row rejection tests |
| AC-C18 architecture and toolchains | Pass, local graphs | pure-boundary fixtures, exact pins, Rust 1.89 headless graph, locked all-target checks, and dependency policy |

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
`d78a22cb69711d411ee857add7aed09d654452b0` publishes the owner's report to all
in-process waiters while retaining bounded durable-state observation as the
crash/restart fallback. Its deterministic two-caller regression passes, as do
20 pressured 237-test server-unit-suite repetitions (4,740 tests), the complete
Section 11 matrix, and the explicit exact Linux native rows. The
documentation-only 0.60.104 reconciliation receives the same complete matrix
after commit so the final handoff can report its immutable result without
making this file self-referential.

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
children and an authenticated loopback node. It is not native Windows, native
macOS, physical-remote, live-provider, CI, deployment, or release evidence.

## Native and Live Acceptance

| Row | Status | Exact evidence or gap |
|---|---|---|
| Linux process-group descendant cleanup | Pass | On exact 0.60.103 repair commit, `cargo test --locked -p pandamux-server --test run_shutdown lifecycle_supervised_process_group_kills_its_descendant -- --exact`: 1 passed |
| Linux real concurrent Git worktrees and ownership checks | Pass | On exact 0.60.103 repair commit, `cargo test --locked -p pandamux-server --test attempt_worktrees`: 11 passed |
| Windows native | Unrun | No exact Phase 5C no-console, NTFS, junction/reparse, locked-file, Job Object, or restart evidence |
| macOS native | Unrun | No exact process-group, ownership/link, or real-worktree evidence |
| Authenticated loopback node | Pass, portable | Private environment-bound RPC, disconnect, status, cancel, and replay tests passed inside `pandamux-server` |
| Physical remote Linux node | Unrun | No exact physical-node baseline, lease, proxy reconnect, cancellation, or provider-turn evidence |
| Live Codex | Unrun | No live provider was contacted |
| Live Claude | Unrun | No live provider was contacted |
| Live Antigravity | Unrun | No live provider was contacted |
| Controlled external HTTP/file/command effect | Unrun | No disposable external service or repository was mutated |
| Target-specific CI | Unrun | No push or CI run was authorized |

The implemented/local classification therefore passes. The specification's
complete local/native classification does not pass because required Windows
and macOS native evidence is absent.

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
| Implementation | Complete for Phase 5C's private runtime at exact 0.60.103 repair commit |
| Portable local verification | Pass |
| Complete local/native acceptance | Not established; Windows and macOS rows unrun |
| Physical remote/live acceptance | Not established |
| Checkpoint commits | C-A through C-J, both verification repairs, and the initial C-K documentation committed; final C-K reconciliation is this documentation commit |
| Push | Not performed |
| CI | Not run |
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
- Missing Windows, macOS, physical-remote, live-provider, controlled-external,
  and CI evidence is recorded as unrun rather than inferred from mocks,
  cross-builds, Linux, or source inspection.
- The internal recovery-review resolution method is tested and audited, but a
  human-facing resolver is intentionally unavailable until Phase 5D.
- Migration 14 has no destructive down migration. Rollback requires stopping
  the server, verifying and restoring the complete pre-v14 schema-13 backup,
  preserving the v14 database, and retaining all ambiguous worktrees,
  artifacts, ledgers, and reviews.
