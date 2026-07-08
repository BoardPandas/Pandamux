use crate::persistence::SessionStore;
use iced::futures::SinkExt;
use iced::{Element, Size, Subscription, Task, Theme, application, keyboard, stream, time, window};
use pandamux_core::{
    AppIntent, AppState, NewNotification, NotificationSource, Notifications, PaneIntent,
    SplitDirection, SplitNode, SplitPaneParams, SurfaceId, SurfaceIntent, SurfaceType,
    WorkspaceIntent, get_all_pane_ids,
};
use pandamux_term::{
    GridSize, PtyCommand, PtySessionManager, SearchOptions, detect_links, search_lines,
};
use pandamux_ui::{
    ChromeState, FindViewState, LinkSpan, NotificationCard, NotificationsViewState, Overlay,
    PaletteItem, PaletteViewState, QuickLaunchViewState, RailItem, SessionActivity,
    SessionsViewState, SettingsSection, SettingsViewState, ShellKind, ShellMessage, ShellViewModel,
    TerminalSnapshot, UiTheme, app_view, filter_items, project_sessions, project_workspace_shell,
    shell_view,
};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;

/// Pending replies for embedded pipe requests, keyed by correlation id. The
/// subscription task inserts a one-shot sender before emitting a `PipeRequest`;
/// the runtime removes and completes it after dispatching on the single-writer
/// path.
type PipeRegistry = Arc<StdMutex<HashMap<u64, oneshot::Sender<String>>>>;

/// Ticks between block-cursor blinks (~1.1s at a 100ms tick).
const CURSOR_BLINK_TICKS: u64 = 11;
/// Ticks between session autosaves (~30s at a 100ms tick).
const AUTOSAVE_TICKS: u64 = 300;

pub struct NativeShellRuntime {
    app_state: AppState,
    ptys: PtySessionManager,
    live_ptys: bool,
    chrome: ChromeState,
    tick: u64,
    store: SessionStore,
    find: FindViewState,
    find_matches: Vec<(usize, usize, usize)>,
    notifications: Notifications,
    notifications_open: bool,
    notif_seq: u64,
    copy_mode: bool,
    /// Command-palette state (query + selection persist across refreshes; items
    /// are rebuilt each refresh).
    palette: PaletteViewState,
    /// Which settings section is open.
    settings_section: SettingsSection,
    view_model: ShellViewModel,
    terminals: Vec<TerminalSnapshot>,
    last_error: Option<String>,
    /// Named pipe the embedded server binds (only when `live_ptys`).
    pipe_name: String,
    /// Pending embedded-pipe replies, shared with the subscription task.
    pipe_registry: PipeRegistry,
    /// Correlation-id source for embedded pipe requests.
    pipe_seq: Arc<AtomicU64>,
}

impl Default for NativeShellRuntime {
    fn default() -> Self {
        Self::new(false)
    }
}

impl NativeShellRuntime {
    pub fn new(live_ptys: bool) -> Self {
        let store = SessionStore::new(SessionStore::default_dir());
        // Only the real (live) app touches disk. Tests/smoke use default state so
        // they stay hermetic. On a version change the volatile auto-session is
        // cleared, so we start clean; otherwise restore the last layout.
        let app_state = if live_ptys {
            let version_changed = store.handle_version_change(env!("CARGO_PKG_VERSION"));
            if version_changed {
                AppState::default()
            } else {
                store.load_session().unwrap_or_default()
            }
        } else {
            AppState::default()
        };
        let chrome = ChromeState::default();
        let view_model = initial_view_model(&app_state, &chrome);
        let mut runtime = Self {
            app_state,
            ptys: PtySessionManager::new(),
            live_ptys,
            chrome,
            tick: 0,
            store,
            find: FindViewState::default(),
            find_matches: Vec::new(),
            notifications: Notifications::new(),
            notifications_open: false,
            notif_seq: 0,
            copy_mode: false,
            palette: PaletteViewState::default(),
            settings_section: SettingsSection::default(),
            view_model,
            terminals: Vec::new(),
            last_error: None,
            pipe_name: std::env::var("PANDAMUX_PIPE")
                .unwrap_or_else(|_| r"\\.\pipe\pandamux".to_string()),
            pipe_registry: Arc::new(StdMutex::new(HashMap::new())),
            pipe_seq: Arc::new(AtomicU64::new(1)),
        };
        if live_ptys {
            runtime.raise_notification(NewNotification {
                workspace_id: None,
                surface_id: None,
                title: "PandaMUX Everywhere is running".to_string(),
                body: "Native shell ready. Ctrl+F to find, Ctrl+B to toggle the status bar."
                    .to_string(),
                source: NotificationSource::Build,
            });
        }
        runtime.refresh_terminal_snapshots();
        runtime
    }

    pub fn view_model(&self) -> &ShellViewModel {
        &self.view_model
    }

    fn cursor_on(&self) -> bool {
        (self.tick / CURSOR_BLINK_TICKS).is_multiple_of(2)
    }

    /// Raise a notification with a generated id and wall-clock timestamp.
    pub fn raise_notification(&mut self, note: NewNotification) {
        self.notif_seq += 1;
        let id = format!("notif-{}", self.notif_seq);
        self.notifications.push(note, id, now_ms());
    }

    fn toggle_notifications(&mut self) {
        self.notifications_open = !self.notifications_open;
        if self.notifications_open {
            self.notifications.mark_all_read(None);
        }
    }

    /// Open a centered overlay, closing any other. Opening the palette resets its
    /// query and selection so it starts fresh.
    fn open_overlay(&mut self, overlay: Overlay) {
        if overlay == Overlay::CommandPalette {
            self.palette.query.clear();
            self.palette.selected = 0;
        }
        self.chrome.active_overlay = overlay;
    }

    /// Advance the current find match by `delta`, wrapping around.
    fn find_step(&mut self, delta: i64) {
        let count = self.find.match_count as i64;
        if count == 0 {
            return;
        }
        let index0 = (self.find.current as i64 - 1 + delta).rem_euclid(count);
        self.find.current = index0 as usize + 1;
    }

