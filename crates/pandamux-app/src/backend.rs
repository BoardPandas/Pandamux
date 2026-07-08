//! The single backend dispatch code path shared by both clients of canonical
//! state: the named-pipe server (CLI / agents / orchestrator) and the live Iced
//! runtime. Per the rewrite's state model (plan Section 6.2), there is exactly
//! one writer and one place intents are applied; a CLI-driven split and a
//! UI-driven split are indistinguishable at this layer because they both go
//! through [`handle_line`].
//!
//! [`handle_line`] is synchronous: it borrows the canonical `AppState`,
//! `PtySessionManager`, and `Notifications` by mutable reference and never
//! awaits. The pipe server calls it under an async mutex; the Iced runtime calls
//! it directly inside `update`. This keeps a single implementation instead of
//! two divergent dispatchers.

use pandamux_core::{
    AgentInfo, AgentRegistry, AgentStatus, AppDelta, AppIntent, AppState, LayoutGridParams,
    NewNotification, NotificationSource, Notifications, PaneId, PaneIntent, RpcRequest,
    RpcResponse, SpawnStrategy, SplitDirection, SplitNode, SplitPaneParams, SurfaceId,
    SurfaceIntent, SurfaceType, SystemIntent, WorkspaceId, WorkspaceIntent, find_leaf,
    get_all_pane_ids,
};
use pandamux_term::{GridSize, PtyCommand, PtySessionManager};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

/// Owns the canonical backend state for the headless (no-UI) pipe-server path.
/// The live Iced runtime keeps its own copies of these fields and calls the free
/// [`handle_line`] function directly, so both paths share one dispatcher.
pub struct Backend {
    pub app: AppState,
    pub ptys: PtySessionManager,
    pub notifications: Notifications,
    pub notif_seq: u64,
    pub agents: AgentRegistry,
    pub spawn_ptys: bool,
}

impl Backend {
    pub fn new(spawn_ptys: bool) -> Self {
        Self {
            app: AppState::default(),
            ptys: PtySessionManager::new(),
            notifications: Notifications::new(),
            notif_seq: 0,
            agents: AgentRegistry::new(),
            spawn_ptys,
        }
    }

    /// Handle one protocol line and return the reply to write back to the client.
    pub fn handle_line(&mut self, line: &str) -> String {
        handle_line(
            line,
            &mut self.app,
            &mut self.ptys,
            &mut self.notifications,
            &mut self.notif_seq,
            &mut self.agents,
            now_ms(),
            self.spawn_ptys,
        )
    }
}

