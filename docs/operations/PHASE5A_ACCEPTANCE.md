# Phase 5A Deterministic Orchestrator Acceptance Record

Date: 2026-10-09

Status: **Pass for the pure-domain Phase 5A checkpoint on exact version
0.60.76. This is not Phase 5 runtime acceptance, Phase 4 exit, native-platform
acceptance, CI execution, deployment, or release evidence. P4-C5 remains
Partial and P4-C7 remains open.**

## Deciding source and scope

- Exact deciding source: clean `codex/recovery-r0` commit
  `423594e3ad77497119aac09ba098a82bd499aa74`, version `0.60.76`.
- Isolated worktree:
  `/home/brandon/.codex/worktrees/recovery-r0/Pandamux`.
- Phase 5A specification commit: `161153b`, with dependency and boundary gate
  `7b874b0d64761306bb4efe2696726b576912e0ca` preserved.
- Detailed contract:
  [`tasks/2026-10-09-phase-5a-deterministic-orchestrator.md`](../../tasks/2026-10-09-phase-5a-deterministic-orchestrator.md).
- This acceptance-record follow-up uses workspace version `0.60.77`. It adds
  documentation and version metadata only. Its targeted pure-domain
  re-verification is recorded below; no new behavior, native, runtime,
  provider, or live result is claimed.

Phase 5A implements pure parsing, compilation, validation, routing, policy,
model-output, command decision, event application, and replay behavior. It
does not add a database migration, durable event or effect outbox, effect
interpreter, provider dispatch, worktree owner, run RPC, CLI command, desktop
surface, or live-provider path.

## Verified checkpoint chain

| Checkpoint | Version | Commit | Recorded outcome |
|---|---:|---|---|
| A | 0.60.68 | `70d0228d0c6bae2339c6f6638e75defa32d947c1` | Froze exact legacy V0 compatibility and explicit migration boundaries. |
| B | 0.60.69 | `948066f395b5a3ed181ff754bedc10c7c922bf98` | Added canonical shared run, agent, bundle, event, effect, approval, budget, and blackboard records. |
| C | 0.60.70 | `26f25dfa13deebf8444e52ef269335877874d2c5` | Added strict versioned authored schemas, limits, and safe diagnostics. |
| D | 0.60.71 | `9c5ca8b8984c51dbb3273bd3500915ed1e380c5b` | Added trusted reference resolution, layer reconciliation, canonical bytes, and bundle digest verification. |
| E | 0.60.72 | `0c303d4f23b4d3e4bec4e1b67b88f53478a558c3` | Added deterministic DAG normalization and fail-closed plan validation. |
| F | 0.60.73 | `b5791bde061f177da555353162ffcfcbd0c0a003` | Added deterministic two-step routing and one approval reconciler for both modes. |
| G | 0.60.74 | `1bc014fa597638bad2020768e327f058f8f1ba67` | Added bounded model parsing plus total, replayable command, event, and effect state. |
| H candidate | 0.60.75 | `77b8d67cb5ff71f39803db30ec1b9eb0a78ac09e` | Added CI coverage and maintained documentation; the first full workspace test exposed the digest defect below. |
| Acceptance repair | 0.60.76 | `423594e3ad77497119aac09ba098a82bd499aa74` | Canonicalized nested approval action objects independently of Serde JSON map features and passed the complete local Section 12 matrix. |

Each checkpoint was committed only after its focused formatting, strict Clippy,
tests, boundary, or documentation checks passed. The final complete command
matrix below decides Phase 5A acceptance.

## Contract-to-evidence mapping

| Contract | Exact evidence | Result |
|---|---|---|
| Byte-equivalent replay with no duplicate effect | `state_replay::full_replay_snapshot_suffix_and_every_prefix_are_byte_identical` and `adjacent_and_nonadjacent_identical_events_are_noops_without_duplicate_effects` | Pass |
| Identical route and explanation | `routing_policy::route_and_explanations_are_stable_under_input_permutations` plus stable tie and immutable-route tests | Pass |
| Unknown or malformed authored and model input fails closed | `schema_fixtures` matrix plus `model_output` strict decode, hostile field, bound, and one parse-only retry tests | Pass |
| Invalid DAG cannot dispatch | `dag_validation` duplicate, missing, cycle, cross-organization, output, budget, capability, and approval tests plus run plan-rejection tests | Pass |
| Auto cannot widen authority | Hostile bundle layer tests, `routing_policy::auto_never_skips_forbidden_safety_classes`, and state approval identity, revision, digest, and expiry tests | Pass |
| One-task and one-node work uses the full pipeline | `routing_policy::one_task_one_node_still_runs_the_complete_pipeline` | Pass |
| Pure orchestrator boundary | Five-target resolved dependency graph check and Linux target checks | Pass locally |
| Legacy records are never silently reinterpreted | `compatibility_fixtures` exact V0, later-field, and malformed-record tests | Pass |

