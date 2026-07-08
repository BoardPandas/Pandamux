use iced::{Element, Size, Subscription, Task, Theme, application, keyboard, time, window};
use pandamux_core::{
    AppIntent, AppState, PaneIntent, SplitNode, SplitPaneParams, SurfaceId, SurfaceIntent,
    SurfaceType, get_all_pane_ids,
};
use pandamux_term::{GridSize, PtyCommand, PtySessionManager};
use pandamux_ui::{
    ChromeState, RailItem, ShellKind, ShellMessage, ShellViewModel, TerminalSnapshot, UiTheme,
    app_view, project_workspace_shell, shell_view,
};
use std::collections::HashSet;
use std::time::Duration;

/// Ticks between block-cursor blinks (~1.1s at a 100ms tick).
const CURSOR_BLINK_TICKS: u64 = 11;

pub struct NativeShellRuntime {
    app_state: AppState,
    ptys: PtySessionManager,
    live_ptys: bool,
    chrome: ChromeState,
    tick: u64,
    view_model: ShellViewModel,
    terminals: Vec<TerminalSnapshot>,
    last_error: Option<String>,
}

impl Default for NativeShellRuntime {
    fn default() -> Self {
        Self::new(false)
    }
}

impl NativeShellRuntime {
    pub fn new(live_ptys: bool) -> Self {
        let app_state = AppState::default();
        let chrome = ChromeState::default();
        let view_model = build_view_model(&app_state, &[], &chrome, true);
        let mut runtime = Self {
            app_state,
            ptys: PtySessionManager::new(),
            live_ptys,
            chrome,
            tick: 0,
            view_model,
            terminals: Vec::new(),
            last_error: None,
        };
        runtime.refresh_terminal_snapshots();
        runtime
    }

    pub fn view_model(&self) -> &ShellViewModel {
        &self.view_model
    }

    fn cursor_on(&self) -> bool {
        (self.tick / CURSOR_BLINK_TICKS).is_multiple_of(2)
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
            }
            ShellMessage::OverlayRequested(item) => {
                // Overlays (palette/notifications/settings/quick-launch) land in
                // Phases 4-5. For now we reflect the request in the rail
                // highlight and clear the notification badge when opened.
                self.chrome.active_rail = item;
                if item == RailItem::Notifications {
                    self.chrome.unread_notifications = false;
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
        self.rebuild_chrome();
        self.view_model = build_view_model(
            &self.app_state,
            &self.terminals,
            &self.chrome,
            self.cursor_on(),
        );
    }

    /// Refresh the chrome view state derived from canonical state (session/pane
    /// counts, active shell/session). Pollers (git/ports) fill the rest later.
    fn rebuild_chrome(&mut self) {
        self.chrome.session_count = self.app_state.workspaces.len();
        self.chrome.version = env!("CARGO_PKG_VERSION").to_string();
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

fn subscription_iced_shell(_state: &NativeShellRuntime) -> Subscription<ShellMessage> {
    Subscription::batch([
        time::every(Duration::from_millis(100)).map(|_| ShellMessage::Tick),
        keyboard::listen().map(map_key_event),
    ])
}

fn map_key_event(event: keyboard::Event) -> ShellMessage {
    use keyboard::{Event, Key};
    if let Event::KeyPressed { key, modifiers, .. } = event
        && let Key::Character(character) = key.as_ref()
    {
        return shortcut_for(
            modifiers.control(),
            modifiers.shift(),
            &character.to_ascii_lowercase(),
        );
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
            Some(TerminalSnapshot {
                surface_id,
                lines,
                columns: 120,
                rows: 30,
            })
        })
        .collect();
    Ok(snapshots)
}

fn build_view_model(
    app_state: &AppState,
    terminals: &[TerminalSnapshot],
    chrome: &ChromeState,
    cursor_on: bool,
) -> ShellViewModel {
    let workspace = app_state
        .active_workspace()
        .expect("default app state should always have an active workspace");
    ShellViewModel {
        projection: project_workspace_shell(workspace),
        terminals: terminals.to_vec(),
        chrome: chrome.clone(),
        cursor_on,
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