    /// Recompute find matches against the focused terminal's visible lines.
    /// Preserves the current index (only clamping it) so next/prev keep working.
    fn recompute_find_matches(&mut self) {
        self.find_matches.clear();
        if !self.find.open || self.find.query.is_empty() {
            self.find.match_count = 0;
            self.find.current = 0;
            self.find.current_match = None;
            return;
        }
        let lines = self.focused_terminal_lines();
        let options = SearchOptions {
            case_sensitive: self.find.case_sensitive,
            whole_word: false,
        };
        self.find_matches = search_lines(&lines, &self.find.query, options)
            .into_iter()
            .map(|hit| (hit.line, hit.start, hit.end))
            .collect();
        self.find.match_count = self.find_matches.len();
        if self.find.match_count == 0 {
            self.find.current = 0;
            self.find.current_match = None;
        } else {
            if self.find.current == 0 || self.find.current > self.find.match_count {
                self.find.current = 1;
            }
            self.find.current_match = self.find_matches.get(self.find.current - 1).copied();
        }
    }

    /// The visible lines of the focused pane's active terminal snapshot.
    fn focused_terminal_lines(&self) -> Vec<String> {
        let Some(focused) = self
            .app_state
            .active_workspace()
            .and_then(|workspace| workspace.focused_pane_id.clone())
        else {
            return Vec::new();
        };
        let projection = self
            .app_state
            .active_workspace()
            .map(project_workspace_shell);
        let Some(surface_id) = projection.and_then(|projection| {
            projection
                .visible_panes
                .into_iter()
                .find(|pane| pane.id == focused)
                .and_then(|pane| pane.active_surface_id)
        }) else {
            return Vec::new();
        };
        self.terminals
            .iter()
            .find(|snapshot| snapshot.surface_id == surface_id)
            .map(|snapshot| snapshot.lines.clone())
            .unwrap_or_default()
    }

    fn notifications_view(&self) -> NotificationsViewState {
        let now = now_ms();
        let cards = self
            .notifications
            .list()
            .iter()
            .rev()
            .map(|note| NotificationCard {
                id: note.id.clone(),
                title: note.title.clone(),
                body: note.body.clone(),
                source: note.source,
                read: note.read,
                age: relative_age(now, note.timestamp_ms),
            })
            .collect();
        NotificationsViewState {
            open: self.notifications_open,
            cards,
        }
    }

    /// Persist the auto-restore session every [`AUTOSAVE_TICKS`] ticks (~30s),
    /// but only for the live app (tests/smoke never touch disk).
    fn autosave_if_due(&mut self) {
        if self.live_ptys
            && self.tick.is_multiple_of(AUTOSAVE_TICKS)
            && let Err(error) = self.store.save_session(&self.app_state)
        {
            self.last_error = Some(format!("autosave failed: {error}"));
        }
    }

    /// Handle a chrome/window message directly (returns a window [`Task`] when
    /// the message targets the OS window), or route everything else through the
    /// core-intent path.
    pub fn update(&mut self, message: ShellMessage) -> Task<ShellMessage> {
        match message {
            ShellMessage::WindowDragStarted => window::latest().and_then(window::drag),
            ShellMessage::WindowMinimizePressed => {
                window::latest().and_then(|id| window::minimize(id, true))
            }
            ShellMessage::WindowMaximizeToggled => {
                window::latest().and_then(window::toggle_maximize)
            }
            ShellMessage::WindowClosePressed => window::latest().and_then(window::close),
            ShellMessage::PollRequested => Task::perform(crate::pollers::poll_all(), |result| {
                ShellMessage::PollUpdate {
                    git_branch: result.git_branch,
                    git_ahead: result.git_ahead,
                    ports: result.ports,
                }
            }),
            other => {
                self.update_shell(other);
                Task::none()
            }
        }
    }

    pub fn update_shell(&mut self, message: ShellMessage) {
        match message {
            ShellMessage::Tick => {
                self.tick = self.tick.wrapping_add(1);
                self.autosave_if_due();
                self.refresh_terminal_snapshots();
                return;
            }
            ShellMessage::Noop
            | ShellMessage::WindowDragStarted
            | ShellMessage::WindowMinimizePressed
            | ShellMessage::WindowMaximizeToggled
            | ShellMessage::WindowClosePressed => {
                // Window actions are handled by `update`; nothing to do here.
                return;
            }
            ShellMessage::RailSelected(item) => {
                self.chrome.active_rail = item;
                match item {
                    RailItem::Sessions => {
                        self.chrome.session_panel_open = !self.chrome.session_panel_open;
                    }
                    RailItem::NewSession => self.open_overlay(Overlay::QuickLaunch),
                    _ => {}
                }
            }
            ShellMessage::SessionSelected {
                workspace_id,
                surface_id,
            } => {
                // Focus/activate the shell context: select its workspace, then
                // focus its surface. Never swaps the layout (plan 12.1 #2).
                let result = self
                    .app_state
                    .apply(AppIntent::Workspace(WorkspaceIntent::Select {
                        workspace_id: workspace_id.clone(),
                    }))
                    .and_then(|_| {
                        self.app_state
                            .apply(AppIntent::Surface(SurfaceIntent::Focus {
                                workspace_id: Some(workspace_id),
                                surface_id,
                            }))
                    });
                self.last_error = result.err();
            }
            ShellMessage::SessionGroupingChanged(grouping) => {
                self.chrome.session_grouping = grouping;
            }
            ShellMessage::NewSessionRequested => self.open_overlay(Overlay::QuickLaunch),
            ShellMessage::OverlayDismissed => {
                self.chrome.active_overlay = Overlay::None;
            }
            ShellMessage::PaletteQueryChanged(query) => {
                self.palette.query = query;
                self.palette.selected = 0;
            }
            ShellMessage::PaletteMoveSelection(delta) => {
                let count = self.palette.items.len();
                if count > 0 {
                    let index = (self.palette.selected as i64 + delta as i64)
                        .rem_euclid(count as i64) as usize;
                    self.palette.selected = index;
                }
            }
            ShellMessage::PaletteActivate => {
                if let Some(item) = self.palette.items.get(self.palette.selected).cloned() {
                    self.chrome.active_overlay = Overlay::None;
                    self.update_shell(item.action);
                    return;
                }
            }
            ShellMessage::LaunchProfile { shell, title } => {
                self.chrome.active_overlay = Overlay::None;
                self.launch_session(Some(shell), Some(title));
            }
            ShellMessage::SettingsSectionSelected(section) => {
                self.settings_section = section;
            }
            ShellMessage::AccentSelected(accent) => {
                self.chrome.accent = accent;
            }
            ShellMessage::PollRequested => {
                // Kicked off from `update` as a Task; nothing to do here.
                return;
            }
            ShellMessage::PollUpdate {
                git_branch,
                git_ahead,
                ports,
            } => {
                self.chrome.git_branch = git_branch;
                self.chrome.git_ahead = git_ahead;
                self.chrome.ports = ports;
            }
            ShellMessage::OverlayRequested(item) => {
                self.chrome.active_rail = item;
                match item {
                    RailItem::CommandPalette => self.open_overlay(Overlay::CommandPalette),
                    RailItem::Settings => self.open_overlay(Overlay::Settings),
                    RailItem::Notifications => self.toggle_notifications(),
                    _ => {}
                }
                self.refresh_terminal_snapshots();
                return;
            }
            ShellMessage::ToggleStatusBar => {
                self.chrome.show_status_bar = !self.chrome.show_status_bar;
            }
            ShellMessage::ToggleTheme => {
                self.chrome.ui_theme = self.chrome.ui_theme.toggled();
            }
            ShellMessage::CycleAccent => {
                self.chrome.accent = self.chrome.accent.next();
            }
            ShellMessage::FindOpened => {
                self.find.open = true;
                if self.find.current == 0 {
                    self.find.current = 1;
                }
            }
            ShellMessage::FindClosed => {
                self.find.open = false;
                self.find.current_match = None;
            }
            ShellMessage::FindQueryChanged(query) => {
                self.find.query = query;
                self.find.current = 1;
            }
            ShellMessage::FindNext => self.find_step(1),
            ShellMessage::FindPrev => self.find_step(-1),
            ShellMessage::FindCaseToggled => {
                self.find.case_sensitive = !self.find.case_sensitive;
            }
            ShellMessage::CopyModeToggled => {
                self.copy_mode = !self.copy_mode;
            }
            ShellMessage::NotificationsToggled => self.toggle_notifications(),
            ShellMessage::NotificationCleared(id) => {
                self.notifications.clear(&id);
            }
            ShellMessage::NotificationsClearedAll => {
                self.notifications.clear_all();
            }
            ShellMessage::PipeRequest { id, payload } => {
                // Single-writer path: CLI / agent / orchestrator lines apply to
                // the same canonical state the UI mutates, via the same
                // dispatcher the standalone server uses. Then the UI repaints
                // (falls through to refresh below), so a CLI-driven split or
                // `notify` shows up live.
                let reply = crate::backend::handle_line(
                    &payload,
                    &mut self.app_state,
                    &mut self.ptys,
                    &mut self.notifications,
                    &mut self.notif_seq,
                    now_ms(),
                    self.live_ptys,
                );
                if let Ok(mut registry) = self.pipe_registry.lock()
                    && let Some(tx) = registry.remove(&id)
                {
                    let _ = tx.send(reply);
                }
            }
            core_message => {
                let result = self.apply_core_message(core_message);
                self.last_error = result.err();
            }
        }
        self.refresh_terminal_snapshots();
    }

