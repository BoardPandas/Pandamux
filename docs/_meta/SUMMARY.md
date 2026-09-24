# Documentation Generation Summary

## Incremental Update - 2026-09-24

- **Mode:** update (accuracy, onboarding, history labels, and citation repair)
- **Source snapshot:** `bd341e05acc22a0cea49d98ec041a358eb789a47` on `master`
- **Previous documentation snapshot:** `3accc145c64c10d86785466dbe59b344f6100665`
- **Working tree:** modified by this documentation update; the snapshot identifies the source baseline, not a documentation commit
- **Scope:** 15 maintained pages and 79 generated sections

### Outcomes

- Added an AI fast path and an explicit authority order to `AGENTS.md`, with `CLAUDE.md` as the detailed contributor guide and `docs/README.md` as the wiki index.
- Reconciled current behavior across root guidance and generated docs: JSON settings, complete rewrite and release phases, manual orchestrator-plugin installation, V1 `ping` and `report_pwd`, and the currently unvalidated V2 token field.
- Moved the obsolete TOML configuration note to `docs/archive/CONFIG_TOML_LEGACY.md` and labeled all archived or historical design records so agents do not treat them as current implementation guidance.
- Repaired 872 Markdown targets so source citations resolve relative to each documentation page, then updated the doc-sync citation policy and template to preserve that behavior on future runs.
- Added six missing source mappings to `_toc.yaml`: the commit rule, term and UI manifests, UI metrics, app latency, and the OS clipboard bridge.
- Replaced 15 dead shared-template path globs in the LL-G and BP rules with live PandaMUX paths, including the Rust crates, repository tooling, release configuration, and agent configuration.

### Coverage and known boundaries

- **Crate source coverage:** 52 of 52 `crates/*/src/*.rs` files are cited (100%).
- **Source-map additions:** 6 of 6 previously omitted sources are now represented in `_toc.yaml`.
- **Explicit gaps:** 0 `_TBD_` markers remain in maintained generated pages. Historical documents are excluded from current-accuracy validation.
- **Protocol boundary:** the CLI supplies `PANDAMUX_PIPE_TOKEN`, but the server does not validate it; the local V2 pipe is not described as authenticated.
- **Plugin boundary:** PandaMUX bundles the orchestrator plugin but does not write to `~/.claude` or install the plugin automatically.

### Validation

- **YAML:** `_toc.yaml` parses successfully with 15 pages.
- **Structure:** 15 of 15 pages have first-line PAGE_ID markers; all 79 BEGIN/END AUTOGEN pairs match `_toc.yaml`, with no missing or orphan sections.
- **Links:** 1,155 local links across active documentation resolve from their containing pages, with 0 broken targets.
- **Citations:** 1,083 line-linked citations resolve to existing files and valid line ranges.
- **Mermaid:** 13 diagram blocks pass static opening checks. `mmdc` is unavailable in this environment, so rendered-diagram validation was not performed.
- **Claude wiring:** `node scripts/check-claude-wiring.mjs` passes; all path-scoped rule globs resolve and hook matchers are functional.

## Historical context

The July 2026 full regeneration replaced the Electron-oriented wiki with documentation for the five-crate Rust workspace. Electron-only pages remain in `docs/archive/` for history, and `docs/superpowers/` retains older plans and specifications as design records rather than current authority.
