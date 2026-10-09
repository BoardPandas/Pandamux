# Phase 5B Trusted Configuration Acceptance Record

Date: 2026-10-09

Status: **Pass for the authorized local matrix and the contract's required
native Windows rows on exact version 0.60.88. External Git and SFTP, CI,
production activation, Phase 4 exit, deployment, and release were not in this
acceptance. P4-C5 remains Partial and P4-C7 remains open.**

## Deciding source and scope

- Exact deciding implementation and native acceptance source: clean
  `codex/recovery-r0` commit
  `a0e6defb859ee0809737b464e90e730ff8373d05`, version `0.60.88`.
- Isolated worktree:
  `/home/brandon/.codex/worktrees/recovery-r0/Pandamux`.
- Detailed contract:
  [`tasks/2026-10-09-phase-5b-trusted-configuration.md`](../../tasks/2026-10-09-phase-5b-trusted-configuration.md).
- Phase 5A deciding source and evidence remain separate in
  [`PHASE5A_ACCEPTANCE.md`](PHASE5A_ACCEPTANCE.md).
- The initial B-G documentation checkpoint is exact commit
  `285e768042f3c5e07840fed31e4edb1e347f1e51`, version `0.60.84`. Native
  acceptance then exposed and repaired four platform-specific gaps without
  changing the Phase 5B product scope.
- This final reconciliation uses workspace version `0.60.89`. It changes
  documentation and version metadata only. The document uses `this
  documentation commit` below because a commit cannot contain its own final
  hash without changing that hash.

Phase 5B implements the trusted configuration control plane around the pure
Phase 5A compiler: validated identities, one private app-data graph, additive
migration 13, revisioned settings, managed personal Git, bounded instructions
mirrors, candidate inspection, exact project-layer trust, layered compilation,
immutable bundle activation, authenticated node distribution, durable mutation
proposals, and local review surfaces.

It does not implement Phase 5C run dispatch, Phase 5D full agent authoring,
Phase 5E memory, Phase 5F broad mutating MCP, Phase 6 schedule mutation, new
provider authentication, hosted team services, packaging, signing,
publication, deployment, or release channels.

## Verified checkpoint chain

| Checkpoint | Version | Commit | Recorded outcome |
|---|---:|---|---|
| B-A | 0.60.78 | `ed1f203d1f36ec45912b2be111a32e8459568487` | Added typed configuration identities, canonical digest use, private `DataPaths`, migration 13, revisioned settings, protocol contracts, and desktop revision propagation. |
| B-B | 0.60.79 | `04e2f0936afa689c9f110be9de3f77ab112cf43f` | Added the private plumbing-only personal repository with bounded validation, request replay, ref compare-and-swap, failpoint recovery, history, diff, and forward-only revert. |
| B-C | 0.60.80 | `7d5bd51789134291b2e1cad9555a1ba6135638d6` | Added private instructions mirroring, branch, tag, and commit tracking, no-checkout inspection, signatures, complete diagnostics, polling, and last-good retention. |
| B-D | 0.60.81 | `599fa0cce041297dd3e33abd6138d6200d1f4978` | Added exact repository and project-layer trust, deterministic behavioral inspection, four-layer origins, disabled-agent routing exclusion, and local FullAccess opt-in. |
| B-E | 0.60.82 | `7070f550a750205deca301c5f3b967b0ed821735` | Added immutable bundle materialization, atomic local activation and outbox, typed authenticated upload, independent node verification, stable receipts, and reconnect drain. |
| B-F | 0.60.83 | `ce30ce7c5e9fa2c9ee6978dafe715093028cf29c` | Added durable mutation proposals, exact local resolution, expiry and disconnect handling, proposal-only MCP, redacted projections, confirmation cards, and trusted configuration status, trust, history, diff, and revert panels. |
| B-G | 0.60.84 | `285e768042f3c5e07840fed31e4edb1e347f1e51` | Reconciled root and maintained docs, mapped every acceptance row, and recorded the initial exact local evidence and native acceptance gate. |
| Native storage | 0.60.85 | `8fc9d0f433bd8686e617437d5ed7a66ff96bed83` | Added protected Windows directory creation, owner and DACL validation, reparse rejection, durable atomic personal-repository installation, long-path and fsync Git policy, and managed-path validation. |
| Native poller | 0.60.86 | `0fd31ac4cd8b8ad6f3a82fddda51d54f8cbf821d` | Stabilized the native polling acceptance fixture after Windows scheduling exposed a timing-only test race. |
| Native keyboard | 0.60.87 | `536022db1c7f11e5dba3db3d2ae6a0dd8a4e1b1f` | Added confirmation-card tab grouping and render-level keyboard tests after physical Tab traversal could not reach the actions. |
| Native semantics | 0.60.88 | `a0e6defb859ee0809737b464e90e730ff8373d05` | Added explicit disabled AccessKit semantics after the physical v0.60.87 run found terminal actions still reported enabled to UI Automation. |
| Final evidence | 0.60.89 | this documentation commit | Reconciles the accepted local and physical evidence, exact hashes, security review, and remaining external and production boundaries. |