    fn apply_core_message(&mut self, message: ShellMessage) -> Result<(), String> {
        let intent = match message {
            ShellMessage::PaneFocused(pane_id) => AppIntent::Pane(PaneIntent::Focus {
                workspace_id: None,
                pane_id,
            }),
            ShellMessage::PaneSplit { pane_id, direction } => {
                AppIntent::Pane(PaneIntent::Split(SplitPaneParams {
                    workspace_id: None,
                    target_pane_id: Some(pane_id),
                    target_surface_id: None,
                    direction,
                    surface_type: SurfaceType::Terminal,
                }))
            }
            ShellMessage::PaneClosed(pane_id) => AppIntent::Pane(PaneIntent::Close {
                workspace_id: None,
                pane_id,
            }),
            ShellMessage::PaneZoomToggled(pane_id) => AppIntent::Pane(PaneIntent::Zoom {
                workspace_id: None,
                pane_id: Some(pane_id),
            }),
            ShellMessage::TerminalSurfaceCreated(pane_id) => {
                AppIntent::Surface(SurfaceIntent::Create {
                    workspace_id: None,
                    pane_id: Some(pane_id),
                    surface_type: SurfaceType::Terminal,
                })
            }
            ShellMessage::SurfaceFocused(surface_id) => AppIntent::Surface(SurfaceIntent::Focus {
                workspace_id: None,
                surface_id,
            }),
            ShellMessage::SurfaceClosed(surface_id) => AppIntent::Surface(SurfaceIntent::Close {
                workspace_id: None,
                surface_id,
            }),
            unexpected => {
                return Err(format!(
                    "non-core shell message routed to core: {unexpected:?}"
                ));
            }
        };
        self.app_state.apply(intent).map(|_| ())
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    fn refresh_terminal_snapshots(&mut self) {
        if let Err(error) = self.sync_terminal_sessions() {
            self.last_error = Some(error);
        }
        self.terminals = terminal_snapshots(&self.app_state, &mut self.ptys, self.live_ptys)
            .unwrap_or_else(|error| {
                self.last_error = Some(error);
                fallback_terminal_snapshots(&self.app_state)
            });
        self.recompute_find_matches();
        self.rebuild_chrome();
        let active_surface_id = self.active_surface_id();
        let sessions = project_sessions(
            &self.app_state,
            self.chrome.session_grouping,
            self.chrome.session_panel_open,
            active_surface_id.as_ref(),
        );
        // Rebuild the palette item list, then filter it by the live query and
        // clamp the selection.
        let all_items = self.build_palette_items(&sessions);
        self.palette.items = filter_items(&all_items, &self.palette.query);
        if self.palette.items.is_empty() {
            self.palette.selected = 0;
        } else if self.palette.selected >= self.palette.items.len() {
            self.palette.selected = self.palette.items.len() - 1;
        }
        let settings = SettingsViewState {
            section: self.settings_section,
            ui_theme: self.chrome.ui_theme,
            accent: self.chrome.accent,
            show_status_bar: self.chrome.show_status_bar,
            ..SettingsViewState::default()
        };
        self.view_model = ShellViewModel {
            projection: self
                .app_state
                .active_workspace()
                .map(project_workspace_shell)
                .expect("default app state always has an active workspace"),
            terminals: self.terminals.clone(),
            chrome: self.chrome.clone(),
            cursor_on: self.cursor_on(),
            find: self.find.clone(),
            notifications: self.notifications_view(),
            copy_mode: self.copy_mode,
            sessions,
            palette: self.palette.clone(),
            quick_launch: QuickLaunchViewState::default(),
            settings,
        };
    }

    /// Build the full command-palette item list (commands + pane actions +
    /// session switches). The runtime then filters it against the live query.
    fn build_palette_items(&self, sessions: &SessionsViewState) -> Vec<PaletteItem> {
        let mut items = vec![
            PaletteItem::new(
                "+",
                "New session",
                Some("Ctrl T"),
                ShellMessage::NewSessionRequested,
            ),
            PaletteItem::new(
                "\u{1f50d}",
                "Find in terminal",
                Some("Ctrl F"),
                ShellMessage::FindOpened,
            ),
            PaletteItem::new(
                "\u{2699}",
                "Open settings",
                Some("Ctrl ,"),
                ShellMessage::OverlayRequested(RailItem::Settings),
            ),
            PaletteItem::new(
                "\u{1f514}",
                "Toggle notifications",
                Some("Ctrl N"),
                ShellMessage::NotificationsToggled,
            ),
            PaletteItem::new(
                "\u{2637}",
                "Toggle status bar",
                Some("Ctrl B"),
                ShellMessage::ToggleStatusBar,
            ),
            PaletteItem::new(
                "\u{25d1}",
                "Toggle theme",
                Some("Ctrl Shift T"),
                ShellMessage::ToggleTheme,
            ),
            PaletteItem::new(
                "\u{25c9}",
                "Cycle accent",
                Some("Ctrl Shift A"),
                ShellMessage::CycleAccent,
            ),
        ];

        if let Some(pane_id) = self
            .app_state
            .active_workspace()
            .and_then(|workspace| workspace.focused_pane_id.clone())
        {
            items.push(PaletteItem::new(
                "\u{25eb}",
                "Split pane right",
                Some("Ctrl D"),
                ShellMessage::PaneSplit {
                    pane_id: pane_id.clone(),
                    direction: SplitDirection::Horizontal,
                },
            ));
            items.push(PaletteItem::new(
                "\u{2b12}",
                "Split pane down",
                Some("Ctrl Shift D"),
                ShellMessage::PaneSplit {
                    pane_id: pane_id.clone(),
                    direction: SplitDirection::Vertical,
                },
            ));
            items.push(PaletteItem::new(
                "\u{2922}",
                "Zoom pane",
                Some("Ctrl Enter"),
                ShellMessage::PaneZoomToggled(pane_id.clone()),
            ));
            items.push(PaletteItem::new(
                "\u{00d7}",
                "Close pane",
                Some("Ctrl W"),
                ShellMessage::PaneClosed(pane_id),
            ));
        }

        for group in &sessions.groups {
            for entry in &group.entries {
                if entry.is_active {
                    continue;
                }
                items.push(PaletteItem::new(
                    entry.kind.abbreviation(),
                    format!("Switch to {}", entry.name),
                    None,
                    ShellMessage::SessionSelected {
                        workspace_id: entry.workspace_id.clone(),
                        surface_id: entry.surface_id.clone(),
                    },
                ));
            }
        }

        items
    }

    /// Create a new session (workspace + terminal) with the given shell and make
    /// it active.
    fn launch_session(&mut self, shell: Option<String>, title: Option<String>) {
        let result = self
            .app_state
            .apply(AppIntent::Workspace(WorkspaceIntent::Create {
                title,
                shell,
            }));
        self.last_error = result.err();
    }

    /// The focused pane's active surface in the active workspace (the "active
    /// session"), if any.
    fn active_surface_id(&self) -> Option<SurfaceId> {
        let workspace = self.app_state.active_workspace()?;
        let focused = workspace.focused_pane_id.clone()?;
        project_workspace_shell(workspace)
            .visible_panes
            .into_iter()
            .find(|pane| pane.id == focused)
            .and_then(|pane| pane.active_surface_id)
    }

    /// Refresh the chrome view state derived from canonical state (session/pane
    /// counts, active shell/session, activity, unread badge). Pollers (git/ports)
    /// fill the rest later.
    fn rebuild_chrome(&mut self) {
        self.chrome.session_count = self.app_state.workspaces.len();
        self.chrome.version = env!("CARGO_PKG_VERSION").to_string();
        self.chrome.unread_notifications = self.notifications.unread_count(None) > 0;
        // Running when live shells are attached; busy-agent detection lands with
        // the Phase 5 agent observer.
        self.chrome.activity = if self.live_ptys {
            SessionActivity::Running
        } else {
            SessionActivity::Idle
        };
        if let Some(workspace) = self.app_state.active_workspace() {
            self.chrome.pane_count = get_all_pane_ids(&workspace.split_tree).len();
            self.chrome.shell_label = workspace.shell.clone();
            self.chrome.shell_kind = ShellKind::classify(&workspace.shell);
            self.chrome.active_session_name = workspace.title.clone();
        }
    }

    fn sync_terminal_sessions(&mut self) -> Result<(), String> {
        if !self.live_ptys {
            return Ok(());
        }

        let mut expected_session_ids = HashSet::new();
        for workspace in &self.app_state.workspaces {
            for surface_id in terminal_surface_ids(&workspace.split_tree) {
                let session_id = surface_id.to_string();
                expected_session_ids.insert(session_id.clone());
                if self.ptys.has(&session_id) {
                    continue;
                }
                self.ptys
                    .spawn(
                        session_id,
                        &PtyCommand::new(workspace.shell.clone()),
                        GridSize::new(120, 30),
                    )
                    .map_err(|error| error.to_string())?;
            }
        }

        for session_id in self.ptys.session_ids() {
            if !expected_session_ids.contains(&session_id) {
                self.ptys
                    .kill(&session_id)
                    .map_err(|error| error.to_string())?;
            }
        }

        Ok(())
    }
}

pub fn run_iced_shell() -> Result<(), Box<dyn std::error::Error>> {
    application(
        || NativeShellRuntime::new(true),
        update_iced_shell,
        view_iced_shell,
    )
    .title("PandaMUX Everywhere")
    .window(window::Settings {
        size: Size::new(1280.0, 800.0),
        min_size: Some(Size::new(760.0, 480.0)),
        decorations: false,
        transparent: true,
        ..window::Settings::default()
    })
    .subscription(subscription_iced_shell)
    .theme(theme_iced_shell)
    .run()
    .map_err(|error| error.into())
}

pub fn run_iced_shell_smoke() -> Result<(), String> {
    let runtime = NativeShellRuntime::default();
    if let Some(error) = runtime.last_error() {
        return Err(format!(
            "native shell smoke captured runtime error: {error}"
        ));
    }
    let model = runtime.view_model();
    if model.projection.visible_panes.is_empty() {
        return Err("native shell smoke found no visible panes".to_string());
    }
    if model.terminals.is_empty() {
        return Err("native shell smoke found no terminal snapshots".to_string());
    }
    let _app = app_view(model);
    let _workspace = shell_view(model);
    println!("PANDAMUX_ICED_SHELL_SMOKE_OK");
    Ok(())
}

fn update_iced_shell(state: &mut NativeShellRuntime, message: ShellMessage) -> Task<ShellMessage> {
    state.update(message)
}

fn view_iced_shell(state: &NativeShellRuntime) -> Element<'_, ShellMessage> {
    app_view(state.view_model())
}