The focused orchestrator suite contains 63 passing tests. The workspace suite
contains the same tests under the full unified feature graph, which is material
to the repaired approval digest contract.

## Exact local verification

All commands in this table ran in the isolated worktree on clean exact commit
`423594e3ad77497119aac09ba098a82bd499aa74`, version `0.60.76`, on Linux.

| Command | Exact result |
|---|---|
| `cargo fmt --all --check` | Pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass with zero warnings |
| `cargo test -p pandamux-orchestrator` | Pass: 63 passed, 0 failed, 0 ignored |
| `cargo test --workspace` | Pass: 526 passed, 0 failed, 2 intentionally ignored real-shell ConPTY tests; 528 tests discovered |
| `cargo run --locked -p pandamux-boundary-check --quiet` | Pass: all 5 declared resolved dependency targets satisfied |
| `cargo deny check advisories licenses bans sources --hide-inclusion-graph` | Pass: advisories, licenses, bans, and sources all OK; configured warnings remained for one unmatched license allowance, duplicate transitives, and yanked transitive `chacha20 0.10.1` and `yoke-derive 0.8.3` |
| `cargo +1.89.0 check --locked --workspace --all-targets --exclude pandamux-desktop` | Pass for the documented Linux headless MSRV graph |
| `node scripts/check-claude-wiring.mjs` | Pass: about 7,476 always-on tokens, 3 rules, 1 every-session rule |
| `git diff --check` | Pass |

The version 0.60.77 documentation follow-up then passed
`cargo fmt --all --check`, focused locked strict Clippy, all 63 locked
orchestrator tests, the five-target locked boundary checker, repository wiring,
and `git diff --check`. This targeted re-verification confirms the version-only
manifest change without replacing the complete deciding 0.60.76 matrix above.

The final maintained documentation audit for this acceptance follow-up passed
with 16 pages, 98 ordered generated sections, 1,515 local links,
1,389 valid line-range citations, all 154 current crate source files mapped,
and 19 statically valid Mermaid blocks. No Mermaid renderer was installed, so
rendered-diagram validation was not performed.

## Rejected candidate and repair

The first complete workspace run on exact 0.60.75 commit `77b8d67` failed
`canonical_approval_identity_and_one_mode_reconciler_deduplicate_exact_actions`.
Focused orchestrator tests had passed because their Serde JSON map backend
sorted keys. The full workspace unified GPUI's `preserve_order` feature into
Serde JSON, which made a nested approval action object preserve insertion
order and changed its digest.

Exact 0.60.76 commit `423594e` fixed the source contract rather than weakening
the test. Approval canonicalization now recursively sorts nested JSON object
keys before hashing, independent of the selected map backend. The regression
pins digest
`6f8d1c3d05d97eed01c2075d8b214890582453321b8597acedb3158391128712`.
The focused regression, workspace-unified regression, strict Clippy, and the
complete Section 12 matrix then passed.

## Evidence boundary and remaining gates

- The results above are local Linux pure-domain and compile evidence only.
- The configured macOS orchestrator test and musl x64 and arm64 all-target
  checks have not run in CI. No CI result is claimed.
- No exact Phase 5A artifact ran natively on Windows or macOS.
- No database, persistence, filesystem, worktree, provider process, RPC, CLI,
  desktop UI, node, laptop, or live-provider path exists for this checkpoint.
- No runtime, accessibility, production, served-version, activation, push,
  deployment, signing, packaging, publication, or release evidence is claimed.
- The Phase 5 sequencing exception does not close the provider-controlled
  P4-C5 capacity-failover row or the final P4-C7 documentation archive.

## Preservation and rollback

Work remained inside the designated recovery worktree. The preserved original
checkout, retained Galahad databases and evidence, exact 0.60.64 Phase 4
runtime candidate, and all 11 protected `.pandamux-*.bundle` files were not
modified or removed. The protected bundles remain untracked. No push,
deployment, signing, packaging, publication, or release occurred.

Phase 5A has no database migration. Rollback is the checkpoint chain above in
reverse order, starting with the acceptance repair and ending with Checkpoint
A. Do not reset or clean the worktree to perform that rollback, and retain the
frozen V0 boundary unless a fixture proves it incorrect.