Each implementation checkpoint was committed only after its checkpoint-focused
formatting, tests, lint, boundary, or diff checks passed. The complete Section
11 matrix passed on the exact final implementation source recorded below.
Native candidates were accepted only after the physical behavior matched the
contract; v0.60.87 is retained as a rejected accessibility candidate.

## Acceptance criteria to retained evidence

| ID | Retained evidence | Result |
|---|---|---|
| AC-B01 | `personal_repo::tests::empty_bootstrap_creates_valid_main_and_concurrent_init_converges` | Pass |
| AC-B02 | `personal_repo::tests::initialization_failpoints_recover_without_an_invalid_active_repo` and `every_edit_failpoint_preserves_head_and_retry_applies_once` | Pass |
| AC-B03 | `personal_repo::tests::revert_creates_a_child_commit_and_restores_exact_tree` | Pass |
| AC-B04 | `personal_repo::tests::secret_and_runtime_rejections_create_no_reachable_commit`, `case_collisions_fail_before_document_parsing_or_object_creation`, and exact-byte and hook isolation coverage | Pass |
| AC-B05 | `instructions_service::tests::branch_no_change_skips_fetch_and_branch_advance_creates_candidate` | Pass |
| AC-B06 | `instructions_service::tests::annotated_lightweight_and_exact_commit_tracking_retain_distinct_identity` plus branch-advance coverage | Pass |
| AC-B07 | `instructions_service::tests::force_push_requires_local_approval_and_retains_last_good`, `offline_and_missing_refs_return_safe_reasons_without_moving_active_state`, and `invalid_special_mode_and_unsigned_candidates_are_blocked_with_complete_issues` | Pass with local bare repositories |
| AC-B08 | `configuration_service::tests::trust_is_exact_hash_scoped_and_identity_changes_revoke_it`, `repository_root_change_invalidates_trust_without_folder_fallback`, and `store::configuration_phase5b::tests::project_trust_requires_every_exact_key_and_identity_change_revokes` | Pass |
| AC-B09 | `configuration_inspection::hostile_project_layer_retains_every_independent_finding` plus invalid-project and symlink coverage in `configuration_service` | Pass |
| AC-B10 | `bundle_store::tests::materialization_is_verified_immutable_and_reused`, `store::configuration_phase5b::tests::activation_crash_boundaries_leave_old_or_new_complete_state`, and `bundle_sync::tests::offline_outbox_drains_once_and_existing_run_stays_pinned` | Pass |
| AC-B11 | `bundle_store::tests::node_rejects_corruption_and_replays_the_original_receipt`, `bundle_sync::tests::offline_outbox_drains_once_and_existing_run_stays_pinned`, and `router::tests::bundle_upload_rpc_is_node_proxy_only_and_environment_bound` | Pass in local authenticated transport fixtures; no live node claim |
| AC-B12 | `mutation_service::tests::canonical_identity_deduplicates_key_order_and_conflicts_fail_closed` | Pass |
| AC-B13 | `mutation_service::tests::rejection_expiry_disconnect_and_revision_drift_never_mutate` and `approve_reject_race_uses_independent_sqlite_connections` | Pass |
| AC-B14 | `mutation_service::tests::apply_is_atomic_redacted_and_response_retry_is_idempotent` and `git_backed_revert_reconciles_a_lost_audit_response_once` | Pass |
| AC-B15 | `router::tests::every_settings_widening_requires_one_local_confirmation`, `mutation_service::tests::only_authenticated_local_control_can_resolve`, and MCP context and proposal tests | Pass |
| AC-B16 | `configuration_projection::tests::confirmation_redacts_values_and_retains_review_fields`, `expired_confirmation_has_text_status_and_disables_actions`, all four `mutation_confirmation` tests, both `trusted_configuration_view` tests, and the exact physical keyboard/UI Automation run | Pass locally and on physical Windows for the required confirmation behavior |
| AC-B17 | `store::migrations::tests::migration_thirteen_preserves_existing_rows_and_adds_trusted_configuration`, `migration_thirteen_failure_rolls_back_to_schema_twelve`, `store::tests::backup_failure_stops_before_schema_mutation`, and `settings_cas_rejects_stale_independent_connection` | Pass |
| AC-B18 | `configuration::tests::digest_and_revision_wrappers_reject_malformed_values`, protocol round trips, and the resolved five-target boundary checker with its negative fixture workspaces | Pass locally |

