# Generation Metadata

- **Commit:** `a41c5bd40861fb37de7a53252d96dcb055828362`
- **Commit role:** source baseline inspected for this refresh, not the documentation commit
- **Documentation branch:** `codex/recovery-r0`
- **Generated:** 2026-10-08T23:26:49-04:00
- **Working tree:** modified by this documentation update
- **Mode:** incremental update after the full active-wiki refresh
- **Previous documentation source baseline:** `4b8db04c2d6ce9e2fe9b5406937c1d9479b98507`
- **Pages maintained:** 15
- **Sections maintained:** 90
- **Current Rust source coverage:** 142 of 142 files

## Notes

The maintained generated wiki covers the nine-crate GPUI client, local server,
provider, terminal, remote-node, CLI, and protocol architecture. This
incremental pass reconciles implementation changes since the full refresh,
including Windows shell validation, provider-login and Surfaces behavior, and
the protocol-v2 proxy heartbeat, node lease, and bridge termination. The source
baseline identifies the implementation inspected for this update, not the
documentation commit that will record it.

Generated sections changed in `core/TERMINAL_ENGINE.md`, `core/UI_SHELL.md`,
`features/SSH_REMOTE.md`, `operations/RELEASE.md`, and `GLOSSARY.md`. The
maintained Usage page, Phase 3 and Phase 4 acceptance records, root guides, and
current rewrite plan were reconciled manually because their acceptance
narrative is not owned by the generated blocks. No Rust implementation source
was changed.

## Validation boundary

The active wiki passed structural, source-coverage, local-link, citation-range,
and static Mermaid checks: 15 pages, 90 ordered generated sections, 1,314 local
links, 1,218 line-range citations, all 142 current Rust source files, and 17
Mermaid blocks produced zero validation errors. A Mermaid renderer was not
installed, so rendered diagram validation was not performed. Historical
documents remain excluded from current-accuracy validation and may
intentionally retain links to removed source layouts.

This refresh records the exact 0.60.64 physical P4-C6 Pass. It is an interim
P4-C7 synchronization because real pre-output provider-capacity failover is
still open and the final archive must follow that result. It does not declare
the Phase 4 production exit complete. The documentation-only 0.60.65 workspace
also passed locked metadata, formatting, strict Clippy, 458 tests with two
intentional real-shell ignores, both repository guards, desktop headless smoke,
dependency policy, and diff hygiene.
