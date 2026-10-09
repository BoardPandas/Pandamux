# Generation Metadata

- **Commit:** `b0e5f744c1a76f1e6d1a90f3d8d881a869c86b7a`
- **Commit role:** source baseline inspected for this refresh, not the documentation commit
- **Documentation branch:** `codex/recovery-r0`
- **Generated:** 2026-10-09T00:29:29-04:00
- **Working tree:** modified by this documentation update
- **Mode:** incremental update after the full active-wiki refresh
- **Previous documentation source baseline:** `a41c5bd40861fb37de7a53252d96dcb055828362`
- **Pages maintained:** 16
- **Sections maintained:** 96
- **Current Rust source coverage:** 144 of 144 crate source files

## Notes

The maintained generated wiki covers ten product crates plus the workspace
dependency checker. This incremental pass adds the pure Phase 5 orchestrator
boundary, fail-closed legacy compatibility input, exact direct dependency pins,
the target-resolved graph guard and negative fixtures, the pinned development
toolchain, and scoped MSRV policy. The source baseline identifies the clean
commit inspected before these working-tree changes, not the documentation
commit that will record them.

Generated sections changed in `OVERVIEW.md`, `GETTING_STARTED.md`,
`core/ARCHITECTURE.md`, `core/CORE_DOMAIN.md`, and `operations/RELEASE.md`.
`operations/DEPENDENCIES.md` adds six generated sections. The documentation
index and active rewrite plan record the explicit sequencing exception while
leaving P4-C5 Partial and P4-C7 open.

## Validation boundary

The active wiki passed structural, source-coverage, local-link, citation-range,
and static Mermaid checks: 16 pages, 96 ordered generated sections, 1,430 local
links, 1,323 line-range citations, all 144 current crate source files, and 18
Mermaid blocks produced zero validation errors. A Mermaid renderer was not
installed, so rendered diagram validation was not performed. Historical
documents remain excluded from current-accuracy validation and may
intentionally retain links to removed source layouts.

At workspace version 0.60.66, formatting, strict all-target Clippy, 464 tests
with two intentional real-shell ignores, resolved boundaries for five declared
targets, boundary negative fixtures, repository wiring, all four cargo-deny
policy families, and the Rust 1.89 Linux headless compile gate passed locally.
Cargo-deny retained configured warnings for duplicate and yanked transitive
packages. The Rust 1.89 Windows product job is implemented but has not run in
CI, and no native, push, deployment, signing, publication, or release result is
claimed. P4-C5 remains Partial and final P4-C7 remains open.
