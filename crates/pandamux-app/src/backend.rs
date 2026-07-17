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
    AgentInfo, AgentRegistry, AgentStatus, AppDelta, AppIntent, AppState, ClipboardConfig,
    DropZone, LayoutGridParams, Locale, Localizer, NewNotification, NotificationSource,
    Notifications, PaneId, PaneIntent, ProjectError, ProjectLocation, RpcRequest, RpcResponse,
    SidebarState, SpawnStrategy, SplitDirection, SplitNode, SplitPaneParams, SshAuthConfig,
    SshHostProfile, SshProfileId, SshProfiles, SurfaceContents, SurfaceId, SurfaceIntent,
    SurfaceType, SystemIntent, ThemeStore, WorkspaceId, WorkspaceIntent, find_leaf,
    get_all_pane_ids, import_windows_terminal, parse_ghostty_theme,
};
use pandamux_term::{
    ClipboardStore, DEFAULT_GRID_SIZE, GridSize, PtyCommand, PtySessionManager,
    RemoteSessionManager, SshAuth, SshConfig, wrap_paste,
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

/// The mutable backend state a single dispatch borrows. Grouping these into one
/// struct (rather than a long `&mut` parameter list) keeps `handle_line` and the
/// sub-dispatchers readable as more surfaces gain state; both clients of the
/// single writer (the pipe server and the live Iced runtime) build one of these
/// per request and pass it in.
pub struct DispatchCtx<'a> {
    pub app: &'a mut AppState,
    pub ptys: &'a mut PtySessionManager,
    pub notifications: &'a mut Notifications,
    pub notif_seq: &'a mut u64,
    pub agents: &'a mut AgentRegistry,
    pub sidebar: &'a mut SidebarState,
    pub contents: &'a mut SurfaceContents,
    pub themes: &'a mut ThemeStore,
    pub localizer: &'a mut Localizer,
    /// Per-surface terminal color-scheme overrides (surface id -> theme name).
    pub surface_schemes: &'a mut HashMap<SurfaceId, String>,
    /// SSH remote terminal sessions (plan F2). A remote surface's byte source is
    /// an SSH channel instead of a local PTY; terminal I/O routes here for it.
    pub remotes: &'a mut RemoteSessionManager,
    /// Which surfaces are remote and how to reach them (kept so reconciliation
    /// skips them for local-PTY spawning and I/O routes correctly).
    pub remote_configs: &'a mut HashMap<SurfaceId, SshConfig>,
    /// Saved SSH host profiles (imported from `~/.ssh/config` or entered).
    pub ssh_profiles: &'a mut SshProfiles,
    /// Persistent clipboard policy (OSC 52 size cap + per-host load opt-in).
    pub clipboard_config: &'a mut ClipboardConfig,
    /// Persistent user settings (`config.get` / `config.set`). The live
    /// runtime persists and live-applies changes after dispatch.
    pub settings: &'a mut pandamux_core::UserSettings,
    pub now_ms: u64,
    pub spawn_ptys: bool,
}

/// Owns the canonical backend state for the headless (no-UI) pipe-server path.
/// The live Iced runtime keeps its own copies of these fields and calls the free
/// [`handle_line`] function directly, so both paths share one dispatcher.
pub struct Backend {
    pub app: AppState,
    pub ptys: PtySessionManager,
    pub notifications: Notifications,
    pub notif_seq: u64,
    pub agents: AgentRegistry,
    pub sidebar: SidebarState,
    pub contents: SurfaceContents,
    pub themes: ThemeStore,
    pub localizer: Localizer,
    pub surface_schemes: HashMap<SurfaceId, String>,
    pub remotes: RemoteSessionManager,
    pub remote_configs: HashMap<SurfaceId, SshConfig>,
    pub ssh_profiles: SshProfiles,
    pub clipboard_config: ClipboardConfig,
    /// In-memory settings for the headless pipe server (the live runtime
    /// persists its own copy to `config/settings.json`).
    pub settings: pandamux_core::UserSettings,
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
            sidebar: SidebarState::new(),
            contents: SurfaceContents::new(),
            themes: ThemeStore::new(),
            localizer: Localizer::default(),
            surface_schemes: HashMap::new(),
            remotes: RemoteSessionManager::default(),
            remote_configs: HashMap::new(),
            ssh_profiles: SshProfiles::new(),
            clipboard_config: ClipboardConfig::default(),
            settings: pandamux_core::UserSettings::default(),
            spawn_ptys,
        }
    }

    /// Handle one protocol line and return the reply to write back to the client.
    pub fn handle_line(&mut self, line: &str) -> String {
        let ctx = DispatchCtx {
            app: &mut self.app,
            ptys: &mut self.ptys,
            notifications: &mut self.notifications,
            notif_seq: &mut self.notif_seq,
            agents: &mut self.agents,
            sidebar: &mut self.sidebar,
            contents: &mut self.contents,
            themes: &mut self.themes,
            localizer: &mut self.localizer,
            surface_schemes: &mut self.surface_schemes,
            remotes: &mut self.remotes,
            remote_configs: &mut self.remote_configs,
            ssh_profiles: &mut self.ssh_profiles,
            clipboard_config: &mut self.clipboard_config,
            settings: &mut self.settings,
            now_ms: now_ms(),
            spawn_ptys: self.spawn_ptys,
        };
        handle_line(line, ctx)
    }

    /// Forward any OSC 52 clipboard-store events captured from local or remote
    /// terminal grids to the OS clipboard (plan F1). Best-effort. Called on each
    /// UI refresh and after headless dispatch.
    pub fn drain_clipboards(&mut self) {
        drain_clipboard_stores(
            &mut self.ptys,
            &mut self.remotes,
            self.clipboard_config.max_store_bytes,
        );
    }
}

/// Drain OSC 52 stores from every local PTY and remote session, writing the most
/// recent within the size cap to the OS clipboard. Best-effort (a headless box
/// without a clipboard fails softly).
pub fn drain_clipboard_stores(
    ptys: &mut PtySessionManager,
    remotes: &mut RemoteSessionManager,
    max_store_bytes: usize,
) {
    let mut latest: Option<String> = None;
    for session_id in ptys.session_ids() {
        // Polling advances the grid so captured OSC 52 events surface.
        let _ = ptys.poll(&session_id);
        for store in ptys.take_clipboard_stores(&session_id) {
            consider_store(store, max_store_bytes, &mut latest);
        }
    }
    for session_id in remotes.session_ids() {
        let _ = remotes.poll(&session_id);
        for store in remotes.take_clipboard_stores(&session_id) {
            consider_store(store, max_store_bytes, &mut latest);
        }
    }
    if let Some(text) = latest {
        let _ = crate::clipboard_os::set_text(&text);
    }
}

