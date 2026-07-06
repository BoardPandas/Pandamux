use pandamux_core::{
    AppDelta, AppIntent, AppState, LayoutGridParams, PaneIntent, RpcRequest, RpcResponse,
    SplitDirection, SplitNode, SplitPaneParams, SurfaceId, SurfaceIntent, SurfaceType,
    SystemIntent, WorkspaceIntent, find_leaf,
};
use pandamux_term::{GridSize, PtyCommand, PtySessionManager};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

type SharedState = Arc<Mutex<BackendState>>;

struct BackendState {
    app: AppState,
    ptys: PtySessionManager,
    spawn_ptys: bool,
}

impl BackendState {
    fn new(spawn_ptys: bool) -> Self {
        Self {
            app: AppState::default(),
            ptys: PtySessionManager::new(),
            spawn_ptys,
        }
    }
}

#[cfg(windows)]
pub async fn run(pipe_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let state = Arc::new(Mutex::new(BackendState::new(true)));

    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(false)
            .create(pipe_name)?;
        server.connect().await?;
        let state = state.clone();

        tokio::spawn(async move {
            if let Err(error) = handle_connection(server, state).await {
                eprintln!("pipe connection error: {error}");
            }
        });
    }
}

#[cfg(not(windows))]
pub async fn run(_pipe_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    Err("named pipes are only implemented on Windows".into())
}

async fn handle_connection<T>(stream: T, state: SharedState) -> std::io::Result<()>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let reply = handle_message(line.trim(), &state).await;
    let mut stream = reader.into_inner();
    stream.write_all(reply.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.shutdown().await
}

async fn handle_message(message: &str, state: &SharedState) -> String {
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
    match dispatch(request, state).await {
        Ok(result) => serialize_response(RpcResponse::result(id, result)),
        Err((code, message)) => serialize_response(RpcResponse::error(id, code, message)),
    }
}

async fn dispatch(request: RpcRequest, state: &SharedState) -> Result<Value, (i32, String)> {
    if let Some(result) = dispatch_terminal_io(&request, state).await? {
        return Ok(result);
    }

    let intent = intent_for_request(&request)?;
    let mut backend = state.lock().await;
    let delta = backend
        .app
        .apply(intent)
        .map_err(|message| (-32000, message))?;
    sync_terminal_sessions(&mut backend).map_err(|message| (-32000, message))?;
    Ok(delta_to_result(delta))
}

async fn dispatch_terminal_io(
    request: &RpcRequest,
    state: &SharedState,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "surface.send_text" => {
            let mut backend = state.lock().await;
            sync_terminal_sessions(&mut backend).map_err(|message| (-32000, message))?;
            let surface_id = resolve_terminal_surface_id(&backend, &request.params)?;
            let text = opt_string(&request.params, "text").unwrap_or_default();
            backend
                .ptys
                .write_all(surface_id.as_str(), text.as_bytes())
                .map_err(|error| (-32000, error.to_string()))?;
            Ok(Some(json!({ "ok": true })))
        }
        "surface.send_key" => {
            let mut backend = state.lock().await;
            sync_terminal_sessions(&mut backend).map_err(|message| (-32000, message))?;
            let surface_id = resolve_terminal_surface_id(&backend, &request.params)?;
            let bytes = key_bytes(&request.params)?;
            backend
                .ptys
                .write_all(surface_id.as_str(), &bytes)
                .map_err(|error| (-32000, error.to_string()))?;
            Ok(Some(json!({ "ok": true })))
        }
        "surface.read_text" => {
            let mut backend = state.lock().await;
            sync_terminal_sessions(&mut backend).map_err(|message| (-32000, message))?;
            let surface_id = resolve_terminal_surface_id(&backend, &request.params)?;
            let lines = request
                .params
                .get("lines")
                .and_then(Value::as_u64)
                .unwrap_or(50) as usize;
            let text = backend
                .ptys
                .screen_text_lines(surface_id.as_str(), lines)
                .map_err(|error| (-32000, error.to_string()))?;
            Ok(Some(json!({ "text": text })))
        }
        "surface.resize" | "pty.resize" => {
            let mut backend = state.lock().await;
            sync_terminal_sessions(&mut backend).map_err(|message| (-32000, message))?;
            let surface_id = resolve_terminal_surface_id(&backend, &request.params)?;
            let size = grid_size_param(&request.params)?;
            backend
                .ptys
                .resize(surface_id.as_str(), size)
                .map_err(|error| (-32000, error.to_string()))?;
            Ok(Some(json!({ "ok": true })))
        }
        "surface.kill" | "pty.kill" => {
            let mut backend = state.lock().await;
            let surface_id = resolve_terminal_surface_id(&backend, &request.params)?;
            backend
                .ptys
                .kill(surface_id.as_str())
                .map_err(|error| (-32000, error.to_string()))?;
            Ok(Some(json!({ "ok": true })))
        }
        "surface.trigger_flash" => Ok(Some(json!({ "ok": true }))),
        _ => Ok(None),
    }
}

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