fn subscription_iced_shell(state: &NativeShellRuntime) -> Subscription<ShellMessage> {
    let mut subscriptions = vec![
        time::every(Duration::from_millis(100)).map(|_| ShellMessage::Tick),
        keyboard::listen().map(map_key_event),
    ];
    // Only the live app embeds the pipe server (headless smoke/tests never bind
    // a pipe). This is what unifies the CLI/agents/orchestrator with the running
    // UI onto the single-writer path.
    if state.live_ptys {
        subscriptions.push(Subscription::run_with(
            PipeServerConfig {
                pipe_name: state.pipe_name.clone(),
                registry: state.pipe_registry.clone(),
                seq: state.pipe_seq.clone(),
            },
            pipe_subscription,
        ));
        // Poll git/ports for the status bar every few seconds.
        subscriptions
            .push(time::every(Duration::from_secs(5)).map(|_| ShellMessage::PollRequested));
    }
    Subscription::batch(subscriptions)
}

/// Identity + shared handles for the embedded pipe-server subscription. Only
/// `pipe_name` participates in the subscription's identity hash; the `Arc`
/// handles are captured by the running stream and must not change it (so the
/// server is spawned exactly once and survives view rebuilds).
struct PipeServerConfig {
    pipe_name: String,
    registry: PipeRegistry,
    seq: Arc<AtomicU64>,
}

