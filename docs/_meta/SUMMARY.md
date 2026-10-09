# Documentation Generation Summary

## Full Active-Wiki Refresh - 2026-10-08

- **Mode:** full active-wiki refresh for the current GPUI architecture
- **Source baseline:** `4b8db04c2d6ce9e2fe9b5406937c1d9479b98507` on `codex/recovery-r0`
- **Previous documentation snapshot:** `bd341e05acc22a0cea49d98ec041a358eb789a47`
- **Working tree:** modified by this documentation update; the source baseline is not the documentation commit
- **Scope:** 15 maintained pages and 90 generated sections

### Outcomes

- Replaced the removed five-crate Iced model with the implemented nine-crate GPUI client and local-server architecture.
- Mapped all 142 current Rust source modules into `_toc.yaml` and rebuilt every generated page from that map.
- Added a current provider-routing page covering health evidence, explicit login, organization policy, usage state, failover, and continuation behavior.
- Rewrote the active documentation index and corrected root routing language so current implementation, maintained acceptance evidence, and historical material are clearly separated.
- Archived the obsolete Iced shell-integration page under `docs/archive/iced-multiplexer/` instead of deleting the historical record.
- Preserved the hand-maintained Phase 4 acceptance and usage-observability pages because they already state the current evidence and open gates.

### Coverage and known boundaries

- **Rust source coverage:** 142 of 142 `crates/*/src/**/*.rs` files are represented in `_toc.yaml` and cited by the generated wiki.
- **Explicit gaps:** 0 `_TBD_` markers remain in maintained generated pages.
- **Historical scope:** archived and historical documents are labeled and excluded from active-accuracy validation; their links were not rewritten as current architecture claims.
- **Release boundary:** the release workflow and bundled remote-binary manifest are documented as dormant or placeholder infrastructure, not as a working release path.
- **Acceptance boundary:** this is interim P4-C7 synchronization. Exact 0.60.64 closes both physical P4-C6 rows; real pre-output provider-capacity failover and the final post-P4-C5 archive remain open.

### Validation

- **YAML and structure:** `_toc.yaml` parses with 15 pages; all 15 PAGE_ID markers and all 90 ordered BEGIN/END AUTOGEN pairs match.
- **Links:** 1,292 local links across active documentation resolve with 0 broken targets; 1,245 are in generated pages.
- **Citations:** 1,197 line-linked citations resolve to existing files, valid anchors, and valid line ranges.
- **Source map:** all 142 current Rust source files are mapped, with 0 missing modules.
- **Mermaid:** 17 blocks pass static type and delimiter checks. `mmdc` is unavailable, so renderer-level validation was not performed.
- **Semantic review:** each generated page group was reviewed against current implementation and the repository authority order separately from the structural validator.

## Incremental Reconciliation - 2026-10-08

- **Mode:** incremental update from source baseline `4b8db04c` through exact
  runtime baseline `a41c5bd` on `codex/recovery-r0`
- **Generated pages changed:** Terminal Engine, UI Shell, SSH Remote Nodes,
  Release and Packaging, and Glossary
- **Maintained records changed:** Usage Observability, Phase 3 Acceptance,
  Phase 4 Acceptance, the documentation index, root guides, and the current
  rewrite plan
- **Implementation coverage:** Windows shell selection now records bounded
  executable validation; provider-login confirmation, terminal handoff, and
  Surfaces toggle semantics match the accepted 0.60.62 behavior; SSH proxy
  documentation now covers protocol-v2 keepalives, the 90-second node lease,
  and split bridge termination
- **Acceptance outcome:** exact 0.60.64 commit `a41c5bd` passed both mandatory
  physical P4-C6 rows on the MSI laptop. The real pre-output provider-capacity
  row remains open, so this is not Phase 4 exit, release, deployment, or served
  version evidence.
- **Validation:** 15 pages, 90 ordered AUTOGEN sections, 1,314 active local
  links, 1,218 line-range citations, all 142 current Rust source files, and 17
  Mermaid blocks passed with zero structural errors. `mmdc` is unavailable, so
  renderer-level Mermaid validation was not performed. Locked metadata,
  formatting, strict all-target and all-feature Clippy, 458 workspace tests
  with two intentional real-shell ignores, both repository guards, desktop
  headless smoke, dependency policy, and diff hygiene also passed at 0.60.65.

## Phase 5 Dependency-Boundary Update - 2026-10-09