fn sync_terminal_sessions(backend: &mut BackendState) -> Result<(), String> {
    if !backend.spawn_ptys {
        return Ok(());
    }

    let mut expected_session_ids = HashSet::new();
    for workspace in &backend.app.workspaces {
        for surface_id in terminal_surface_ids(&workspace.split_tree) {
            let session_id = surface_id.to_string();
            expected_session_ids.insert(session_id.clone());
            if backend.ptys.has(&session_id) {
                continue;
            }
            backend
                .ptys
                .spawn(
                    session_id,
                    &PtyCommand::new(workspace.shell.clone()),
                    GridSize::new(120, 30),
                )
                .map_err(|error| error.to_string())?;
        }
    }

    for session_id in backend.ptys.session_ids() {
        if !expected_session_ids.contains(&session_id) {
            backend
                .ptys
                .kill(&session_id)
                .map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

fn terminal_surface_ids(tree: &SplitNode) -> Vec<pandamux_core::SurfaceId> {
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
    backend: &BackendState,
    params: &Value,
) -> Result<SurfaceId, (i32, String)> {
    if let Some(surface_id) =
        opt_id::<SurfaceId>(params, "surfaceId").or_else(|| opt_id(params, "id"))
    {
        if backend.ptys.has(surface_id.as_str()) {
            return Ok(surface_id);
        }
        return Err((-32000, format!("terminal surface not found: {surface_id}")));
    }

    let workspace_id =
        opt_id(params, "workspaceId").unwrap_or_else(|| backend.app.active_workspace_id.clone());
    let workspace = backend
        .app
        .workspace(&workspace_id)
        .ok_or_else(|| (-32000, format!("workspace not found: {workspace_id}")))?;

    if let Some(pane_id) = workspace.focused_pane_id.as_ref() {
        if let Some(leaf) = find_leaf(&workspace.split_tree, pane_id) {
            if let Some(surface) = leaf.surfaces.get(leaf.active_surface_index) {
                if surface.surface_type == SurfaceType::Terminal
                    && backend.ptys.has(surface.id.as_str())
                {
                    return Ok(surface.id.clone());
                }
            }
        }
    }

    terminal_surface_ids(&workspace.split_tree)
        .into_iter()
        .find(|surface_id| backend.ptys.has(surface_id.as_str()))
        .ok_or_else(|| (-32000, "no terminal surface available".to_string()))
}

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

    fn test_state() -> SharedState {
        Arc::new(Mutex::new(BackendState::new(false)))
    }

    #[tokio::test]
    async fn handles_v1_ping() {
        assert_eq!(handle_message("ping", &test_state()).await, "pong");
    }

    #[tokio::test]
    async fn handles_identify() {
        let response = handle_message(
            r#"{"method":"system.identify","params":{},"id":1}"#,
            &test_state(),
        )
        .await;
        let parsed: Value = serde_json::from_str(&response).expect("valid json");
        assert_eq!(parsed["result"]["name"], "pandamux");
        assert_eq!(parsed["id"], 1);
    }

    #[tokio::test]
    async fn handles_layout_grid() {
        let state = test_state();
        let response = handle_message(
            r#"{"method":"layout.grid","params":{"count":3,"type":"terminal"},"id":2}"#,
            &state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&response).expect("valid json");
        assert_eq!(parsed["result"]["newPaneIds"].as_array().unwrap().len(), 2);
        assert_eq!(parsed["id"], 2);
    }

    #[tokio::test]
    async fn handles_workspace_create_and_list() {
        let state = test_state();
        let create = handle_message(
            r#"{"method":"workspace.create","params":{"title":"Agents","shell":"pwsh"},"id":4}"#,
            &state,
        )
        .await;
        let created: Value = serde_json::from_str(&create).expect("valid json");
        assert_eq!(created["result"]["workspace"]["title"], "Agents");

        let list =
            handle_message(r#"{"method":"workspace.list","params":{},"id":5}"#, &state).await;
        let listed: Value = serde_json::from_str(&list).expect("valid json");
        assert_eq!(listed["result"]["workspaces"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn reports_panes_and_surfaces() {
        let state = test_state();
        let panes = handle_message(r#"{"method":"pane.list","params":{},"id":6}"#, &state).await;
        let panes: Value = serde_json::from_str(&panes).expect("valid json");
        assert_eq!(panes["result"]["panes"].as_array().unwrap().len(), 1);

        let surfaces =
            handle_message(r#"{"method":"surface.list","params":{},"id":7}"#, &state).await;
        let surfaces: Value = serde_json::from_str(&surfaces).expect("valid json");
        assert_eq!(surfaces["result"]["surfaces"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn handles_pane_split_focus_and_close() {
        let state = test_state();
        let split = handle_message(
            r#"{"method":"pane.split","params":{"paneId":"pane-default","direction":"down","type":"terminal"},"id":11}"#,
            &state,
        )
        .await;
        let split: Value = serde_json::from_str(&split).expect("valid json");
        let pane_id = split["result"]["paneId"].as_str().expect("pane id");

        let focus = handle_message(
            &format!(r#"{{"method":"pane.focus","params":{{"id":"{pane_id}"}},"id":12}}"#),
            &state,
        )
        .await;
        let focus: Value = serde_json::from_str(&focus).expect("valid json");
        assert_eq!(focus["result"]["ok"], true);

        let close = handle_message(
            &format!(r#"{{"method":"pane.close","params":{{"id":"{pane_id}"}},"id":13}}"#),
            &state,
        )
        .await;
        let close: Value = serde_json::from_str(&close).expect("valid json");
        assert_eq!(close["result"]["ok"], true);
    }

    #[tokio::test]
    async fn handles_pane_zoom() {
        let state = test_state();
        let response = handle_message(
            r#"{"method":"pane.zoom","params":{"id":"pane-default"},"id":17}"#,
            &state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&response).expect("valid json");
        assert_eq!(parsed["result"]["ok"], true);
    }

    #[tokio::test]
    async fn handles_surface_create_focus_and_close() {
        let state = test_state();
        let create = handle_message(
            r#"{"method":"surface.create","params":{"paneId":"pane-default","type":"markdown"},"id":14}"#,
            &state,
        )
        .await;
        let create: Value = serde_json::from_str(&create).expect("valid json");
        let surface_id = create["result"]["surfaceId"].as_str().expect("surface id");

        let focus = handle_message(
            &format!(r#"{{"method":"surface.focus","params":{{"id":"{surface_id}"}},"id":15}}"#),
            &state,
        )
        .await;
        let focus: Value = serde_json::from_str(&focus).expect("valid json");
        assert_eq!(focus["result"]["ok"], true);

        let close = handle_message(
            &format!(r#"{{"method":"surface.close","params":{{"id":"{surface_id}"}},"id":16}}"#),
            &state,
        )
        .await;
        let close: Value = serde_json::from_str(&close).expect("valid json");
        assert_eq!(close["result"]["ok"], true);
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

    #[tokio::test]
    async fn shapes_workspace_mutation_replies_like_electron_bridge() {
        let state = test_state();
        let create = handle_message(
            r#"{"method":"workspace.create","params":{"title":"Agents"},"id":8}"#,
            &state,
        )
        .await;
        let created: Value = serde_json::from_str(&create).expect("valid json");
        let workspace_id = created["result"]["workspaceId"]
            .as_str()
            .expect("workspace id");

        let select = handle_message(
            &format!(
                r#"{{"method":"workspace.select","params":{{"id":"{workspace_id}"}},"id":9}}"#
            ),
            &state,
        )
        .await;
        let selected: Value = serde_json::from_str(&select).expect("valid json");
        assert_eq!(selected["result"]["ok"], true);

        let rename = handle_message(
            &format!(
                r#"{{"method":"workspace.rename","params":{{"id":"{workspace_id}","title":"Renamed"}},"id":10}}"#
            ),
            &state,
        )
        .await;
        let renamed: Value = serde_json::from_str(&rename).expect("valid json");
        assert_eq!(renamed["result"]["ok"], true);
    }

    #[tokio::test]
    async fn rejects_browser_grid_surface() {
        let response = handle_message(
            r#"{"method":"layout.grid","params":{"count":2,"type":"browser"},"id":3}"#,
            &test_state(),
        )
        .await;
        let parsed: Value = serde_json::from_str(&response).expect("valid json");
        assert_eq!(parsed["error"]["code"], -32602);
    }
}