impl Hash for PipeServerConfig {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        "pandamux-embedded-pipe".hash(hasher);
        self.pipe_name.hash(hasher);
    }
}

// A concrete (boxed) return type is required so this coerces to the `fn(&D) -> S`
// pointer `Subscription::run_with` wants; an `impl Stream` return is treated as
// borrowing `&config` and fails the coercion.
type PipeStream = std::pin::Pin<Box<dyn iced::futures::Stream<Item = ShellMessage> + Send>>;

fn pipe_subscription(config: &PipeServerConfig) -> PipeStream {
    let pipe_name = config.pipe_name.clone();
    let registry = config.registry.clone();
    let seq = config.seq.clone();
    Box::pin(stream::channel(64, move |output| async move {
        run_embedded_pipe_server(pipe_name, registry, seq, output).await;
    }))
}

/// The embedded named-pipe accept loop. Runs for the life of the app, spawning a
/// task per connection that forwards the line into the Iced message loop and
/// writes back the reply the runtime produces.
async fn run_embedded_pipe_server(
    pipe_name: String,
    registry: PipeRegistry,
    seq: Arc<AtomicU64>,
    output: iced::futures::channel::mpsc::Sender<ShellMessage>,
) {
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ServerOptions;
        loop {
            let server = match ServerOptions::new()
                .first_pipe_instance(false)
                .create(&pipe_name)
            {
                Ok(server) => server,
                Err(error) => {
                    eprintln!("embedded pipe server bind error: {error}");
                    return;
                }
            };
            if server.connect().await.is_err() {
                continue;
            }
            let registry = registry.clone();
            let seq = seq.clone();
            let output = output.clone();
            tokio::spawn(async move {
                if let Err(error) = handle_embedded_connection(server, registry, seq, output).await
                {
                    eprintln!("embedded pipe connection error: {error}");
                }
            });
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (pipe_name, registry, seq, output);
    }
}

async fn handle_embedded_connection<T>(
    stream: T,
    registry: PipeRegistry,
    seq: Arc<AtomicU64>,
    mut output: iced::futures::channel::mpsc::Sender<ShellMessage>,
) -> std::io::Result<()>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let id = seq.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    if let Ok(mut registry) = registry.lock() {
        registry.insert(id, tx);
    }

    let request = ShellMessage::PipeRequest {
        id,
        payload: line.trim().to_string(),
    };
    if output.send(request).await.is_err() {
        if let Ok(mut registry) = registry.lock() {
            registry.remove(&id);
        }
        return Ok(());
    }

    let reply = rx.await.unwrap_or_default();
    let mut stream = reader.into_inner();
    stream.write_all(reply.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.shutdown().await
}

fn map_key_event(event: keyboard::Event) -> ShellMessage {
    use keyboard::key::Named;
    use keyboard::{Event, Key};
    if let Event::KeyPressed { key, modifiers, .. } = event {
        match key.as_ref() {
            Key::Character(character) => {
                return shortcut_for(
                    modifiers.control(),
                    modifiers.shift(),
                    &character.to_ascii_lowercase(),
                );
            }
            // Escape dismisses any open centered overlay (no-op otherwise).
            Key::Named(Named::Escape) => return ShellMessage::OverlayDismissed,
            _ => {}
        }
    }
    ShellMessage::Noop
}

/// Pure shortcut table (Ctrl-based). Kept separate from event decoding so it is
/// unit-testable without constructing a full keyboard event.
fn shortcut_for(ctrl: bool, shift: bool, character: &str) -> ShellMessage {
    if !ctrl {
        return ShellMessage::Noop;
    }
    match (shift, character) {
        (false, "b") => ShellMessage::ToggleStatusBar,
        (true, "t") => ShellMessage::ToggleTheme,
        (true, "a") => ShellMessage::CycleAccent,
        (false, "k") => ShellMessage::OverlayRequested(RailItem::CommandPalette),
        (false, "t") => ShellMessage::NewSessionRequested,
        (false, ",") => ShellMessage::OverlayRequested(RailItem::Settings),
        (false, "f") => ShellMessage::FindOpened,
        (false, "n") => ShellMessage::NotificationsToggled,
        _ => ShellMessage::Noop,
    }
}

fn theme_iced_shell(state: &NativeShellRuntime) -> Theme {
    match state.chrome.ui_theme {
        UiTheme::Dark => Theme::Dark,
        UiTheme::Light => Theme::Light,
    }
}