The tests above are retained in the workspace and run again through the full
workspace suite, so the result does not depend only on focused crate feature
selection.

## Exact local verification

The following complete matrix ran in the isolated worktree against exact clean
commit `a0e6defb859ee0809737b464e90e730ff8373d05`, version 0.60.88. Every
command exited zero.

| Command | Exact result |
|---|---|
| `cargo fmt --all -- --check` | Pass |
| `cargo test -p pandamux-core` | Pass: 71 passed, 0 failed, 0 ignored |
| `cargo test -p pandamux-orchestrator` | Pass: 72 passed, 0 failed, 0 ignored |
| `cargo test -p pandamux-protocol` | Pass: 16 passed, 0 failed, 0 ignored |
| `cargo test -p pandamux-server` | Pass: 214 passed, 0 failed, 0 ignored across library, binary, proxy transport, and lifecycle targets |
| `cargo test -p pandamux-client` | Pass: 20 passed, 0 failed, 0 ignored |
| `cargo test -p pandamux-desktop` | Pass: 75 passed, 0 failed, 0 ignored |
| `cargo test -p pandamux-remote` | Pass: 10 passed, 0 failed, 0 ignored |
| `cargo test -p pandamux-boundary-check` | Pass: 3 passed, 0 failed, 0 ignored, including the named negative fixture workspaces |
| `cargo run -p pandamux-boundary-check` | Pass: all 5 declared resolved Rust dependency targets satisfied |
| `cargo test --workspace` | Pass: 603 passed, 0 failed, 2 intentional real-shell ConPTY tests ignored |
| `cargo check --workspace --all-targets` | Pass across all workspace targets |
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass with zero warnings |
| `git diff --check` | Pass |

The version 0.60.89 final-evidence tree passed the same complete matrix before
commit. The matrix is rerun at the exact documentation commit, and that
post-commit result is reported in the task handoff because this file cannot
self-record its own content hash.

## Native Windows storage and Git verification

Exact versions 0.60.85 and 0.60.86 ran natively on the physical MSI host under
limited user `MSI\B_StL`. The final native server gate at v0.60.86 passed 181
tests with zero failures. Four unrelated pre-existing Windows portability cases
were deliberately excluded: a CRLF checkpoint assumption, one path-separator
assumption, one Antigravity slash assumption, and a SQLite migration test whose
open file handle returned `AccessDenied` on Windows. Those exclusions do not
cover any Phase 5B path, Git, or atomic-recovery row.

The retained native tests established all of the following:

