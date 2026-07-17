# Documentation Generation Summary

## Incremental Update — 2026-07-17

- **Mode:** update (full regeneration)
- **Commit range:** `0ab9e64..3accc14`
- **Reason:** The base commit predates the Electron-to-Rust rewrite. All 69 intervening commits replaced the TypeScript/Electron codebase with the Rust workspace, so every page was regenerated against `pandamux-core`/`pandamux-term`/`pandamux-ui`/`pandamux-app`/`pandamux-cli` rather than diff-scoped.

### Phase A — TOC drift

- **New pages: 6**
  - pandamux_04_core-domain → core/CORE_DOMAIN.md
  - pandamux_05_terminal-engine → core/TERMINAL_ENGINE.md
  - pandamux_06_ui-shell → core/UI_SHELL.md
  - pandamux_07_app-runtime → core/APP_RUNTIME.md
  - pandamux_11_ssh-remote → features/SSH_REMOTE.md
  - pandamux_14_release → operations/RELEASE.md (operations/ folder created)
- **Removed pages: 4 (moved to archive/)** — no Rust successor:
  - core/MAIN_PROCESS.md, core/RENDERER_AND_STATE.md (Electron process model)
  - features/BROWSER_CDP.md (browser/CDP pane intentionally dropped)
  - features/AI_INTEGRATION.md (folded into AGENT_ORCHESTRATION + APP_RUNTIME)
- **Rewritten in place: 9** — OVERVIEW, GETTING_STARTED, GLOSSARY, core/ARCHITECTURE, core/CONFIGURATION, api/CLI_REFERENCE, features/NAMED_PIPE_IPC, features/AGENT_ORCHESTRATION, features/SHELL_INTEGRATION.

### Phase B — Source diff

- **Files changed:** the entire application source (TypeScript `src/` deleted; Rust `crates/` added). Every page's `source_files` was re-pointed at Rust modules.
- **Sections generated:** 79 across 15 pages.
- **Pages touched:** 15 (all).

### Coverage

- **Crate source files:** 52 of 52 `crates/*/src/*.rs` files are cited (100%).
- **Per-page citations range 24–140** with tables and code snippets on every page.
- **`_TBD_` gaps (6 total):**
  - core/CONFIGURATION.md (2) — settings/keymap fields with no default expressed in code.
  - features/NAMED_PIPE_IPC.md (3) — method arms delegated elsewhere; flagged rather than guessed.
  - features/AGENT_ORCHESTRATION.md (1) — a plugin detail not present in the cited source.

### Accuracy notes surfaced during generation

- `pandamux-app::backend::handle_line` special-cases only the V1 `report_pwd` hook; other shell-integration messages (`report_git_branch`, `report_shell_state`, `ports_kick`, `report_pr`) fall through and are instead recomputed independently by `pandamux-app::pollers`. Documented in SHELL_INTEGRATION.md rather than presented as fully wired.
- `pandamux-app` has no dedicated `claude_context.rs` at this commit (contrary to a line in CLAUDE.md); the Claude-context startup wiring lives in `iced_runtime.rs`/`backend.rs`. Recorded in the TOC notes and AGENT_ORCHESTRATION.md.

### Validation

- **Structure:** 15/15 pages have exactly one PAGE_ID, first-line PAGE_ID, and matched BEGIN/END AUTOGEN markers whose counts and ids match `_toc.yaml` exactly (0 orphans, 0 missing).
- **Mermaid:** 13 diagram blocks; `mmdc` unavailable, so static checks applied (valid opening line, `graph TD` for flowcharts, quoted flowchart labels, balanced brackets, explicit sequence activation, `;` placeholders). All pass. classDiagram members and stateDiagram `[*]` nodes are valid unquoted syntax.
- **Internal navigation links** (Related Pages + README index): all resolve.
- **Source citations** use repo-root-relative paths (e.g. `crates/pandamux-core/src/state.rs#L40-L70`), consistent with the citation-policy convention and the prior doc set; they resolve from the repo root. Archived pages under `docs/archive/` retain their original (now historical) links and are intentionally left unmaintained.

## Prior generation — 2026-07-05 (init, Electron)

The original 14-page set documented the Electron/TypeScript prototype at commit `0ab9e64`. Those pages were archived or rewritten by this run.