- **Mode:** incremental update from source baseline `a41c5bd` through clean
  documentation baseline `b0e5f744` plus the current 0.60.66 working tree on
  `codex/recovery-r0`
- **Generated pages updated:** Overview, Getting Started, Architecture, Core
  Domain, and Release and Packaging
- **Generated page added:** Toolchain and Dependency Policy, with six sections
  covering toolchains, exact pins, resolved graph boundaries, negative
  fixtures, supply-chain checks, and the safe dependency-change workflow
- **Implementation coverage:** the wiki maps the new pure orchestrator crate,
  fail-closed legacy compatibility fixtures, exact direct registry pin policy,
  Rust graph checker, five declared metadata targets, four negative fixture
  workspaces, Rust 1.98.1 development pin, and scoped Rust 1.89 jobs
- **Acceptance boundary:** this is an explicitly authorized Phase 5 sequencing
  exception. It does not pass P4-C5, close final P4-C7, replace exact 0.60.64
  Phase 4 runtime evidence, or alter the retained Galahad databases.
- **Validation:** 16 pages, 96 ordered AUTOGEN sections, 1,430 active local
  links, 1,323 line-range citations, all 144 current crate source files, and 18
  Mermaid blocks passed with zero structural errors. `mmdc` is unavailable, so
  renderer-level Mermaid validation was not performed.
- **Code gates:** formatting, strict all-target Clippy, 464 tests with two
  intentional real-shell ignores, the five-target resolved graph, negative
  boundary fixtures, repository wiring, all four cargo-deny policy families,
  and the local Rust 1.89 Linux headless graph passed. Cargo-deny emitted its
  configured duplicate and yanked-transitive warnings. The Windows MSRV job is
  implemented but remains unexecuted until CI runs it.

## Phase 5A Pure-Orchestrator Update - 2026-10-09

- **Mode:** incremental update through clean deciding source `423594e3`,
  version 0.60.76, plus the documentation-only 0.60.77 acceptance follow-up on
  `codex/recovery-r0`
- **Generated pages updated:** Overview, Getting Started, Architecture, Core
  Domain, Application Runtime, and Agent Orchestration
- **Implementation coverage:** all canonical run and instruction-bundle records
  plus strict schemas, safe diagnostics, trusted compilation, validated DAGs,
  deterministic routing, approval reconciliation, bounded model proposals, and
  replayable state and effects are mapped into the active wiki
- **CI coverage configured:** the macOS job explicitly tests the pure
  orchestrator, and compile-only `--all-targets` checks cover musl x64 and arm64
  while the Windows workspace, x64 musl binaries, and Linux/Windows MSRV jobs
  remain intact
- **Acceptance boundary:** Phase 5A contains no SQLite migration, outbox or
  effect interpreter, provider dispatch, worktree owner, run RPC, CLI, desktop
  orchestration UI, or live-provider path. P4-C5 stays Partial and P4-C7 stays
  open.
- **Documentation validation:** 16 pages, 98 ordered AUTOGEN sections, 1,515
  active local links, 1,389 line-range citations, all 154 current crate source
  files, and 19 Mermaid blocks pass with zero structural errors. `mmdc` remains
  unavailable, so renderer-level Mermaid validation was not performed.
- **Code-gate status:** the first complete workspace run on exact 0.60.75
  exposed approval-digest instability under Serde JSON workspace feature
  unification. Exact 0.60.76 recursively canonicalizes nested objects and pins
  the expected digest. Formatting, strict all-target Clippy, all 63 focused
  orchestrator tests, 526 workspace tests with 2 intentional ignores, the
  five-target boundary checker, all cargo-deny families, Rust 1.89 Linux
  headless graph, repository wiring, and diff hygiene then passed.
- **Follow-up verification:** version 0.60.77 passed formatting, focused locked
  strict Clippy, all 63 locked orchestrator tests, the locked five-target
  boundary checker, repository wiring, and diff hygiene. It changes no Phase
  5A behavior.
- **Evidence record:**
  [Phase 5A Deterministic Orchestrator Acceptance](../operations/PHASE5A_ACCEPTANCE.md)
  records every checkpoint, exact command result, rejected candidate, repair,
  preservation boundary, and absent CI, native, runtime, and live evidence.

## Phase 5B Trusted-Configuration Update - 2026-10-09

- **Mode:** incremental update from the prior Phase 5A documentation source
  `423594e3` through clean deciding implementation source `ce30ce7`, version
  0.60.83, plus the documentation-only 0.60.84 B-G working tree on
  `codex/recovery-r0`