- newly created managed roots receive a protected inheritable current-user
  DACL, while foreign ownership and broad access are rejected;
- existing roots and descendants accept only the current user and the explicit
  SYSTEM and Administrators recovery principals;
- symlinks and Windows junction/reparse substitution are rejected before use;
- every managed Git path component is revalidated, long paths and durable
  writes are forced, and Git for Windows read-only loose objects do not make a
  completed atomic candidate appear corrupt;
- personal-repository initialization reaches every staged failpoint and leaves
  either the old complete state or the new complete state; and
- native desktop tests and the isolated headless smoke passed at v0.60.86.

The Windows-wide Clippy run was not used as native acceptance because two
unrelated Windows-only style lints in existing code remained. The exact Linux
all-target strict Clippy gate passed. This is a stated verification boundary,
not a relaxation of any Phase 5B native behavior row.

## Physical confirmation accessibility

### Rejected v0.60.87 candidate

The first keyboard-repaired physical candidate proved that Tab reached
`Approve once` and `Reject`, and that Enter activated approval. It was rejected
because the resulting terminal actions were inert but UI Automation still
reported them enabled. Commit
`a0e6defb859ee0809737b464e90e730ff8373d05` added an AccessKit wrapper that
publishes the disabled state without making either action focusable or
invokable.

### Accepted exact v0.60.88 run

The accepted source was a clean detached checkout at
`C:\Users\B_StL\AppData\Local\PandaMUX-Acceptance\phase5b-msi-a0e6def-0.60.88-exact\source`.
Its complete Git source bundle is retained locally at
`target/native-acceptance/phase5b-msi-a0e6def-0.60.88-exact/source-exact.bundle`
with SHA-256
`0aa191bcf7e9389ea7a7bf24edc268b54d66540ce35c9ed82b5baa7badd294b1`;
the Linux and Windows copies matched. The bundle is source-transfer evidence,
not a packaged product artifact.

The native build record reports a clean source status, 75 desktop tests passed,
the server, CLI, and desktop built, and the isolated desktop headless smoke
passed. The exact desktop, server, and CLI SHA-256 values are respectively
`84961cbcb1801461b7ae3ee9f397418fa714b2853e3a1fbadc0cc08da8960e94`,
`fa2daad0c8bd415dc9996170dd86da7cb7e00a1d43fece00537f857726a09ff6`,
and `8078057fd0e53b2660493fad226d7834e8354c8bb018da7c8ed1357ddf5b2a15`.

The physical run used the limited interactive user and isolated local-app-data
root. UI Automation exposed the pending card as a named group containing the
complete redacted safe summary. Both pending actions were named buttons,
keyboard-focusable, enabled, and exposed `InvokePattern`. Physical Tab reached
Approve at step 7 and Reject at step 8; the harness returned to Approve and
Enter activated it at step 28. The durable card then reported `Approved and
applied`. Both terminal controls remained visible but were buttons with
`keyboardFocusable=false`, `enabled=false`, and no invokable patterns.

During the run, 35 managed Git child-process observations produced zero visible
windows. The pending screenshot has SHA-256
`944ac675968f1dd84c2c2fc5260abd3d3337425dd3ff0ff77cd9742906b7b9d3`.
The complete build logs, result JSON, and screenshot remain under ignored local
root `target/native-acceptance/phase5b-msi-a0e6def-0.60.88-exact`.

The accepted result has `passed=true`, `lastStep=passed`, and no cleanup errors.
The desktop, server, and MCP processes stopped; all Phase 5B acceptance
scheduled tasks were removed; and final inspection found no matching PandaMUX,
Cargo, or rustc process.

## Security and privacy evidence

- Personal repository tests seed secret and runtime sentinels before Git object
  creation and inspect the controlled repository object and ref state after
  rejection. No network repository is used.
- Proposal persistence contains only the trusted-settings subset for settings
  changes. Provider paths, environment overrides, arbitrary provider settings,
  raw proposal values, and other potentially secret fields are excluded from
  the desktop projection.
