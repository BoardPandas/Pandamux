# Generation Metadata

- **Commit:** `1bc014fa597638bad2020768e327f058f8f1ba67`
- **Commit role:** source baseline inspected for this refresh, not the documentation commit
- **Documentation branch:** `codex/recovery-r0`
- **Generated:** 2026-10-09T01:42:25-04:00
- **Working tree:** modified by this documentation update
- **Mode:** incremental update after the full active-wiki refresh
- **Previous documentation source baseline:** `b0e5f744c1a76f1e6d1a90f3d8d881a869c86b7a`
- **Pages maintained:** 16
- **Sections maintained:** 98
- **Current Rust source coverage:** 154 of 154 crate source files

## Notes

The maintained generated wiki covers ten product crates plus the workspace
dependency checker. This incremental pass documents the complete pure Phase 5A
surface: canonical run and bundle records, strict authored schemas, trusted
bundle compilation, DAG validation, deterministic two-step routing, approval
reconciliation, bounded model proposals, and replayable state and effects. The
source baseline identifies the clean commit inspected before these working-tree
changes, not the documentation commit that will record them.

Generated sections changed in `OVERVIEW.md`, `GETTING_STARTED.md`,
`core/ARCHITECTURE.md`, `core/CORE_DOMAIN.md`, and
`features/AGENT_ORCHESTRATION.md`; `core/APP_RUNTIME.md` received one citation
range repair. The documentation index and active rewrite plan preserve the
explicit sequencing exception while leaving P4-C5 Partial and P4-C7 open.

## Validation boundary

The active wiki passed structural, source-coverage, local-link, citation-range,
and static Mermaid checks: 16 pages, 98 ordered generated sections, 1,513 local
links, 1,389 line-range citations, all 154 current crate source files, and 19
Mermaid blocks produced zero validation errors. A Mermaid renderer was not
installed, so rendered diagram validation was not performed. Historical
documents remain excluded from current-accuracy validation and may
intentionally retain links to removed source layouts.

Checkpoint commits through version 0.60.74 passed their focused formatting,
strict Clippy, core/orchestrator tests, boundary, and diff checks before this
documentation update. The complete Section 12 command set will be run against
the exact committed 0.60.75 source and recorded in the separate Phase 5A
acceptance record. The configured macOS and musl CI jobs remain unexecuted until
CI runs, and no native platform, runtime, UI, live-provider, push, deployment,
signing, packaging, publication, or release result is claimed. P4-C5 remains
Partial and final P4-C7 remains open.
