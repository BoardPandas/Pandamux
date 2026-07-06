use iced::{Element, Subscription, Theme, application, time};
use pandamux_core::{
    AppIntent, AppState, PaneIntent, SplitNode, SplitPaneParams, SurfaceId, SurfaceIntent,
    SurfaceType,
};
use pandamux_term::{GridSize, PtyCommand, PtySessionManager};
use pandamux_ui::{
    ShellMessage, ShellViewModel, TerminalSnapshot, project_workspace_shell, shell_view,
};
use std::collections::HashSet;
use std::time::Duration;

pub struct NativeShellRuntime {
    app_state: AppState,
    ptys: PtySessionManager,
    live_ptys: bool,
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
        let mut runtime = Self {
            app_state,
            ptys: PtySessionManager::new(),
            live_ptys,
            view_model: empty_view_model(),
            terminals: Vec::new(),
            last_error: None,
        };
        runtime.refresh_terminal_snapshots();
        runtime
    }

    pub fn view_model(&self) -> &ShellViewModel {
        &self.view_model
    }

    pub fn update_shell(&mut self, message: ShellMessage) {
        if message == ShellMessage::Tick {
            self.refresh_terminal_snapshots();
            return;
        }

        let result = match message {
            ShellMessage::Tick => unreachable!("tick messages return before core intent routing"),
            ShellMessage::PaneFocused(pane_id) => {
                self.app_state.apply(AppIntent::Pane(PaneIntent::Focus {
                    workspace_id: None,
                    pane_id,
                }))
            }
            ShellMessage::PaneSplit { pane_id, direction } => {
                self.app_state
                    .apply(AppIntent::Pane(PaneIntent::Split(SplitPaneParams {
                        workspace_id: None,
                        target_pane_id: Some(pane_id),
                        target_surface_id: None,
                        direction,
                        surface_type: SurfaceType::Terminal,
                    })))
            }
            ShellMessage::PaneClosed(pane_id) => {
                self.app_state.apply(AppIntent::Pane(PaneIntent::Close {
                    workspace_id: None,
                    pane_id,
                }))
            }
            ShellMessage::PaneZoomToggled(pane_id) => {
                self.app_state.apply(AppIntent::Pane(PaneIntent::Zoom {
                    workspace_id: None,
                    pane_id: Some(pane_id),
                }))
            }
            ShellMessage::TerminalSurfaceCreated(pane_id) => {
                self.app_state
                    .apply(AppIntent::Surface(SurfaceIntent::Create {
                        workspace_id: None,
                        pane_id: Some(pane_id),
                        surface_type: SurfaceType::Terminal,
                    }))
            }
            ShellMessage::SurfaceFocused(surface_id) => {
                self.app_state
                    .apply(AppIntent::Surface(SurfaceIntent::Focus {
                        workspace_id: None,
                        surface_id,
                    }))
            }
            ShellMessage::SurfaceClosed(surface_id) => {
                self.app_state
                    .apply(AppIntent::Surface(SurfaceIntent::Close {
                        workspace_id: None,
                        surface_id,
                    }))
            }
        };

        self.last_error = result.err();
        self.refresh_terminal_snapshots();
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
        self.view_model = build_view_model(&self.app_state, &self.terminals);
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
    .title("PandaMUX Native")
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
    let _view = shell_view(model);
    println!("PANDAMUX_ICED_SHELL_SMOKE_OK");
    Ok(())
}

fn update_iced_shell(state: &mut NativeShellRuntime, message: ShellMessage) {
    state.update_shell(message);
}

fn view_iced_shell(state: &NativeShellRuntime) -> Element<'_, ShellMessage> {
    shell_view(state.view_model())
}

fn subscription_iced_shell(_state: &NativeShellRuntime) -> Subscription<ShellMessage> {
    time::every(Duration::from_millis(100)).map(|_| ShellMessage::Tick)
}

fn theme_iced_shell(_state: &NativeShellRuntime) -> Theme {
    Theme::Dark
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

fn build_view_model(app_state: &AppState, terminals: &[TerminalSnapshot]) -> ShellViewModel {
    let workspace = app_state
        .active_workspace()
        .expect("default app state should always have an active workspace");
    ShellViewModel {
        projection: project_workspace_shell(workspace),
        terminals: terminals.to_vec(),
    }
}

fn empty_view_model() -> ShellViewModel {
    build_view_model(&AppState::default(), &[])
}

fn fallback_terminal_snapshots(app_state: &AppState) -> Vec<TerminalSnapshot> {
    terminal_snapshots(app_state, &mut PtySessionManager::new(), false).unwrap_or_default()
}

fn fallback_lines() -> Vec<String> {
    vec![
        "PandaMUX Native".to_string(),
        "Runtime shell wiring is active.".to_string(),
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
        let _view = shell_view(&model);
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