- Git stderr and diagnostics are bounded and mapped to stable safe reasons.
  Candidate tree inspection retains all safe issues without echoing hostile
  content into the confirmation projection.
- Caller role, authenticated environment, client identity, capability scope,
  and connection identity come from the server-owned `RequestContext`. Request
  JSON cannot forge those fields.
- Provider children and node proxies cannot propose or resolve mutations. MCP
  may use the one typed proposal-only tool but cannot apply settings directly.
  Only authenticated local control can approve once or reject.
- The scoped security review reported zero Critical, High, or Medium findings.
  Its two Low findings are retained local Phase 4 credential-shaped fixture
  files outside tracked source and yanked transitive `chacha20 0.10.1` and
  `yoke-derive 0.8.3` packages. `cargo deny check advisories` found no known
  advisory; `cargo audit` was unavailable. The review report remains untracked
  and was not added to this checkpoint.

## Deliberate deviations and implementation decisions

### Personal Git and SQLite cannot share one transaction

Database-backed mutations commit target data, immutable audit, and terminal
resolution in one SQLite transaction. A personal revert must first move a Git
ref, so the implementation uses the existing request-idempotent ref
compare-and-swap and then reconciles the one audit result. Retrying after a lost
response returns the existing commit and audit instead of creating another
revert. This is the specification's explicit Git-backed exception, not a
weaker silent transaction claim.

### Desktop diffs are category-safe projections

The desktop shows exact proposal identity, revision, digests, affected scopes,
and safe behavioral categories, but never renders raw old and new JSON values.
Unknown keys collapse to `Other reviewed changes`. This deliberately favors
secret safety over a generic raw-value diff while preserving the required
review dimensions.

### MCP remains proposal-only

The optional `propose_agent_access` MCP tool was implemented because immutable
request context is now threaded through the MCP call. It persists one exact
proposal and returns immediately. Direct MCP settings mutation remains disabled
and MCP cannot resolve its own proposal.

### Local fixtures do not stand in for live systems

Instructions tests use temporary local bare repositories and distribution tests
use authenticated local transport fixtures. They verify the deterministic and
failure behavior without contacting GitHub or another external host, but they
do not claim real hosting credentials, real SFTP infrastructure, a live remote
node, or production activation.

## Evidence boundary and remaining gates

- The deciding results include the exact local Linux matrix, native Windows
  path/Git/atomic tests, and a physical limited-user keyboard and UI Automation
  run on the exact v0.60.88 binaries.
- High Contrast was off, animations were on, and the physical preflight used
  the host's normal display configuration. Phase 5B-specific High Contrast,
  scaling, reduced-motion, and Narrator runs were not performed and are not
  implied by the accepted confirmation result.
- No live instructions host, external Git network path, remote SFTP node, or
  real reconnect distribution was exercised. Those network paths were
  explicitly out of scope. Local bare and authenticated fake transport fixtures
  remain the only acquisition and distribution evidence.
- CI did not run. No CI result is claimed.
- No push, deployment, activation in a production environment, packaging,
  signing, publication, release, served-version check, GitHub repository write,
  or pull request occurred.
- Phase 5B does not complete the provider-controlled P4-C5 capacity-failover
  row or final P4-C7 synchronization. P4-C5 remains Partial and P4-C7 remains
  open.

## Preservation and rollback

All task work remained inside the designated recovery worktree. The original
checkout, retained Galahad databases and evidence, exact 0.60.64 Phase 4
runtime candidate, and all 11 protected `.pandamux-*.bundle` files were not
modified, removed, or staged. The protected bundles remain untracked.

Migration 13 has no destructive live down migration. To run an older binary,
stop the server, verify the private pre-v13 backup checksum, preserve the v13
database, restore the complete matching backup, and leave personal repositories,
mirrors, and bundles preserved but inactive. Do not drop tables or decrement
`_migrations` in place.

Configuration rollback is forward-only. Reactivating an earlier bundle creates
a new activation audit and normal node outbox, reverting personal content
creates a new Git commit, revoking project trust creates an audit, and changing
tracking policy requires a newly reviewed proposal.