fn terminal_snapshots(
    app_state: &AppState,
    ptys: &mut PtySessionManager,
    live_ptys: bool,
) -> Result<Vec<TerminalSnapshot>, String> {
    let Some(workspace) = app_state.active_workspace() else {
        return Ok(Vec::new());
    };
    let snapshots = project_workspace_shell(workspace)
        .visible_panes
        .into_iter()
        .filter_map(|pane| {
            let surface_id = pane.active_surface_id?;
            let is_terminal = pane.surfaces.iter().any(|surface| {
                surface.id == surface_id && surface.surface_type == SurfaceType::Terminal
            });
            if !is_terminal {
                return None;
            }
            let lines = if live_ptys {
                let text = ptys
                    .screen_text_lines(surface_id.as_str(), 30)
                    .map_err(|error| error.to_string())
                    .ok()?;
                non_empty_lines(text)
            } else {
                fallback_lines()
            };
            let links = detect_links(&lines)
                .into_iter()
                .map(|link| LinkSpan {
                    line: link.line,
                    start: link.start,
                    end: link.end,
                })
                .collect();
            Some(TerminalSnapshot {
                surface_id,
                lines,
                columns: 120,
                rows: 30,
                links,
            })
        })
        .collect();
    Ok(snapshots)
}

/// A minimal view model for construction time; overwritten by the first
/// `refresh_terminal_snapshots` call.
fn initial_view_model(app_state: &AppState, chrome: &ChromeState) -> ShellViewModel {
    let workspace = app_state
        .active_workspace()
        .expect("default app state should always have an active workspace");
    ShellViewModel {
        projection: project_workspace_shell(workspace),
        terminals: Vec::new(),
        chrome: chrome.clone(),
        cursor_on: true,
        find: FindViewState::default(),
        notifications: NotificationsViewState::default(),
        copy_mode: false,
        sessions: SessionsViewState::default(),
        palette: PaletteViewState::default(),
        quick_launch: QuickLaunchViewState::default(),
        settings: SettingsViewState::default(),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Human relative age like "just now", "3m ago", "2h ago".
fn relative_age(now_ms: u64, then_ms: u64) -> String {
    let secs = now_ms.saturating_sub(then_ms) / 1000;
    if secs < 5 {
        "just now".to_string()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

fn fallback_terminal_snapshots(app_state: &AppState) -> Vec<TerminalSnapshot> {
    terminal_snapshots(app_state, &mut PtySessionManager::new(), false).unwrap_or_default()
}

fn fallback_lines() -> Vec<String> {
    vec![
        "PandaMUX Everywhere".to_string(),
        "Native shell runtime is active.".to_string(),
    ]
}

fn non_empty_lines(text: String) -> Vec<String> {
    let lines = text.lines().map(ToString::to_string).collect::<Vec<_>>();
    if lines.iter().any(|line| !line.trim().is_empty()) {
        lines
    } else {
        vec![String::new()]
    }
}

fn terminal_surface_ids(tree: &SplitNode) -> Vec<SurfaceId> {
    match tree {
        SplitNode::Leaf(leaf) => leaf
            .surfaces
            .iter()
            .filter(|surface| surface.surface_type == SurfaceType::Terminal)
            .map(|surface| surface.id.clone())
            .collect(),
        SplitNode::Branch(branch) => {
            let mut ids = terminal_surface_ids(&branch.children[0]);
            ids.extend(terminal_surface_ids(&branch.children[1]));
            ids
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{PaneId, SplitDirection, SplitPaneParams, SurfaceType};
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn builds_view_model_for_default_state() {
        let runtime = NativeShellRuntime::default();
        let model = runtime.view_model();

        assert_eq!(model.projection.visible_panes.len(), 1);
        assert_eq!(model.terminals.len(), 1);
        assert_eq!(model.terminals[0].lines, fallback_lines());
        assert_eq!(model.chrome.session_count, 1);
        assert_eq!(model.chrome.pane_count, 1);
        let _view = app_view(model);
    }

    #[test]
    fn routes_shell_messages_to_core_state() {
        let mut runtime = NativeShellRuntime::default();
        runtime
            .app_state
            .apply(AppIntent::Pane(PaneIntent::Split(SplitPaneParams {
                workspace_id: None,
                target_pane_id: Some(PaneId::from("pane-default")),
                target_surface_id: None,
                direction: SplitDirection::Horizontal,
                surface_type: SurfaceType::Terminal,
            })))
            .expect("split should apply");
        let pane_id = runtime
            .view_model()
            .projection
            .visible_panes
            .last()
            .expect("split pane")
            .id
            .clone();

        runtime.update_shell(ShellMessage::PaneZoomToggled(pane_id.clone()));

        assert_eq!(
            runtime
                .app_state
                .active_workspace()
                .unwrap()
                .zoomed_pane_id
                .as_ref(),
            Some(&pane_id)
        );
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    fn routes_shell_split_and_tab_messages_to_core_state() {
        let mut runtime = NativeShellRuntime::default();
        let pane_id = runtime
            .view_model()
            .projection
            .visible_panes
            .first()
            .expect("default pane")
            .id
            .clone();

        runtime.update_shell(ShellMessage::TerminalSurfaceCreated(pane_id.clone()));

        let first_pane = runtime
            .view_model()
            .projection
            .visible_panes
            .first()
            .expect("default pane");
        assert_eq!(first_pane.surfaces.len(), 2);
        assert_eq!(runtime.last_error(), None);

        runtime.update_shell(ShellMessage::PaneSplit {
            pane_id,
            direction: SplitDirection::Horizontal,
        });

        assert_eq!(runtime.view_model().projection.visible_panes.len(), 2);
        assert_eq!(runtime.view_model().chrome.pane_count, 2);
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    fn routes_shell_close_messages_to_core_state() {
        let mut runtime = NativeShellRuntime::default();
        let pane_id = runtime
            .view_model()
            .projection
            .visible_panes
            .first()
            .expect("default pane")
            .id
            .clone();
        runtime.update_shell(ShellMessage::TerminalSurfaceCreated(pane_id.clone()));
        let surface_id = runtime
            .view_model()
            .projection
            .visible_panes
            .first()
            .and_then(|pane| pane.surfaces.last())
            .expect("second surface")
            .id
            .clone();

        runtime.update_shell(ShellMessage::SurfaceClosed(surface_id));

        assert_eq!(
            runtime
                .view_model()
                .projection
                .visible_panes
                .first()
                .expect("default pane")
                .surfaces
                .len(),
            1
        );
        assert_eq!(runtime.last_error(), None);

        runtime.update_shell(ShellMessage::PaneSplit {
            pane_id,
            direction: SplitDirection::Horizontal,
        });
        let pane_to_close = runtime
            .view_model()
            .projection
            .visible_panes
            .last()
            .expect("split pane")
            .id
            .clone();

        runtime.update_shell(ShellMessage::PaneClosed(pane_to_close));

        assert_eq!(runtime.view_model().projection.visible_panes.len(), 1);
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    fn toggles_chrome_view_state() {
        let mut runtime = NativeShellRuntime::default();
        assert!(runtime.view_model().chrome.show_status_bar);
        runtime.update_shell(ShellMessage::ToggleStatusBar);
        assert!(!runtime.view_model().chrome.show_status_bar);

        assert_eq!(runtime.view_model().chrome.ui_theme, UiTheme::Dark);
        runtime.update_shell(ShellMessage::ToggleTheme);
        assert_eq!(runtime.view_model().chrome.ui_theme, UiTheme::Light);

        runtime.update_shell(ShellMessage::RailSelected(RailItem::NewSession));
        assert_eq!(
            runtime.view_model().chrome.active_rail,
            RailItem::NewSession
        );
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    fn find_matches_over_focused_terminal_and_steps() {
        let mut runtime = NativeShellRuntime::default();
        runtime.update_shell(ShellMessage::FindOpened);
        // Fallback lines contain "shell" once ("Native shell runtime is active.").
        runtime.update_shell(ShellMessage::FindQueryChanged("shell".to_string()));
        let find = &runtime.view_model().find;
        assert!(find.open);
        assert_eq!(find.match_count, 1);
        assert_eq!(find.current, 1);
        assert!(find.current_match.is_some());

        // Stepping wraps within a single match.
        runtime.update_shell(ShellMessage::FindNext);
        assert_eq!(runtime.view_model().find.current, 1);

        // No matches clears the highlight.
        runtime.update_shell(ShellMessage::FindQueryChanged("zzz".to_string()));
        assert_eq!(runtime.view_model().find.match_count, 0);
        assert!(runtime.view_model().find.current_match.is_none());

        runtime.update_shell(ShellMessage::FindClosed);
        assert!(!runtime.view_model().find.open);
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    fn notifications_raise_toggle_and_clear() {
        let mut runtime = NativeShellRuntime::default();
        // Non-live runtime starts with no notifications.
        assert!(runtime.view_model().notifications.cards.is_empty());
        assert!(!runtime.view_model().chrome.unread_notifications);

        runtime.raise_notification(NewNotification::generic("Build done", "ok"));
        runtime.update_shell(ShellMessage::Tick);
        assert_eq!(runtime.view_model().notifications.cards.len(), 1);
        assert!(runtime.view_model().chrome.unread_notifications);

        // Opening the panel marks all read and clears the badge.
        runtime.update_shell(ShellMessage::NotificationsToggled);
        assert!(runtime.view_model().notifications.open);
        assert!(!runtime.view_model().chrome.unread_notifications);

        let id = runtime.view_model().notifications.cards[0].id.clone();
        runtime.update_shell(ShellMessage::NotificationCleared(id));
        assert!(runtime.view_model().notifications.cards.is_empty());
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    fn copy_mode_toggles() {
        let mut runtime = NativeShellRuntime::default();
        assert!(!runtime.view_model().copy_mode);
        runtime.update_shell(ShellMessage::CopyModeToggled);
        assert!(runtime.view_model().copy_mode);
    }

    #[test]
    fn pipe_request_routes_through_single_writer_and_replies() {
        // A line that would arrive on the named pipe (as a CLI `notify`) applies
        // to the same canonical state the UI owns, shows up in the live view
        // model, and the reply is delivered to the waiting connection.
        let mut runtime = NativeShellRuntime::default();
        let (tx, rx) = oneshot::channel();
        let id = 42;
        runtime.pipe_registry.lock().unwrap().insert(id, tx);

        runtime.update_shell(ShellMessage::PipeRequest {
            id,
            payload: r#"{"method":"notification.raise","params":{"title":"CLI ping","source":"agent"},"id":1}"#
                .to_string(),
        });

        assert_eq!(runtime.view_model().notifications.cards.len(), 1);
        assert_eq!(
            runtime.view_model().notifications.cards[0].title,
            "CLI ping"
        );

        let reply = rx.blocking_recv().expect("reply should be delivered");
        let parsed: serde_json::Value = serde_json::from_str(&reply).expect("valid json");
        assert_eq!(parsed["result"]["ok"], true);
    }

    #[test]
    fn session_panel_projects_selects_and_toggles() {
        let mut runtime = NativeShellRuntime::default();
        let original_ws = runtime.app_state.active_workspace_id.clone();
        assert_eq!(runtime.view_model().sessions.total, 1);

        // Launching a quick-launch profile creates a workspace and switches to it.
        runtime.update_shell(ShellMessage::LaunchProfile {
            shell: "pwsh".to_string(),
            title: "PowerShell 7".to_string(),
        });
        assert_ne!(runtime.app_state.active_workspace_id, original_ws);
        assert_eq!(runtime.view_model().sessions.total, 2);

        // Selecting the original session activates its workspace (no layout swap).
        let entry = runtime
            .view_model()
            .sessions
            .groups
            .iter()
            .flat_map(|group| &group.entries)
            .find(|entry| entry.workspace_id == original_ws)
            .expect("original session")
            .clone();
        runtime.update_shell(ShellMessage::SessionSelected {
            workspace_id: entry.workspace_id.clone(),
            surface_id: entry.surface_id.clone(),
        });
        assert_eq!(runtime.app_state.active_workspace_id, original_ws);
        assert_eq!(runtime.last_error(), None);

        // Grouping switch is live.
        runtime.update_shell(ShellMessage::SessionGroupingChanged(
            pandamux_ui::SessionGrouping::Type,
        ));
        assert_eq!(
            runtime.view_model().sessions.grouping,
            pandamux_ui::SessionGrouping::Type
        );

        // The Sessions rail toggles the panel.
        assert!(runtime.view_model().chrome.session_panel_open);
        runtime.update_shell(ShellMessage::RailSelected(RailItem::Sessions));
        assert!(!runtime.view_model().chrome.session_panel_open);
    }

    #[test]
    fn command_palette_opens_filters_and_activates() {
        let mut runtime = NativeShellRuntime::default();
        assert_eq!(runtime.view_model().chrome.active_overlay, Overlay::None);

        // Ctrl+K opens the palette with the full command list.
        runtime.update_shell(ShellMessage::OverlayRequested(RailItem::CommandPalette));
        assert_eq!(
            runtime.view_model().chrome.active_overlay,
            Overlay::CommandPalette
        );
        assert!(runtime.view_model().palette.items.len() > 3);

        // Filtering narrows to matching commands.
        runtime.update_shell(ShellMessage::PaletteQueryChanged("theme".to_string()));
        let items = &runtime.view_model().palette.items;
        assert!(
            items
                .iter()
                .all(|item| item.label.to_lowercase().contains("theme"))
        );

        // Activating the selected item runs its action and closes the palette.
        assert_eq!(runtime.view_model().chrome.ui_theme, UiTheme::Dark);
        runtime.update_shell(ShellMessage::PaletteActivate);
        assert_eq!(runtime.view_model().chrome.active_overlay, Overlay::None);
        assert_eq!(runtime.view_model().chrome.ui_theme, UiTheme::Light);
    }

    #[test]
    fn quick_launch_and_settings_overlays_flow() {
        let mut runtime = NativeShellRuntime::default();

        // New-session opens quick-launch; picking a profile creates a session.
        runtime.update_shell(ShellMessage::NewSessionRequested);
        assert_eq!(
            runtime.view_model().chrome.active_overlay,
            Overlay::QuickLaunch
        );
        runtime.update_shell(ShellMessage::LaunchProfile {
            shell: "wsl.exe".to_string(),
            title: "WSL".to_string(),
        });
        assert_eq!(runtime.view_model().chrome.active_overlay, Overlay::None);
        assert_eq!(runtime.view_model().sessions.total, 2);

        // Settings overlay: accent selection and Escape dismiss.
        runtime.update_shell(ShellMessage::OverlayRequested(RailItem::Settings));
        assert_eq!(
            runtime.view_model().chrome.active_overlay,
            Overlay::Settings
        );
        runtime.update_shell(ShellMessage::AccentSelected(
            pandamux_ui::theme::Accent::Blue,
        ));
        assert_eq!(
            runtime.view_model().settings.accent,
            pandamux_ui::theme::Accent::Blue
        );
        runtime.update_shell(ShellMessage::OverlayDismissed);
        assert_eq!(runtime.view_model().chrome.active_overlay, Overlay::None);
    }

    #[test]
    fn poll_update_populates_status_bar_chrome() {
        let mut runtime = NativeShellRuntime::default();
        assert!(runtime.view_model().chrome.git_branch.is_none());
        assert!(runtime.view_model().chrome.ports.is_empty());

        runtime.update_shell(ShellMessage::PollUpdate {
            git_branch: Some("master".to_string()),
            git_ahead: 2,
            ports: vec![5173, 8080],
        });

        assert_eq!(
            runtime.view_model().chrome.git_branch.as_deref(),
            Some("master")
        );
        assert_eq!(runtime.view_model().chrome.git_ahead, 2);
        assert_eq!(runtime.view_model().chrome.ports, vec![5173, 8080]);

        // A later refresh (e.g. a tick) keeps the polled values.
        runtime.update_shell(ShellMessage::Tick);
        assert_eq!(
            runtime.view_model().chrome.git_branch.as_deref(),
            Some("master")
        );
    }

    #[test]
    fn pipe_request_split_shows_in_live_projection() {
        let mut runtime = NativeShellRuntime::default();
        let (tx, rx) = oneshot::channel();
        runtime.pipe_registry.lock().unwrap().insert(7, tx);

        runtime.update_shell(ShellMessage::PipeRequest {
            id: 7,
            payload: r#"{"method":"pane.split","params":{"paneId":"pane-default","direction":"right"},"id":1}"#
                .to_string(),
        });

        assert_eq!(runtime.view_model().projection.visible_panes.len(), 2);
        let reply = rx.blocking_recv().expect("reply should be delivered");
        assert!(reply.contains("paneId"));
    }

    #[test]
    fn maps_known_keyboard_shortcuts() {
        assert_eq!(
            shortcut_for(true, false, "b"),
            ShellMessage::ToggleStatusBar
        );
        assert_eq!(shortcut_for(true, true, "t"), ShellMessage::ToggleTheme);
        assert_eq!(shortcut_for(true, true, "a"), ShellMessage::CycleAccent);
        assert_eq!(
            shortcut_for(true, false, "k"),
            ShellMessage::OverlayRequested(RailItem::CommandPalette)
        );
        assert_eq!(shortcut_for(false, false, "b"), ShellMessage::Noop);
        assert_eq!(shortcut_for(true, false, "z"), ShellMessage::Noop);
    }

    #[test]
    fn tick_refreshes_terminal_snapshots_without_mutating_core_state() {
        let mut runtime = NativeShellRuntime::default();
        let active_workspace = runtime.app_state.active_workspace_id.clone();
        let active_pane = runtime
            .app_state
            .active_workspace()
            .and_then(|workspace| workspace.focused_pane_id.clone())
            .expect("focused pane");

        runtime.update_shell(ShellMessage::Tick);

        assert_eq!(runtime.app_state.active_workspace_id, active_workspace);
        assert_eq!(
            runtime
                .app_state
                .active_workspace()
                .and_then(|workspace| workspace.focused_pane_id.as_ref()),
            Some(&active_pane)
        );
        assert_eq!(runtime.last_error(), None);
        assert_eq!(runtime.view_model().terminals[0].lines, fallback_lines());
    }

    #[test]
    fn iced_shell_smoke_builds_view_once() {
        run_iced_shell_smoke().expect("smoke should build the shell view");
    }

    #[test]
    #[ignore = "spawns a real shell through ConPTY, run manually during Iced runtime work"]
    fn live_runtime_snapshots_capture_pty_output() {
        let mut runtime = NativeShellRuntime::new(true);
        let surface_id = runtime.view_model().terminals[0].surface_id.clone();
        let marker = "PANDAMUX_ICED_RUNTIME_PTY_OK";
        runtime
            .ptys
            .write_all(
                surface_id.as_str(),
                format!("Write-Output {marker}\r").as_bytes(),
            )
            .expect("write should reach pty");

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            runtime.refresh_terminal_snapshots();
            if runtime
                .view_model()
                .terminals
                .iter()
                .flat_map(|terminal| terminal.lines.iter())
                .any(|line| line.contains(marker))
            {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }

        panic!("marker was not found in terminal snapshots");
    }
}
