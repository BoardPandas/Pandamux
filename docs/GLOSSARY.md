<!-- PAGE_ID: pandamux_15_glossary -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [crates/pandamux-core/src/ids.rs:1-48](crates/pandamux-core/src/ids.rs#L1-L48)
- [crates/pandamux-core/src/state.rs:1-49](crates/pandamux-core/src/state.rs#L1-L49)
- [crates/pandamux-core/src/state.rs:67-269](crates/pandamux-core/src/state.rs#L67-L269)
- [crates/pandamux-core/src/state.rs:271-389](crates/pandamux-core/src/state.rs#L271-L389)
- [crates/pandamux-core/src/split_tree.rs:95-132](crates/pandamux-core/src/split_tree.rs#L95-L132)
- [CLAUDE.md](../CLAUDE.md)

</details>

# Glossary

> **Related Pages**: [Overview](OVERVIEW.md), [Architecture](core/ARCHITECTURE.md)

---

<!-- BEGIN:AUTOGEN pandamux_15_glossary_terms -->
## Terms

This page collects the branded identifier types and architecture vocabulary used throughout the PandaMUX Rust workspace, so other generated pages can link a single canonical definition instead of re-explaining each term.

| Term | Definition |
|---|---|
| **AppDelta** | The enum of state-change events emitted by `AppState::apply` after an intent is processed; variants include `WorkspaceCreated`, `PaneSplit`, `SurfaceCreated`, and `SurfaceListReported` among others. This is the "delta-out" half of the intent-in/delta-out pattern ([state.rs:271-384](crates/pandamux-core/src/state.rs#L271-L384)). |
| **AppIntent** | The top-level enum of caller-submitted mutations (`System`, `Workspace`, `Pane`, `Surface`, `Project`, `Home`), each wrapping a domain-specific intent enum. This is the single "intent-in" entry point both the UI and the pipe server submit to ([state.rs:68-76](crates/pandamux-core/src/state.rs#L68-L76)). |
| **AppState** | The root, serializable snapshot of all workspaces, the active workspace, the project registry, and the Home dashboard layout; this is the canonical state persisted to `session.json` ([state.rs:16-31](crates/pandamux-core/src/state.rs#L16-L31)). |
| **Backend-owned state (intent-in, delta-out, single writer)** | The design rule that `pandamux-app` owns the canonical workspace/pane/surface split tree while the Iced UI holds only a read-projection and submits intents; the named-pipe server (CLI/agents/orchestrator) and the UI submit the same intents to the same sync dispatcher (`pandamux-app::backend::handle_line`), so CLI-driven and UI-driven mutations are indistinguishable at the state layer (CLAUDE.md). |
| **BranchNode** | A `SplitNode` variant holding a `SplitDirection`, a split `ratio`, and exactly two boxed child `SplitNode`s; the internal-node half of the split tree ([split_tree.rs:95-99](crates/pandamux-core/src/split_tree.rs#L95-L99)). |
| **Capabilities** | A struct reported by `system.capabilities` describing whether `browser`, `layout_grid`, and `native` features are available; `browser` is always reported `false` because MCP/CDP support was intentionally dropped (CLAUDE.md; [state.rs:59-65](crates/pandamux-core/src/state.rs#L59-L65)). |
| **Crate-isolation invariant** | The CI-enforced hard rule (`scripts/check-rust-boundaries.ps1`) that `pandamux-core` and `pandamux-term` have zero Iced dependency, `pandamux-ui` is the only crate that imports Iced, and `alacritty_terminal` types never appear outside `pandamux-term`, so an engine or framework swap touches only one crate (CLAUDE.md). |
| **Immutable split tree** | The design decision that pane/surface layouts are a binary `SplitNode` tree in `pandamux-core::split_tree`, where mutations produce new trees rather than mutating in place; the UI renders a 2-level column projection over arbitrary-depth trees (CLAUDE.md; [split_tree.rs:101-106](crates/pandamux-core/src/split_tree.rs#L101-L106)). |
| **Intent-in, delta-out** | The submission/response contract for state mutation: callers submit an `AppIntent`, and `AppState::apply` returns an `AppDelta` describing what changed, rather than exposing mutable state directly ([state.rs:417](crates/pandamux-core/src/state.rs#L417)). |
| **PaneId** | A branded identifier (`pane-<uuid>`) for a single pane slot within a workspace's split tree, generated via `PaneId::generate()` ([ids.rs:44](crates/pandamux-core/src/ids.rs#L44)). |
| **PaneIntent** | The enum of pane-scoped mutations: `Split`, `Close`, `Focus`, `Zoom`, `LayoutGrid`, and `List` ([state.rs:178-198](crates/pandamux-core/src/state.rs#L178-L198)). |
| **PaneSummary** | A struct summarizing a pane by id for `pane.list` responses ([state.rs:386-389](crates/pandamux-core/src/state.rs#L386-L389)). |
| **prefixed_id! macro** | The macro that generates every branded ID type in `ids.rs` (`WorkspaceId`, `PaneId`, `SurfaceId`, `WindowId`, `SshProfileId`, `ProjectId`), giving each a string prefix, a `generate()` UUID constructor, `Display`, and `From<&str>`/`From<String>` impls ([ids.rs:4-48](crates/pandamux-core/src/ids.rs#L4-L48)). |
| **ProjectId** | A branded identifier (`proj-<uuid>`) for a project-registry record, the stable identity above `ProjectKey` (spec 1.4) ([ids.rs:48](crates/pandamux-core/src/ids.rs#L48); [state.rs:25-27](crates/pandamux-core/src/state.rs#L25-L27)). |
| **ProjectIntent** | The enum of project-registry mutations: `List`, `Rename`, `Merge` (fold one record into another), `Split` (detach a workspace into a fresh record), and `AttachMatcher` ([state.rs:108-133](crates/pandamux-core/src/state.rs#L108-L133)). |
| **PTY = Surface ID** | The design decision that each terminal surface keeps its `Term`/PTY alive in memory keyed by surface id, so switching tabs never reconstructs grid state (CLAUDE.md). |
| **Read-projection** | The UI-side pattern where `pandamux-ui` holds a read-only view of the backend's canonical state and submits intents rather than mutating state directly, per the backend-owned state design (CLAUDE.md). |
| **SessionType** | An enum recorded on a surface describing what runs in it (spec 2.2/2.7 session types), set via `SurfaceIntent::SetSessionType` ([split_tree.rs:23](crates/pandamux-core/src/split_tree.rs#L23); [state.rs:237-242](crates/pandamux-core/src/state.rs#L237-L242)). |
| **SplitDirection** | An enum used by `BranchNode` and `SplitPaneParams` to indicate horizontal vs. vertical splitting ([split_tree.rs:80](crates/pandamux-core/src/split_tree.rs#L80); [state.rs:259-267](crates/pandamux-core/src/state.rs#L259-L267)). |
| **SplitNode** | The immutable binary tree type (`Leaf` or `Branch`) representing a workspace's pane layout; mutations produce new trees rather than mutating in place ([split_tree.rs:101-106](crates/pandamux-core/src/split_tree.rs#L101-L106)). |
| **SshProfileId** | A branded identifier (`ssh-<uuid>`) for a saved SSH remote-host profile ([ids.rs:47](crates/pandamux-core/src/ids.rs#L47)). |
| **SurfaceId** | A branded identifier (`surf-<uuid>`) for a single terminal/markdown/diff surface, the unit that a PTY is keyed by ([ids.rs:45](crates/pandamux-core/src/ids.rs#L45)). |
| **SurfaceIntent** | The enum of surface-scoped mutations: `Create`, `CreateWithId` (preallocated id from the launch coordinator), `Focus`, `Close`, `Move` (drag-drop to a drop target), `Rename`, `SetSessionType`, and `List` ([state.rs:200-247](crates/pandamux-core/src/state.rs#L200-L247)). |
| **SystemIntent** | The enum of system-level queries: `Identify`, `Capabilities`, and `Tree` (dump a workspace's split tree) ([state.rs:135-141](crates/pandamux-core/src/state.rs#L135-L141)). |
| **V1 protocol** | The plain-text named-pipe protocol used by shell-integration hooks (e.g., `report_pwd`), predating the JSON-RPC V2 protocol (CLAUDE.md). |
| **V2 protocol (pipe protocol)** | The token-authenticated JSON-RPC protocol over `\\.\pipe\pandamux` used by the CLI, agents, and the orchestrator, covering `system.*`, `workspace.*`, `pane.*`, `layout.grid`, `surface.*`, `markdown.*`, `diff.*`, `notification.*`, `sidebar.*`, `agent.*`, `clipboard.*`, `ssh.*`, `window.*`, `config.*`, `theme.*`, and `hook.event` (CLAUDE.md). |
| **WindowId** | A branded identifier (`win-<uuid>`) for an OS-level application window ([ids.rs:46](crates/pandamux-core/src/ids.rs#L46)). |
| **WorkspaceId** | A branded identifier (`ws-<uuid>`) for a workspace, the top-level container holding one split tree ([ids.rs:43](crates/pandamux-core/src/ids.rs#L43)). |
| **WorkspaceIntent** | The enum of workspace-scoped mutations: `Create`, `CreateProject` (transactional creation with caller-provided ids), `Select`, `Rename`, `Close`, `CloseAll`, and `List` ([state.rs:143-176](crates/pandamux-core/src/state.rs#L143-L176)). |
| **WorkspaceState** | The struct holding one workspace's live state: id, title, shell, project spec/id, split tree, and focused/zoomed pane ids ([state.rs:33-48](crates/pandamux-core/src/state.rs#L33-L48)). |
| **WorkspaceSummary** | A lightweight struct (id, title, shell, project) used to report workspaces without the full split tree, e.g., in `WorkspaceListReported` ([state.rs:50-57](crates/pandamux-core/src/state.rs#L50-L57)). |
| **pandamux-orchestrator** | The bundled Claude Code plugin (`resources/pandamux-orchestrator/`) that decomposes complex dev tasks into parallel Claude Code agents in visible panes, coordinated through the pipe protocol with state kept in a JSON file in `TMPDIR` and no daemon (CLAUDE.md). |

Sources: [ids.rs:1-48](crates/pandamux-core/src/ids.rs#L1-L48), [state.rs:1-389](crates/pandamux-core/src/state.rs#L1-L389), [split_tree.rs:95-132](crates/pandamux-core/src/split_tree.rs#L95-L132), CLAUDE.md
<!-- END:AUTOGEN pandamux_15_glossary_terms -->

---