/// Wall-clock milliseconds since the Unix epoch (notification timestamps).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The single dispatch entry point. Handles the V1 `ping` text protocol and the
/// V2 JSON-RPC methods. `now_ms` is injected so callers (and tests) control the
/// clock. Returns the serialized reply; an empty string means "write nothing".
#[allow(clippy::too_many_arguments)]
pub fn handle_line(
    line: &str,
    app: &mut AppState,
    ptys: &mut PtySessionManager,
    notifications: &mut Notifications,
    notif_seq: &mut u64,
    agents: &mut AgentRegistry,
    now_ms: u64,
    spawn_ptys: bool,
) -> String {
    let message = line.trim();
    if message == "ping" {
        return "pong".to_string();
    }

    if !message.starts_with('{') {
        return String::new();
    }

    let request = match serde_json::from_str::<RpcRequest>(message) {
        Ok(request) => request,
        Err(error) => {
            return serialize_response(RpcResponse::error(
                Value::Null,
                -32700,
                format!("parse error: {error}"),
            ));
        }
    };

    let id = request.id.clone();
    match dispatch(
        &request,
        app,
        ptys,
        notifications,
        notif_seq,
        agents,
        now_ms,
        spawn_ptys,
    ) {
        Ok(result) => serialize_response(RpcResponse::result(id, result)),
        Err((code, message)) => serialize_response(RpcResponse::error(id, code, message)),
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    request: &RpcRequest,
    app: &mut AppState,
    ptys: &mut PtySessionManager,
    notifications: &mut Notifications,
    notif_seq: &mut u64,
    agents: &mut AgentRegistry,
    now_ms: u64,
    spawn_ptys: bool,
) -> Result<Value, (i32, String)> {
    if let Some(result) = dispatch_notifications(request, notifications, notif_seq, now_ms)? {
        return Ok(result);
    }

    if let Some(result) = dispatch_agents(request, app, ptys, agents, spawn_ptys)? {
        return Ok(result);
    }

    if let Some(result) = dispatch_terminal_io(request, app, ptys, spawn_ptys)? {
        return Ok(result);
    }

    // Browser automation is intentionally dropped in the native build (plan
    // Section 4.1). Reject it with a clear message instead of a generic
    // "method not found" so agents/CLI callers get actionable feedback.
    if request.method.starts_with("browser.") || request.method == "cdp" {
        return Err((
            -32601,
            "browser automation is not supported in the native build; use Claude Code's browser tooling".to_string(),
        ));
    }

    let intent = intent_for_request(request)?;
    let delta = app.apply(intent).map_err(|message| (-32000, message))?;
    sync_terminal_sessions(app, ptys, spawn_ptys).map_err(|message| (-32000, message))?;
    Ok(delta_to_result(delta))
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

fn dispatch_notifications(
    request: &RpcRequest,
    notifications: &mut Notifications,
    notif_seq: &mut u64,
    now_ms: u64,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "notification.raise" | "notification.fire" => {
            let params = &request.params;
            let title = opt_string(params, "title")
                .or_else(|| opt_string(params, "text"))
                .unwrap_or_default();
            if title.is_empty() {
                return Err((-32602, "notification.raise requires title/text".to_string()));
            }
            let body = opt_string(params, "body").unwrap_or_default();
            let source = parse_source(opt_string(params, "source").as_deref());
            *notif_seq += 1;
            let id = format!("notif-{notif_seq}");
            notifications.push(
                NewNotification {
                    workspace_id: opt_id(params, "workspaceId"),
                    surface_id: opt_id(params, "surfaceId"),
                    title,
                    body,
                    source,
                },
                id.clone(),
                now_ms,
            );
            Ok(Some(json!({ "id": id, "ok": true })))
        }
        "notification.list" => Ok(Some(json!({ "notifications": notifications.list() }))),
        "notification.clear" => {
            match opt_string(&request.params, "id").or_else(|| opt_string(&request.params, "text"))
            {
                Some(id) => {
                    notifications.clear(&id);
                }
                None => notifications.clear_all(),
            }
            Ok(Some(json!({ "ok": true })))
        }
        _ => Ok(None),
    }
}

fn parse_source(source: Option<&str>) -> NotificationSource {
    match source {
        Some("build") => NotificationSource::Build,
        Some("agent") => NotificationSource::Agent,
        Some("deploy") => NotificationSource::Deploy,
        Some("port") => NotificationSource::Port,
        _ => NotificationSource::Generic,
    }
}

// ---------------------------------------------------------------------------
// Agents
// ---------------------------------------------------------------------------

/// Where an agent's surface is placed.
enum Placement {
    /// A new tab in an existing (or the focused) pane.
    InPane(Option<PaneId>),
    /// A fresh split pane.
    NewSplit,
}

fn dispatch_agents(
    request: &RpcRequest,
    app: &mut AppState,
    ptys: &mut PtySessionManager,
    agents: &mut AgentRegistry,
    spawn_ptys: bool,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "agent.spawn" => {
            let params = &request.params;
            let command = opt_string(params, "cmd")
                .or_else(|| opt_string(params, "command"))
                .ok_or_else(|| (-32602, "agent.spawn requires cmd".to_string()))?;
            let label = opt_string(params, "label").unwrap_or_else(|| "agent".to_string());
            let cwd = opt_string(params, "cwd");
            let placement =
                Placement::InPane(opt_id(params, "pane").or_else(|| opt_id(params, "paneId")));
            let info = spawn_agent(
                app, ptys, agents, label, command, cwd, placement, spawn_ptys,
            )?;
            Ok(Some(agent_json(&info)))
        }
        "agent.spawn_batch" => {
            let strategy = SpawnStrategy::parse(
                opt_string(&request.params, "strategy")
                    .as_deref()
                    .unwrap_or("distribute"),
            );
            let specs = batch_specs(&request.params)?;
            if specs.is_empty() {
                return Err((-32602, "agent.spawn_batch requires agents".to_string()));
            }

            // Distribute round-robins across the panes that exist up front.
            let workspace_id = app.active_workspace_id.clone();
            let panes = app
                .workspace(&workspace_id)
                .map(|workspace| get_all_pane_ids(&workspace.split_tree))
                .unwrap_or_default();

            let mut spawned = Vec::new();
            for (index, spec) in specs.iter().enumerate() {
                let command = spec
                    .get("cmd")
                    .or_else(|| spec.get("command"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| (-32602, "each agent requires cmd".to_string()))?
                    .to_string();
                let label = spec
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("agent")
                    .to_string();
                let cwd = spec.get("cwd").and_then(Value::as_str).map(str::to_string);
                let placement = match strategy {
                    SpawnStrategy::Split => Placement::NewSplit,
                    SpawnStrategy::Stack => Placement::InPane(None),
                    SpawnStrategy::Distribute => {
                        Placement::InPane(panes.get(index % panes.len().max(1)).cloned())
                    }
                };
                let info = spawn_agent(
                    app, ptys, agents, label, command, cwd, placement, spawn_ptys,
                )?;
                spawned.push(agent_json(&info));
            }
            Ok(Some(json!({ "agents": spawned })))
        }
        "agent.status" => {
            let id = opt_string(&request.params, "id")
                .ok_or_else(|| (-32602, "agent.status requires id".to_string()))?;
            refresh_agent_status(ptys, agents, spawn_ptys);
            let info = agents
                .get(&id)
                .ok_or_else(|| (-32000, format!("agent not found: {id}")))?;
            Ok(Some(agent_json(info)))
        }
        "agent.list" => {
            refresh_agent_status(ptys, agents, spawn_ptys);
            Ok(Some(json!({ "agents": agents.list() })))
        }
        "agent.kill" => {
            let id = opt_string(&request.params, "id")
                .ok_or_else(|| (-32602, "agent.kill requires id".to_string()))?;
            let removed = agents
                .remove(&id)
                .ok_or_else(|| (-32000, format!("agent not found: {id}")))?;
            if spawn_ptys && ptys.has(removed.surface_id.as_str()) {
                let _ = ptys.kill(removed.surface_id.as_str());
            }
            // Close the agent's surface too, ignoring the "last surface" guard.
            let _ = app.apply(AppIntent::Surface(SurfaceIntent::Close {
                workspace_id: Some(removed.workspace_id.clone()),
                surface_id: removed.surface_id.clone(),
            }));
            Ok(Some(json!({ "ok": true })))
        }
        _ => Ok(None),
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_agent(
    app: &mut AppState,
    ptys: &mut PtySessionManager,
    agents: &mut AgentRegistry,
    label: String,
    command: String,
    cwd: Option<String>,
    placement: Placement,
    spawn_ptys: bool,
) -> Result<AgentInfo, (i32, String)> {
    let (workspace_id, pane_id, surface_id): (WorkspaceId, PaneId, SurfaceId) = match placement {
        Placement::InPane(pane_id) => {
            let delta = app
                .apply(AppIntent::Surface(SurfaceIntent::Create {
                    workspace_id: None,
                    pane_id,
                    surface_type: SurfaceType::Terminal,
                }))
                .map_err(|message| (-32000, message))?;
            match delta {
                AppDelta::SurfaceCreated {
                    workspace_id,
                    pane_id,
                    surface,
                } => (workspace_id, pane_id, surface.id),
                _ => return Err((-32000, "agent surface was not created".to_string())),
            }
        }
        Placement::NewSplit => {
            let delta = app
                .apply(AppIntent::Pane(PaneIntent::Split(SplitPaneParams {
                    workspace_id: None,
                    target_pane_id: None,
                    target_surface_id: None,
                    direction: SplitDirection::Horizontal,
                    surface_type: SurfaceType::Terminal,
                })))
                .map_err(|message| (-32000, message))?;
            match delta {
                AppDelta::PaneSplit {
                    workspace_id,
                    pane_id,
                    surface_id,
                    ..
                } => (workspace_id, pane_id, surface_id),
                _ => return Err((-32000, "agent pane was not created".to_string())),
            }
        }
    };

    if spawn_ptys {
        let pty_command = parse_command(&command, cwd.clone());
        ptys.spawn(surface_id.to_string(), &pty_command, GridSize::new(120, 30))
            .map_err(|error| (-32000, error.to_string()))?;
    }

    let info = AgentInfo {
        id: agents.next_id(),
        label,
        command,
        cwd,
        workspace_id,
        pane_id,
        surface_id,
        status: AgentStatus::Starting,
    };
    agents.add(info.clone());
    Ok(info)
}

/// Parse a command line into a `PtyCommand` (naive whitespace split; quoting is
/// not yet honored). Empty falls back to the default shell.
fn parse_command(command: &str, cwd: Option<String>) -> PtyCommand {
    let mut parts = command.split_whitespace();
    match parts.next() {
        Some(program) => PtyCommand::new(program)
            .with_args(parts.map(str::to_string))
            .with_cwd(cwd),
        None => PtyCommand::new(default_shell()).with_cwd(cwd),
    }
}

fn default_shell() -> &'static str {
    if cfg!(windows) { "pwsh" } else { "sh" }
}

/// Mark agents running/exited from their PTY child state (only meaningful when
/// PTYs are live).
fn refresh_agent_status(
    ptys: &mut PtySessionManager,
    agents: &mut AgentRegistry,
    spawn_ptys: bool,
) {
    if !spawn_ptys {
        return;
    }
    let updates: Vec<(String, AgentStatus)> = agents
        .list()
        .iter()
        .map(|agent| {
            let status = if ptys.is_running(agent.surface_id.as_str()) {
                AgentStatus::Running
            } else {
                AgentStatus::Exited
            };
            (agent.id.clone(), status)
        })
        .collect();
    for (id, status) in updates {
        agents.set_status(&id, status);
    }
}

fn batch_specs(params: &Value) -> Result<Vec<Value>, (i32, String)> {
    // Accept either an inline array (`agents`/`json`) or a JSON string (`json`).
    if let Some(array) = params
        .get("agents")
        .or_else(|| params.get("json"))
        .and_then(Value::as_array)
    {
        return Ok(array.clone());
    }
    if let Some(text) = opt_string(params, "json") {
        return serde_json::from_str::<Vec<Value>>(&text)
            .map_err(|error| (-32602, format!("invalid agents json: {error}")));
    }
    Ok(Vec::new())
}

fn agent_json(info: &AgentInfo) -> Value {
    json!({
        "id": info.id,
        "label": info.label,
        "workspaceId": info.workspace_id,
        "paneId": info.pane_id,
        "surfaceId": info.surface_id,
        "status": info.status,
        "command": info.command,
    })
}

// ---------------------------------------------------------------------------
// Terminal I/O
// ---------------------------------------------------------------------------

fn dispatch_terminal_io(
    request: &RpcRequest,
    app: &AppState,
    ptys: &mut PtySessionManager,
    spawn_ptys: bool,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "surface.send_text" => {
            sync_terminal_sessions(app, ptys, spawn_ptys).map_err(|message| (-32000, message))?;
            let surface_id = resolve_terminal_surface_id(app, ptys, &request.params)?;
            let text = opt_string(&request.params, "text").unwrap_or_default();
            ptys.write_all(surface_id.as_str(), text.as_bytes())
                .map_err(|error| (-32000, error.to_string()))?;
            Ok(Some(json!({ "ok": true })))
        }
        "surface.send_key" => {
            sync_terminal_sessions(app, ptys, spawn_ptys).map_err(|message| (-32000, message))?;
            let surface_id = resolve_terminal_surface_id(app, ptys, &request.params)?;
            let bytes = key_bytes(&request.params)?;
            ptys.write_all(surface_id.as_str(), &bytes)
                .map_err(|error| (-32000, error.to_string()))?;
            Ok(Some(json!({ "ok": true })))
        }
        "surface.read_text" => {
            sync_terminal_sessions(app, ptys, spawn_ptys).map_err(|message| (-32000, message))?;
            let surface_id = resolve_terminal_surface_id(app, ptys, &request.params)?;
            let lines = request
                .params
                .get("lines")
                .and_then(Value::as_u64)
                .unwrap_or(50) as usize;
            let text = ptys
                .screen_text_lines(surface_id.as_str(), lines)
                .map_err(|error| (-32000, error.to_string()))?;
            Ok(Some(json!({ "text": text })))
        }
        "surface.resize" | "pty.resize" => {
            sync_terminal_sessions(app, ptys, spawn_ptys).map_err(|message| (-32000, message))?;
            let surface_id = resolve_terminal_surface_id(app, ptys, &request.params)?;
            let size = grid_size_param(&request.params)?;
            ptys.resize(surface_id.as_str(), size)
                .map_err(|error| (-32000, error.to_string()))?;
            Ok(Some(json!({ "ok": true })))
        }
        "surface.kill" | "pty.kill" => {
            let surface_id = resolve_terminal_surface_id(app, ptys, &request.params)?;
            ptys.kill(surface_id.as_str())
                .map_err(|error| (-32000, error.to_string()))?;
            Ok(Some(json!({ "ok": true })))
        }
        "surface.trigger_flash" => Ok(Some(json!({ "ok": true }))),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Intent mapping
// ---------------------------------------------------------------------------

fn intent_for_request(request: &RpcRequest) -> Result<AppIntent, (i32, String)> {
    match request.method.as_str() {
        "system.identify" => Ok(AppIntent::System(SystemIntent::Identify)),
        "system.capabilities" => Ok(AppIntent::System(SystemIntent::Capabilities)),
        "system.tree" => Ok(AppIntent::System(SystemIntent::Tree {
            workspace_id: opt_id(&request.params, "workspaceId"),
        })),
        "workspace.create" => Ok(AppIntent::Workspace(WorkspaceIntent::Create {
            title: opt_string(&request.params, "title"),
            shell: opt_string(&request.params, "shell"),
        })),
        "workspace.close" => Ok(AppIntent::Workspace(WorkspaceIntent::Close {
            workspace_id: required_id(&request.params, &["id", "workspaceId"])?,
        })),
        "workspace.select" => Ok(AppIntent::Workspace(WorkspaceIntent::Select {
            workspace_id: required_id(&request.params, &["id", "workspaceId"])?,
        })),
        "workspace.rename" => Ok(AppIntent::Workspace(WorkspaceIntent::Rename {
            workspace_id: required_id(&request.params, &["id", "workspaceId"])?,
            title: opt_string(&request.params, "title")
                .ok_or_else(|| (-32602, "workspace.rename requires title".to_string()))?,
        })),
        "workspace.list" => Ok(AppIntent::Workspace(WorkspaceIntent::List)),
        "layout.grid" => Ok(AppIntent::Pane(PaneIntent::LayoutGrid(layout_grid_params(
            &request.params,
        )?))),
        "pane.split" => Ok(AppIntent::Pane(PaneIntent::Split(split_pane_params(
            &request.params,
        )?))),
        "pane.close" => Ok(AppIntent::Pane(PaneIntent::Close {
            workspace_id: opt_id(&request.params, "workspaceId"),
            pane_id: required_id(&request.params, &["id", "paneId"])?,
        })),
        "pane.focus" => Ok(AppIntent::Pane(PaneIntent::Focus {
            workspace_id: opt_id(&request.params, "workspaceId"),
            pane_id: required_id(&request.params, &["id", "paneId"])?,
        })),
        "pane.zoom" => Ok(AppIntent::Pane(PaneIntent::Zoom {
            workspace_id: opt_id(&request.params, "workspaceId"),
            pane_id: opt_id(&request.params, "id").or_else(|| opt_id(&request.params, "paneId")),
        })),
        "pane.list" => Ok(AppIntent::Pane(PaneIntent::List {
            workspace_id: opt_id(&request.params, "workspaceId"),
        })),
        "surface.create" => Ok(AppIntent::Surface(SurfaceIntent::Create {
            workspace_id: opt_id(&request.params, "workspaceId"),
            pane_id: opt_id(&request.params, "paneId"),
            surface_type: surface_type_param(&request.params)?,
        })),
        "surface.focus" => Ok(AppIntent::Surface(SurfaceIntent::Focus {
            workspace_id: opt_id(&request.params, "workspaceId"),
            surface_id: required_id(&request.params, &["id", "surfaceId"])?,
        })),
        "surface.close" => Ok(AppIntent::Surface(SurfaceIntent::Close {
            workspace_id: opt_id(&request.params, "workspaceId"),
            surface_id: required_id(&request.params, &["id", "surfaceId"])?,
        })),
        "surface.list" => Ok(AppIntent::Surface(SurfaceIntent::List {
            workspace_id: opt_id(&request.params, "workspaceId"),
            pane_id: opt_id(&request.params, "paneId"),
        })),
        _ => Err((-32601, format!("Method not found: {}", request.method))),
    }
}

fn split_pane_params(params: &Value) -> Result<SplitPaneParams, (i32, String)> {
    Ok(SplitPaneParams {
        workspace_id: opt_id(params, "workspaceId"),
        target_pane_id: opt_id(params, "targetPaneId").or_else(|| opt_id(params, "paneId")),
        target_surface_id: opt_id(params, "targetSurfaceId")
            .or_else(|| opt_id(params, "surfaceId")),
        direction: split_direction_param(params)?,
        surface_type: surface_type_param(params)?,
    })
}

fn layout_grid_params(params: &Value) -> Result<LayoutGridParams, (i32, String)> {
    let count = params
        .get("count")
        .and_then(Value::as_u64)
        .ok_or_else(|| (-32602, "layout.grid requires count".to_string()))?
        as usize;
    let surface_type = surface_type_param(params)?;

    Ok(LayoutGridParams {
        workspace_id: opt_id(params, "workspaceId"),
        anchor_pane_id: opt_id(params, "anchorPaneId"),
        anchor_surface_id: opt_id(params, "anchorSurfaceId"),
        count,
        surface_type,
    })
}

fn surface_type_param(params: &Value) -> Result<SurfaceType, (i32, String)> {
    params
        .get("type")
        .or_else(|| params.get("surfaceType"))
        .and_then(Value::as_str)
        .map(parse_surface_type)
        .transpose()
        .map(|surface_type| surface_type.unwrap_or(SurfaceType::Terminal))
}

fn parse_surface_type(value: &str) -> Result<SurfaceType, (i32, String)> {
    match value {
        "terminal" => Ok(SurfaceType::Terminal),
        "markdown" => Ok(SurfaceType::Markdown),
        "diff" => Ok(SurfaceType::Diff),
        "browser" => Err((-32602, "browser surfaces are not supported".to_string())),
        other => Err((-32602, format!("unsupported surface type: {other}"))),
    }
}

fn split_direction_param(params: &Value) -> Result<SplitDirection, (i32, String)> {
    if params.get("down").and_then(Value::as_bool) == Some(true) {
        return Ok(SplitDirection::Vertical);
    }

    match params
        .get("direction")
        .or_else(|| params.get("splitDirection"))
        .and_then(Value::as_str)
        .unwrap_or("horizontal")
    {
        "horizontal" | "right" => Ok(SplitDirection::Horizontal),
        "vertical" | "down" => Ok(SplitDirection::Vertical),
        other => Err((-32602, format!("unsupported split direction: {other}"))),
    }
}

fn grid_size_param(params: &Value) -> Result<GridSize, (i32, String)> {
    let columns = params
        .get("columns")
        .or_else(|| params.get("cols"))
        .and_then(Value::as_u64)
        .ok_or_else(|| (-32602, "resize requires columns".to_string()))? as usize;
    let rows = params
        .get("rows")
        .and_then(Value::as_u64)
        .ok_or_else(|| (-32602, "resize requires rows".to_string()))? as usize;

    Ok(GridSize::new(columns, rows))
}

fn key_bytes(params: &Value) -> Result<Vec<u8>, (i32, String)> {
    let key = opt_string(params, "key").unwrap_or_default();
    let has_ctrl = params.get("ctrl").and_then(Value::as_bool).unwrap_or(false)
        || modifier_present(params, "ctrl");
    let has_alt = params.get("alt").and_then(Value::as_bool).unwrap_or(false)
        || modifier_present(params, "alt");
    let has_shift = params
        .get("shift")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || modifier_present(params, "shift");

    let mut bytes = named_key_bytes(&key, has_shift)
        .ok_or_else(|| (-32602, format!("unknown key name: {key}")))?;
    if has_ctrl && bytes.len() == 1 {
        let upper = bytes[0].to_ascii_uppercase();
        if upper.is_ascii_uppercase() {
            bytes = vec![upper - 64];
        }
    }
    if has_alt {
        let mut with_alt = vec![0x1b];
        with_alt.extend(bytes);
        bytes = with_alt;
    }
    Ok(bytes)
}

fn modifier_present(params: &Value, modifier: &str) -> bool {
    params
        .get("modifiers")
        .and_then(Value::as_array)
        .map(|modifiers| {
            modifiers
                .iter()
                .any(|value| value.as_str() == Some(modifier))
        })
        .unwrap_or(false)
}

fn named_key_bytes(key: &str, has_shift: bool) -> Option<Vec<u8>> {
    let bytes = match key.to_ascii_lowercase().as_str() {
        "enter" => b"\r".to_vec(),
        "tab" => b"\t".to_vec(),
        "esc" | "escape" => b"\x1b".to_vec(),
        "backspace" => b"\x7f".to_vec(),
        "delete" => b"\x1b[3~".to_vec(),
        "up" => b"\x1b[A".to_vec(),
        "down" => b"\x1b[B".to_vec(),
        "right" => b"\x1b[C".to_vec(),
        "left" => b"\x1b[D".to_vec(),
        "home" => b"\x1b[H".to_vec(),
        "end" => b"\x1b[F".to_vec(),
        "pageup" => b"\x1b[5~".to_vec(),
        "pagedown" => b"\x1b[6~".to_vec(),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => b"\x1b[15~".to_vec(),
        "f6" => b"\x1b[17~".to_vec(),
        "f7" => b"\x1b[18~".to_vec(),
        "f8" => b"\x1b[19~".to_vec(),
        "f9" => b"\x1b[20~".to_vec(),
        "f10" => b"\x1b[21~".to_vec(),
        "f11" => b"\x1b[23~".to_vec(),
        "f12" => b"\x1b[24~".to_vec(),
        _ => {
            let mut chars = key.chars();
            let first = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            let literal = if has_shift {
                first.to_uppercase().collect::<String>()
            } else {
                first.to_string()
            };
            literal.into_bytes()
        }
    };

    Some(bytes)
}

// ---------------------------------------------------------------------------
// Delta -> JSON result (wire-compatible with the Electron bridge shapes)
// ---------------------------------------------------------------------------

fn delta_to_result(delta: AppDelta) -> Value {
    match delta {
        AppDelta::Identified {
            name,
            version,
            native,
        } => json!({
            "name": name,
            "version": version,
            "native": native,
        }),
        AppDelta::CapabilitiesReported { capabilities } => json!(capabilities),
        AppDelta::TreeReported { tree, .. } => json!({ "tree": tree }),
        AppDelta::WorkspaceCreated { workspace, tree } => json!({
            "workspace": workspace,
            "workspaceId": workspace.id,
            "tree": tree,
        }),
        AppDelta::WorkspaceSelected { .. }
        | AppDelta::WorkspaceRenamed { .. }
        | AppDelta::WorkspaceClosed { .. } => json!({ "ok": true }),
        AppDelta::WorkspaceListReported { workspaces } => json!({ "workspaces": workspaces }),
        AppDelta::LayoutGridApplied {
            workspace_id,
            tree,
            new_pane_ids,
        } => json!({
            "workspaceId": workspace_id,
            "tree": tree,
            "newPaneIds": new_pane_ids,
        }),
        AppDelta::PaneSplit {
            workspace_id,
            pane_id,
            surface_id,
            tree,
        } => json!({
            "workspaceId": workspace_id,
            "paneId": pane_id,
            "surfaceId": surface_id,
            "tree": tree,
        }),
        AppDelta::PaneClosed { .. }
        | AppDelta::PaneFocused { .. }
        | AppDelta::PaneZoomed { .. }
        | AppDelta::SurfaceFocused { .. }
        | AppDelta::SurfaceClosed { .. } => json!({ "ok": true }),
        AppDelta::PaneListReported { panes, .. } => json!({ "panes": panes }),
        AppDelta::SurfaceCreated {
            workspace_id,
            pane_id,
            surface,
        } => json!({
            "workspaceId": workspace_id,
            "paneId": pane_id,
            "surface": surface,
            "surfaceId": surface.id,
        }),
        AppDelta::SurfaceListReported { surfaces, .. } => json!({ "surfaces": surfaces }),
    }
}

// ---------------------------------------------------------------------------
// Terminal session reconciliation
// ---------------------------------------------------------------------------

/// Ensure a live PTY exists for every terminal surface and kill orphaned ones.
/// No-op unless `spawn_ptys` is set (tests/smoke stay hermetic).
pub fn sync_terminal_sessions(
    app: &AppState,
    ptys: &mut PtySessionManager,
    spawn_ptys: bool,
) -> Result<(), String> {
    if !spawn_ptys {
        return Ok(());
    }

    let mut expected_session_ids = HashSet::new();
    for workspace in &app.workspaces {
        for surface_id in terminal_surface_ids(&workspace.split_tree) {
            let session_id = surface_id.to_string();
            expected_session_ids.insert(session_id.clone());
            if ptys.has(&session_id) {
                continue;
            }
            ptys.spawn(
                session_id,
                &PtyCommand::new(workspace.shell.clone()),
                GridSize::new(120, 30),
            )
            .map_err(|error| error.to_string())?;
        }
    }

    for session_id in ptys.session_ids() {
        if !expected_session_ids.contains(&session_id) {
            ptys.kill(&session_id).map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

pub fn terminal_surface_ids(tree: &SplitNode) -> Vec<SurfaceId> {
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

fn resolve_terminal_surface_id(
    app: &AppState,
    ptys: &PtySessionManager,
    params: &Value,
) -> Result<SurfaceId, (i32, String)> {
    if let Some(surface_id) =
        opt_id::<SurfaceId>(params, "surfaceId").or_else(|| opt_id(params, "id"))
    {
        if ptys.has(surface_id.as_str()) {
            return Ok(surface_id);
        }
        return Err((-32000, format!("terminal surface not found: {surface_id}")));
    }

    let workspace_id =
        opt_id(params, "workspaceId").unwrap_or_else(|| app.active_workspace_id.clone());
    let workspace = app
        .workspace(&workspace_id)
        .ok_or_else(|| (-32000, format!("workspace not found: {workspace_id}")))?;

    if let Some(pane_id) = workspace.focused_pane_id.as_ref()
        && let Some(leaf) = find_leaf(&workspace.split_tree, pane_id)
        && let Some(surface) = leaf.surfaces.get(leaf.active_surface_index)
        && surface.surface_type == SurfaceType::Terminal
        && ptys.has(surface.id.as_str())
    {
        return Ok(surface.id.clone());
    }

    terminal_surface_ids(&workspace.split_tree)
        .into_iter()
        .find(|surface_id| ptys.has(surface_id.as_str()))
        .ok_or_else(|| (-32000, "no terminal surface available".to_string()))
}

// ---------------------------------------------------------------------------
// Param helpers
// ---------------------------------------------------------------------------

fn opt_string(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn opt_id<T>(params: &Value, key: &str) -> Option<T>
where
    T: From<String>,
{
    opt_string(params, key).map(T::from)
}

fn required_id<T>(params: &Value, keys: &[&str]) -> Result<T, (i32, String)>
where
    T: From<String>,
{
    for key in keys {
        if let Some(value) = opt_id(params, key) {
            return Ok(value);
        }
    }

    Err((-32602, format!("missing required id: {}", keys.join("|"))))
}

fn serialize_response(response: RpcResponse) -> String {
    serde_json::to_string(&response).unwrap_or_else(|error| {
        format!(
            r#"{{"error":{{"code":-32603,"message":"serialization error: {error}"}},"id":null}}"#
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(backend: &mut Backend, line: &str) -> Value {
        let reply = backend.handle_line(line);
        serde_json::from_str(&reply).expect("valid json")
    }

    #[test]
    fn handles_v1_ping() {
        let mut backend = Backend::new(false);
        assert_eq!(backend.handle_line("ping"), "pong");
    }

    #[test]
    fn handles_identify() {
        let mut backend = Backend::new(false);
        let parsed = handle(
            &mut backend,
            r#"{"method":"system.identify","params":{},"id":1}"#,
        );
        assert_eq!(parsed["result"]["name"], "pandamux");
        assert_eq!(parsed["id"], 1);
    }

    #[test]
    fn handles_layout_grid() {
        let mut backend = Backend::new(false);
        let parsed = handle(
            &mut backend,
            r#"{"method":"layout.grid","params":{"count":3,"type":"terminal"},"id":2}"#,
        );
        assert_eq!(parsed["result"]["newPaneIds"].as_array().unwrap().len(), 2);
        assert_eq!(parsed["id"], 2);
    }

    #[test]
    fn handles_workspace_create_and_list() {
        let mut backend = Backend::new(false);
        let created = handle(
            &mut backend,
            r#"{"method":"workspace.create","params":{"title":"Agents","shell":"pwsh"},"id":4}"#,
        );
        assert_eq!(created["result"]["workspace"]["title"], "Agents");

        let listed = handle(
            &mut backend,
            r#"{"method":"workspace.list","params":{},"id":5}"#,
        );
        assert_eq!(listed["result"]["workspaces"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn reports_panes_and_surfaces() {
        let mut backend = Backend::new(false);
        let panes = handle(&mut backend, r#"{"method":"pane.list","params":{},"id":6}"#);
        assert_eq!(panes["result"]["panes"].as_array().unwrap().len(), 1);

        let surfaces = handle(
            &mut backend,
            r#"{"method":"surface.list","params":{},"id":7}"#,
        );
        assert_eq!(surfaces["result"]["surfaces"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn handles_pane_split_focus_and_close() {
        let mut backend = Backend::new(false);
        let split = handle(
            &mut backend,
            r#"{"method":"pane.split","params":{"paneId":"pane-default","direction":"down","type":"terminal"},"id":11}"#,
        );
        let pane_id = split["result"]["paneId"]
            .as_str()
            .expect("pane id")
            .to_string();

        let focus = handle(
            &mut backend,
            &format!(r#"{{"method":"pane.focus","params":{{"id":"{pane_id}"}},"id":12}}"#),
        );
        assert_eq!(focus["result"]["ok"], true);

        let close = handle(
            &mut backend,
            &format!(r#"{{"method":"pane.close","params":{{"id":"{pane_id}"}},"id":13}}"#),
        );
        assert_eq!(close["result"]["ok"], true);
    }

    #[test]
    fn handles_pane_zoom() {
        let mut backend = Backend::new(false);
        let parsed = handle(
            &mut backend,
            r#"{"method":"pane.zoom","params":{"id":"pane-default"},"id":17}"#,
        );
        assert_eq!(parsed["result"]["ok"], true);
    }

    #[test]
    fn handles_surface_create_focus_and_close() {
        let mut backend = Backend::new(false);
        let create = handle(
            &mut backend,
            r#"{"method":"surface.create","params":{"paneId":"pane-default","type":"markdown"},"id":14}"#,
        );
        let surface_id = create["result"]["surfaceId"]
            .as_str()
            .expect("surface id")
            .to_string();

        let focus = handle(
            &mut backend,
            &format!(r#"{{"method":"surface.focus","params":{{"id":"{surface_id}"}},"id":15}}"#),
        );
        assert_eq!(focus["result"]["ok"], true);

        let close = handle(
            &mut backend,
            &format!(r#"{{"method":"surface.close","params":{{"id":"{surface_id}"}},"id":16}}"#),
        );
        assert_eq!(close["result"]["ok"], true);
    }

    #[test]
    fn raises_lists_and_clears_notifications() {
        let mut backend = Backend::new(false);
        let raised = handle(
            &mut backend,
            r#"{"method":"notification.raise","params":{"title":"Build done","body":"ok","source":"build"},"id":20}"#,
        );
        assert_eq!(raised["result"]["ok"], true);
        let id = raised["result"]["id"]
            .as_str()
            .expect("notif id")
            .to_string();

        let listed = handle(
            &mut backend,
            r#"{"method":"notification.list","params":{},"id":21}"#,
        );
        let notes = listed["result"]["notifications"].as_array().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0]["title"], "Build done");
        assert_eq!(notes[0]["source"], "build");

        let cleared = handle(
            &mut backend,
            &format!(r#"{{"method":"notification.clear","params":{{"id":"{id}"}},"id":22}}"#),
        );
        assert_eq!(cleared["result"]["ok"], true);
        let listed = handle(
            &mut backend,
            r#"{"method":"notification.list","params":{},"id":23}"#,
        );
        assert!(
            listed["result"]["notifications"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn notification_raise_requires_title() {
        let mut backend = Backend::new(false);
        let parsed = handle(
            &mut backend,
            r#"{"method":"notification.raise","params":{},"id":24}"#,
        );
        assert_eq!(parsed["error"]["code"], -32602);
    }

    #[test]
    fn translates_terminal_keys() {
        assert_eq!(key_bytes(&json!({ "key": "enter" })).unwrap(), b"\r");
        assert_eq!(
            key_bytes(&json!({ "key": "c", "ctrl": true })).unwrap(),
            vec![3]
        );
        assert_eq!(
            key_bytes(&json!({ "key": "left", "alt": true })).unwrap(),
            b"\x1b\x1b[D"
        );
    }

    #[test]
    fn parses_terminal_resize_size() {
        assert_eq!(
            grid_size_param(&json!({ "columns": 132, "rows": 40 })).unwrap(),
            GridSize::new(132, 40)
        );
        assert_eq!(
            grid_size_param(&json!({ "cols": 100, "rows": 30 })).unwrap(),
            GridSize::new(100, 30)
        );
    }

    #[test]
    fn shapes_workspace_mutation_replies_like_electron_bridge() {
        let mut backend = Backend::new(false);
        let created = handle(
            &mut backend,
            r#"{"method":"workspace.create","params":{"title":"Agents"},"id":8}"#,
        );
        let workspace_id = created["result"]["workspaceId"]
            .as_str()
            .expect("workspace id")
            .to_string();

        let selected = handle(
            &mut backend,
            &format!(
                r#"{{"method":"workspace.select","params":{{"id":"{workspace_id}"}},"id":9}}"#
            ),
        );
        assert_eq!(selected["result"]["ok"], true);

        let renamed = handle(
            &mut backend,
            &format!(
                r#"{{"method":"workspace.rename","params":{{"id":"{workspace_id}","title":"Renamed"}},"id":10}}"#
            ),
        );
        assert_eq!(renamed["result"]["ok"], true);
    }

    #[test]
    fn spawns_lists_and_kills_an_agent() {
        let mut backend = Backend::new(false);
        let spawn = handle(
            &mut backend,
            r#"{"method":"agent.spawn","params":{"cmd":"claude --dangerously","label":"worker"},"id":40}"#,
        );
        assert_eq!(spawn["result"]["label"], "worker");
        assert_eq!(spawn["result"]["status"], "starting");
        let id = spawn["result"]["id"]
            .as_str()
            .expect("agent id")
            .to_string();
        // The agent created a real terminal surface in the tree.
        assert!(spawn["result"]["surfaceId"].as_str().is_some());

        let list = handle(
            &mut backend,
            r#"{"method":"agent.list","params":{},"id":41}"#,
        );
        assert_eq!(list["result"]["agents"].as_array().unwrap().len(), 1);

        let status = handle(
            &mut backend,
            &format!(r#"{{"method":"agent.status","params":{{"id":"{id}"}},"id":42}}"#),
        );
        assert_eq!(status["result"]["id"], id);

        let kill = handle(
            &mut backend,
            &format!(r#"{{"method":"agent.kill","params":{{"id":"{id}"}},"id":43}}"#),
        );
        assert_eq!(kill["result"]["ok"], true);
        let list = handle(
            &mut backend,
            r#"{"method":"agent.list","params":{},"id":44}"#,
        );
        assert!(list["result"]["agents"].as_array().unwrap().is_empty());
    }

    #[test]
    fn spawns_a_batch_of_agents() {
        let mut backend = Backend::new(false);
        let batch = handle(
            &mut backend,
            r#"{"method":"agent.spawn_batch","params":{"strategy":"split","json":"[{\"cmd\":\"claude a\",\"label\":\"a\"},{\"cmd\":\"claude b\",\"label\":\"b\"}]"},"id":45}"#,
        );
        assert_eq!(batch["result"]["agents"].as_array().unwrap().len(), 2);
        let list = handle(
            &mut backend,
            r#"{"method":"agent.list","params":{},"id":46}"#,
        );
        assert_eq!(list["result"]["agents"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn agent_spawn_requires_cmd() {
        let mut backend = Backend::new(false);
        let parsed = handle(
            &mut backend,
            r#"{"method":"agent.spawn","params":{"label":"x"},"id":47}"#,
        );
        assert_eq!(parsed["error"]["code"], -32602);
    }

    #[test]
    fn rejects_browser_methods_with_clear_message() {
        let mut backend = Backend::new(false);
        let parsed = handle(
            &mut backend,
            r#"{"method":"browser.open","params":{"url":"https://example.com"},"id":30}"#,
        );
        assert_eq!(parsed["error"]["code"], -32601);
        assert!(
            parsed["error"]["message"]
                .as_str()
                .unwrap()
                .contains("not supported")
        );
    }

    #[test]
    fn rejects_browser_grid_surface() {
        let mut backend = Backend::new(false);
        let parsed = handle(
            &mut backend,
            r#"{"method":"layout.grid","params":{"count":2,"type":"browser"},"id":3}"#,
        );
        assert_eq!(parsed["error"]["code"], -32602);
    }

    #[test]
    fn non_json_line_returns_empty() {
        let mut backend = Backend::new(false);
        assert_eq!(backend.handle_line("not-json"), "");
    }
}
