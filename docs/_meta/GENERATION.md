# Generation Metadata

- **Commit:** `4b8db04c2d6ce9e2fe9b5406937c1d9479b98507`
- **Commit role:** source baseline inspected for this refresh, not the documentation commit
- **Documentation branch:** `codex/recovery-r0`
- **Generated:** 2026-10-08T08:58:40-04:00
- **Working tree:** modified by this documentation update
- **Mode:** full active-wiki refresh for the GPUI client and local server architecture
- **Previous documentation snapshot:** `bd341e05acc22a0cea49d98ec041a358eb789a47`
- **Pages maintained:** 15
- **Sections maintained:** 90
- **Current Rust source coverage:** 142 of 142 files

## Notes

The maintained generated wiki now covers the nine-crate GPUI client, local server,
provider, terminal, remote-node, CLI, and protocol architecture. The source
baseline identifies the implementation inspected for this refresh, not the
documentation commit that will record it.

The obsolete Iced shell-integration page moved to
`docs/archive/iced-multiplexer/`. The hand-maintained Phase 4 acceptance and
usage-observability pages were preserved because they already describe the
current evidence boundary. No Rust implementation source was changed.

## Validation boundary

The active wiki passed structural, source-coverage, local-link, citation-range,
and static Mermaid checks. A Mermaid renderer was not installed, so rendered
diagram validation was not performed. Historical documents remain excluded
from current-accuracy validation and may intentionally retain links to removed
source layouts.

This refresh is interim P4-C7 preparation. It does not close the remaining real
provider-capacity failover or physical-laptop disconnect and sleep acceptance
rows, and it does not declare the Phase 4 production exit complete.
