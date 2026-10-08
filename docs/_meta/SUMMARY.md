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
- **Acceptance boundary:** this is interim P4-C7 preparation. Real pre-output provider-capacity failover and physical-laptop disconnect and sleep evidence remain open.

### Validation

- **YAML and structure:** `_toc.yaml` parses with 15 pages; all 15 PAGE_ID markers and all 90 ordered BEGIN/END AUTOGEN pairs match.
- **Links:** 1,292 local links across active documentation resolve with 0 broken targets; 1,245 are in generated pages.
- **Citations:** 1,197 line-linked citations resolve to existing files, valid anchors, and valid line ranges.
- **Source map:** all 142 current Rust source files are mapped, with 0 missing modules.
- **Mermaid:** 17 blocks pass static type and delimiter checks. `mmdc` is unavailable, so renderer-level validation was not performed.
- **Semantic review:** each generated page group was reviewed against current implementation and the repository authority order separately from the structural validator.

## Historical context

The previous generated wiki described the removed Iced multiplexer architecture.
Electron and Iced documents retained under `docs/archive/` and older records under
`docs/superpowers/` remain design history rather than current implementation
authority.
