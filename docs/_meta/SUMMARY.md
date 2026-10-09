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

## Historical context

The previous generated wiki described the removed Iced multiplexer architecture.
Electron and Iced documents retained under `docs/archive/` and older records under
`docs/superpowers/` remain design history rather than current implementation
authority.
