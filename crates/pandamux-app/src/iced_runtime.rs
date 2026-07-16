use crate::persistence::{SessionStore, SshProfileConfig, SshProfileStore};
use crate::project_launcher::{EphemeralCredential, LaunchTarget};
use iced::futures::SinkExt;
use iced::{Element, Size, Subscription, Task, Theme, application, keyboard, stream, time, window};
use pandamux_core::{
    AgentRegistry, AppIntent, AppState, ClipboardConfig, Localizer, NewNotification,
    NotificationSource, Notifications, PaneId, PaneIntent, ProjectError, ProjectErrorCategory,
    ProjectLocation, SidebarState, SplitDirection, SplitNode, SplitPaneParams, SshProfileId,
    SshProfiles, SurfaceContents, SurfaceId, SurfaceIntent, SurfaceType, ThemeStore, WorkspaceId,
    WorkspaceIntent, find_leaf, find_pane_id_for_surface, get_all_pane_ids, parse_ghostty_theme,
};
use pandamux_term::{
    DEFAULT_GRID_SIZE, GridSize, PtyCommand, PtySessionManager, RemoteSessionManager, RemoteStatus,
    SearchOptions, SshConfig, detect_links, search_lines,
};
use pandamux_ui::{
    ChromeState, DragView, FindViewState, LauncherStep, LinkSpan, NotificationCard,
    NotificationsViewState, Overlay, PaletteItem, PaletteViewState, QuickLaunchViewState, RailItem,
    SessionActivity, SessionLauncherViewState, SessionsViewState, SettingsSection,
    SettingsViewState, ShellKind, ShellMessage, ShellViewModel, SshProfileForm, TermScheme,
    TerminalSnapshot, UiTheme, app_view, filter_items, project_sessions_with_profiles,
    project_workspace_shell, shell_view,
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
/// Interval between GitHub release update checks (6 hours).
const UPDATE_CHECK_INTERVAL_SECS: u64 = 6 * 60 * 60;

pub struct NativeShellRuntime {
    app_state: AppState,
    ptys: PtySessionManager,
    live_ptys: bool,
    chrome: ChromeState,
    tick: u64,
    store: SessionStore,
    profile_store: SshProfileStore,
    profile_config: SshProfileConfig,
    profile_store_available: bool,
    find: FindViewState,
    find_matches: Vec<(usize, usize, usize)>,
    notifications: Notifications,
    notifications_open: bool,
    notif_seq: u64,
    /// Live agents spawned via the pipe (CLI / orchestrator).
    agents: AgentRegistry,
    /// Sidebar status/progress/log written via the pipe (CLI / orchestrator).
    sidebar: SidebarState,
    /// Markdown/diff surface content set via the pipe (CLI / orchestrator).
    contents: SurfaceContents,
    /// Loaded terminal themes (bundled `.theme` files + imports) and the active one.
    themes: ThemeStore,
    /// Localization state (set via the pipe / CLI `set-locale`).
    localizer: Localizer,
    /// Per-surface terminal color-scheme overrides (surface id -> theme name).
    surface_schemes: HashMap<SurfaceId, String>,
    /// SSH remote terminal sessions (plan F2).
    remotes: RemoteSessionManager,
    /// Which surfaces are SSH-remote and how to reach them.
    remote_configs: HashMap<SurfaceId, SshConfig>,
    /// Saved SSH host profiles.
    ssh_profiles: SshProfiles,
    credential_cache: HashMap<SshProfileId, EphemeralCredential>,
    launcher: SessionLauncherViewState,
    launcher_trust_unknown: bool,
    pending_remote_launch: Option<PendingRemoteLaunch>,
    pending_plus_workspace: Option<WorkspaceId>,
    /// Persistent clipboard policy (plan F1).
    clipboard_config: ClipboardConfig,
    /// Active drag-and-drop of a tab, if any (plan Section 12.3).
    drag: Option<DragView>,
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
    /// The last release version we raised an update toast for (dedupes the
    /// periodic check so the same version is not toasted repeatedly).
    last_update_offer: Option<String>,
    /// Whether the launch-time update check has been kicked yet.
    update_checked_once: bool,
}

#[derive(Clone, Debug)]
struct PendingRemoteLaunch {
    target: LaunchTarget,
    config: SshConfig,
}

impl Default for NativeShellRuntime {
    fn default() -> Self {
        Self::new(false)
    }
}

impl NativeShellRuntime {
    pub fn new(live_ptys: bool) -> Self {
        let store = SessionStore::new(SessionStore::default_dir());
        let profile_store = SshProfileStore::new(SshProfileStore::default_dir());
        let (profile_config, profile_load_error, profile_store_available) = if live_ptys {
            match profile_store.load() {
                Ok(config) => (config, None, true),
                Err(error) => (SshProfileConfig::default(), Some(error.to_string()), false),
            }
        } else {
            (SshProfileConfig::default(), None, true)
        };
        let ssh_profiles = profile_config.registry();
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
            profile_store,
            profile_config,
            profile_store_available,
            find: FindViewState::default(),
            find_matches: Vec::new(),
            notifications: Notifications::new(),
            notifications_open: false,
            notif_seq: 0,
            agents: AgentRegistry::new(),
            sidebar: SidebarState::new(),
            contents: SurfaceContents::new(),
            themes: ThemeStore::new(),
            localizer: Localizer::default(),
            surface_schemes: HashMap::new(),
            remotes: RemoteSessionManager::default(),
            remote_configs: HashMap::new(),
            ssh_profiles,
            credential_cache: HashMap::new(),
            launcher: SessionLauncherViewState::default(),
            launcher_trust_unknown: false,
            pending_remote_launch: None,
            pending_plus_workspace: None,
            clipboard_config: ClipboardConfig::default(),
            drag: None,
            copy_mode: false,
            palette: PaletteViewState::default(),
            settings_section: SettingsSection::default(),
            view_model,
            terminals: Vec::new(),
            last_error: profile_load_error,
            pipe_name: std::env::var("PANDAMUX_PIPE")
                .unwrap_or_else(|_| r"\\.\pipe\pandamux".to_string()),
            pipe_registry: Arc::new(StdMutex::new(HashMap::new())),
            pipe_seq: Arc::new(AtomicU64::new(1)),
            last_update_offer: None,
            update_checked_once: false,
        };
        if live_ptys {
            runtime.load_bundled_themes();
            runtime.raise_notification(NewNotification {
                workspace_id: None,
                surface_id: None,
                title: "PandaMUX is running".to_string(),
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

    /// Load bundled `.theme` files (Ghostty color format) into the theme store.
    /// One-time, at startup on the UI thread, so it never runs on an async worker.
    fn load_bundled_themes(&mut self) {
        let Some(dir) = themes_dir() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("theme")
                && let Ok(content) = std::fs::read_to_string(&path)
            {
                let name = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("theme")
                    .to_string();
                self.themes.insert(parse_ghostty_theme(name, &content));
            }
        }
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
        if overlay == Overlay::QuickLaunch {
            self.launcher = SessionLauncherViewState {
                profiles: self.ssh_profiles.list().to_vec(),
                ..SessionLauncherViewState::default()
            };
            self.launcher_trust_unknown = false;
            self.pending_plus_workspace = None;
        }
        self.chrome.active_overlay = overlay;
    }

    fn save_profiles(&mut self) {
        if !self.profile_store_available {
            self.last_error = Some(
                "SSH profile file needs repair before connection changes can be saved".to_string(),
            );
            return;
        }
        self.profile_config.set_registry(&self.ssh_profiles);
        if self.live_ptys
            && let Err(error) = self.profile_store.save(&self.profile_config)
        {
            self.last_error = Some(format!("save SSH profiles: {error}"));
        }
        self.launcher.profiles = self.ssh_profiles.list().to_vec();
    }

    fn launcher_folder_task(&mut self) -> Task<ShellMessage> {
        self.launcher.loading = true;
        self.launcher.error = None;
        self.view_model.launcher = self.launcher.clone();
        let path = self.launcher.path.clone();
        if self.launcher.remote {
            let Some(profile_id) = self.launcher.selected_profile_id.clone() else {
                return Task::none();
            };
            let Some(profile) = self.ssh_profiles.get(&profile_id).cloned() else {
                self.launcher.loading = false;
                self.launcher.error = Some(ProjectError::new(
                    "ssh_profile_missing",
                    ProjectErrorCategory::ProfileMissing,
                    "The selected SSH profile no longer exists",
                    false,
                ));
                return Task::none();
            };
            let credential = self.credential_cache.get(&profile_id);
            let config = match crate::project_launcher::ssh_config(
                &profile,
                path.clone(),
                credential,
                self.launcher_trust_unknown,
            ) {
                Ok(config) => config,
                Err(error) => {
                    self.launcher.loading = false;
                    self.launcher.error = Some(error);
                    return Task::none();
                }
            };
            Task::perform(
                crate::project_launcher::list_remote_folders(config, path),
                ShellMessage::LauncherFolderLoaded,
            )
        } else {
            Task::perform(
                crate::project_launcher::list_local_folders(path),
                ShellMessage::LauncherFolderLoaded,
            )
        }
    }

    fn select_launcher_profile(&mut self, profile_id: SshProfileId) -> Task<ShellMessage> {
        self.launcher.selected_profile_id = Some(profile_id.clone());
        self.launcher.remote = true;
        self.launcher.error = None;
        let Some(profile) = self.ssh_profiles.get(&profile_id) else {
            return Task::none();
        };
        if matches!(profile.auth, pandamux_core::SshAuthConfig::Password)
            && !self.credential_cache.contains_key(&profile_id)
        {
            self.launcher.step = LauncherStep::Credential;
            self.view_model.launcher = self.launcher.clone();
            return Task::none();
        }
        self.launcher.step = LauncherStep::Folder;
        // "." canonicalizes to the SFTP login home, so a first browse starts at
        // the user's home folder rather than the filesystem root.
        self.launcher.path = self
            .profile_config
            .last_selected_folder_by_profile
            .get(&profile_id)
            .cloned()
            .unwrap_or_else(|| ".".to_string());
        self.launcher_folder_task()
    }

    fn start_remote_launch(
        &mut self,
        profile_id: SshProfileId,
        remote_cwd: String,
    ) -> Result<(), ProjectError> {
        if self.pending_remote_launch.is_some() {
            return Ok(());
        }
        let profile = self.ssh_profiles.get(&profile_id).cloned().ok_or_else(|| {
            ProjectError::new(
                "ssh_profile_missing",
                ProjectErrorCategory::ProfileMissing,
                format!("SSH profile not found: {profile_id}"),
                false,
            )
        })?;
        let location = ProjectLocation::Ssh {
            profile_id: profile_id.clone(),
            remote_cwd: remote_cwd.clone(),
        };
        let target = crate::project_launcher::prepare_launch(&self.app_state, location)?;
        let config = crate::project_launcher::ssh_config(
            &profile,
            remote_cwd,
            self.credential_cache.get(&profile_id),
            self.launcher_trust_unknown,
        )?;
        self.remotes
            .connect(
                target.surface_id.to_string(),
                config.clone(),
                DEFAULT_GRID_SIZE,
            )
            .map_err(|message| {
                ProjectError::new(
                    "ssh_pty_start_failed",
                    ProjectErrorCategory::PtyStart,
                    message,
                    true,
                )
            })?;
        self.launcher.step = LauncherStep::Launching;
        self.launcher.launching = true;
        self.pending_remote_launch = Some(PendingRemoteLaunch { target, config });
        Ok(())
    }

    fn start_selected_folder(&mut self) {
        if self.launcher.launching {
            return;
        }
        let Some(listing) = self.launcher.listing.as_ref() else {
            return;
        };
        self.launcher.launching = true;
        let path = listing.canonical_path.clone();
        let result = if self.launcher.remote {
            let Some(profile_id) = self.launcher.selected_profile_id.clone() else {
                return;
            };
            self.start_remote_launch(profile_id, path)
        } else {
            crate::project_launcher::launch_local(
                &mut self.app_state,
                &mut self.ptys,
                path.clone(),
                self.live_ptys,
            )
            .map(|_| {
                self.profile_config.last_selected_local_folder = Some(path);
                self.save_profiles();
                if self.live_ptys {
                    let _ = self.store.save_session(&self.app_state);
                }
                self.chrome.active_overlay = Overlay::None;
            })
        };
        if let Err(error) = result {
            self.launcher.error = Some(error);
            self.launcher.step = LauncherStep::Folder;
            self.launcher.launching = false;
        }
        self.view_model.launcher = self.launcher.clone();
    }

    fn poll_pending_remote_launch(&mut self) {
        let Some(pending) = self.pending_remote_launch.clone() else {
            return;
        };
        let id = pending.target.surface_id.as_str();
        let _ = self.remotes.poll(id);
        match self.remotes.status(id) {
            Some(RemoteStatus::Ready) => {
                match crate::project_launcher::commit_prestarted(
                    &mut self.app_state,
                    &pending.target,
                    "ssh",
                ) {
                    Ok(_) => {
                        self.remote_configs
                            .insert(pending.target.surface_id.clone(), pending.config);
                        if let ProjectLocation::Ssh {
                            profile_id,
                            remote_cwd,
                        } = &pending.target.location
                        {
                            self.profile_config
                                .last_selected_folder_by_profile
                                .insert(profile_id.clone(), remote_cwd.clone());
                            self.save_profiles();
                        }
                        if self.live_ptys {
                            let _ = self.store.save_session(&self.app_state);
                        }
                        self.chrome.active_overlay = Overlay::None;
                        self.launcher.launching = false;
                        self.pending_remote_launch = None;
                        self.pending_plus_workspace = None;
                    }
                    Err(error) => {
                        let _ = self.remotes.kill(id);
                        self.launcher.error = Some(error);
                        self.launcher.step = LauncherStep::Folder;
                        self.launcher.launching = false;
                        self.pending_remote_launch = None;
                    }
                }
            }
            Some(RemoteStatus::Failed | RemoteStatus::Closed) | None => {
                let message = self
                    .remotes
                    .last_error(id)
                    .unwrap_or("SSH terminal failed before ready")
                    .to_string();
                let _ = self.remotes.kill(id);
                self.launcher.error = Some(ProjectError::new(
                    "ssh_pty_start_failed",
                    ProjectErrorCategory::PtyStart,
                    message,
                    true,
                ));
                self.launcher.step = LauncherStep::Folder;
                self.launcher.launching = false;
                self.pending_remote_launch = None;
            }
            _ => {}
        }
    }

    fn add_project_session(&mut self, workspace_id: WorkspaceId) {
        if self.pending_remote_launch.is_some() {
            return;
        }
        let Some(location) = self
            .app_state
            .workspace(&workspace_id)
            .map(|workspace| workspace.project.location.clone())
        else {
            return;
        };
        match location {
            ProjectLocation::Local { cwd, .. } => {
                if let Err(error) = crate::project_launcher::launch_local(
                    &mut self.app_state,
                    &mut self.ptys,
                    cwd,
                    self.live_ptys,
                ) {
                    self.last_error = Some(error.message);
                } else if self.live_ptys {
                    let _ = self.store.save_session(&self.app_state);
                }
            }
            ProjectLocation::Ssh {
                profile_id,
                remote_cwd,
            } => {
                let needs_password = self.ssh_profiles.get(&profile_id).is_some_and(|profile| {
                    matches!(profile.auth, pandamux_core::SshAuthConfig::Password)
                }) && !self.credential_cache.contains_key(&profile_id);
                if needs_password {
                    self.open_overlay(Overlay::QuickLaunch);
                    self.pending_plus_workspace = Some(workspace_id);
                    self.launcher.selected_profile_id = Some(profile_id);
                    self.launcher.remote = true;
                    self.launcher.step = LauncherStep::Credential;
                } else if let Err(error) = self.start_remote_launch(profile_id, remote_cwd) {
                    self.last_error = Some(error.message);
                }
            }
            ProjectLocation::Legacy => {
                self.open_overlay(Overlay::QuickLaunch);
            }
        }
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
            ShellMessage::LauncherLocalSelected => {
                self.launcher.remote = false;
                self.launcher.step = LauncherStep::Folder;
                self.launcher.path = self
                    .profile_config
                    .last_selected_local_folder
                    .clone()
                    .or_else(crate::project_launcher::local_home_folder)
                    .or_else(|| {
                        std::env::current_dir()
                            .ok()
                            .map(|path| path.to_string_lossy().to_string())
                    })
                    .unwrap_or_else(|| "C:\\".to_string());
                self.launcher_folder_task()
            }
            ShellMessage::LauncherFolderHome => {
                // "." canonicalizes to the SFTP login home on the remote side.
                self.launcher.path = if self.launcher.remote {
                    ".".to_string()
                } else {
                    crate::project_launcher::local_home_folder()
                        .unwrap_or_else(|| "C:\\".to_string())
                };
                self.launcher_folder_task()
            }
            ShellMessage::LauncherProfileSelected(profile_id) => {
                self.select_launcher_profile(profile_id)
            }
            ShellMessage::LauncherCredentialSubmit => {
                let Some(profile_id) = self.launcher.selected_profile_id.clone() else {
                    return Task::none();
                };
                let credential = if self.ssh_profiles.get(&profile_id).is_some_and(|profile| {
                    matches!(profile.auth, pandamux_core::SshAuthConfig::Password)
                }) {
                    EphemeralCredential::Password(std::mem::take(&mut self.launcher.credential))
                } else {
                    EphemeralCredential::KeyPassphrase(std::mem::take(
                        &mut self.launcher.credential,
                    ))
                };
                self.credential_cache.insert(profile_id.clone(), credential);
                if let Some(workspace_id) = self.pending_plus_workspace.clone()
                    && let Some(ProjectLocation::Ssh { remote_cwd, .. }) = self
                        .app_state
                        .workspace(&workspace_id)
                        .map(|workspace| workspace.project.location.clone())
                {
                    if let Err(error) = self.start_remote_launch(profile_id, remote_cwd) {
                        self.launcher.error = Some(error);
                    }
                    self.view_model.launcher = self.launcher.clone();
                    Task::none()
                } else {
                    self.launcher.step = LauncherStep::Folder;
                    self.launcher.path = self
                        .profile_config
                        .last_selected_folder_by_profile
                        .get(&profile_id)
                        .cloned()
                        .unwrap_or_else(|| ".".to_string());
                    self.launcher_folder_task()
                }
            }
            ShellMessage::LauncherFolderGo => self.launcher_folder_task(),
            ShellMessage::LauncherFolderNavigate(path) => {
                self.launcher.path = path;
                self.launcher_folder_task()
            }
            ShellMessage::LauncherHostTrustConfirmed => {
                self.launcher_trust_unknown = true;
                self.launcher.step = LauncherStep::Folder;
                self.launcher_folder_task()
            }
            ShellMessage::LauncherFolderSelected => {
                self.start_selected_folder();
                Task::none()
            }
            ShellMessage::LauncherProfileImport => {
                let path = std::env::var("USERPROFILE")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
                    .join(".ssh")
                    .join("config");
                Task::perform(
                    async move {
                        tokio::fs::read_to_string(&path).await.map_err(|error| {
                            ProjectError::new(
                                "ssh_config_import_failed",
                                ProjectErrorCategory::Filesystem,
                                format!("read {}: {error}", path.display()),
                                true,
                            )
                        })
                    },
                    ShellMessage::LauncherProfilesImported,
                )
            }
            ShellMessage::WindowDragStarted => window::latest().and_then(window::drag),
            ShellMessage::WindowMinimizePressed => {
                window::latest().and_then(|id| window::minimize(id, true))
            }
            ShellMessage::WindowMaximizeToggled => {
                window::latest().and_then(window::toggle_maximize)
            }
            ShellMessage::WindowClosePressed => window::latest().and_then(window::close),
            ShellMessage::PollRequested => {
                let cwd = self.focused_cwd();
                let poll = Task::perform(crate::pollers::poll_all(cwd), |result| {
                    ShellMessage::PollUpdate {
                        git_branch: result.git_branch,
                        git_ahead: result.git_ahead,
                        ports: result.ports,
                    }
                });
                // Kick a one-off update check shortly after launch (the first
                // status poll), then let the long-interval subscription drive it.
                if !self.update_checked_once && self.live_ptys {
                    self.update_checked_once = true;
                    Task::batch([poll, self.update_check_task()])
                } else {
                    poll
                }
            }
            ShellMessage::UpdateCheckRequested => self.update_check_task(),
            other => {
                self.update_shell(other);
                Task::none()
            }
        }
    }

    /// Build the async task that checks GitHub for a newer release and maps the
    /// result to an `UpdateAvailable` toast message (or `Noop` when there is
    /// nothing to offer). No-op off the live GUI build.
    fn update_check_task(&self) -> Task<ShellMessage> {
        if !self.live_ptys {
            return Task::none();
        }
        let current = env!("CARGO_PKG_VERSION").to_string();
        let now_unix = now_ms() / 1000;
        Task::perform(
            crate::updater::check_for_update(
                current,
                now_unix,
                crate::updater::DEFAULT_QUARANTINE_SECS,
            ),
            |found| match found {
                Some(release) => ShellMessage::UpdateAvailable {
                    version: release.version,
                    tag: release.tag,
                    url: release.installer_url,
                    notes: release.notes,
                },
                None => ShellMessage::Noop,
            },
        )
    }

    pub fn update_shell(&mut self, message: ShellMessage) {
        match message {
            ShellMessage::Tick => {
                self.tick = self.tick.wrapping_add(1);
                self.poll_pending_remote_launch();
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
            ShellMessage::ProjectSessionRequested(workspace_id) => {
                self.add_project_session(workspace_id);
            }
            ShellMessage::LauncherProfileAdd => {
                self.launcher.form = SshProfileForm::default();
                self.launcher.step = LauncherStep::ProfileForm;
            }
            ShellMessage::LauncherProfileEdit(profile_id) => {
                if let Some(profile) = self.ssh_profiles.get(&profile_id) {
                    self.launcher.form = SshProfileForm::from_profile(profile);
                    self.launcher.step = LauncherStep::ProfileForm;
                }
            }
            ShellMessage::LauncherProfileDelete(profile_id) => {
                self.ssh_profiles.remove(&profile_id);
                self.credential_cache.remove(&profile_id);
                self.save_profiles();
            }
            ShellMessage::LauncherProfilesImported(result) => match result {
                Ok(content) => {
                    self.ssh_profiles.import_config(&content);
                    self.save_profiles();
                }
                Err(error) => self.launcher.error = Some(error),
            },
            ShellMessage::LauncherProfileNameChanged(value) => self.launcher.form.name = value,
            ShellMessage::LauncherProfileHostChanged(value) => self.launcher.form.host = value,
            ShellMessage::LauncherProfilePortChanged(value) => self.launcher.form.port = value,
            ShellMessage::LauncherProfileAuthChanged(auth) => self.launcher.form.auth = auth,
            ShellMessage::LauncherIdentityFileChanged(value) => {
                self.launcher.form.identity_file = value.clone();
                self.launcher.form.auth = pandamux_core::SshAuthConfig::KeyFile { path: value };
            }
            ShellMessage::LauncherProfileSave => {
                let form = self.launcher.form.clone();
                let (user, host) = form.host.split_once('@').map_or_else(
                    || {
                        (
                            if form.user.is_empty() {
                                std::env::var("USERNAME").unwrap_or_else(|_| "root".to_string())
                            } else {
                                form.user.clone()
                            },
                            form.host.clone(),
                        )
                    },
                    |(user, host)| (user.to_string(), host.to_string()),
                );
                let id = form.id.unwrap_or_else(SshProfileId::generate);
                if self
                    .ssh_profiles
                    .has_duplicate_name(form.name.trim(), Some(&id))
                {
                    self.launcher.form.error = Some("Connection name already exists".to_string());
                } else if let Ok(port) = form.port.parse::<u16>() {
                    let auth = match form.auth {
                        pandamux_core::SshAuthConfig::KeyFile { .. } => {
                            pandamux_core::SshAuthConfig::KeyFile {
                                path: form.identity_file,
                            }
                        }
                        auth => auth,
                    };
                    self.ssh_profiles.upsert(pandamux_core::SshHostProfile {
                        id,
                        name: form.name.trim().to_string(),
                        host,
                        port,
                        user,
                        auth,
                        jump: None,
                    });
                    self.save_profiles();
                    self.launcher.step = LauncherStep::Connection;
                }
            }
            ShellMessage::LauncherCredentialChanged(value) => self.launcher.credential = value,
            ShellMessage::LauncherPathChanged(value) => self.launcher.path = value,
            ShellMessage::LauncherFolderLoaded(result) => {
                self.launcher.loading = false;
                match result {
                    Ok(listing) => {
                        self.launcher.path = listing.canonical_path.clone();
                        self.launcher.listing = Some(listing);
                        self.launcher.error = None;
                    }
                    Err(error) if error.category == ProjectErrorCategory::HostKeyUnknown => {
                        self.launcher.fingerprint = error.fingerprint.clone();
                        self.launcher.error = Some(error);
                        self.launcher.step = LauncherStep::HostConfirmation;
                    }
                    Err(error)
                        if error.category == ProjectErrorCategory::Authentication
                            && self
                                .launcher
                                .selected_profile_id
                                .as_ref()
                                .and_then(|id| self.ssh_profiles.get(id))
                                .is_some_and(|profile| {
                                    matches!(
                                        profile.auth,
                                        pandamux_core::SshAuthConfig::KeyFile { .. }
                                    )
                                }) =>
                    {
                        self.launcher.error = Some(error);
                        self.launcher.step = LauncherStep::Credential;
                    }
                    Err(error) => self.launcher.error = Some(error),
                }
            }
            ShellMessage::LauncherBack => {
                self.launcher.error = None;
                self.launcher.step = match self.launcher.step {
                    LauncherStep::ProfileForm | LauncherStep::Credential => {
                        LauncherStep::Connection
                    }
                    LauncherStep::HostConfirmation => LauncherStep::Folder,
                    LauncherStep::Folder => LauncherStep::Connection,
                    step => step,
                };
            }
            ShellMessage::LauncherLocalSelected
            | ShellMessage::LauncherProfileSelected(_)
            | ShellMessage::LauncherProfileImport
            | ShellMessage::LauncherCredentialSubmit
            | ShellMessage::LauncherHostTrustConfirmed
            | ShellMessage::LauncherFolderGo
            | ShellMessage::LauncherFolderHome
            | ShellMessage::LauncherFolderNavigate(_)
            | ShellMessage::LauncherFolderSelected => {
                // These messages return tasks from `update`.
            }
            ShellMessage::OverlayDismissed => {
                let had_overlay = self.chrome.active_overlay != Overlay::None;
                let had_drag = self.drag.is_some();
                self.chrome.active_overlay = Overlay::None;
                // Esc also cancels an in-flight drag.
                self.drag = None;
                // A bare Esc with nothing to dismiss goes to the terminal.
                if !had_overlay && !had_drag {
                    self.write_terminal_input(&[0x1b]);
                }
            }
            ShellMessage::TerminalInput(bytes) => {
                // Suppressed while an overlay is open; its own text inputs consume
                // typing (the global key subscription still fires in parallel).
                if self.chrome.active_overlay == Overlay::None {
                    self.write_terminal_input(&bytes);
                }
            }
            ShellMessage::TabDragArmed {
                surface_id,
                pane_id,
            } => {
                self.drag = Some(DragView {
                    surface_id,
                    source_pane_id: pane_id,
                    over: None,
                    active: false,
                });
            }
            ShellMessage::DragMoved => {
                if let Some(drag) = self.drag.as_mut() {
                    drag.active = true;
                }
            }
            ShellMessage::DragOverZone { pane_id, zone } => {
                // Only track a zone once the drag is active (moved).
                if let Some(drag) = self.drag.as_mut()
                    && drag.active
                {
                    drag.over = Some((pane_id, zone));
                }
            }
            ShellMessage::DragReleased => {
                if let Some(drag) = self.drag.take() {
                    // Over a zone -> move/split; otherwise the press-release was a
                    // plain tab click -> focus the surface.
                    let intent = match drag.over {
                        Some((target_pane_id, zone)) => AppIntent::Surface(SurfaceIntent::Move {
                            workspace_id: None,
                            surface_id: drag.surface_id,
                            target_pane_id,
                            zone,
                        }),
                        None => AppIntent::Surface(SurfaceIntent::Focus {
                            workspace_id: None,
                            surface_id: drag.surface_id,
                        }),
                    };
                    let result = self.app_state.apply(intent);
                    self.last_error = result.err();
                }
            }
            ShellMessage::PaletteQueryChanged(query) => {
                self.palette.query = query;
                self.palette.selected = 0;
            }
            ShellMessage::PaletteMoveSelection(delta) => {
                // With no overlay open, Up/Down are terminal history navigation.
                if self.chrome.active_overlay == Overlay::None {
                    let seq: &[u8] = if delta < 0 { b"\x1b[A" } else { b"\x1b[B" };
                    self.write_terminal_input(seq);
                    return;
                }
                // Otherwise arrows only navigate while the palette is open.
                if self.chrome.active_overlay != Overlay::CommandPalette {
                    return;
                }
                let count = self.palette.items.len();
                if count > 0 {
                    let index = (self.palette.selected as i64 + delta as i64)
                        .rem_euclid(count as i64) as usize;
                    self.palette.selected = index;
                }
            }
            ShellMessage::PaletteActivate => {
                // With no overlay open, plain Enter is a terminal carriage return.
                if self.chrome.active_overlay == Overlay::None {
                    self.write_terminal_input(b"\r");
                    return;
                }
                // Enter only activates a palette item while the palette is open.
                if self.chrome.active_overlay != Overlay::CommandPalette {
                    return;
                }
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
            ShellMessage::UpdateAvailable {
                version,
                tag,
                url,
                notes,
            } => {
                // Toast once per version; the periodic check re-emits otherwise.
                if self.last_update_offer.as_deref() != Some(version.as_str()) {
                    self.last_update_offer = Some(version.clone());
                    let first_line = notes.lines().find(|line| !line.trim().is_empty());
                    let body = match first_line {
                        Some(line) => format!("{} - {}", version, line.trim()),
                        None => format!("Version {version} is available."),
                    };
                    self.raise_notification(NewNotification {
                        workspace_id: None,
                        surface_id: None,
                        title: "Update available".to_string(),
                        body,
                        source: NotificationSource::Deploy,
                    });
                    // tag + url are consumed by the download-and-run step, which is
                    // wired when packaging lands; the checker already found them.
                    let _ = (tag, url);
                }
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
            ShellMessage::SplitFocused(direction) => {
                if let Some(pane_id) = self.focused_pane_id() {
                    self.update_shell(ShellMessage::PaneSplit { pane_id, direction });
                    return;
                }
            }
            ShellMessage::CloseFocusedPane => {
                if let Some(pane_id) = self.focused_pane_id() {
                    self.update_shell(ShellMessage::PaneClosed(pane_id));
                    return;
                }
            }
            ShellMessage::ZoomFocusedPane => {
                if let Some(pane_id) = self.focused_pane_id() {
                    self.update_shell(ShellMessage::PaneZoomToggled(pane_id));
                    return;
                }
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
                let ctx = crate::backend::DispatchCtx {
                    app: &mut self.app_state,
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
                    now_ms: now_ms(),
                    spawn_ptys: self.live_ptys,
                };
                let reply = crate::backend::handle_line(&payload, ctx);
                if let Ok(request) = serde_json::from_str::<pandamux_core::RpcRequest>(&payload) {
                    if matches!(
                        request.method.as_str(),
                        "ssh.save_profile"
                            | "ssh.remove_profile"
                            | "ssh.import_config"
                            | "ssh.profile.save"
                            | "ssh.profile.remove"
                            | "ssh.profile.import_config"
                    ) {
                        self.save_profiles();
                    }
                    if matches!(
                        request.method.as_str(),
                        "project.create" | "project.add_session"
                    ) && self.live_ptys
                    {
                        let _ = self.store.save_session(&self.app_state);
                    }
                }
                if let Ok(mut registry) = self.pipe_registry.lock()
                    && let Some(tx) = registry.remove(&id)
                {
                    let _ = tx.send(reply);
                }
            }
            ShellMessage::SurfaceClosed(surface_id) => {
                // The tab X lives on the active workspace's active pane.
                self.close_session(None, surface_id);
            }
            ShellMessage::SessionClosed {
                workspace_id,
                surface_id,
            } => {
                self.close_session(Some(workspace_id), surface_id);
            }
            core_message => {
                let result = self.apply_core_message(core_message);
                self.last_error = result.err();
            }
        }
        self.refresh_terminal_snapshots();
    }

    /// Close the terminal behind a tab or session row. A "tab" is a surface
    /// inside a pane, so the X can hit three different structural cases; the user
    /// only cares that the terminal goes away. We cascade: drop the surface if
    /// its pane has sibling tabs, else drop the whole pane, else drop the whole
    /// workspace. The very last surface in the app is left in place (there is
    /// nothing to fall back to, and an empty window is worse than a no-op). This
    /// keeps the core `surface.close` / `pane.close` intents (and their CLI
    /// contract) unchanged: the cascade is UI-layer close policy.
    fn close_session(&mut self, workspace_id: Option<WorkspaceId>, surface_id: SurfaceId) {
        let workspace_id =
            workspace_id.unwrap_or_else(|| self.app_state.active_workspace_id.clone());
        let Some(workspace) = self.app_state.workspace(&workspace_id) else {
            self.last_error = Some(format!("workspace not found: {workspace_id}"));
            return;
        };
        let Some(pane_id) = find_pane_id_for_surface(&workspace.split_tree, &surface_id) else {
            self.last_error = Some(format!("surface not found: {surface_id}"));
            return;
        };
        let surfaces_in_pane = find_leaf(&workspace.split_tree, &pane_id)
            .map(|leaf| leaf.surfaces.len())
            .unwrap_or(0);
        let pane_count = get_all_pane_ids(&workspace.split_tree).len();
        let intent = if surfaces_in_pane > 1 {
            AppIntent::Surface(SurfaceIntent::Close {
                workspace_id: Some(workspace_id),
                surface_id,
            })
        } else if pane_count > 1 {
            AppIntent::Pane(PaneIntent::Close {
                workspace_id: Some(workspace_id),
                pane_id,
            })
        } else if self.app_state.workspaces.len() > 1 {
            AppIntent::Workspace(WorkspaceIntent::Close { workspace_id })
        } else {
            // Last terminal in the app: nothing to fall back to.
            return;
        };
        self.last_error = self.app_state.apply(intent).err();
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
        // Forward any OSC 52 copies (local or over SSH) to the OS clipboard.
        if self.live_ptys {
            crate::backend::drain_clipboard_stores(
                &mut self.ptys,
                &mut self.remotes,
                self.clipboard_config.max_store_bytes,
            );
        }
        self.terminals = terminal_snapshots(
            &self.app_state,
            &mut self.ptys,
            &mut self.remotes,
            &self.remote_configs,
            self.live_ptys,
        )
        .unwrap_or_else(|error| {
            self.last_error = Some(error);
            fallback_terminal_snapshots(&self.app_state)
        });
        self.recompute_find_matches();
        self.rebuild_chrome();
        let active_surface_id = self.active_surface_id();
        let pending_projects = self
            .pending_remote_launch
            .as_ref()
            .map(|pending| HashSet::from([pending.target.workspace_id.clone()]))
            .unwrap_or_default();
        let sessions = project_sessions_with_profiles(
            &self.app_state,
            &self.ssh_profiles,
            self.chrome.session_grouping,
            self.chrome.session_panel_open,
            active_surface_id.as_ref(),
            &pending_projects,
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
            launcher: self.launcher.clone(),
            settings,
            surface_contents: self.contents.snapshot(),
            drag: self.drag.clone(),
            term_scheme: self
                .themes
                .active()
                .map(TermScheme::from_theme)
                .unwrap_or_default(),
            surface_term_schemes: self
                .surface_schemes
                .iter()
                .filter_map(|(surface_id, name)| {
                    self.themes
                        .get(name)
                        .map(|theme| (surface_id.clone(), TermScheme::from_theme(theme)))
                })
                .collect(),
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

    /// Write raw input bytes to the focused pane's live terminal (local PTY or
    /// SSH channel). No-op when the focused surface is not a live terminal, so a
    /// keystroke over a markdown/diff surface is harmlessly dropped.
    fn write_terminal_input(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || !self.live_ptys {
            return;
        }
        let Some(surface_id) = self.active_surface_id() else {
            return;
        };
        let id = surface_id.as_str();
        let result = if self.remotes.has(id) {
            self.remotes.write_all(id, bytes)
        } else if self.ptys.has(id) {
            self.ptys
                .write_all(id, bytes)
                .map_err(|error| error.to_string())
        } else {
            return;
        };
        if let Err(error) = result {
            self.last_error = Some(error);
        }
    }

    /// The focused pane of the active workspace, if any (keyboard shortcuts that
    /// act on "the focused pane" resolve it here).
    fn focused_pane_id(&self) -> Option<PaneId> {
        self.app_state
            .active_workspace()
            .and_then(|workspace| workspace.focused_pane_id.clone())
    }

    /// The focused session's reported working directory, if the shell integration
    /// has reported one (scopes the git poller to that session).
    fn focused_cwd(&self) -> Option<std::path::PathBuf> {
        let surface_id = self.active_surface_id()?;
        self.ptys
            .cwd(surface_id.as_str())
            .map(std::path::PathBuf::from)
    }

    /// Refresh the chrome view state derived from canonical state (session/pane
    /// counts, active shell/session, activity, unread badge). Pollers (git/ports)
    /// fill the rest later.
    fn rebuild_chrome(&mut self) {
        self.chrome.session_count = self.app_state.workspaces.len();
        self.chrome.version = env!("CARGO_PKG_VERSION").to_string();
        self.chrome.unread_notifications = self.notifications.unread_count(None) > 0;
        self.chrome.sidebar_progress = self
            .sidebar
            .progress
            .as_ref()
            .map(|progress| (progress.value, progress.label.clone().unwrap_or_default()));
        // Gold busy-agent dot when agents are registered, else running when live
        // shells are attached, else idle.
        self.chrome.activity = if !self.agents.is_empty() {
            SessionActivity::BusyAgent
        } else if self.live_ptys {
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
                if let ProjectLocation::Ssh {
                    profile_id,
                    remote_cwd,
                } = &workspace.project.location
                {
                    if !self.remotes.has(&session_id) {
                        let Some(profile) = self.ssh_profiles.get(profile_id) else {
                            self.last_error = Some(format!(
                                "SSH profile missing for Project {}: {profile_id}",
                                workspace.title
                            ));
                            continue;
                        };
                        if matches!(profile.auth, pandamux_core::SshAuthConfig::Password) {
                            self.last_error = Some(format!(
                                "Project {} is waiting for SSH credentials",
                                workspace.title
                            ));
                            continue;
                        }
                        match crate::project_launcher::ssh_config(
                            profile,
                            remote_cwd.clone(),
                            self.credential_cache.get(profile_id),
                            false,
                        ) {
                            Ok(config) => {
                                self.remotes.connect(
                                    session_id.clone(),
                                    config.clone(),
                                    DEFAULT_GRID_SIZE,
                                )?;
                                self.remote_configs.insert(surface_id.clone(), config);
                            }
                            Err(error) => self.last_error = Some(error.message),
                        }
                    }
                    continue;
                }
                expected_session_ids.insert(session_id.clone());
                if self.ptys.has(&session_id) {
                    continue;
                }
                let command = match &workspace.project.location {
                    ProjectLocation::Local { cwd, shell } => PtyCommand::new(shell.clone())
                        .with_cwd(Some(cwd.clone()))
                        .with_env(crate::backend::pandamux_env(&session_id, None)),
                    ProjectLocation::Legacy => PtyCommand::new(workspace.shell.clone())
                        .with_env(crate::backend::pandamux_env(&session_id, None)),
                    ProjectLocation::Ssh { .. } => unreachable!(),
                };
                self.ptys
                    .spawn(session_id, &command, DEFAULT_GRID_SIZE)
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
    .title("PandaMUX")
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
        // Check GitHub for a newer release periodically (a launch-time check is
        // kicked from the first status poll; see `update`).
        subscriptions.push(
            time::every(Duration::from_secs(UPDATE_CHECK_INTERVAL_SECS))
                .map(|_| ShellMessage::UpdateCheckRequested),
        );
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
    use keyboard::{Event, Key};
    let Event::KeyPressed {
        key,
        modifiers,
        text,
        ..
    } = event
    else {
        return ShellMessage::Noop;
    };
    let ctrl = modifiers.control();
    let shift = modifiers.shift();
    match key.as_ref() {
        Key::Character(character) => decode_character(character, text.as_deref(), ctrl, shift),
        Key::Named(named) => map_named_key(named, ctrl, shift),
        _ => ShellMessage::Noop,
    }
}

/// Decode a character key into either a chrome shortcut or terminal bytes. Split
/// out from `map_key_event` so it is testable without constructing a full
/// keyboard event. `text` is the composed text the OS produced (honours shift +
/// keyboard layout); it is preferred over the base `character` for plain typing.
fn decode_character(character: &str, text: Option<&str>, ctrl: bool, shift: bool) -> ShellMessage {
    if ctrl {
        // A mapped Ctrl chrome shortcut wins; an unmapped Ctrl+key still reaches
        // the terminal as its control code (Ctrl+C, Ctrl+L, ...).
        let shortcut = shortcut_for(true, shift, &character.to_ascii_lowercase());
        if shortcut != ShellMessage::Noop {
            return shortcut;
        }
        return match control_byte(character) {
            Some(byte) => ShellMessage::TerminalInput(vec![byte]),
            None => ShellMessage::Noop,
        };
    }
    let content = text
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| character.to_string());
    ShellMessage::TerminalInput(content.into_bytes())
}

/// Decode a named key into either a chrome message or terminal bytes. The three
/// context-sensitive keys (Escape / Up / Down / Enter) keep their chrome message;
/// their handlers forward the key to the terminal when no overlay is open. The
/// rest are pure terminal navigation/editing keys with no chrome conflict.
fn map_named_key(named: keyboard::key::Named, ctrl: bool, shift: bool) -> ShellMessage {
    use keyboard::key::Named;
    match named {
        // Escape dismisses an open overlay, else sends ESC to the terminal.
        Named::Escape => ShellMessage::OverlayDismissed,
        // Arrows drive palette selection when it is open, else terminal history.
        Named::ArrowUp => ShellMessage::PaletteMoveSelection(-1),
        Named::ArrowDown => ShellMessage::PaletteMoveSelection(1),
        // Ctrl+Enter zooms the focused pane; plain Enter activates a palette item
        // when open, else sends a carriage return to the terminal.
        Named::Enter => {
            if ctrl {
                ShellMessage::ZoomFocusedPane
            } else {
                ShellMessage::PaletteActivate
            }
        }
        Named::Space => ShellMessage::TerminalInput(vec![b' ']),
        Named::Backspace => ShellMessage::TerminalInput(vec![0x7f]),
        Named::Tab => ShellMessage::TerminalInput(if shift {
            b"\x1b[Z".to_vec()
        } else {
            vec![b'\t']
        }),
        Named::ArrowLeft => ShellMessage::TerminalInput(b"\x1b[D".to_vec()),
        Named::ArrowRight => ShellMessage::TerminalInput(b"\x1b[C".to_vec()),
        Named::Home => ShellMessage::TerminalInput(b"\x1b[H".to_vec()),
        Named::End => ShellMessage::TerminalInput(b"\x1b[F".to_vec()),
        Named::Delete => ShellMessage::TerminalInput(b"\x1b[3~".to_vec()),
        Named::Insert => ShellMessage::TerminalInput(b"\x1b[2~".to_vec()),
        Named::PageUp => ShellMessage::TerminalInput(b"\x1b[5~".to_vec()),
        Named::PageDown => ShellMessage::TerminalInput(b"\x1b[6~".to_vec()),
        _ => ShellMessage::Noop,
    }
}

/// Map a single character to its Ctrl control code (Ctrl+A == 0x01, ...). Returns
/// `None` for multi-character input or keys with no control encoding.
fn control_byte(character: &str) -> Option<u8> {
    let mut chars = character.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    match first.to_ascii_lowercase() {
        c @ 'a'..='z' => Some((c as u8 - b'a') + 1),
        '@' | ' ' => Some(0x00),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' | '?' => Some(0x1f),
        _ => None,
    }
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
        (true, "p") => ShellMessage::OverlayRequested(RailItem::CommandPalette),
        (false, "t") => ShellMessage::NewSessionRequested,
        (false, ",") => ShellMessage::OverlayRequested(RailItem::Settings),
        (false, "f") => ShellMessage::FindOpened,
        (false, "n") => ShellMessage::NotificationsToggled,
        (false, "d") => ShellMessage::SplitFocused(SplitDirection::Horizontal),
        (true, "d") => ShellMessage::SplitFocused(SplitDirection::Vertical),
        (false, "w") => ShellMessage::CloseFocusedPane,
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
    remotes: &mut RemoteSessionManager,
    remote_configs: &HashMap<SurfaceId, SshConfig>,
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
            let is_remote = remote_configs.contains_key(&surface_id);
            // Pull the styled grid (per-cell color + cursor). A remote read error
            // yields an empty screen (pane still renders); a live-PTY read error
            // skips the pane, matching the previous text-path behavior.
            let screen = if is_remote {
                remotes.screen_cells(surface_id.as_str()).ok()
            } else if live_ptys {
                Some(
                    ptys.screen_cells(surface_id.as_str())
                        .map_err(|error| error.to_string())
                        .ok()?,
                )
            } else {
                None
            };

            // Derive plain-text rows (for link detection + text consumers) from
            // the same rows the viewport renders, so line indices stay aligned.
            let (lines, cells, cursor, columns, rows) = match screen {
                Some(screen) => {
                    let lines = cells_to_lines(&screen.rows);
                    let columns = screen.rows.iter().map(|row| row.len()).max().unwrap_or(120);
                    let rows = screen.rows.len();
                    (lines, screen.rows, screen.cursor, columns, rows)
                }
                None if is_remote => (Vec::new(), Vec::new(), (0, 0), 120, 30),
                None => (fallback_lines(), Vec::new(), (0, 0), 120, 30),
            };
            let links = detect_links(&lines)
                .into_iter()
                .map(|link| LinkSpan {
                    line: link.line,
                    start: link.start,
                    end: link.end,
                })
                .collect();
            let remote_host = remote_configs
                .get(&surface_id)
                .map(|config| config.host.clone());
            Some(TerminalSnapshot {
                surface_id,
                lines,
                cells,
                cursor,
                columns,
                rows,
                links,
                remote_host,
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
        launcher: SessionLauncherViewState::default(),
        settings: SettingsViewState::default(),
        surface_contents: HashMap::new(),
        drag: None,
        term_scheme: TermScheme::default(),
        surface_term_schemes: HashMap::new(),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Resolve the bundled themes directory: `PANDAMUX_THEMES_DIR` if set, else
/// `<exe dir>/resources/themes`, else walk up from the cwd to a `resources/themes`
/// (dev checkout). Returns `None` if none is found.
fn themes_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("PANDAMUX_THEMES_DIR") {
        return Some(std::path::PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let candidate = parent.join("resources").join("themes");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join("resources").join("themes");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
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
    terminal_snapshots(
        app_state,
        &mut PtySessionManager::new(),
        &mut RemoteSessionManager::default(),
        &HashMap::new(),
        false,
    )
    .unwrap_or_default()
}

fn fallback_lines() -> Vec<String> {
    vec![
        "PandaMUX".to_string(),
        "Native shell runtime is active.".to_string(),
    ]
}

/// Flatten styled grid rows into trimmed plain-text lines for link detection and
/// other text consumers, keeping one line per grid row (indices stay aligned with
/// the styled `cells`).
fn cells_to_lines(rows: &[Vec<pandamux_term::StyledCell>]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let mut line: String = row.iter().map(|cell| cell.c).collect();
            line.truncate(line.trim_end().len());
            line
        })
        .collect()
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
    #[cfg(windows)]
    fn duplicate_launcher_submit_creates_at_most_one_project() {
        let mut runtime = NativeShellRuntime::default();
        let path = std::env::current_dir()
            .unwrap()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();
        runtime.launcher.step = LauncherStep::Folder;
        runtime.launcher.path = path.clone();
        runtime.launcher.listing = Some(pandamux_core::FolderListing {
            canonical_path: path,
            parent_path: None,
            breadcrumbs: Vec::new(),
            directories: Vec::new(),
            drives: Vec::new(),
        });
        runtime.start_selected_folder();
        runtime.start_selected_folder();
        assert_eq!(runtime.app_state.workspaces.len(), 2);
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
    fn closing_the_only_tab_in_a_pane_cascades_to_the_workspace() {
        let mut runtime = NativeShellRuntime::default();
        // The default workspace has a single pane with a single surface, so its
        // tab X hits the "last surface in a pane" guard. Capture its ids first.
        let default_ws = runtime.app_state.active_workspace_id.clone();
        let surface_id = runtime
            .view_model()
            .projection
            .visible_panes
            .first()
            .and_then(|pane| pane.surfaces.first())
            .expect("default surface")
            .id
            .clone();

        // A second workspace makes the default one closable.
        runtime
            .app_state
            .apply(AppIntent::Workspace(WorkspaceIntent::Create {
                title: Some("Second".to_string()),
                shell: None,
            }))
            .expect("create workspace");

        // Closing that sole tab cascades to closing the whole workspace instead
        // of silently failing on the last-surface guard.
        runtime.update_shell(ShellMessage::SessionClosed {
            workspace_id: default_ws.clone(),
            surface_id,
        });

        assert_eq!(runtime.app_state.workspaces.len(), 1);
        assert!(runtime.app_state.workspace(&default_ws).is_none());
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    fn closing_the_only_session_in_the_app_is_a_noop() {
        let mut runtime = NativeShellRuntime::default();
        let surface_id = runtime
            .view_model()
            .projection
            .visible_panes
            .first()
            .and_then(|pane| pane.surfaces.first())
            .expect("default surface")
            .id
            .clone();

        runtime.update_shell(ShellMessage::SurfaceClosed(surface_id));

        // The very last terminal stays put; no error is raised.
        assert_eq!(runtime.app_state.workspaces.len(), 1);
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
    fn update_available_toast_dedupes_per_version() {
        let mut runtime = NativeShellRuntime::default();
        let offer = |version: &str| ShellMessage::UpdateAvailable {
            version: version.to_string(),
            tag: format!("v{version}"),
            url: Some("https://example/Setup.exe".to_string()),
            notes: "Highlights of the release".to_string(),
        };

        runtime.update_shell(offer("0.34.0"));
        runtime.update_shell(ShellMessage::Tick);
        assert_eq!(runtime.view_model().notifications.cards.len(), 1);
        assert_eq!(
            runtime.view_model().notifications.cards[0].title,
            "Update available"
        );

        // The same version re-offered by the periodic check does not re-toast.
        runtime.update_shell(offer("0.34.0"));
        runtime.update_shell(ShellMessage::Tick);
        assert_eq!(runtime.view_model().notifications.cards.len(), 1);

        // A newer version does toast again.
        runtime.update_shell(offer("0.35.0"));
        runtime.update_shell(ShellMessage::Tick);
        assert_eq!(runtime.view_model().notifications.cards.len(), 2);
        assert_eq!(runtime.last_error(), None);
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
        assert_eq!(
            shortcut_for(true, false, "d"),
            ShellMessage::SplitFocused(SplitDirection::Horizontal)
        );
        assert_eq!(
            shortcut_for(true, true, "d"),
            ShellMessage::SplitFocused(SplitDirection::Vertical)
        );
        assert_eq!(
            shortcut_for(true, false, "w"),
            ShellMessage::CloseFocusedPane
        );
        assert_eq!(
            shortcut_for(true, true, "p"),
            ShellMessage::OverlayRequested(RailItem::CommandPalette)
        );
        assert_eq!(shortcut_for(false, false, "b"), ShellMessage::Noop);
        assert_eq!(shortcut_for(true, false, "z"), ShellMessage::Noop);
    }

    #[test]
    fn focused_pane_shortcuts_split_close_and_zoom() {
        let mut runtime = NativeShellRuntime::default();
        // Ctrl+D splits the focused pane.
        runtime.update_shell(ShellMessage::SplitFocused(SplitDirection::Horizontal));
        assert_eq!(runtime.view_model().projection.visible_panes.len(), 2);
        // Ctrl+Enter zooms the focused pane.
        runtime.update_shell(ShellMessage::ZoomFocusedPane);
        assert!(
            runtime
                .app_state
                .active_workspace()
                .unwrap()
                .zoomed_pane_id
                .is_some()
        );
        // Unzoom, then Ctrl+W closes the focused pane back to one.
        runtime.update_shell(ShellMessage::ZoomFocusedPane);
        runtime.update_shell(ShellMessage::CloseFocusedPane);
        assert_eq!(runtime.view_model().projection.visible_panes.len(), 1);
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    fn palette_arrow_navigation_gated_to_open_palette() {
        let mut runtime = NativeShellRuntime::default();
        // Closed palette: arrow/activate are no-ops.
        runtime.update_shell(ShellMessage::PaletteMoveSelection(1));
        assert_eq!(runtime.view_model().palette.selected, 0);
        runtime.update_shell(ShellMessage::PaletteActivate);
        assert_eq!(runtime.view_model().chrome.active_overlay, Overlay::None);

        // Open it, move down, and activating runs the highlighted item.
        runtime.update_shell(ShellMessage::OverlayRequested(RailItem::CommandPalette));
        assert_eq!(
            runtime.view_model().chrome.active_overlay,
            Overlay::CommandPalette
        );
        runtime.update_shell(ShellMessage::PaletteMoveSelection(1));
        assert_eq!(runtime.view_model().palette.selected, 1);
        runtime.update_shell(ShellMessage::PaletteActivate);
        assert_eq!(runtime.view_model().chrome.active_overlay, Overlay::None);
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
    fn drag_move_splits_pane_via_core_intent() {
        let mut runtime = NativeShellRuntime::default();
        let pane_id = runtime.view_model().projection.visible_panes[0].id.clone();
        // Add a second tab so a directional drop can split the pane.
        runtime.update_shell(ShellMessage::TerminalSurfaceCreated(pane_id.clone()));
        let surface_id = runtime.view_model().projection.visible_panes[0]
            .surfaces
            .last()
            .expect("second surface")
            .id
            .clone();

        // Arm, move (activate), hover the bottom zone, release.
        runtime.update_shell(ShellMessage::TabDragArmed {
            surface_id,
            pane_id: pane_id.clone(),
        });
        runtime.update_shell(ShellMessage::DragMoved);
        runtime.update_shell(ShellMessage::DragOverZone {
            pane_id,
            zone: pandamux_core::DropZone::Bottom,
        });
        runtime.update_shell(ShellMessage::DragReleased);

        assert_eq!(runtime.view_model().projection.visible_panes.len(), 2);
        assert!(runtime.view_model().drag.is_none());
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    fn selecting_a_theme_updates_the_terminal_scheme() {
        let mut runtime = NativeShellRuntime::default();
        assert_eq!(runtime.view_model().term_scheme, TermScheme::default());

        // Import a theme over the pipe, then select it; the derived terminal
        // scheme should pick up its colors.
        let (tx, rx) = oneshot::channel();
        runtime.pipe_registry.lock().unwrap().insert(1, tx);
        runtime.update_shell(ShellMessage::PipeRequest {
            id: 1,
            payload: r#"{"method":"config.import_ghostty","params":{"name":"Neon","content":"background = #010203\nforeground = #fafbfc\n"},"id":1}"#.to_string(),
        });
        let _ = rx.blocking_recv();

        let (tx2, rx2) = oneshot::channel();
        runtime.pipe_registry.lock().unwrap().insert(2, tx2);
        runtime.update_shell(ShellMessage::PipeRequest {
            id: 2,
            payload: r#"{"method":"theme.select","params":{"name":"Neon"},"id":2}"#.to_string(),
        });
        let _ = rx2.blocking_recv();

        let scheme = runtime.view_model().term_scheme;
        assert_eq!(scheme.background, iced::Color::from_rgb8(0x01, 0x02, 0x03));
        assert_eq!(scheme.text, iced::Color::from_rgb8(0xfa, 0xfb, 0xfc));
    }

    #[test]
    fn tab_click_without_drag_focuses_not_moves() {
        let mut runtime = NativeShellRuntime::default();
        let pane_id = runtime.view_model().projection.visible_panes[0].id.clone();
        // Two tabs; the first is inactive after creating the second.
        runtime.update_shell(ShellMessage::TerminalSurfaceCreated(pane_id.clone()));
        let first_surface = runtime.view_model().projection.visible_panes[0].surfaces[0]
            .id
            .clone();

        // Arm on the first tab and release without moving: a plain click focuses,
        // does not split.
        runtime.update_shell(ShellMessage::TabDragArmed {
            surface_id: first_surface.clone(),
            pane_id,
        });
        runtime.update_shell(ShellMessage::DragReleased);

        assert!(runtime.view_model().drag.is_none());
        assert_eq!(runtime.view_model().projection.visible_panes.len(), 1);
        assert_eq!(
            runtime.view_model().projection.visible_panes[0].active_surface_id,
            Some(first_surface)
        );
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    fn decode_character_routes_typing_and_control_codes() {
        // Plain characters become terminal bytes, preferring the OS-composed text
        // (so shift + layout land the right glyph).
        assert_eq!(
            decode_character("a", Some("a"), false, false),
            ShellMessage::TerminalInput(b"a".to_vec())
        );
        assert_eq!(
            decode_character("a", Some("A"), false, true),
            ShellMessage::TerminalInput(b"A".to_vec())
        );
        assert_eq!(
            decode_character("1", Some("!"), false, true),
            ShellMessage::TerminalInput(b"!".to_vec())
        );
        // No composed text: fall back to the base character.
        assert_eq!(
            decode_character("z", None, false, false),
            ShellMessage::TerminalInput(b"z".to_vec())
        );
        // Ctrl+C is not a chrome shortcut, so it reaches the terminal as 0x03.
        assert_eq!(
            decode_character("c", None, true, false),
            ShellMessage::TerminalInput(vec![0x03])
        );
        // Ctrl+D IS a chrome shortcut (split), so it wins over the control code.
        assert_eq!(
            decode_character("d", None, true, false),
            ShellMessage::SplitFocused(SplitDirection::Horizontal)
        );
    }

    #[test]
    fn named_keys_map_to_terminal_sequences() {
        use keyboard::key::Named;
        assert_eq!(
            map_named_key(Named::Backspace, false, false),
            ShellMessage::TerminalInput(vec![0x7f])
        );
        assert_eq!(
            map_named_key(Named::Tab, false, false),
            ShellMessage::TerminalInput(vec![b'\t'])
        );
        assert_eq!(
            map_named_key(Named::Tab, false, true),
            ShellMessage::TerminalInput(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            map_named_key(Named::ArrowLeft, false, false),
            ShellMessage::TerminalInput(b"\x1b[D".to_vec())
        );
        // Context keys keep their chrome message; handlers add the terminal path.
        assert_eq!(
            map_named_key(Named::Enter, false, false),
            ShellMessage::PaletteActivate
        );
        assert_eq!(
            map_named_key(Named::Enter, true, false),
            ShellMessage::ZoomFocusedPane
        );
        assert_eq!(
            map_named_key(Named::Escape, false, false),
            ShellMessage::OverlayDismissed
        );
    }

    #[test]
    fn control_byte_maps_letters_and_symbols() {
        assert_eq!(control_byte("a"), Some(0x01));
        assert_eq!(control_byte("C"), Some(0x03));
        assert_eq!(control_byte("z"), Some(0x1a));
        assert_eq!(control_byte("["), Some(0x1b));
        assert_eq!(control_byte(" "), Some(0x00));
        assert_eq!(control_byte("ab"), None);
        assert_eq!(control_byte("1"), None);
    }

    #[test]
    fn terminal_input_is_suppressed_while_an_overlay_is_open() {
        let mut runtime = NativeShellRuntime::default();
        // Open the command palette, then "type": the char must not leak to a
        // terminal write path; it belongs to the palette's own text input.
        runtime.update_shell(ShellMessage::OverlayRequested(RailItem::CommandPalette));
        runtime.update_shell(ShellMessage::TerminalInput(b"l".to_vec()));
        assert_eq!(
            runtime.view_model().chrome.active_overlay,
            Overlay::CommandPalette
        );
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    #[ignore = "spawns a real shell through ConPTY, run manually during Iced runtime work"]
    fn live_typing_reaches_pty() {
        // End-to-end: drive the public keyboard path (TerminalInput per keystroke
        // + Enter via the PaletteActivate fallback) and confirm the echoed marker
        // shows up in the terminal snapshot.
        let mut runtime = NativeShellRuntime::new(true);
        let marker = "PANDAMUX_TYPING_E2E_OK";
        for byte in format!("Write-Output {marker}").into_bytes() {
            runtime.update_shell(ShellMessage::TerminalInput(vec![byte]));
        }
        // No overlay is open, so PaletteActivate sends a carriage return.
        runtime.update_shell(ShellMessage::PaletteActivate);

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
        panic!("typed command output never appeared in terminal snapshots");
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