- **Generated page added:** Trusted Configuration, with eight sections covering
  architecture and ownership, storage and migration, source layers and trust,
  activation and node distribution, mutation review, desktop accessibility,
  failure modes, and evidence boundaries
- **Maintained records updated:** root README and CLAUDE guide, docs index, TOC,
  generation metadata, this summary, and the Phase 5B acceptance record
- **Source coverage:** all 175 current `crates/*/src/**/*.rs` files are mapped,
  including every new Phase 5B core, orchestrator, protocol, server, client, and
  desktop module
- **Acceptance mapping:** AC-B01 through AC-B18 each name retained evidence;
  the checkpoint table records B-A through B-F exact hashes and versions, while
  B-G is labeled as the self-referential documentation commit
- **Evidence boundary:** local Linux tests and source-level UI checks do not
  establish native Windows Git-child, private-path, reparse, replacement, or
  accessibility behavior, or a live external Git and SFTP distribution path.
  P4-C5 remains Partial and P4-C7 remains open.
- **Documentation validation:** 17 TOC pages, 106 ordered AUTOGEN sections,
  1,612 active local links, 1,496 valid line-range citations, all 175 current
  Rust source files, and 21 Mermaid blocks passed with zero structural errors.
  Two stale Phase 5A `state.rs` upper bounds were corrected from line 1320 to
  the current line 1300. `mmdc` is unavailable, so renderer-level Mermaid
  validation was not performed.
- **Code-gate status:** exact 0.60.83 passed formatting, every focused crate
  test command, the five-target boundary checker, 600 workspace tests with 2
  intentional real-shell ignores, all-target checks, strict Clippy, and diff
  hygiene. The identical complete matrix also passed on the version 0.60.84
  B-G tree before commit and is rerun at the exact documentation commit before
  completion.
- **Remote status:** no push, deployment, production activation, package,
  signature, publication, release, served-version check, GitHub write, or pull
  request occurred.

## Phase 5B Native-Acceptance Reconciliation - 2026-10-09

- **Mode:** incremental update from initial B-G documentation commit
  `285e7680` through clean deciding implementation and native-acceptance source
  `a0e6defb`, version 0.60.88, plus the documentation-only 0.60.89 working tree
  on `codex/recovery-r0`
- **Generated page updated:** Trusted Configuration; five affected AUTOGEN
  sections now map the revised desktop evidence plus Windows private-directory
  security, complete managed-Git path and durability policy, keyboard and
  AccessKit semantics, and the exact evidence boundary
- **Source-map change:** `windows_security.rs` was added to the Trusted
  Configuration TOC sources; all 175 current Rust source files remain mapped
- **Maintained records updated:** the task status, changelog, root README and
  CLAUDE guide, docs index, TOC, generation metadata, this summary, and the
  Phase 5B acceptance record
- **Acceptance outcome:** exact 0.60.88 passed the complete local matrix and
  the contract's native Windows private-path, ownership, reparse, atomic
  recovery, no-console Git, keyboard, and UI Automation rows. The v0.60.87
  physical candidate was rejected because terminal controls still appeared
  enabled to UI Automation; v0.60.88 corrected and passed that state.
- **Evidence boundary:** no external Git or SFTP path, CI, production
  activation, push, deployment, packaging, signing, publication, release, or
  served-version check was exercised. High Contrast, scaling, reduced-motion,
  and Narrator were not rerun specifically for Phase 5B. P4-C5 remains Partial
  and P4-C7 remains open.
- **Documentation validation:** 17 TOC pages, 106 ordered AUTOGEN sections,
  1,630 active local links, 1,514 valid line-range citations, all 175 current
  Rust source files, and 21 Mermaid blocks passed with
  zero structural errors. `mmdc` is unavailable, so renderer-level Mermaid
  validation was not performed.
- **Code-gate status:** exact 0.60.88 passed formatting, every focused crate
  test, the five-target boundary checker, 603 workspace tests with 2 intentional
  real-shell ignores, all-target checks, strict Clippy, and diff hygiene. The
  identical complete matrix passed on the v0.60.89 documentation tree before
  commit and is rerun at the exact documentation commit before completion.

## Historical context

The previous generated wiki described the removed Iced multiplexer architecture.
Electron and Iced documents retained under `docs/archive/` and older records under
`docs/superpowers/` remain design history rather than current implementation
authority.