fn consider_store(store: ClipboardStore, max_store_bytes: usize, latest: &mut Option<String>) {
    if store.text.len() <= max_store_bytes {
        *latest = Some(store.text);
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
/// V2 JSON-RPC methods. The clock (`ctx.now_ms`) is injected so callers (and
/// tests) control it. Returns the serialized reply; an empty string means "write
/// nothing".
pub fn handle_line(line: &str, ctx: DispatchCtx<'_>) -> String {
    let message = line.trim();
    if message == "ping" {
        return "pong".to_string();
    }

    // V1 shell-integration cwd report: `report_pwd <surfaceId> <path>` (bash/pwsh
    // report over the pipe; cmd reports inline via OSC, parsed in the term layer).
    if let Some(rest) = message.strip_prefix("report_pwd ") {
        let mut parts = rest.splitn(2, ' ');
        if let (Some(surface_id), Some(path)) = (parts.next(), parts.next()) {
            ctx.ptys.set_cwd(surface_id, path.trim());
        }
        return String::new();
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
    match dispatch(&request, ctx) {
        Ok(result) => serialize_response(RpcResponse::result(id, result)),
        Err((code, message)) => serialize_response(RpcResponse::error(id, code, message)),
    }
}

fn dispatch(request: &RpcRequest, ctx: DispatchCtx<'_>) -> Result<Value, (i32, String)> {
    let DispatchCtx {
        app,
        ptys,
        notifications,
        notif_seq,
        agents,
        sidebar,
        contents,
        themes,
        localizer,
        surface_schemes,
        remotes,
        remote_configs,
        ssh_profiles,
        clipboard_config,
        settings,
        now_ms,
        spawn_ptys,
    } = ctx;

    if let Some(result) = dispatch_notifications(request, notifications, notif_seq, now_ms)? {
        return Ok(result);
    }

    if let Some(result) = dispatch_sidebar(request, sidebar)? {
        return Ok(result);
    }

    if let Some(result) = dispatch_settings(request, settings)? {
        return Ok(result);
    }

    if let Some(result) = dispatch_config(request, themes, localizer)? {
        return Ok(result);
    }

    if let Some(result) = dispatch_window(request)? {
        return Ok(result);
    }

    if let Some(result) = dispatch_surface_scheme(request, app, themes, surface_schemes)? {
        return Ok(result);
    }

    if let Some(result) = dispatch_agents(request, app, ptys, agents, spawn_ptys)? {
        return Ok(result);
    }

    if let Some(result) = dispatch_projects(
        request,
        app,
        ptys,
        remotes,
        remote_configs,
        ssh_profiles,
        spawn_ptys,
    )? {
        return Ok(result);
    }

    if let Some(result) = dispatch_ssh(
        request,
        app,
        remotes,
        remote_configs,
        ssh_profiles,
        spawn_ptys,
    )? {
        return Ok(result);
    }

    if let Some(result) = dispatch_clipboard(request, clipboard_config)? {
        return Ok(result);
    }

    if let Some(result) =
        dispatch_terminal_io(request, app, ptys, remotes, remote_configs, spawn_ptys)?
    {
        return Ok(result);
    }

    if let Some(result) = dispatch_surface_content(request, app, contents)? {
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
    sync_terminal_sessions(app, ptys, remotes, spawn_ptys).map_err(|message| (-32000, message))?;
    // Drop content + color-scheme overrides for surfaces the mutation may have closed.
    let live = all_surface_ids(app);
    contents.retain_live(&live);
    surface_schemes.retain(|surface_id, _| live.contains(surface_id));
    // Kill remote sessions whose surface was closed and forget their config.
    sync_remote_sessions(&live, remotes, remote_configs);
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
// Sidebar (status / progress / log)
// ---------------------------------------------------------------------------

fn dispatch_sidebar(
    request: &RpcRequest,
    sidebar: &mut SidebarState,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "sidebar.set_status" => {
            let key = opt_string(&request.params, "key")
                .ok_or_else(|| (-32602, "sidebar.set_status requires key".to_string()))?;
            let value = opt_string(&request.params, "value").unwrap_or_default();
            sidebar.set_status(key, value);
            Ok(Some(json!({ "ok": true })))
        }
        "sidebar.set_progress" => {
            let value = request
                .params
                .get("value")
                .and_then(Value::as_f64)
                .ok_or_else(|| (-32602, "sidebar.set_progress requires value".to_string()))?;
            let label = opt_string(&request.params, "label");
            sidebar.set_progress(value.round().clamp(0.0, 255.0) as u8, label);
            Ok(Some(json!({ "ok": true })))
        }
        "sidebar.log" => {
            let level = opt_string(&request.params, "level").unwrap_or_else(|| "info".to_string());
            let message = opt_string(&request.params, "message").unwrap_or_default();
            sidebar.log(level, message);
            Ok(Some(json!({ "ok": true })))
        }
        "sidebar.get_state" => Ok(Some(serde_json::to_value(&*sidebar).unwrap_or(json!({})))),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// User settings (config.get / config.set)
// ---------------------------------------------------------------------------

/// Read/write persistent user settings by dotted camelCase key, matching
/// `config/settings.json` one to one. The live runtime persists and
/// live-applies mutations after dispatch; the headless server keeps them in
/// memory for the process lifetime.
fn dispatch_settings(
    request: &RpcRequest,
    settings: &mut pandamux_core::UserSettings,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "config.get" => {
            let key = request
                .params
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or("");
            let value =
                pandamux_core::settings_get(settings, key).map_err(|message| (-32602, message))?;
            if key.is_empty() {
                Ok(Some(json!({ "settings": value })))
            } else {
                Ok(Some(json!({ "key": key, "value": value })))
            }
        }
        "config.set" => {
            let key = request
                .params
                .get("key")
                .and_then(Value::as_str)
                .ok_or_else(|| (-32602, "config.set requires a string key".to_string()))?;
            let value = request
                .params
                .get("value")
                .cloned()
                .ok_or_else(|| (-32602, "config.set requires a value".to_string()))?;
            pandamux_core::settings_set(settings, key, value.clone())
                .map_err(|message| (-32602, message))?;
            let value =
                pandamux_core::settings_get(settings, key).map_err(|message| (-32602, message))?;
            Ok(Some(json!({ "ok": true, "key": key, "value": value })))
        }
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Themes / config / i18n
// ---------------------------------------------------------------------------

fn dispatch_config(
    request: &RpcRequest,
    themes: &mut ThemeStore,
    localizer: &mut Localizer,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "theme.list" => Ok(Some(json!({
            "themes": themes.names(),
            "active": themes.active_name(),
        }))),
        "theme.select" => {
            let name = opt_string(&request.params, "name")
                .or_else(|| opt_string(&request.params, "id"))
                .ok_or_else(|| (-32602, "theme.select requires name".to_string()))?;
            if themes.set_active(&name) {
                Ok(Some(json!({ "ok": true, "active": name })))
            } else {
                Err((-32000, format!("theme not found: {name}")))
            }
        }
        "theme.get" => {
            let name = opt_string(&request.params, "name")
                .ok_or_else(|| (-32602, "theme.get requires name".to_string()))?;
            let theme = themes
                .get(&name)
                .ok_or_else(|| (-32000, format!("theme not found: {name}")))?;
            Ok(Some(serde_json::to_value(theme).unwrap_or(json!({}))))
        }
        "config.import_windows_terminal" => {
            let content = opt_string(&request.params, "content")
                .ok_or_else(|| (-32602, "import requires content".to_string()))?;
            let imported = import_windows_terminal(&content).map_err(|error| (-32000, error))?;
            let names: Vec<String> = imported.iter().map(|theme| theme.name.clone()).collect();
            for theme in imported {
                themes.insert(theme);
            }
            Ok(Some(json!({ "imported": names })))
        }
        "config.import_ghostty" => {
            let content = opt_string(&request.params, "content")
                .ok_or_else(|| (-32602, "import requires content".to_string()))?;
            let name =
                opt_string(&request.params, "name").unwrap_or_else(|| "imported".to_string());
            themes.insert(parse_ghostty_theme(name.clone(), &content));
            Ok(Some(json!({ "name": name })))
        }
        "config.show" => Ok(Some(json!({
            "activeTheme": themes.active_name(),
            "themeCount": themes.len(),
            "locale": localizer.locale().code(),
        }))),
        "config.path" => Ok(Some(json!({
            "path": std::env::var("PANDAMUX_THEMES_DIR")
                .unwrap_or_else(|_| "resources/themes".to_string()),
        }))),
        "config.reload" => Ok(Some(json!({ "ok": true, "themeCount": themes.len() }))),
        "i18n.set_locale" => {
            let code = opt_string(&request.params, "locale")
                .ok_or_else(|| (-32602, "i18n.set_locale requires locale".to_string()))?;
            let locale = Locale::parse(&code)
                .ok_or_else(|| (-32602, format!("unsupported locale: {code}")))?;
            localizer.set_locale(locale);
            Ok(Some(json!({ "ok": true, "locale": locale.code() })))
        }
        "i18n.translate" => {
            let key = opt_string(&request.params, "key")
                .ok_or_else(|| (-32602, "i18n.translate requires key".to_string()))?;
            Ok(Some(json!({ "text": localizer.t(&key) })))
        }
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Windows (multi-window parity)
// ---------------------------------------------------------------------------

/// The native build is single-window; `window.list` / `window.focus` report and
/// act on the one window so the CLI contract (`list-windows` / `focus-window`)
/// stays satisfied. Spawning additional OS windows needs the Iced multi-window
/// (daemon) runtime and is out of scope here.
fn dispatch_window(request: &RpcRequest) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "window.list" => Ok(Some(json!({
            "windows": [{
                "id": "win-main",
                "title": "PandaMUX",
                "focused": true,
            }],
        }))),
        "window.focus" => {
            let id = opt_string(&request.params, "id")
                .or_else(|| opt_string(&request.params, "windowId"))
                .unwrap_or_else(|| "win-main".to_string());
            if id == "win-main" {
                Ok(Some(json!({ "ok": true, "id": id })))
            } else {
                Err((-32000, format!("window not found: {id}")))
            }
        }
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Surface color scheme (per-surface terminal theme override)
// ---------------------------------------------------------------------------

fn dispatch_surface_scheme(
    request: &RpcRequest,
    app: &AppState,
    themes: &ThemeStore,
    surface_schemes: &mut HashMap<SurfaceId, String>,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "surface.set_color_scheme" => {
            let surface_id = content_surface_id(app, &request.params)?;
            let scheme = opt_string(&request.params, "scheme")
                .or_else(|| opt_string(&request.params, "name"))
                .ok_or_else(|| {
                    (
                        -32602,
                        "surface.set_color_scheme requires scheme".to_string(),
                    )
                })?;
            if themes.get(&scheme).is_none() {
                return Err((-32000, format!("theme not found: {scheme}")));
            }
            surface_schemes.insert(surface_id, scheme.clone());
            Ok(Some(json!({ "ok": true, "scheme": scheme })))
        }
        "surface.clear_color_scheme" => {
            let surface_id = content_surface_id(app, &request.params)?;
            surface_schemes.remove(&surface_id);
            Ok(Some(json!({ "ok": true })))
        }
        _ => Ok(None),
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
            let panes = app
                .active_workspace()
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
            // Map through `agent_json` so list items carry `agentId` (the
            // orchestrator's monitoring loop reads it) and match `agent.spawn`.
            let agents_json: Vec<Value> = agents.list().iter().map(agent_json).collect();
            Ok(Some(json!({ "agents": agents_json })))
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

    // Tag the surface with its session type so restores and the rail/type
    // grouping see the agent as an agent (spec 2.2).
    let _ = app.apply(AppIntent::Surface(SurfaceIntent::SetSessionType {
        workspace_id: Some(workspace_id.clone()),
        surface_id: surface_id.clone(),
        session: session_type_for_command(&command),
    }));

    // Mint the id before spawning so the child carries PANDAMUX_AGENT_ID (the
    // orchestrator's on-agent-stop / on-tool-use hooks key per-agent state on it).
    let agent_id = agents.next_id();
    if spawn_ptys {
        let pty_command = parse_command(&command, cwd.clone())
            .with_env(pandamux_env(&surface_id.to_string(), Some(&agent_id)));
        ptys.spawn(surface_id.to_string(), &pty_command, DEFAULT_GRID_SIZE)
            .map_err(|error| (-32000, error.to_string()))?;
    }

    let info = AgentInfo {
        id: agent_id,
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

/// The `PANDAMUX_*` environment injected into every spawned shell/agent so
/// shell-integration scripts, the CLI, and the orchestrator hooks can find the
/// pipe and identify their surface/agent. Ported from the env vars the Electron
/// build set on spawned shells (`PANDAMUX`, `PANDAMUX_SURFACE_ID`,
/// `PANDAMUX_PIPE`), plus `PANDAMUX_AGENT_ID` for agent surfaces.
pub(crate) fn pandamux_env(surface_id: &str, agent_id: Option<&str>) -> Vec<(String, String)> {
    let pipe = std::env::var("PANDAMUX_PIPE").unwrap_or_else(|_| r"\\.\pipe\pandamux".to_string());
    let mut env = vec![
        ("PANDAMUX".to_string(), "1".to_string()),
        ("PANDAMUX_SURFACE_ID".to_string(), surface_id.to_string()),
        ("PANDAMUX_PIPE".to_string(), pipe),
    ];
    if let Some(agent_id) = agent_id {
        env.push(("PANDAMUX_AGENT_ID".to_string(), agent_id.to_string()));
    }
    env
}

/// Classify an agent command line into a session type for badges/restores.
fn session_type_for_command(command: &str) -> pandamux_core::SessionType {
    let program = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".exe")
        .trim_end_matches(".cmd")
        .to_ascii_lowercase();
    match program.as_str() {
        "claude" => pandamux_core::SessionType::Claude,
        "codex" => pandamux_core::SessionType::Codex,
        "gemini" => pandamux_core::SessionType::Gemini,
        _ => pandamux_core::SessionType::Custom {
            command: command.to_string(),
        },
    }
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
        // Alias: the orchestrator's spawn-agents.sh reads `.agentId`.
        "agentId": info.id,
        "label": info.label,
        "workspaceId": info.workspace_id,
        "paneId": info.pane_id,
        "surfaceId": info.surface_id,
        "status": info.status,
        "command": info.command,
    })
}

// ---------------------------------------------------------------------------
// Project launcher operations
// ---------------------------------------------------------------------------

fn dispatch_projects(
    request: &RpcRequest,
    app: &mut AppState,
    ptys: &mut PtySessionManager,
    remotes: &mut RemoteSessionManager,
    remote_configs: &mut HashMap<SurfaceId, SshConfig>,
    ssh_profiles: &SshProfiles,
    spawn_ptys: bool,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        // Registry-shaped listing (spec 1.4): one entry per project identity,
        // its workspaces nested (sessions from any host group under it).
        // Legacy workspaces without an identity are listed separately.
        "project.list" => Ok(Some(json!({
            "projects": app.projects.iter().map(|record| json!({
                "projectId": record.id,
                "name": record.name,
                "manual": record.manual,
                "matchers": record.matchers,
                "workspaces": app.workspaces.iter()
                    .filter(|workspace| workspace.project_id.as_ref() == Some(&record.id))
                    .map(|workspace| json!({
                        "workspaceId": workspace.id,
                        "title": workspace.title,
                        "location": workspace.project.location,
                    }))
                    .collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "unassignedWorkspaces": app.workspaces.iter()
                .filter(|workspace| workspace.project_id.is_none())
                .map(|workspace| json!({
                    "workspaceId": workspace.id,
                    "title": workspace.title,
                    "location": workspace.project.location,
                }))
                .collect::<Vec<_>>(),
        }))),
        "project.create" => {
            let location_value = request
                .params
                .get("location")
                .cloned()
                .unwrap_or_else(|| request.params.clone());
            let location: ProjectLocation = serde_json::from_value(location_value)
                .map_err(|error| (-32602, format!("invalid Project location: {error}")))?;
            let session = session_type_param(&request.params)?;
            let result = launch_project_location(
                app,
                ptys,
                remotes,
                remote_configs,
                ssh_profiles,
                location,
                request
                    .params
                    .get("trustUnknownHost")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                spawn_ptys,
                &session,
            );
            Ok(Some(project_launch_result(result)))
        }
        "project.add_session" => {
            let workspace_id: WorkspaceId = required_id(&request.params, &["workspaceId", "id"])?;
            let location = app
                .workspace(&workspace_id)
                .ok_or_else(|| (-32000, format!("workspace not found: {workspace_id}")))?
                .project
                .location
                .clone();
            if matches!(location, ProjectLocation::Legacy) {
                return Ok(Some(project_error_result(ProjectError::new(
                    "legacy_project_needs_folder",
                    pandamux_core::ProjectErrorCategory::Validation,
                    "Legacy Projects require folder selection before adding a session",
                    true,
                ))));
            }
            let session = session_type_param(&request.params)?;
            let result = launch_project_location(
                app,
                ptys,
                remotes,
                remote_configs,
                ssh_profiles,
                location,
                false,
                spawn_ptys,
                &session,
            );
            Ok(Some(project_launch_result(result)))
        }
        "ssh.folder.list" => {
            let profile_id: SshProfileId = required_id(&request.params, &["profileId", "id"])?;
            let profile = ssh_profiles
                .get(&profile_id)
                .ok_or_else(|| (-32000, format!("SSH profile not found: {profile_id}")))?;
            let path = opt_string(&request.params, "path").unwrap_or_else(|| "/".to_string());
            let config = match crate::project_launcher::ssh_config(
                profile,
                path.clone(),
                None,
                request
                    .params
                    .get("trustUnknownHost")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ) {
                Ok(config) => config,
                Err(error) => return Ok(Some(project_error_result(error))),
            };
            let result = remotes
                .browse_folders_blocking(config, path, std::time::Duration::from_secs(30))
                .map(|listing| {
                    let canonical_path = listing.canonical_path;
                    json!({
                        "canonicalPath": canonical_path,
                        "parentPath": pandamux_core::posix_parent(&canonical_path),
                        "breadcrumbs": pandamux_core::posix_breadcrumbs(&canonical_path),
                        "directories": listing.directories.into_iter().map(|entry| json!({
                            "name": entry.name,
                            "canonicalPath": entry.canonical_path,
                        })).collect::<Vec<_>>(),
                    })
                });
            match result {
                Ok(listing) => Ok(Some(json!({ "ok": true, "listing": listing }))),
                Err(error) => Ok(Some(project_error_result(
                    crate::project_launcher::project_error_from_ssh(error),
                ))),
            }
        }
        _ => Ok(None),
    }
}

fn launch_project_location(
    app: &mut AppState,
    ptys: &mut PtySessionManager,
    remotes: &mut RemoteSessionManager,
    remote_configs: &mut HashMap<SurfaceId, SshConfig>,
    ssh_profiles: &SshProfiles,
    location: ProjectLocation,
    trust_unknown_host: bool,
    spawn_ptys: bool,
    session: &pandamux_core::SessionType,
) -> Result<crate::project_launcher::LaunchSuccess, ProjectError> {
    match location {
        ProjectLocation::Local { cwd, .. } => crate::project_launcher::launch_local(
            app,
            ptys,
            cwd,
            spawn_ptys,
            DEFAULT_GRID_SIZE,
            session,
        ),
        ProjectLocation::Ssh {
            profile_id,
            remote_cwd,
        } => {
            let profile = ssh_profiles.get(&profile_id).ok_or_else(|| {
                ProjectError::new(
                    "ssh_profile_missing",
                    pandamux_core::ProjectErrorCategory::ProfileMissing,
                    format!("SSH profile not found: {profile_id}"),
                    false,
                )
            })?;
            crate::project_launcher::launch_remote_blocking(
                app,
                remotes,
                remote_configs,
                profile,
                remote_cwd,
                None,
                trust_unknown_host,
                spawn_ptys,
                DEFAULT_GRID_SIZE,
                session,
            )
        }
        ProjectLocation::Legacy => Err(ProjectError::new(
            "legacy_project_needs_folder",
            pandamux_core::ProjectErrorCategory::Validation,
            "Legacy Projects require folder selection",
            true,
        )),
    }
}

fn project_launch_result(
    result: Result<crate::project_launcher::LaunchSuccess, ProjectError>,
) -> Value {
    match result {
        Ok(success) => json!({
            "ok": true,
            "workspaceId": success.workspace_id,
            "paneId": success.pane_id,
            "surfaceId": success.surface_id,
            "reusedProject": success.reused_project,
        }),
        Err(error) => project_error_result(error),
    }
}

fn project_error_result(error: ProjectError) -> Value {
    json!({ "ok": false, "error": error })
}

// ---------------------------------------------------------------------------
// SSH remote surfaces (plan F2 / F3)
// ---------------------------------------------------------------------------

fn dispatch_ssh(
    request: &RpcRequest,
    app: &mut AppState,
    remotes: &mut RemoteSessionManager,
    remote_configs: &mut HashMap<SurfaceId, SshConfig>,
    ssh_profiles: &mut SshProfiles,
    spawn_ptys: bool,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "ssh.connect" => {
            let config = ssh_config_from_params(&request.params)?;
            let workspace_id = opt_id(&request.params, "workspaceId")
                .or_else(|| app.active_workspace_id.clone())
                .ok_or_else(|| (-32000, "no workspace is open".to_string()))?;
            let pane_id = opt_id(&request.params, "paneId").or_else(|| {
                app.workspace(&workspace_id)
                    .and_then(|workspace| workspace.focused_pane_id.clone())
            });
            let surface_id = SurfaceId::generate();
            // Preserve the historical RPC meaning, but prestart before the core
            // mutation so an async connection failure cannot leave ghost state.
            if spawn_ptys {
                remotes
                    .connect_ready(
                        surface_id.to_string(),
                        config.clone(),
                        DEFAULT_GRID_SIZE,
                        std::time::Duration::from_secs(30),
                    )
                    .map_err(|error| (-32000, error))?;
            }
            let delta = app
                .apply(AppIntent::Surface(SurfaceIntent::CreateWithId {
                    workspace_id,
                    pane_id,
                    surface_id: surface_id.clone(),
                    surface_type: SurfaceType::Terminal,
                }))
                .map_err(|message| {
                    if spawn_ptys {
                        let _ = remotes.kill(surface_id.as_str());
                    }
                    (-32000, message)
                })?;
            let (workspace_id, pane_id, surface_id) = match delta {
                AppDelta::SurfaceCreated {
                    workspace_id,
                    pane_id,
                    surface,
                } => (workspace_id, pane_id, surface.id),
                _ => return Err((-32000, "remote surface was not created".to_string())),
            };
            remote_configs.insert(surface_id.clone(), config.clone());
            Ok(Some(json!({
                "ok": true,
                "surfaceId": surface_id,
                "paneId": pane_id,
                "workspaceId": workspace_id,
                "host": config.host,
            })))
        }
        "ssh.disconnect" => {
            let surface_id: SurfaceId = required_id(&request.params, &["surfaceId", "id"])?;
            if spawn_ptys && remotes.has(surface_id.as_str()) {
                let _ = remotes.kill(surface_id.as_str());
            }
            remote_configs.remove(&surface_id);
            let _ = app.apply(AppIntent::Surface(SurfaceIntent::Close {
                workspace_id: None,
                surface_id: surface_id.clone(),
            }));
            Ok(Some(json!({ "ok": true })))
        }
        "ssh.list" => {
            let sessions: Vec<Value> = remote_configs
                .iter()
                .map(|(surface_id, config)| {
                    json!({
                        "surfaceId": surface_id,
                        "host": config.host,
                        "port": config.port,
                        "user": config.user,
                        "running": remotes.has(surface_id.as_str()),
                    })
                })
                .collect();
            Ok(Some(json!({ "sessions": sessions })))
        }
        "ssh.profiles" | "ssh.profile.list" => Ok(Some(json!({ "profiles": ssh_profiles.list() }))),
        "ssh.save_profile" | "ssh.profile.save" => {
            let mut profile = ssh_profile_from_params(&request.params)?;
            if request.method == "ssh.save_profile"
                && request.params.get("profileId").is_none()
                && request.params.get("id").is_none()
                && let Some(existing) = ssh_profiles.get_by_name(&profile.name)
            {
                profile.id = existing.id.clone();
            }
            let name = profile.name.clone();
            if ssh_profiles.has_duplicate_name(&profile.name, Some(&profile.id)) {
                return Err((
                    -32602,
                    format!("SSH profile name already exists: {}", profile.name),
                ));
            }
            let id = profile.id.clone();
            ssh_profiles.upsert(profile);
            Ok(Some(json!({ "ok": true, "id": id, "name": name })))
        }
        "ssh.remove_profile" => {
            let name = opt_string(&request.params, "name")
                .ok_or_else(|| (-32602, "ssh.remove_profile requires name".to_string()))?;
            let removed = ssh_profiles.remove_by_name(&name);
            Ok(Some(json!({ "ok": removed })))
        }
        "ssh.profile.remove" => {
            let id: SshProfileId = required_id(&request.params, &["profileId", "id"])?;
            let removed = ssh_profiles.remove(&id);
            Ok(Some(json!({ "ok": removed })))
        }
        "ssh.import_config" | "ssh.profile.import_config" => {
            let content = match opt_string(&request.params, "content") {
                Some(content) => content,
                None => {
                    let path = opt_string(&request.params, "path").ok_or_else(|| {
                        (
                            -32602,
                            "ssh.import_config requires content or path".to_string(),
                        )
                    })?;
                    std::fs::read_to_string(&path)
                        .map_err(|error| (-32000, format!("read {path}: {error}")))?
                }
            };
            let names = ssh_profiles.import_config(&content);
            Ok(Some(json!({ "imported": names })))
        }
        _ => Ok(None),
    }
}

fn ssh_config_from_params(params: &Value) -> Result<SshConfig, (i32, String)> {
    let host = opt_string(params, "host")
        .ok_or_else(|| (-32602, "ssh.connect requires host".to_string()))?;
    let user = opt_string(params, "user")
        .ok_or_else(|| (-32602, "ssh.connect requires user".to_string()))?;
    let port = params
        .get("port")
        .and_then(Value::as_u64)
        .map(|port| port as u16)
        .unwrap_or(22);
    let auth = ssh_auth_from_params(params)?;
    Ok(SshConfig::new(host, user, auth).with_port(port))
}

/// Parse the runtime auth (`SshAuth`, which may carry a secret) from connect
/// params. Defaults to the Windows OpenSSH-compatible agent pipe.
fn ssh_auth_from_params(params: &Value) -> Result<SshAuth, (i32, String)> {
    match opt_string(params, "auth").as_deref().unwrap_or("agent") {
        "agent" => Ok(SshAuth::Agent {
            pipe_path: opt_string(params, "pipePath")
                .unwrap_or_else(|| r"\\.\pipe\openssh-ssh-agent".to_string()),
        }),
        "key" | "keyfile" => Ok(SshAuth::KeyFile {
            path: opt_string(params, "keyPath")
                .ok_or_else(|| (-32602, "key auth requires keyPath".to_string()))?
                .into(),
            passphrase: opt_string(params, "passphrase"),
        }),
        "password" => Ok(SshAuth::Password {
            password: opt_string(params, "password")
                .ok_or_else(|| (-32602, "password auth requires password".to_string()))?,
        }),
        other => Err((-32602, format!("unsupported ssh auth: {other}"))),
    }
}

/// Parse a saved (secretless) host profile from params.
fn ssh_profile_from_params(params: &Value) -> Result<SshHostProfile, (i32, String)> {
    let host = opt_string(params, "host")
        .ok_or_else(|| (-32602, "ssh.save_profile requires host".to_string()))?;
    let user = opt_string(params, "user")
        .ok_or_else(|| (-32602, "ssh.save_profile requires user".to_string()))?;
    let name = opt_string(params, "name").unwrap_or_else(|| host.clone());
    let mut profile = SshHostProfile::new(name, host, user);
    if let Some(id) = opt_id::<SshProfileId>(params, "profileId").or_else(|| opt_id(params, "id")) {
        profile.id = id;
    }
    if let Some(port) = params.get("port").and_then(Value::as_u64) {
        profile.port = port as u16;
    }
    profile.auth = match opt_string(params, "auth").as_deref().unwrap_or("agent") {
        "agent" => SshAuthConfig::Agent,
        "key" | "keyfile" => SshAuthConfig::KeyFile {
            path: opt_string(params, "keyPath")
                .ok_or_else(|| (-32602, "key auth requires keyPath".to_string()))?,
        },
        "password" => SshAuthConfig::Password,
        other => return Err((-32602, format!("unsupported ssh auth: {other}"))),
    };
    profile.jump = opt_string(params, "jump");
    Ok(profile)
}

// ---------------------------------------------------------------------------
// Clipboard (plan F1)
// ---------------------------------------------------------------------------

fn dispatch_clipboard(
    request: &RpcRequest,
    clipboard_config: &mut ClipboardConfig,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "clipboard.copy" => {
            let text = opt_string(&request.params, "text").unwrap_or_default();
            if text.len() > clipboard_config.max_store_bytes {
                return Err((-32000, "clipboard payload exceeds size cap".to_string()));
            }
            crate::clipboard_os::set_text(&text).map_err(|error| (-32000, error))?;
            Ok(Some(json!({ "ok": true })))
        }
        "clipboard.get" => {
            let text = crate::clipboard_os::get_text().map_err(|error| (-32000, error))?;
            Ok(Some(json!({ "text": text })))
        }
        "clipboard.policy" => {
            // Read (no fields) or update (maxStoreBytes / host+allowLoad).
            if let Some(max) = request.params.get("maxStoreBytes").and_then(Value::as_u64) {
                clipboard_config.max_store_bytes = max as usize;
            }
            if let Some(host) = opt_string(&request.params, "host") {
                let allow = request
                    .params
                    .get("allowLoad")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if allow {
                    clipboard_config.allow_load(host);
                } else {
                    clipboard_config.deny_load(&host);
                }
            }
            Ok(Some(json!({
                "maxStoreBytes": clipboard_config.max_store_bytes,
                "loadAllowedHosts": clipboard_config.load_allowed_hosts,
            })))
        }
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Terminal I/O (routes local-PTY and SSH-remote surfaces uniformly)
// ---------------------------------------------------------------------------

fn dispatch_terminal_io(
    request: &RpcRequest,
    app: &AppState,
    ptys: &mut PtySessionManager,
    remotes: &mut RemoteSessionManager,
    remote_configs: &HashMap<SurfaceId, SshConfig>,
    spawn_ptys: bool,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "surface.send_text" => {
            sync_terminal_sessions(app, ptys, remotes, spawn_ptys)
                .map_err(|message| (-32000, message))?;
            let target = resolve_io_target(app, ptys, remote_configs, &request.params)?;
            let text = opt_string(&request.params, "text").unwrap_or_default();
            write_target(ptys, remotes, &target, text.as_bytes())?;
            Ok(Some(json!({ "ok": true })))
        }
        "surface.send_key" => {
            sync_terminal_sessions(app, ptys, remotes, spawn_ptys)
                .map_err(|message| (-32000, message))?;
            let target = resolve_io_target(app, ptys, remote_configs, &request.params)?;
            let bytes = key_bytes(&request.params)?;
            write_target(ptys, remotes, &target, &bytes)?;
            Ok(Some(json!({ "ok": true })))
        }
        // Bracketed-paste-aware paste (plan F1): wrap the text in
        // ESC[200~..ESC[201~ when the target has requested bracketed paste.
        "surface.paste" => {
            sync_terminal_sessions(app, ptys, remotes, spawn_ptys)
                .map_err(|message| (-32000, message))?;
            let target = resolve_io_target(app, ptys, remote_configs, &request.params)?;
            let text = opt_string(&request.params, "text").unwrap_or_default();
            let bracketed = match &target {
                IoTarget::Remote(id) => remotes.bracketed_paste_active(id.as_str()),
                IoTarget::Local(id) => ptys.bracketed_paste_active(id.as_str()),
            };
            let bytes = wrap_paste(text.as_bytes(), bracketed);
            write_target(ptys, remotes, &target, &bytes)?;
            Ok(Some(json!({ "ok": true, "bracketed": bracketed })))
        }
        // Transfer a local image to the remote host over SFTP (plan F3) and
        // inject its path, or inject a local path for a local surface.
        "surface.paste_image" => {
            sync_terminal_sessions(app, ptys, remotes, spawn_ptys)
                .map_err(|message| (-32000, message))?;
            let path = opt_string(&request.params, "path")
                .ok_or_else(|| (-32602, "surface.paste_image requires path".to_string()))?;
            let target = resolve_io_target(app, ptys, remote_configs, &request.params)?;
            let injected = match &target {
                IoTarget::Remote(id) => remotes
                    .upload_image(id.as_str(), &path)
                    .map_err(|error| (-32000, error))?,
                IoTarget::Local(_) => path.clone(),
            };
            // Inject the path plus a trailing space (Claude Code accepts image
            // file paths in prompts).
            let payload = format!("{injected} ");
            write_target(ptys, remotes, &target, payload.as_bytes())?;
            Ok(Some(json!({ "ok": true, "path": injected })))
        }
        "surface.read_text" => {
            sync_terminal_sessions(app, ptys, remotes, spawn_ptys)
                .map_err(|message| (-32000, message))?;
            let target = resolve_io_target(app, ptys, remote_configs, &request.params)?;
            let lines = request
                .params
                .get("lines")
                .and_then(Value::as_u64)
                .unwrap_or(50) as usize;
            let text = match &target {
                IoTarget::Remote(id) => remotes
                    .screen_text_lines(id.as_str(), lines)
                    .map_err(|error| (-32000, error))?,
                IoTarget::Local(id) => ptys
                    .screen_text_lines(id.as_str(), lines)
                    .map_err(|error| (-32000, error.to_string()))?,
            };
            Ok(Some(json!({ "text": text })))
        }
        "surface.resize" | "pty.resize" => {
            sync_terminal_sessions(app, ptys, remotes, spawn_ptys)
                .map_err(|message| (-32000, message))?;
            let target = resolve_io_target(app, ptys, remote_configs, &request.params)?;
            let size = grid_size_param(&request.params)?;
            match &target {
                IoTarget::Remote(id) => remotes
                    .resize(id.as_str(), size)
                    .map_err(|error| (-32000, error))?,
                IoTarget::Local(id) => ptys
                    .resize(id.as_str(), size)
                    .map_err(|error| (-32000, error.to_string()))?,
            }
            Ok(Some(json!({ "ok": true })))
        }
        "surface.kill" | "pty.kill" => {
            let target = resolve_io_target(app, ptys, remote_configs, &request.params)?;
            match &target {
                IoTarget::Remote(id) => {
                    remotes.kill(id.as_str()).map_err(|error| (-32000, error))?
                }
                IoTarget::Local(id) => ptys
                    .kill(id.as_str())
                    .map_err(|error| (-32000, error.to_string()))?,
            }
            Ok(Some(json!({ "ok": true })))
        }
        "surface.trigger_flash" => Ok(Some(json!({ "ok": true }))),
        _ => Ok(None),
    }
}

/// A resolved terminal-I/O target: either a local PTY or an SSH remote surface.
enum IoTarget {
    Local(SurfaceId),
    Remote(SurfaceId),
}

/// Resolve which surface a terminal-I/O method targets and whether it is remote.
/// An explicit id that names a remote surface routes to SSH; otherwise fall back
/// to the focused/any local terminal.
fn resolve_io_target(
    app: &AppState,
    ptys: &PtySessionManager,
    remote_configs: &HashMap<SurfaceId, SshConfig>,
    params: &Value,
) -> Result<IoTarget, (i32, String)> {
    if let Some(surface_id) =
        opt_id::<SurfaceId>(params, "surfaceId").or_else(|| opt_id(params, "id"))
    {
        if remote_configs.contains_key(&surface_id) {
            return Ok(IoTarget::Remote(surface_id));
        }
        if ptys.has(surface_id.as_str()) {
            return Ok(IoTarget::Local(surface_id));
        }
        return Err((-32000, format!("terminal surface not found: {surface_id}")));
    }
    let surface_id = resolve_terminal_surface_id(app, ptys, params)?;
    Ok(IoTarget::Local(surface_id))
}

fn write_target(
    ptys: &mut PtySessionManager,
    remotes: &mut RemoteSessionManager,
    target: &IoTarget,
    bytes: &[u8],
) -> Result<(), (i32, String)> {
    match target {
        IoTarget::Remote(id) => remotes
            .write_all(id.as_str(), bytes)
            .map_err(|error| (-32000, error)),
        IoTarget::Local(id) => ptys
            .write_all(id.as_str(), bytes)
            .map_err(|error| (-32000, error.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Surface content (markdown / diff)
// ---------------------------------------------------------------------------

fn dispatch_surface_content(
    request: &RpcRequest,
    app: &AppState,
    contents: &mut SurfaceContents,
) -> Result<Option<Value>, (i32, String)> {
    match request.method.as_str() {
        "markdown.set_content" | "diff.set_content" => {
            let surface_id = content_surface_id(app, &request.params)?;
            let content = opt_string(&request.params, "content").unwrap_or_default();
            contents.set(surface_id, content);
            Ok(Some(json!({ "ok": true })))
        }
        // `load_file`/`refresh` accept an inline `content` (preferred: the CLI
        // reads the file client-side) or a server-readable `path`. Reading the
        // file here is a small, one-shot blocking read on the sync dispatch path
        // (consistent with the PTY spawns it already performs), not a hot loop.
        "markdown.load_file" | "diff.refresh" => {
            let surface_id = content_surface_id(app, &request.params)?;
            let content = match opt_string(&request.params, "content") {
                Some(content) => content,
                None => {
                    let path = opt_string(&request.params, "path").ok_or_else(|| {
                        (
                            -32602,
                            format!("{} requires content or path", request.method),
                        )
                    })?;
                    std::fs::read_to_string(&path)
                        .map_err(|error| (-32000, format!("read {path}: {error}")))?
                }
            };
            contents.set(surface_id, content);
            Ok(Some(json!({ "ok": true })))
        }
        _ => Ok(None),
    }
}

/// Resolve the required surface id for a content method. Content surfaces are
/// always addressed explicitly (there is no "focused content surface" fallback).
fn content_surface_id(app: &AppState, params: &Value) -> Result<SurfaceId, (i32, String)> {
    let surface_id: SurfaceId = opt_id(params, "surfaceId")
        .or_else(|| opt_id(params, "id"))
        .ok_or_else(|| (-32602, "content method requires surfaceId".to_string()))?;
    if all_surface_ids(app).contains(&surface_id) {
        Ok(surface_id)
    } else {
        Err((-32000, format!("surface not found: {surface_id}")))
    }
}

/// Every surface id across all workspaces (terminal and non-terminal).
fn all_surface_ids(app: &AppState) -> HashSet<SurfaceId> {
    let mut ids = HashSet::new();
    for workspace in &app.workspaces {
        collect_surface_ids(&workspace.split_tree, &mut ids);
    }
    ids
}

fn collect_surface_ids(tree: &SplitNode, ids: &mut HashSet<SurfaceId>) {
    match tree {
        SplitNode::Leaf(leaf) => {
            for surface in &leaf.surfaces {
                ids.insert(surface.id.clone());
            }
        }
        SplitNode::Branch(branch) => {
            collect_surface_ids(&branch.children[0], ids);
            collect_surface_ids(&branch.children[1], ids);
        }
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
        "surface.move" => Ok(AppIntent::Surface(SurfaceIntent::Move {
            workspace_id: opt_id(&request.params, "workspaceId"),
            surface_id: required_id(&request.params, &["surfaceId", "id"])?,
            target_pane_id: required_id(&request.params, &["targetPaneId", "paneId"])?,
            zone: parse_zone(&request.params)?,
        })),
        "surface.list" => Ok(AppIntent::Surface(SurfaceIntent::List {
            workspace_id: opt_id(&request.params, "workspaceId"),
            pane_id: opt_id(&request.params, "paneId"),
        })),
        "surface.rename" => Ok(AppIntent::Surface(SurfaceIntent::Rename {
            workspace_id: opt_id(&request.params, "workspaceId"),
            surface_id: required_id(&request.params, &["id", "surfaceId"])?,
            name: opt_string(&request.params, "name"),
        })),
        "surface.set_session_type" => Ok(AppIntent::Surface(SurfaceIntent::SetSessionType {
            workspace_id: opt_id(&request.params, "workspaceId"),
            surface_id: required_id(&request.params, &["id", "surfaceId"])?,
            session: session_type_param(&request.params)?,
        })),
        "workspace.close_all" => Ok(AppIntent::Workspace(WorkspaceIntent::CloseAll {
            project_id: opt_id(&request.params, "projectId"),
        })),
        "project.registry" => Ok(AppIntent::Project(pandamux_core::ProjectIntent::List)),
        "project.rename" => Ok(AppIntent::Project(pandamux_core::ProjectIntent::Rename {
            project_id: required_id(&request.params, &["id", "projectId"])?,
            name: opt_string(&request.params, "name")
                .ok_or_else(|| (-32602, "project.rename requires name".to_string()))?,
        })),
        "project.merge" => Ok(AppIntent::Project(pandamux_core::ProjectIntent::Merge {
            source: required_id(&request.params, &["source", "sourceProjectId"])?,
            target: required_id(&request.params, &["target", "targetProjectId"])?,
        })),
        "project.split" => Ok(AppIntent::Project(pandamux_core::ProjectIntent::Split {
            workspace_id: required_id(&request.params, &["workspaceId", "id"])?,
        })),
        _ => Err((-32601, format!("Method not found: {}", request.method))),
    }
}

/// Parse an optional session type from params ("session": tagged object or a
/// plain string like "claude"). Missing means Terminal.
fn session_type_param(params: &Value) -> Result<pandamux_core::SessionType, (i32, String)> {
    let Some(value) = params.get("session").or_else(|| params.get("sessionType")) else {
        return Ok(pandamux_core::SessionType::default());
    };
    if let Some(short) = value.as_str() {
        return Ok(match short {
            "terminal" => pandamux_core::SessionType::Terminal,
            "claude" => pandamux_core::SessionType::Claude,
            "codex" => pandamux_core::SessionType::Codex,
            "gemini" => pandamux_core::SessionType::Gemini,
            other => {
                return Err((-32602, format!("unknown session type: {other}")));
            }
        });
    }
    serde_json::from_value(value.clone())
        .map_err(|error| (-32602, format!("invalid session type: {error}")))
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

fn parse_zone(params: &Value) -> Result<DropZone, (i32, String)> {
    let zone = opt_string(params, "zone").unwrap_or_else(|| "center".to_string());
    DropZone::parse(&zone).ok_or_else(|| (-32602, format!("unsupported drop zone: {zone}")))
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
        AppDelta::WorkspacesClosed { workspace_ids } => json!({
            "ok": true,
            "closedWorkspaceIds": workspace_ids,
        }),
        AppDelta::WorkspaceListReported { workspaces } => json!({ "workspaces": workspaces }),
        AppDelta::ProjectListReported { projects } => json!({ "projects": projects }),
        AppDelta::ProjectRenamed { project_id, name } => json!({
            "ok": true,
            "projectId": project_id,
            "name": name,
        }),
        AppDelta::ProjectsMerged { source, target } => json!({
            "ok": true,
            "source": source,
            "target": target,
        }),
        AppDelta::ProjectSplit {
            workspace_id,
            project_id,
        } => json!({
            "ok": true,
            "workspaceId": workspace_id,
            "projectId": project_id,
        }),
        AppDelta::MatcherAttached { project_id } => json!({
            "ok": true,
            "projectId": project_id,
        }),
        AppDelta::SurfaceRenamed {
            workspace_id,
            surface_id,
            name,
        } => json!({
            "ok": true,
            "workspaceId": workspace_id,
            "surfaceId": surface_id,
            "name": name,
        }),
        AppDelta::SurfaceSessionTypeSet {
            workspace_id,
            surface_id,
            session,
        } => json!({
            "ok": true,
            "workspaceId": workspace_id,
            "surfaceId": surface_id,
            "session": session,
        }),
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
        AppDelta::SurfaceMoved { workspace_id, tree } => json!({
            "workspaceId": workspace_id,
            "tree": tree,
            "ok": true,
        }),
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

/// Ensure a live PTY exists for every local terminal surface and kill orphaned
/// ones. Remote (SSH-backed) surfaces are skipped: their byte source is a
/// [`RemoteSessionManager`] session, not a local PTY. No-op unless `spawn_ptys`
/// is set (tests/smoke stay hermetic).
pub fn sync_terminal_sessions(
    app: &AppState,
    ptys: &mut PtySessionManager,
    remotes: &RemoteSessionManager,
    spawn_ptys: bool,
) -> Result<(), String> {
    if !spawn_ptys {
        return Ok(());
    }

    let mut expected_session_ids = HashSet::new();
    for workspace in &app.workspaces {
        for surface in terminal_surfaces(&workspace.split_tree) {
            let session_id = surface.id.to_string();
            if matches!(workspace.project.location, ProjectLocation::Ssh { .. }) {
                continue;
            }
            // A remote surface has an SSH session, not a local PTY: leave it be.
            if remotes.has(&session_id) {
                continue;
            }
            expected_session_ids.insert(session_id.clone());
            if ptys.has(&session_id) {
                continue;
            }
            // Session-type-aware respawn: a restored Claude tab comes back as
            // Claude, not a bare shell (spec 2.2).
            let session = surface.session.clone().unwrap_or_default();
            let command = match &workspace.project.location {
                ProjectLocation::Local { cwd, shell } => crate::project_launcher::spawn_spec(
                    &session,
                    shell,
                    Some(cwd.clone()),
                    &session_id,
                ),
                ProjectLocation::Legacy => crate::project_launcher::spawn_spec(
                    &session,
                    &workspace.shell,
                    None,
                    &session_id,
                ),
                ProjectLocation::Ssh { .. } => unreachable!(),
            };
            ptys.spawn(session_id, &command, DEFAULT_GRID_SIZE)
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

/// Kill SSH remote sessions whose surface was closed, and forget their config.
/// `live` is the set of all surface ids still present in the tree.
pub fn sync_remote_sessions(
    live: &HashSet<SurfaceId>,
    remotes: &mut RemoteSessionManager,
    remote_configs: &mut HashMap<SurfaceId, SshConfig>,
) {
    let orphaned: Vec<SurfaceId> = remote_configs
        .keys()
        .filter(|surface_id| !live.contains(*surface_id))
        .cloned()
        .collect();
    for surface_id in orphaned {
        if remotes.has(surface_id.as_str()) {
            let _ = remotes.kill(surface_id.as_str());
        }
        remote_configs.remove(&surface_id);
    }
}

pub fn terminal_surface_ids(tree: &SplitNode) -> Vec<SurfaceId> {
    terminal_surfaces(tree)
        .into_iter()
        .map(|surface| surface.id)
        .collect()
}

/// Every terminal [`pandamux_core::SurfaceRef`] in the tree, depth-first.
/// Carries the session type + name so respawn paths restore agents as agents.
pub fn terminal_surfaces(tree: &SplitNode) -> Vec<pandamux_core::SurfaceRef> {
    match tree {
        SplitNode::Leaf(leaf) => leaf
            .surfaces
            .iter()
            .filter(|surface| surface.surface_type == SurfaceType::Terminal)
            .cloned()
            .collect(),
        SplitNode::Branch(branch) => {
            let mut surfaces = terminal_surfaces(&branch.children[0]);
            surfaces.extend(terminal_surfaces(&branch.children[1]));
            surfaces
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

    let workspace_id = opt_id(params, "workspaceId")
        .or_else(|| app.active_workspace_id.clone())
        .ok_or_else(|| (-32000, "no workspace is open".to_string()))?;
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
    fn project_rpc_registry_rename_and_surface_rename() {
        let mut backend = Backend::new(false);
        // Build a project workspace directly: the local launch path needs a
        // real shell on the box, and the surface under test here is the
        // registry + rename RPC, not spawning.
        let workspace_id = WorkspaceId::generate();
        let surface_id = SurfaceId::generate();
        backend
            .app
            .apply(AppIntent::Workspace(WorkspaceIntent::CreateProject {
                workspace_id: workspace_id.clone(),
                pane_id: PaneId::generate(),
                surface_id: surface_id.clone(),
                title: "Repo".to_string(),
                shell: "pwsh".to_string(),
                project: pandamux_core::ProjectSpec {
                    location: ProjectLocation::Local {
                        cwd: "C:\\Dev\\Repo".to_string(),
                        shell: "pwsh".to_string(),
                    },
                },
            }))
            .expect("create project workspace");
        pandamux_core::ensure_project_registry(&mut backend.app, 1);

        // project.list is registry-shaped: one project entry with a name and
        // its workspaces nested.
        let parsed = handle(
            &mut backend,
            r#"{"method":"project.list","params":{},"id":2}"#,
        );
        let projects = parsed["result"]["projects"].as_array().unwrap().clone();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["name"], "Repo");
        let project_id = projects[0]["projectId"].as_str().unwrap().to_string();
        assert_eq!(projects[0]["workspaces"].as_array().unwrap().len(), 1);

        // Rename the project and the surface over the pipe.
        let parsed = handle(
            &mut backend,
            &format!(
                r#"{{"method":"project.rename","params":{{"projectId":"{project_id}","name":"Better Name"}},"id":3}}"#
            ),
        );
        assert_eq!(parsed["result"]["ok"], true, "reply = {parsed}");
        assert_eq!(backend.app.projects[0].name, "Better Name");
        assert!(backend.app.projects[0].manual);

        let parsed = handle(
            &mut backend,
            &format!(
                r#"{{"method":"surface.rename","params":{{"surfaceId":"{surface_id}","name":"Claude: auth"}},"id":4}}"#
            ),
        );
        assert_eq!(parsed["result"]["ok"], true, "reply = {parsed}");

        // surface.set_session_type tags the surface (short string form).
        let parsed = handle(
            &mut backend,
            &format!(
                r#"{{"method":"surface.set_session_type","params":{{"surfaceId":"{surface_id}","session":"claude"}},"id":5}}"#
            ),
        );
        assert_eq!(parsed["result"]["ok"], true, "reply = {parsed}");
        let workspace = backend.app.workspace(&workspace_id).unwrap();
        let surfaces = terminal_surfaces(&workspace.split_tree);
        assert_eq!(surfaces[0].name.as_deref(), Some("Claude: auth"));
        assert_eq!(
            surfaces[0].session,
            Some(pandamux_core::SessionType::Claude)
        );
    }

    #[test]
    fn workspace_close_all_over_the_pipe_reaches_empty_state() {
        let mut backend = Backend::new(false);
        let parsed = handle(
            &mut backend,
            r#"{"method":"workspace.close_all","params":{},"id":1}"#,
        );
        assert_eq!(parsed["result"]["ok"], true);
        assert!(backend.app.workspaces.is_empty());
        // Follow-up calls error cleanly, never panic.
        let parsed = handle(&mut backend, r#"{"method":"pane.list","params":{},"id":2}"#);
        assert_eq!(parsed["error"]["code"], -32000);
    }

    #[test]
    fn config_get_returns_whole_settings_and_dotted_keys() {
        let mut backend = Backend::new(false);
        let parsed = handle(
            &mut backend,
            r#"{"method":"config.get","params":{},"id":1}"#,
        );
        assert_eq!(
            parsed["result"]["settings"]["terminal"]["scrollbackLines"],
            10_000
        );
        let parsed = handle(
            &mut backend,
            r#"{"method":"config.get","params":{"key":"terminal.scrollbackLines"},"id":2}"#,
        );
        assert_eq!(parsed["result"]["value"], 10_000);
    }

    #[test]
    fn config_set_mutates_and_rejects_bad_keys() {
        let mut backend = Backend::new(false);
        let parsed = handle(
            &mut backend,
            r#"{"method":"config.set","params":{"key":"terminal.scrollbackLines","value":50000},"id":1}"#,
        );
        assert_eq!(parsed["result"]["ok"], true);
        assert_eq!(parsed["result"]["value"], 50_000);
        assert_eq!(backend.settings.terminal.scrollback_lines, 50_000);

        let parsed = handle(
            &mut backend,
            r#"{"method":"config.set","params":{"key":"terminal.scrollbak","value":1},"id":2}"#,
        );
        assert_eq!(parsed["error"]["code"], -32602);
        let parsed = handle(
            &mut backend,
            r#"{"method":"config.set","params":{"key":"terminal.scrollbackLines","value":"many"},"id":3}"#,
        );
        assert_eq!(parsed["error"]["code"], -32602);
        assert_eq!(backend.settings.terminal.scrollback_lines, 50_000);
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
    fn sidebar_status_progress_log_and_get_state() {
        let mut backend = Backend::new(false);
        handle(
            &mut backend,
            r#"{"method":"sidebar.set_status","params":{"key":"branch","value":"master"},"id":50}"#,
        );
        handle(
            &mut backend,
            r#"{"method":"sidebar.set_progress","params":{"value":42,"label":"wave 1"},"id":51}"#,
        );
        handle(
            &mut backend,
            r#"{"method":"sidebar.log","params":{"level":"info","message":"spawned"},"id":52}"#,
        );

        let state = handle(
            &mut backend,
            r#"{"method":"sidebar.get_state","params":{},"id":53}"#,
        );
        let result = &state["result"];
        assert_eq!(result["statuses"][0]["key"], "branch");
        assert_eq!(result["statuses"][0]["value"], "master");
        assert_eq!(result["progress"]["value"], 42);
        assert_eq!(result["progress"]["label"], "wave 1");
        assert_eq!(result["logs"][0]["message"], "spawned");
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

    #[test]
    fn report_pwd_v1_line_is_accepted() {
        let mut backend = Backend::new(false);
        // The V1 shell-integration cwd report writes no reply and does not error
        // (there is no live PTY session in this hermetic test, so it is a no-op).
        assert_eq!(backend.handle_line("report_pwd surf-1 C:\\Users\\chaz"), "");
    }

    #[test]
    fn sets_reads_and_prunes_surface_content() {
        let mut backend = Backend::new(false);
        // Create a markdown surface in the default pane.
        let create = handle(
            &mut backend,
            r#"{"method":"surface.create","params":{"paneId":"pane-default","type":"markdown"},"id":1}"#,
        );
        let surface_id = create["result"]["surfaceId"]
            .as_str()
            .expect("surface id")
            .to_string();

        // Set + read back the content.
        let set = handle(
            &mut backend,
            &format!(
                r##"{{"method":"markdown.set_content","params":{{"id":"{surface_id}","content":"# Hi"}},"id":2}}"##
            ),
        );
        assert_eq!(set["result"]["ok"], true);
        assert_eq!(
            backend.contents.get(&SurfaceId::from(surface_id.as_str())),
            Some("# Hi")
        );

        // An unknown surface is rejected.
        let missing = handle(
            &mut backend,
            r#"{"method":"markdown.set_content","params":{"id":"surf-nope","content":"x"},"id":3}"#,
        );
        assert_eq!(missing["error"]["code"], -32000);

        // Closing the surface (after adding a sibling so it is not the last one)
        // prunes its stored content.
        handle(
            &mut backend,
            r#"{"method":"surface.create","params":{"paneId":"pane-default","type":"terminal"},"id":4}"#,
        );
        let close = handle(
            &mut backend,
            &format!(r#"{{"method":"surface.close","params":{{"id":"{surface_id}"}},"id":5}}"#),
        );
        assert_eq!(close["result"]["ok"], true);
        assert!(
            backend
                .contents
                .get(&SurfaceId::from(surface_id.as_str()))
                .is_none()
        );
    }

    #[test]
    fn themes_config_and_i18n_roundtrip() {
        let mut backend = Backend::new(false);

        // Import a Ghostty theme, list, and select it.
        let imported = handle(
            &mut backend,
            r#"{"method":"config.import_ghostty","params":{"name":"Dracula","content":"background = #282a36\npalette = 1=#ff5555\n"},"id":1}"#,
        );
        assert_eq!(imported["result"]["name"], "Dracula");
        let list = handle(
            &mut backend,
            r#"{"method":"theme.list","params":{},"id":2}"#,
        );
        assert!(
            list["result"]["themes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|name| name == "Dracula")
        );
        let selected = handle(
            &mut backend,
            r#"{"method":"theme.select","params":{"name":"Dracula"},"id":3}"#,
        );
        assert_eq!(selected["result"]["active"], "Dracula");
        let unknown = handle(
            &mut backend,
            r#"{"method":"theme.select","params":{"name":"nope"},"id":4}"#,
        );
        assert_eq!(unknown["error"]["code"], -32000);

        // Windows Terminal import (the content is JSON-in-JSON containing "#).
        let wt = handle(
            &mut backend,
            r##"{"method":"config.import_windows_terminal","params":{"content":"{\"schemes\":[{\"name\":\"WT\",\"background\":\"#000000\",\"red\":\"#ff0000\"}]}"},"id":5}"##,
        );
        assert_eq!(wt["result"]["imported"][0], "WT");

        // config.show reflects the active theme + locale.
        let show = handle(
            &mut backend,
            r#"{"method":"config.show","params":{},"id":6}"#,
        );
        assert_eq!(show["result"]["activeTheme"], "Dracula");
        assert_eq!(show["result"]["locale"], "en");

        // i18n: set locale then translate.
        let locale = handle(
            &mut backend,
            r#"{"method":"i18n.set_locale","params":{"locale":"fr"},"id":7}"#,
        );
        assert_eq!(locale["result"]["locale"], "fr");
        let translated = handle(
            &mut backend,
            r#"{"method":"i18n.translate","params":{"key":"settings"},"id":8}"#,
        );
        assert_eq!(translated["result"]["text"], "Paramètres");
    }

    #[test]
    fn windows_list_focus_and_surface_color_scheme() {
        let mut backend = Backend::new(false);

        // Single-window build: list + focus.
        let list = handle(
            &mut backend,
            r#"{"method":"window.list","params":{},"id":1}"#,
        );
        assert_eq!(list["result"]["windows"][0]["id"], "win-main");
        let focus = handle(
            &mut backend,
            r#"{"method":"window.focus","params":{"id":"win-main"},"id":2}"#,
        );
        assert_eq!(focus["result"]["ok"], true);
        let missing = handle(
            &mut backend,
            r#"{"method":"window.focus","params":{"id":"win-x"},"id":3}"#,
        );
        assert_eq!(missing["error"]["code"], -32000);

        // Per-surface color scheme: import a theme, then set/clear it on the
        // default terminal surface.
        handle(
            &mut backend,
            r#"{"method":"config.import_ghostty","params":{"name":"Neon","content":"background = #010203\n"},"id":4}"#,
        );
        let surfaces = handle(
            &mut backend,
            r#"{"method":"surface.list","params":{},"id":5}"#,
        );
        let surface_id = surfaces["result"]["surfaces"][0]["id"]
            .as_str()
            .expect("surface id")
            .to_string();

        let set = handle(
            &mut backend,
            &format!(
                r#"{{"method":"surface.set_color_scheme","params":{{"surfaceId":"{surface_id}","scheme":"Neon"}},"id":6}}"#
            ),
        );
        assert_eq!(set["result"]["scheme"], "Neon");
        assert_eq!(
            backend
                .surface_schemes
                .get(&SurfaceId::from(surface_id.as_str()))
                .map(String::as_str),
            Some("Neon")
        );

        // Unknown theme is rejected.
        let bad = handle(
            &mut backend,
            &format!(
                r#"{{"method":"surface.set_color_scheme","params":{{"surfaceId":"{surface_id}","scheme":"Nope"}},"id":7}}"#
            ),
        );
        assert_eq!(bad["error"]["code"], -32000);

        // Clear removes the override.
        let clear = handle(
            &mut backend,
            &format!(
                r#"{{"method":"surface.clear_color_scheme","params":{{"surfaceId":"{surface_id}"}},"id":8}}"#
            ),
        );
        assert_eq!(clear["result"]["ok"], true);
        assert!(backend.surface_schemes.is_empty());
    }

    /// Wire-compat regression guard for the pandamux-orchestrator plugin. Replays
    /// the exact pipe methods its scripts invoke (via the `pandamux` CLI) and
    /// asserts every response field its `json-tool.js` parser reads. If any of
    /// these shapes drift, scripted orchestration silently breaks.
    #[test]
    fn orchestrator_command_sequence_shapes_match() {
        let mut backend = Backend::new(false);

        // detect-pandamux.sh / spawn-agents.sh: `pandamux ping` -> exact "pong".
        assert_eq!(backend.handle_line("ping"), "pong");

        // spawn-agents.sh: `pandamux layout grid --count 3` -> result.newPaneIds[].
        let grid = handle(
            &mut backend,
            r#"{"method":"layout.grid","params":{"count":3,"type":"terminal"},"id":1}"#,
        );
        let pane_ids = grid["result"]["newPaneIds"]
            .as_array()
            .expect("layout.grid must return newPaneIds");
        assert_eq!(pane_ids.len(), 2);
        let anchor_pane = pane_ids[0].as_str().expect("pane id string").to_string();

        // spawn-agents.sh: `pandamux agent spawn` -> result.agentId + result.surfaceId.
        let spawn = handle(
            &mut backend,
            &format!(
                r#"{{"method":"agent.spawn","params":{{"cmd":"claude work","label":"w1","pane":"{anchor_pane}"}},"id":2}}"#
            ),
        );
        let agent_id = spawn["result"]["agentId"]
            .as_str()
            .expect("agent.spawn must return agentId")
            .to_string();
        assert!(
            spawn["result"]["surfaceId"].as_str().is_some(),
            "agent.spawn must return surfaceId"
        );

        // orchestrate SKILL monitoring loop: `pandamux agent list` -> agents[].status (+ agentId).
        let list = handle(
            &mut backend,
            r#"{"method":"agent.list","params":{},"id":3}"#,
        );
        let agents = list["result"]["agents"]
            .as_array()
            .expect("agent.list must return agents");
        assert_eq!(agents.len(), 1);
        assert!(
            agents[0]["status"].as_str().is_some(),
            "agent.list items must carry status"
        );
        assert_eq!(agents[0]["agentId"].as_str(), Some(agent_id.as_str()));

        // on-agent-stop.sh: `pandamux notify` -> ok.
        let notify = handle(
            &mut backend,
            r#"{"method":"notification.raise","params":{"title":"All agents complete"},"id":4}"#,
        );
        assert_eq!(notify["result"]["ok"], true);

        // orchestrator coordination surface: sidebar set-status / set-progress / log.
        for line in [
            r#"{"method":"sidebar.set_status","params":{"key":"wave","value":"1"},"id":5}"#,
            r#"{"method":"sidebar.set_progress","params":{"value":50,"label":"wave 1"},"id":6}"#,
            r#"{"method":"sidebar.log","params":{"level":"info","message":"spawned"},"id":7}"#,
        ] {
            assert_eq!(handle(&mut backend, line)["result"]["ok"], true);
        }

        // cleanup: `pandamux agent kill <id>`.
        let kill = handle(
            &mut backend,
            &format!(r#"{{"method":"agent.kill","params":{{"id":"{agent_id}"}},"id":8}}"#),
        );
        assert_eq!(kill["result"]["ok"], true);
    }

    #[test]
    fn ssh_connect_creates_remote_surface_lists_and_disconnects() {
        // spawn_ptys=false: records the remote surface without opening a socket.
        let mut backend = Backend::new(false);
        let connect = handle(
            &mut backend,
            r#"{"method":"ssh.connect","params":{"host":"10.55.88.48","user":"chaz","auth":"agent"},"id":1}"#,
        );
        assert_eq!(connect["result"]["ok"], true);
        assert_eq!(connect["result"]["host"], "10.55.88.48");
        let surface_id = connect["result"]["surfaceId"]
            .as_str()
            .expect("remote surface id")
            .to_string();

        let list = handle(&mut backend, r#"{"method":"ssh.list","params":{},"id":2}"#);
        let sessions = list["result"]["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["surfaceId"], surface_id);
        assert_eq!(sessions[0]["host"], "10.55.88.48");
        // Not "running" because no socket was opened in the hermetic backend.
        assert_eq!(sessions[0]["running"], false);

        let disconnect = handle(
            &mut backend,
            &format!(
                r#"{{"method":"ssh.disconnect","params":{{"surfaceId":"{surface_id}"}},"id":3}}"#
            ),
        );
        assert_eq!(disconnect["result"]["ok"], true);
        let list = handle(&mut backend, r#"{"method":"ssh.list","params":{},"id":4}"#);
        assert!(list["result"]["sessions"].as_array().unwrap().is_empty());
    }

    #[test]
    fn ssh_connect_requires_host_and_user() {
        let mut backend = Backend::new(false);
        let missing = handle(
            &mut backend,
            r#"{"method":"ssh.connect","params":{"user":"chaz"},"id":1}"#,
        );
        assert_eq!(missing["error"]["code"], -32602);
    }

    #[test]
    fn ssh_profiles_save_import_and_list() {
        let mut backend = Backend::new(false);
        let saved = handle(
            &mut backend,
            r#"{"method":"ssh.save_profile","params":{"name":"galahad","host":"10.55.88.48","user":"chaz","port":22,"auth":"agent"},"id":1}"#,
        );
        assert_eq!(saved["result"]["name"], "galahad");

        let imported = handle(
            &mut backend,
            r#"{"method":"ssh.import_config","params":{"content":"Host box\n  HostName box.local\n  User root\n"},"id":2}"#,
        );
        assert_eq!(imported["result"]["imported"][0], "box");

        let profiles = handle(
            &mut backend,
            r#"{"method":"ssh.profiles","params":{},"id":3}"#,
        );
        assert_eq!(profiles["result"]["profiles"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn clipboard_policy_get_and_update() {
        let mut backend = Backend::new(false);
        // Read defaults.
        let policy = handle(
            &mut backend,
            r#"{"method":"clipboard.policy","params":{},"id":1}"#,
        );
        assert!(policy["result"]["maxStoreBytes"].as_u64().unwrap() > 0);
        assert!(
            policy["result"]["loadAllowedHosts"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        // Opt a host into clipboard-read, then update the size cap.
        let allowed = handle(
            &mut backend,
            r#"{"method":"clipboard.policy","params":{"host":"galahad","allowLoad":true},"id":2}"#,
        );
        assert_eq!(allowed["result"]["loadAllowedHosts"][0], "galahad");
        let capped = handle(
            &mut backend,
            r#"{"method":"clipboard.policy","params":{"maxStoreBytes":2048},"id":3}"#,
        );
        assert_eq!(capped["result"]["maxStoreBytes"], 2048);
    }

    #[test]
    fn remote_surface_is_not_given_a_local_pty_on_reconcile() {
        // Even in a "live" backend, a remote surface must be skipped by the local
        // PTY reconciler (its bytes come from SSH). We assert the config is
        // tracked and the surface exists without asserting socket behavior.
        let mut backend = Backend::new(false);
        let connect = handle(
            &mut backend,
            r#"{"method":"ssh.connect","params":{"host":"h","user":"u"},"id":1}"#,
        );
        let surface_id = SurfaceId::from(connect["result"]["surfaceId"].as_str().unwrap());
        assert!(backend.remote_configs.contains_key(&surface_id));
        assert!(!backend.ptys.has(surface_id.as_str()));
    }
}
