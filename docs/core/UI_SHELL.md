<!-- PAGE_ID: pandamux_06_ui-shell -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [lib.rs:1-66](crates/pandamux-ui/src/lib.rs#L1-L66)
- [iced_shell.rs:1-325](crates/pandamux-ui/src/iced_shell.rs#L1-L325)
- [iced_shell.rs:1237-1349](crates/pandamux-ui/src/iced_shell.rs#L1237-L1349)
- [shell_projection.rs:1-224](crates/pandamux-ui/src/shell_projection.rs#L1-L224)
- [chrome.rs:31-141](crates/pandamux-ui/src/chrome.rs#L31-L141)
- [chrome.rs:147-227](crates/pandamux-ui/src/chrome.rs#L147-L227)
- [content_views.rs:1-90](crates/pandamux-ui/src/content_views.rs#L1-L90)
- [session_panel.rs:25-121](crates/pandamux-ui/src/session_panel.rs#L25-L121)
- [session_launcher.rs:1-57](crates/pandamux-ui/src/session_launcher.rs#L1-L57)
- [command_palette.rs:1-59](crates/pandamux-ui/src/command_palette.rs#L1-L59)
- [context_menu.rs:1-59](crates/pandamux-ui/src/context_menu.rs#L1-L59)
- [overlays.rs:1-30](crates/pandamux-ui/src/overlays.rs#L1-L30)
- [settings.rs:14-71](crates/pandamux-ui/src/settings.rs#L14-L71)
- [theme.rs:213-272](crates/pandamux-ui/src/theme.rs#L213-L272)
- [icons.rs:1-46](crates/pandamux-ui/src/icons.rs#L1-L46)
- [metrics.rs:1-55](crates/pandamux-ui/src/metrics.rs#L1-L55)

</details>

# UI Shell

> **Related Pages**: [Architecture](ARCHITECTURE.md), [Application Runtime](APP_RUNTIME.md)

---

<!-- BEGIN:AUTOGEN pandamux_06_ui-shell_overview -->
## Overview and Iced Isolation

`pandamux-ui` is the Iced-based native shell: it renders the split workspace, chrome, and every overlay as a read-projection of backend state and never owns canonical data (`crates/pandamux-ui/src/iced_shell.rs` doc comment) ([iced_shell.rs:1-9](crates/pandamux-ui/src/iced_shell.rs#L1-L9)).

Per the crate-isolation invariant, `pandamux-ui` is the ONLY crate in the workspace allowed to import Iced; every module except `shell_projection` is gated behind the `iced-runtime` feature so the projection types stay usable from headless/CLI-facing crates without pulling in the GPU stack ([lib.rs:1-26](crates/pandamux-ui/src/lib.rs#L1-L26)). `shell_projection` is the sole module that compiles unconditionally, which is why it is the boundary other crates (and `pandamux-core` consumers) can depend on ([lib.rs:23](crates/pandamux-ui/src/lib.rs#L23)).

| Module | Responsibility |
|---|---|
| `chrome` | Titlebar, icon rail, status bar; `ChromeState`, `MainView`, `Overlay`, `RailItem`, `SessionActivity` ([lib.rs:2](crates/pandamux-ui/src/lib.rs#L2)) |
| `command_palette` | Ctrl+K command palette and quick-launch popover list picker ([lib.rs:4](crates/pandamux-ui/src/lib.rs#L4)) |
| `content_views` | Renderers for non-terminal surface content (markdown / diff) ([lib.rs:6](crates/pandamux-ui/src/lib.rs#L6)) |
| `context_menu` | Right-click terminal context menu and the session-rail actions menu ([lib.rs:8](crates/pandamux-ui/src/lib.rs#L8)) |
| `iced_shell` | The canvas terminal viewport, `ShellMessage`, `ShellViewModel`, `app_view`/`shell_view` composition ([lib.rs:10](crates/pandamux-ui/src/lib.rs#L10)) |
| `icons` | Line-glyph icon set drawn as `canvas::Program`s ([lib.rs:12](crates/pandamux-ui/src/lib.rs#L12)) |
| `metrics` | Measured monospace `CellMetrics` (width/height) driving grid sizing ([lib.rs:14](crates/pandamux-ui/src/lib.rs#L14)) |
| `overlays` | Find bar, copy-mode indicator, notifications slide-over ([lib.rs:16](crates/pandamux-ui/src/lib.rs#L16)) |
| `session_launcher` | Staged Local/SSH project session launcher (multi-step wizard) ([lib.rs:18](crates/pandamux-ui/src/lib.rs#L18)) |
| `session_panel` | The Sessions rail panel (project/type grouping, rename, close) ([lib.rs:20](crates/pandamux-ui/src/lib.rs#L20)) |
| `settings` | Settings modal (General/Terminal/Keyboard/Notifications/Quick launch) ([lib.rs:22](crates/pandamux-ui/src/lib.rs#L22)) |
| `shell_projection` | Backend-to-UI read-projection of the split tree; the only feature-independent module ([lib.rs:23](crates/pandamux-ui/src/lib.rs#L23)) |
| `theme` | `UiTheme`/`Accent`/`Palette`/`TermScheme` design tokens ([lib.rs:25](crates/pandamux-ui/src/lib.rs#L25)) |

Sources: [lib.rs:1-66](crates/pandamux-ui/src/lib.rs#L1-L66)
<!-- END:AUTOGEN pandamux_06_ui-shell_overview -->

---

<!-- BEGIN:AUTOGEN pandamux_06_ui-shell_shell -->
## The Iced Shell

`iced_shell.rs` is the largest module in the crate (2573 lines) and hosts the `ShellMessage` intent enum, the `ShellViewModel` read model, the canvas-backed `TerminalViewport`, and the top-level view composition functions.

`ShellMessage` is a single flat enum carrying every UI interaction: pane/surface intents (`PaneFocused`, `PaneSplit`, `PaneClosed`), tab drag-and-drop (`TabDragArmed`, `DragOverZone`, `DragReleased`), chrome/window actions, the session panel, overlays (palette/launcher/settings/confirm), the in-app updater, and low-level terminal I/O (`TerminalInput`, `ViewportResized`, `ViewportScrolled`, selection and context-menu variants) ([iced_shell.rs:31-325](crates/pandamux-ui/src/iced_shell.rs#L31-L325)). Every field is documented inline with which spec section it implements, since the runtime is the single place that turns a message into a core intent.

```rust
/// The terminal canvas measured a pane size whose grid dimensions differ
/// from the engine grid. The runtime debounces these and resizes the
/// engine + PTY/SSH channel to match (spec 1.1).
ViewportResized {
    surface_id: SurfaceId,
    columns: usize,
    rows: usize,
},
```

Sources: [iced_shell.rs:243-250](crates/pandamux-ui/src/iced_shell.rs#L243-L250)

`ShellViewModel` is the render-time aggregate: the `ShellProjection` split-tree view, a `Vec<TerminalSnapshot>` (one per live surface), `ChromeState`, and one field per overlay/panel (`find`, `notifications`, `sessions`, `palette`, `launcher`, `settings`, `context_menu`, `rail_menu`, `confirm`, `home`, `update`) ([iced_shell.rs:413-456](crates/pandamux-ui/src/iced_shell.rs#L413-L456)). `TerminalSnapshot` carries both plain-text `lines` (for link detection/text consumers) and styled per-cell `cells`, the write-cursor position, detected `links`, selection spans, and terminal mode flags for input routing ([iced_shell.rs:350-382](crates/pandamux-ui/src/iced_shell.rs#L350-L382)).

`TerminalViewport` is a `canvas::Program<ShellMessage>` that owns per-canvas interaction state (`ViewportState`) and paints the fixed-pitch grid using measured `crate::metrics::CellMetrics` ([metrics.rs:18-29](crates/pandamux-ui/src/metrics.rs#L18-L29)) for column/row sizing ([iced_shell.rs:644-725](crates/pandamux-ui/src/iced_shell.rs#L644-L725)). Its `update` method (part of the `canvas::Program` impl starting at line 817) turns raw mouse/keyboard canvas events into `ShellMessage`s such as `ViewportResized`, `SelectionStarted`/`SelectionUpdated`, and `ViewportScrollTo` ([iced_shell.rs:817-820](crates/pandamux-ui/src/iced_shell.rs#L817-L820)).

Two entry points compose the final `Element`: `app_view` builds the complete chrome (titlebar, icon rail + optional session panel, workspace or Home view, optional update banner, optional status bar) and stacks overlays/notifications/context menus on top with `iced::widget::stack!` ([iced_shell.rs:1253-1349](crates/pandamux-ui/src/iced_shell.rs#L1253-L1349)); `shell_view` renders just the pane workspace and backs the headless smoke path and unit tests ([iced_shell.rs:1352-1354](crates/pandamux-ui/src/iced_shell.rs#L1352-L1354)).

Sources: [iced_shell.rs:1-325](crates/pandamux-ui/src/iced_shell.rs#L1-L325), [iced_shell.rs:350-456](crates/pandamux-ui/src/iced_shell.rs#L350-L456), [iced_shell.rs:644-820](crates/pandamux-ui/src/iced_shell.rs#L644-L820), [iced_shell.rs:1237-1354](crates/pandamux-ui/src/iced_shell.rs#L1237-L1354), [metrics.rs:1-55](crates/pandamux-ui/src/metrics.rs#L1-L55)
<!-- END:AUTOGEN pandamux_06_ui-shell_shell -->

---

<!-- BEGIN:AUTOGEN pandamux_06_ui-shell_projection -->
## Read-Projection

`shell_projection.rs` converts a `pandamux_core::WorkspaceState`'s canonical binary `SplitNode` tree into the UI's `ShellProjection`, which carries the focused/zoomed pane ids, the raw `ShellNodeProjection` tree, a flattened `visible_panes` list, and the design's 2-level `columns` layout ([shell_projection.rs:6-20](crates/pandamux-ui/src/shell_projection.rs#L6-L20)).

`project_workspace_shell` resolves the zoomed pane (if any) to a single-pane root, otherwise projects the full tree via `project_node`, then derives both `visible_panes` (depth-first pane collection) and `columns` (the column layout) from that root ([shell_projection.rs:78-114](crates/pandamux-ui/src/shell_projection.rs#L78-L114)).

`columns_from_node` implements the graceful fallback for arbitrary-depth trees: a `Pane` becomes a single one-pane column; a `Horizontal` split concatenates the columns of each side (side-by-side); a `Vertical` split flattens everything beneath it, including any nested split, into one column's stack. This preserves 2-level design fidelity while guaranteeing deeper CLI/orchestrator-built trees (e.g. `layout.grid`) never drop a pane ([shell_projection.rs:116-155](crates/pandamux-ui/src/shell_projection.rs#L116-L155)).

```rust
fn columns_from_node(node: &ShellNodeProjection) -> Vec<ColumnProjection> {
    match node {
        ShellNodeProjection::Pane(pane) => vec![ColumnProjection {
            panes: vec![pane.clone()],
        }],
        ShellNodeProjection::Split {
            direction: SplitDirection::Horizontal,
            first,
            second,
            ..
        } => {
            let mut columns = columns_from_node(first);
            columns.extend(columns_from_node(second));
            columns
        }
        ShellNodeProjection::Split {
            direction: SplitDirection::Vertical,
            first,
            second,
            ..
        } => {
            let mut panes = Vec::new();
            collect_visible_panes(first, &mut panes);
            collect_visible_panes(second, &mut panes);
            vec![ColumnProjection { panes }]
        }
    }
}
```

Sources: [shell_projection.rs:128-155](crates/pandamux-ui/src/shell_projection.rs#L128-L155)

`project_pane` fills a `PaneProjection` (focused/zoomed flags, the pane's `SurfaceProjection` list, the active surface id) from a core `LeafNode`, and `ratio_percent` converts the branch's `f32` split ratio to a display percentage ([shell_projection.rs:183-223](crates/pandamux-ui/src/shell_projection.rs#L183-L223)). A dedicated test suite exercises the default single-pane workspace, horizontal/vertical splits, zoom, and a 5-pane `layout.grid` tree to assert no pane is ever dropped by the column projection ([shell_projection.rs:225-345](crates/pandamux-ui/src/shell_projection.rs#L225-L345)).

Sources: [shell_projection.rs:1-224](crates/pandamux-ui/src/shell_projection.rs#L1-L224), [shell_projection.rs:225-345](crates/pandamux-ui/src/shell_projection.rs#L225-L345)
<!-- END:AUTOGEN pandamux_06_ui-shell_projection -->

---

<!-- BEGIN:AUTOGEN pandamux_06_ui-shell_chrome -->
## Chrome and Content Views

`chrome.rs` owns the app frame: `ChromeState` aggregates the theme/accent, the active rail item, session-panel open/grouping state, the active overlay, activity/shell-kind, and status-bar fields (git branch/ahead, listening ports, sidebar progress) ([chrome.rs:79-107](crates/pandamux-ui/src/chrome.rs#L79-L107)). `RailItem` (Sessions/CommandPalette/NewSession/Notifications/Settings), `Overlay` (None/CommandPalette/QuickLaunch/Settings/Confirm/CheatSheet), `MainView` (Workspace/Home), and `SessionActivity` (Idle/Running/BusyAgent) are the small enums that drive which chrome elements render ([chrome.rs:31-74](crates/pandamux-ui/src/chrome.rs#L31-L74)).

Three public view functions render the frame's fixed regions:

| Function | Renders |
|---|---|
| `titlebar` | Brand mark, session-switcher pill (opens the command palette), bell/settings icons, window controls ([chrome.rs:147-227](crates/pandamux-ui/src/chrome.rs#L147-L227)) |
| `icon_rail` | The 52px icon rail (`theme::RAIL_WIDTH`): Sessions/Palette/New/Notifications at top, Settings pinned to the bottom ([chrome.rs:252-307](crates/pandamux-ui/src/chrome.rs#L252-L307)) |
| `status_bar` | The bottom status strip (shell kind, git branch/ahead, ports, encoding, version) ([chrome.rs:345-429](crates/pandamux-ui/src/chrome.rs#L345-L429)) |

`content_views.rs` renders the two non-terminal surface kinds inside the same fixed-dark pane box the terminal viewport uses, so both stay theme-independent light-on-dark and never touch `palette.ov` (which flips with the chrome theme) ([content_views.rs:1-11](crates/pandamux-ui/src/content_views.rs#L1-L11)). `markdown_view` is a pragmatic line-based pass (headings, bullets, blockquotes, fenced code, rules, paragraphs) rather than a full CommonMark tree, built to cover the orchestrator dashboard and doc surfaces ([content_views.rs:23-65](crates/pandamux-ui/src/content_views.rs#L23-L65)). `diff_view` colors each line of a unified diff in monospace by its leading character ([content_views.rs:68-81](crates/pandamux-ui/src/content_views.rs#L68-L81)).

Sources: [chrome.rs:31-141](crates/pandamux-ui/src/chrome.rs#L31-L141), [chrome.rs:147-429](crates/pandamux-ui/src/chrome.rs#L147-L429), [content_views.rs:1-90](crates/pandamux-ui/src/content_views.rs#L1-L90)
<!-- END:AUTOGEN pandamux_06_ui-shell_chrome -->

---

<!-- BEGIN:AUTOGEN pandamux_06_ui-shell_panels -->
## Session Panel, Launcher, and Palette

These four modules cover session discovery/switching (the rail panel), creating new sessions (the launcher wizard), running arbitrary commands (the palette), and contextual per-pane actions (the context menus).

| Module | Purpose |
|---|---|
| `session_panel` | Projects `AppState` into `SessionsViewState`: grouped `SessionGroup`s (by `SessionGrouping::Project` or `::Type`) of `SessionEntry` rows, each carrying a type badge, host, activity, and active flag; `project_sessions`/`project_sessions_with_profiles` build the projection and `session_panel` renders it ([session_panel.rs:25-121](crates/pandamux-ui/src/session_panel.rs#L25-L121)) |
| `session_launcher` | A staged wizard (`LauncherStep`: Project → SessionType → Connection/ProfileForm/Credential/HostConfirmation → Folder → Launching) for local and SSH sessions; `LauncherItem` rows carry the `ShellMessage` a keyboard or mouse activation dispatches so both paths hit the same action ([session_launcher.rs:1-57](crates/pandamux-ui/src/session_launcher.rs#L1-L57)) |
| `command_palette` | The Ctrl+K palette and quick-launch popover; `PaletteItem` pairs a glyph/label/shortcut with the `ShellMessage` it fires, and `filter_items` does a case-insensitive substring filter over the label ([command_palette.rs:1-59](crates/pandamux-ui/src/command_palette.rs#L1-L59)) |
| `context_menu` | The right-click terminal menu (`ContextMenuAction`: Copy/Paste/SelectAll/ClearBuffer/Find/SplitRight/SplitDown/CloseTab) and the session-rail actions menu (`RailMenuAction`), both positioned cards over a transparent backdrop ([context_menu.rs:1-59](crates/pandamux-ui/src/context_menu.rs#L1-L59)) |

The launcher's `SshProfileForm` holds the in-progress name/host/port/user/auth/identity-file fields plus a validation `error` string for the connection-profile step ([session_launcher.rs:48-57](crates/pandamux-ui/src/session_launcher.rs#L48-L57)). `SessionEntry::type_label` picks the Type-grouping key: the shell's abbreviation for a plain terminal, or the session kind's label (Claude/Codex/Gemini/custom) otherwise ([session_panel.rs:57-65](crates/pandamux-ui/src/session_panel.rs#L57-L65)).

Sources: [session_panel.rs:25-121](crates/pandamux-ui/src/session_panel.rs#L25-L121), [session_launcher.rs:1-57](crates/pandamux-ui/src/session_launcher.rs#L1-L57), [command_palette.rs:1-59](crates/pandamux-ui/src/command_palette.rs#L1-L59), [context_menu.rs:1-59](crates/pandamux-ui/src/context_menu.rs#L1-L59)
<!-- END:AUTOGEN pandamux_06_ui-shell_panels -->

---

<!-- BEGIN:AUTOGEN pandamux_06_ui-shell_theme -->
## Overlays, Settings, and Theming

`overlays.rs` groups the terminal-adjacent floating surfaces: `FindViewState` (query/case-sensitivity/match count/current match span) backs the find bar ([overlays.rs:14-27](crates/pandamux-ui/src/overlays.rs#L14-L27)); `ConfirmViewState` backs the generic destructive-action confirm modal (`Overlay::Confirm`) whose pending action lives in the runtime ([overlays.rs:87-93](crates/pandamux-ui/src/overlays.rs#L87-L93)); `NotificationsViewState`/`NotificationCard` back the right-side notifications slide-over ([overlays.rs:263-279](crates/pandamux-ui/src/overlays.rs#L263-L279)).

`settings.rs` defines the settings modal's navigation and content: `SettingsSection` (General/Terminal/Keyboard/Notifications/QuickLaunch, with a `SettingsSection::ALL` array driving the nav list) and `TerminalToggle` (WelcomePrompt/RightClickPaste/ConfirmClose) for the Terminal tab's persisted toggles ([settings.rs:14-49](crates/pandamux-ui/src/settings.rs#L14-L49)). `SettingsViewState` carries the active section, theme/accent, the live keyboard-shortcut list sourced from the runtime's keymap (so the cheat sheet and this list can never drift apart), the persistent `pandamux_core::TerminalSettings`, and the in-app `UpdateState` for the General tab's "Check for updates" / Install controls ([settings.rs:51-71](crates/pandamux-ui/src/settings.rs#L51-L71)).

`theme.rs` is the shared design-token source: layout constants (`TITLEBAR_HEIGHT`, `RAIL_WIDTH`, `SESSION_PANEL_WIDTH`, `WORKSPACE_PADDING`, corner radii), font helpers, and text-size constants, plus `Palette` (the chrome color set for a `UiTheme` + `Accent` pair) and `TermScheme` (the terminal canvas color set) ([theme.rs:115-166](crates/pandamux-ui/src/theme.rs#L115-L166)).

Theme *loading* from a `.theme` file happens in `pandamux_core` (`parse_ghostty_theme`/`parse_hex`); `TermScheme::from_theme` is where `pandamux-ui` adapts a loaded `pandamux_core::Theme` into the canvas's color set, falling back to the fixed-dark default for any missing or invalid color (background/foreground direct, `dim`/`success`/`gold` from ANSI palette indices 8/2/3):

```rust
pub fn from_theme(theme: &pandamux_core::Theme) -> Self {
    let base = Self::default();
    let hex = |value: &Option<String>, fallback: Color| {
        value.as_deref().and_then(hex_to_color).unwrap_or(fallback)
    };
    let palette = |index: usize, fallback: Color| {
        theme
            .palette
            .get(index)
            .and_then(|value| hex_to_color(value))
            .unwrap_or(fallback)
    };
    Self {
        background: hex(&theme.background, base.background),
        text: hex(&theme.foreground, base.text),
        dim: palette(8, base.dim),
        success: palette(2, base.success),
        gold: palette(3, base.gold),
        cursor: hex(&theme.cursor, hex(&theme.foreground, base.text)),
        ansi: std::array::from_fn(|index| palette(index, base.ansi[index])),
    }
}
```

Sources: [theme.rs:213-268](crates/pandamux-ui/src/theme.rs#L213-L268)

A unit test confirms the fallback behavior end to end: a partial `.theme` file (background, foreground, and only palette index 2) yields the parsed colors for what is present and the default scheme's `gold` for what is absent ([theme.rs:466-479](crates/pandamux-ui/src/theme.rs#L466-L479)).

`icons.rs` draws every chrome glyph (`Icon`: Search, Bell, Settings, Sessions, Palette, Plus, Git, Terminal, SplitRight/Down, ZoomIn/Out, Close, Minimize, Maximize, Folder, Home, Drive) as a stroked `canvas::Program` in a normalized `[0, 1]` box rather than an icon font, so one glyph definition renders crisply at any requested size ([icons.rs:1-46](crates/pandamux-ui/src/icons.rs#L1-L46)).

Sources: [overlays.rs:1-30](crates/pandamux-ui/src/overlays.rs#L1-L30), [settings.rs:14-71](crates/pandamux-ui/src/settings.rs#L14-L71), [theme.rs:115-272](crates/pandamux-ui/src/theme.rs#L115-L272), [icons.rs:1-46](crates/pandamux-ui/src/icons.rs#L1-L46)
<!-- END:AUTOGEN pandamux_06_ui-shell_theme -->

---
