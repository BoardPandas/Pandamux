use crate::persistence::{SessionStore, SshProfileConfig, SshProfileStore};
use crate::project_launcher::{EphemeralCredential, LaunchTarget};
use iced::futures::SinkExt;
use iced::{Element, Size, Subscription, Task, Theme, application, keyboard, stream, time, window};
use pandamux_core::{
    AgentRegistry, AppIntent, AppState, ClipboardConfig, HomeIntent, LaunchConfig, Localizer,
    NewNotification, NotificationSource, Notifications, PaneId, PaneIntent, ProjectError,
    ProjectErrorCategory, ProjectId, ProjectIntent, ProjectLocation, ProjectMatcher, ProjectRecord,
    SessionType, SidebarState, SplitDirection, SplitNode, SplitPaneParams, SshProfileId,
    SshProfiles, SurfaceContents, SurfaceId, SurfaceIntent, SurfaceType, ThemeStore, UserSettings,
    WorkspaceId, WorkspaceIntent, find_leaf, find_pane_id_for_surface, get_all_pane_ids,
    parse_ghostty_theme,
};
use pandamux_term::{
    DEFAULT_GRID_SIZE, GridSize, PtySessionManager, RemoteSessionManager, RemoteStatus,
    ScrollAmount, SearchOptions, SshConfig, TermModes, detect_links, search_lines, wrap_paste,
};
use pandamux_ui::{
    Accent, ChromeState, ContextMenuAction, ContextMenuViewState, DragView, FindViewState,
    LauncherItem, LauncherStep, LinkSpan, MainView, NotificationCard, NotificationsViewState,
    Overlay, PaletteItem, PaletteViewState, RailItem, RailMenuAction, RailMenuViewState,
    SessionActivity, SessionLauncherViewState, SessionsViewState, SettingsSection,
    SettingsViewState, ShellKind, ShellMessage, ShellViewModel, SshProfileForm, TermScheme,
    TerminalSnapshot, TerminalToggle, UiTheme, app_view, filter_items,
    project_sessions_with_profiles, project_workspace_shell, shell_view,
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
/// Ticks a viewport-reported size must hold steady before the engine and
/// PTY/SSH channel are resized (~200ms, absorbing live window drags).
const RESIZE_SETTLE_TICKS: u64 = 2;
/// Ticks settings changes must be quiet before the debounced save (~500ms).
const SETTINGS_SAVE_SETTLE_TICKS: u64 = 5;
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
    /// Persistent user settings (config/settings.json).
    settings: UserSettings,
    /// Where the settings file lives (async save tasks rebuild the store).
    settings_dir: std::path::PathBuf,
    /// False when the settings file is corrupt: saves are refused so a broken
    /// file is never clobbered (matching the SSH profile store policy).
    settings_store_available: bool,
    settings_dirty: bool,
    settings_dirty_since: u64,
    /// In-progress text of the scrollback-lines input on the Terminal tab.
    scrollback_input: String,
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
    /// Last viewport-derived grid size per surface (drives spawns and resizes).
    viewport_sizes: HashMap<SurfaceId, GridSize>,
    /// Debounced pending engine/PTY resizes: surface -> (target, tick recorded).
    pending_resizes: HashMap<SurfaceId, (GridSize, u64)>,
    /// In-flight launch timings, reported on first output (spec 1.6). Also
    /// drives the "Starting..." feedback for young blank local sessions.
    timings: HashMap<SurfaceId, crate::latency::LaunchTimeline>,
    /// Queued git-remote identity probes (spec 1.4), drained into async tasks
    /// on the next tick so launch paths never block on IO.
    pending_git_hints: Vec<(ProjectId, ProjectLocation, Option<SshConfig>)>,
    /// Saved SSH host profiles.
    ssh_profiles: SshProfiles,
    credential_cache: HashMap<SshProfileId, EphemeralCredential>,
    launcher: SessionLauncherViewState,
    launcher_trust_unknown: bool,
    pending_remote_launch: Option<PendingRemoteLaunch>,
    /// Pinned favorites + recents (config/launcher.json, per-machine v1).
    launcher_prefs: crate::persistence::LauncherPrefsConfig,
    /// Where the SessionType step launches once a type is chosen.
    pending_type_launch: Option<PendingTypeLaunch>,
    /// The chosen type parked while an SSH credential is collected.
    pending_session_type: Option<SessionType>,
    /// The destructive action parked behind the confirm modal (spec 1.5/2.6).
    pending_confirm: Option<PendingConfirm>,
    /// A Home pane waiting for a relaunch to complete (spec 2.5).
    pending_home_assign: Option<PaneId>,
    /// Surfaces the user has typed into (session-local, never persisted). The
    /// welcome chooser only shows on untouched bare terminals (spec 2.7).
    touched_surfaces: HashSet<SurfaceId>,
    /// Surfaces whose welcome chooser was explicitly dismissed this session.
    welcome_dismissed: HashSet<SurfaceId>,
    /// Persistent clipboard policy (plan F1).
    clipboard_config: ClipboardConfig,
    /// Active drag-and-drop of a tab, if any (plan Section 12.3).
    drag: Option<DragView>,
    /// The open right-click context menu, if any (spec 1.3).
    context_menu: Option<ContextMenuViewState>,
    /// The open session-rail actions menu, if any (spec 2.1/1.4).
    rail_menu: Option<RailMenuViewState>,
    /// In-flight session rename: (workspace, surface, input text).
    session_rename: Option<(WorkspaceId, SurfaceId, String)>,
    /// In-flight project rename: (project, input text).
    project_rename: Option<(ProjectId, String)>,
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
    /// What to run once the remote PTY is ready (Terminal = nothing extra).
    session: SessionType,
}

/// A destructive action waiting on the confirm modal (spec 1.5; the
/// close-running-tab variant joins with the keymap stage).
#[derive(Clone, Debug)]
enum PendingConfirm {
    CloseAll { project_id: Option<ProjectId> },
}

/// Where the launcher's SessionType step will launch once a type is chosen
/// (spec 2.2): a project location (new or reused workspace) or a specific
/// pane (the tab-bar plus button).
#[derive(Clone, Debug)]
enum PendingTypeLaunch {
    Location {
        location: ProjectLocation,
    },
    Pane {
        workspace_id: WorkspaceId,
        pane_id: PaneId,
    },
    /// Convert an existing surface in place (the welcome chooser's Custom
    /// path, spec 2.7): set its session type and respawn under the tool.
    Surface {
        surface_id: SurfaceId,
    },
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
        let settings_dir = crate::persistence::SettingsStore::default_dir();
        let (settings, settings_load_error, settings_store_available) = if live_ptys {
            match crate::persistence::SettingsStore::new(&settings_dir).load() {
                Ok(settings) => (settings, None, true),
                Err(error) => (UserSettings::default(), Some(error.to_string()), false),
            }
        } else {
            (UserSettings::default(), None, true)
        };
        let ssh_profiles = profile_config.registry();
        // Only the real (live) app touches disk. Tests/smoke use default state
        // so they stay hermetic. A version change backs the old session up and
        // still restores it (serde defaults keep old files loading); only a
        // parse failure falls back to a clean default state.
        let app_state = if live_ptys {
            store.handle_version_change(env!("CARGO_PKG_VERSION"));
            let mut app_state = store.load_session().unwrap_or_default();
            // Assign project identities to any workspaces that predate the
            // registry (collapses historical per-host duplicates, spec 1.4).
            pandamux_core::ensure_project_registry(&mut app_state, now_ms());
            // Home panes whose sessions did not survive the restart become
            // relaunch placeholders (spec 2.5).
            let alive: HashSet<SurfaceId> = app_state
                .workspaces
                .iter()
                .flat_map(|workspace| {
                    crate::backend::terminal_surfaces(&workspace.split_tree)
                        .into_iter()
                        .map(|surface| surface.id)
                })
                .collect();
            app_state
                .home
                .release_dead_surfaces(&|surface_id| alive.contains(surface_id));
            app_state
        } else {
            AppState::default()
        };
        // Pinned favorites and recents (spec 2.3): per-machine, and validated
        // against the registry so dangling project ids drop out lazily.
        let mut launcher_prefs = if live_ptys {
            crate::persistence::LauncherPrefsStore::new(
                crate::persistence::LauncherPrefsStore::default_dir(),
            )
            .load()
        } else {
            crate::persistence::LauncherPrefsConfig::default()
        };
        launcher_prefs.retain_known_projects(&app_state.projects);
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
            scrollback_input: settings.terminal.scrollback_lines.to_string(),
            settings,
            settings_dir,
            settings_store_available,
            settings_dirty: false,
            settings_dirty_since: 0,
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
            viewport_sizes: HashMap::new(),
            pending_resizes: HashMap::new(),
            timings: HashMap::new(),
            pending_git_hints: Vec::new(),
            ssh_profiles,
            credential_cache: HashMap::new(),
            launcher: SessionLauncherViewState::default(),
            launcher_trust_unknown: false,
            pending_remote_launch: None,
            launcher_prefs,
            pending_type_launch: None,
            pending_session_type: None,
            pending_confirm: None,
            pending_home_assign: None,
            touched_surfaces: HashSet::new(),
            welcome_dismissed: HashSet::new(),
            clipboard_config: ClipboardConfig::default(),
            drag: None,
            context_menu: None,
            rail_menu: None,
            session_rename: None,
            project_rename: None,
            copy_mode: false,
            palette: PaletteViewState::default(),
            settings_section: SettingsSection::default(),
            view_model,
            terminals: Vec::new(),
            last_error: profile_load_error.or(settings_load_error),
            pipe_name: std::env::var("PANDAMUX_PIPE")
                .unwrap_or_else(|_| r"\\.\pipe\pandamux".to_string()),
            pipe_registry: Arc::new(StdMutex::new(HashMap::new())),
            pipe_seq: Arc::new(AtomicU64::new(1)),
            last_update_offer: None,
            update_checked_once: false,
        };
        runtime.apply_settings();
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
            self.pending_type_launch = None;
            self.pending_session_type = None;
            self.populate_launcher_lists();
        }
        self.chrome.active_overlay = overlay;
    }

    /// Rebuild the launcher's Project-step rows: pinned favorites, recents,
    /// existing projects, and the new-project entry points, filtered by the
    /// type-to-filter text (spec 2.2/2.3).
    fn populate_launcher_lists(&mut self) {
        let filter = self.launcher.filter.to_lowercase();
        let matches = |label: &str| filter.is_empty() || label.to_lowercase().contains(&filter);
        let record_for = |id: &ProjectId| {
            self.app_state
                .projects
                .iter()
                .find(|record: &&ProjectRecord| &record.id == id)
        };
        let mut items: Vec<LauncherItem> = Vec::new();
        for pin in &self.launcher_prefs.favorites {
            let Some(record) = record_for(&pin.project_id) else {
                continue;
            };
            let label = format!("{}: {}", record.name, pin.session.label());
            if !matches(&label) {
                continue;
            }
            items.push(LauncherItem {
                tag: "PIN".to_string(),
                label,
                detail: location_detail(record.known_locations.first()),
                message: ShellMessage::LauncherShortcutChosen(pin.clone()),
                pin: Some(ShellMessage::LauncherFavoriteToggled(pin.clone())),
            });
        }
        for recent in self.launcher_prefs.recents.iter().take(10) {
            if self.launcher_prefs.is_favorite(&recent.config) {
                continue;
            }
            let Some(record) = record_for(&recent.config.project_id) else {
                continue;
            };
            let label = format!("{}: {}", record.name, recent.config.session.label());
            if !matches(&label) {
                continue;
            }
            items.push(LauncherItem {
                tag: "RCT".to_string(),
                label,
                detail: format!(
                    "{} \u{00b7} {}",
                    location_detail(record.known_locations.first()),
                    relative_age(now_ms(), recent.last_used_ms)
                ),
                message: ShellMessage::LauncherShortcutChosen(recent.config.clone()),
                pin: None,
            });
        }
        for record in &self.app_state.projects {
            if !matches(&record.name) {
                continue;
            }
            items.push(LauncherItem {
                tag: "PROJ".to_string(),
                label: record.name.clone(),
                detail: location_detail(record.known_locations.first()),
                message: ShellMessage::LauncherProjectChosen(record.id.clone()),
                pin: None,
            });
        }
        items.push(LauncherItem {
            tag: "NEW".to_string(),
            label: "New local folder".to_string(),
            detail: "Browse this computer".to_string(),
            message: ShellMessage::LauncherLocalSelected,
            pin: None,
        });
        items.push(LauncherItem {
            tag: "NEW".to_string(),
            label: "New SSH connection".to_string(),
            detail: "Saved connections and imports".to_string(),
            message: ShellMessage::LauncherNewSsh,
            pin: None,
        });
        self.launcher.selected = self.launcher.selected.min(items.len().saturating_sub(1));
        self.launcher.items = items;
    }

    /// The SessionType step rows (spec 2.2/2.7): Terminal, PowerShell
    /// flavors, Claude, Codex, Gemini (Custom has its own input row).
    fn populate_type_items(&mut self) {
        let types: [(&str, &str, &str, SessionType); 6] = [
            (
                "TERM",
                "Terminal",
                "The project's default shell",
                SessionType::Terminal,
            ),
            (
                "PS",
                "PowerShell 7",
                "pwsh",
                SessionType::PowerShell {
                    program: "pwsh.exe".to_string(),
                },
            ),
            (
                "PS",
                "Windows PowerShell",
                "powershell",
                SessionType::PowerShell {
                    program: "powershell.exe".to_string(),
                },
            ),
            ("CL", "Claude", "Claude Code", SessionType::Claude),
            ("CX", "Codex", "Codex CLI", SessionType::Codex),
            ("GM", "Gemini", "Gemini CLI", SessionType::Gemini),
        ];
        self.launcher.type_items = types
            .into_iter()
            .map(|(tag, label, detail, session)| LauncherItem {
                tag: tag.to_string(),
                label: label.to_string(),
                detail: detail.to_string(),
                message: ShellMessage::LauncherTypeChosen(session),
                pin: None,
            })
            .collect();
        self.launcher.selected = 0;
    }

    fn save_launcher_prefs(&mut self) {
        if self.live_ptys {
            let _ = crate::persistence::LauncherPrefsStore::new(
                crate::persistence::LauncherPrefsStore::default_dir(),
            )
            .save(&self.launcher_prefs);
        }
    }

    /// Record a successful launch for the recents list (spec 2.3).
    fn record_recent(&mut self, project_id: Option<ProjectId>, session: &SessionType) {
        let Some(project_id) = project_id else {
            return;
        };
        self.launcher_prefs.record_recent(
            LaunchConfig {
                project_id,
                session: session.clone(),
            },
            now_ms(),
        );
        self.save_launcher_prefs();
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
        session: SessionType,
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
        let size = self.spawn_size(&target.surface_id);
        self.timings.insert(
            target.surface_id.clone(),
            crate::latency::LaunchTimeline::start(format!("ssh {}", config.host)),
        );
        self.remotes
            .connect(target.surface_id.to_string(), config.clone(), size)
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
        self.pending_remote_launch = Some(PendingRemoteLaunch {
            target,
            config,
            session,
        });
        Ok(())
    }

    /// A folder was picked (local browse or SSH): remember it and advance to
    /// the SessionType step (spec 2.2); the launch happens on type choice.
    fn start_selected_folder(&mut self) {
        if self.launcher.launching {
            return;
        }
        let Some(listing) = self.launcher.listing.as_ref() else {
            return;
        };
        let path = listing.canonical_path.clone();
        let location = if self.launcher.remote {
            let Some(profile_id) = self.launcher.selected_profile_id.clone() else {
                return;
            };
            self.profile_config
                .last_selected_folder_by_profile
                .insert(profile_id.clone(), path.clone());
            self.save_profiles();
            ProjectLocation::Ssh {
                profile_id,
                remote_cwd: path,
            }
        } else {
            self.profile_config.last_selected_local_folder = Some(path.clone());
            self.save_profiles();
            ProjectLocation::Local {
                cwd: path,
                shell: String::new(),
            }
        };
        self.launcher.target_name = pandamux_core::project_title(&location);
        self.pending_type_launch = Some(PendingTypeLaunch::Location { location });
        self.populate_type_items();
        self.launcher.step = LauncherStep::SessionType;
        self.view_model.launcher = self.launcher.clone();
    }

    /// A type was chosen on the SessionType step: launch whatever is pending
    /// (a project location or a specific pane). SSH locations that still need
    /// a password detour through the Credential step and retry from there.
    fn launch_pending(&mut self, session: SessionType) {
        let Some(pending) = self.pending_type_launch.clone() else {
            return;
        };
        match pending {
            PendingTypeLaunch::Pane {
                workspace_id,
                pane_id,
            } => {
                self.pending_type_launch = None;
                match self
                    .app_state
                    .apply(AppIntent::Surface(SurfaceIntent::Create {
                        workspace_id: Some(workspace_id.clone()),
                        pane_id: Some(pane_id),
                        surface_type: SurfaceType::Terminal,
                    })) {
                    Ok(pandamux_core::AppDelta::SurfaceCreated { surface, .. }) => {
                        if session != SessionType::Terminal {
                            let _ = self.app_state.apply(AppIntent::Surface(
                                SurfaceIntent::SetSessionType {
                                    workspace_id: Some(workspace_id.clone()),
                                    surface_id: surface.id.clone(),
                                    session: session.clone(),
                                },
                            ));
                        }
                        let project_id = self
                            .app_state
                            .workspace(&workspace_id)
                            .and_then(|workspace| workspace.project_id.clone());
                        self.record_recent(project_id, &session);
                        if self.live_ptys {
                            let _ = self.store.save_session(&self.app_state);
                        }
                        self.chrome.active_overlay = Overlay::None;
                        self.complete_home_assign(&surface.id);
                    }
                    Ok(_) => {}
                    Err(error) => self.last_error = Some(error),
                }
            }
            PendingTypeLaunch::Surface { surface_id } => {
                // Welcome chooser Custom path (spec 2.7): convert in place.
                self.pending_type_launch = None;
                self.convert_surface_session(&surface_id, session);
                self.chrome.active_overlay = Overlay::None;
            }
            PendingTypeLaunch::Location { location } => match location {
                ProjectLocation::Local { cwd, .. } => {
                    self.pending_type_launch = None;
                    let size = self.focused_viewport_size();
                    match crate::project_launcher::launch_local(
                        &mut self.app_state,
                        &mut self.ptys,
                        cwd.clone(),
                        self.live_ptys,
                        size,
                        &session,
                    ) {
                        Ok(success) => {
                            self.timings.insert(
                                success.surface_id.clone(),
                                crate::latency::LaunchTimeline::start("local shell"),
                            );
                            if let Some(project_id) = success.project_id.clone() {
                                self.pending_git_hints.push((
                                    project_id,
                                    ProjectLocation::Local {
                                        cwd,
                                        shell: String::new(),
                                    },
                                    None,
                                ));
                            }
                            self.record_recent(success.project_id, &session);
                            if self.live_ptys {
                                let _ = self.store.save_session(&self.app_state);
                            }
                            self.chrome.active_overlay = Overlay::None;
                            self.complete_home_assign(&success.surface_id);
                        }
                        Err(error) => {
                            self.pending_home_assign = None;
                            self.launcher.error = Some(error);
                            self.launcher.step = LauncherStep::SessionType;
                        }
                    }
                }
                ProjectLocation::Ssh {
                    profile_id,
                    remote_cwd,
                } => {
                    let needs_password =
                        self.ssh_profiles.get(&profile_id).is_some_and(|profile| {
                            matches!(profile.auth, pandamux_core::SshAuthConfig::Password)
                        }) && !self.credential_cache.contains_key(&profile_id);
                    if needs_password {
                        // Park the choice; the credential submit retries.
                        self.pending_session_type = Some(session);
                        self.launcher.selected_profile_id = Some(profile_id);
                        self.launcher.remote = true;
                        self.launcher.step = LauncherStep::Credential;
                        return;
                    }
                    self.pending_type_launch = None;
                    if let Err(error) = self.start_remote_launch(profile_id, remote_cwd, session) {
                        self.pending_home_assign = None;
                        self.launcher.error = Some(error);
                        self.launcher.step = LauncherStep::SessionType;
                    }
                }
                ProjectLocation::Legacy => {
                    self.pending_type_launch = None;
                    self.pending_home_assign = None;
                }
            },
        }
        self.view_model.launcher = self.launcher.clone();
    }

    /// Open the launcher directly on the SessionType step targeting a known
    /// workspace (sidebar per-project plus, tab-bar plus for legacy panes).
    fn open_type_step_for_workspace(&mut self, workspace_id: WorkspaceId, pane_id: Option<PaneId>) {
        let Some(workspace) = self.app_state.workspace(&workspace_id) else {
            return;
        };
        let target_name = workspace
            .project_id
            .as_ref()
            .and_then(|id| {
                self.app_state
                    .projects
                    .iter()
                    .find(|record| &record.id == id)
            })
            .map(|record| record.name.clone())
            .unwrap_or_else(|| workspace.title.clone());
        let pending = match (&workspace.project.location, pane_id) {
            // A tab-bar plus targets its pane directly; legacy workspaces have
            // no launchable location so they always use the pane path.
            (_, Some(pane_id)) => PendingTypeLaunch::Pane {
                workspace_id: workspace_id.clone(),
                pane_id,
            },
            (ProjectLocation::Legacy, None) => {
                let Some(pane_id) = workspace
                    .focused_pane_id
                    .clone()
                    .or_else(|| get_all_pane_ids(&workspace.split_tree).into_iter().next())
                else {
                    return;
                };
                PendingTypeLaunch::Pane {
                    workspace_id: workspace_id.clone(),
                    pane_id,
                }
            }
            (location, None) => PendingTypeLaunch::Location {
                location: location.clone(),
            },
        };
        self.open_overlay(Overlay::QuickLaunch);
        self.pending_type_launch = Some(pending);
        self.launcher.target_name = target_name;
        self.populate_type_items();
        self.launcher.step = LauncherStep::SessionType;
    }

    /// Whether the welcome chooser strip applies to this surface right now:
    /// enabled in settings, still a bare terminal, and untouched (spec 2.7).
    fn welcome_visible(&self, surface_id: &SurfaceId) -> bool {
        if !self.settings.terminal.welcome_prompt_enabled
            || self.touched_surfaces.contains(surface_id)
            || self.welcome_dismissed.contains(surface_id)
        {
            return false;
        }
        self.app_state.workspaces.iter().any(|workspace| {
            crate::backend::terminal_surfaces(&workspace.split_tree)
                .into_iter()
                .any(|surface| {
                    &surface.id == surface_id
                        && matches!(surface.session.unwrap_or_default(), SessionType::Terminal)
                })
        })
    }

    /// Convert a live surface to a tool in place (welcome chooser, spec 2.7):
    /// record the session type, then respawn the local PTY under the tool (the
    /// sync pass respects the type), or type the tool command into a remote
    /// shell (same as an explicit SSH launch).
    fn convert_surface_session(&mut self, surface_id: &SurfaceId, session: SessionType) {
        let Some(workspace_id) = self
            .app_state
            .workspaces
            .iter()
            .find(|workspace| find_pane_id_for_surface(&workspace.split_tree, surface_id).is_some())
            .map(|workspace| workspace.id.clone())
        else {
            return;
        };
        if let Err(error) =
            self.app_state
                .apply(AppIntent::Surface(SurfaceIntent::SetSessionType {
                    workspace_id: Some(workspace_id.clone()),
                    surface_id: surface_id.clone(),
                    session: session.clone(),
                }))
        {
            self.last_error = Some(error);
            return;
        }
        let id = surface_id.as_str();
        if self.remotes.has(id) {
            if let Some(command) = crate::project_launcher::remote_initial_command(&session) {
                let _ = self
                    .remotes
                    .write_all(id, format!("{command}\n").as_bytes());
            }
        } else if self.ptys.has(id) {
            let _ = self.ptys.kill(id);
            if let Err(error) = self.sync_terminal_sessions() {
                self.last_error = Some(error);
            }
        }
        let project_id = self
            .app_state
            .workspace(&workspace_id)
            .and_then(|workspace| workspace.project_id.clone());
        self.record_recent(project_id, &session);
        if self.live_ptys {
            let _ = self.store.save_session(&self.app_state);
        }
    }

    /// The welcome chooser's Custom path: open the launcher's type step
    /// targeting this surface for in-place conversion (spec 2.7).
    fn open_type_step_for_surface(&mut self, surface_id: SurfaceId) {
        let target_name = self
            .app_state
            .workspaces
            .iter()
            .find(|workspace| {
                find_pane_id_for_surface(&workspace.split_tree, &surface_id).is_some()
            })
            .map(|workspace| workspace.title.clone())
            .unwrap_or_default();
        self.open_overlay(Overlay::QuickLaunch);
        self.pending_type_launch = Some(PendingTypeLaunch::Surface { surface_id });
        self.launcher.target_name = target_name;
        self.populate_type_items();
        self.launcher.step = LauncherStep::SessionType;
    }

    fn poll_pending_remote_launch(&mut self) {
        let Some(pending) = self.pending_remote_launch.clone() else {
            return;
        };
        let id = pending.target.surface_id.as_str();
        let _ = self.remotes.poll(id);
        match self.remotes.status(id) {
            Some(RemoteStatus::Ready) => {
                if let Some(timeline) = self.timings.get_mut(&pending.target.surface_id) {
                    timeline.mark("ready");
                }
                match crate::project_launcher::commit_prestarted(
                    &mut self.app_state,
                    &pending.target,
                    "ssh",
                    &pending.session,
                ) {
                    Ok(success) => {
                        // Non-Terminal types: type the tool command into the
                        // freshly ready remote shell (explicit user launch).
                        if let Some(command) =
                            crate::project_launcher::remote_initial_command(&pending.session)
                        {
                            let _ = self
                                .remotes
                                .write_all(id, format!("{command}\n").as_bytes());
                        }
                        if let Some(project_id) = success.project_id.clone() {
                            self.pending_git_hints.push((
                                project_id,
                                pending.target.location.clone(),
                                Some(pending.config.clone()),
                            ));
                        }
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
                        self.record_recent(success.project_id, &pending.session);
                        if self.live_ptys {
                            let _ = self.store.save_session(&self.app_state);
                        }
                        self.chrome.active_overlay = Overlay::None;
                        self.launcher.launching = false;
                        self.complete_home_assign(&pending.target.surface_id);
                        self.pending_remote_launch = None;
                    }
                    Err(error) => {
                        let _ = self.remotes.kill(id);
                        self.pending_home_assign = None;
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
                if self.pending_type_launch.is_some() {
                    // A type was already chosen; the credential was the only
                    // missing piece. Retry the launch.
                    let session = self.pending_session_type.take().unwrap_or_default();
                    self.launch_pending(session);
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
            ShellMessage::WindowClosePressed => {
                self.save_settings_now();
                window::latest().and_then(window::close)
            }
            ShellMessage::Tick => {
                self.update_shell(ShellMessage::Tick);
                Task::batch([self.settings_flush_task(), self.drain_git_hint_tasks()])
            }
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
                self.flush_pending_resizes();
                self.autosave_if_due();
                self.refresh_terminal_snapshots();
                return;
            }
            ShellMessage::ViewportResized {
                surface_id,
                columns,
                rows,
            } => {
                // Record only; the flush on a later tick applies it once the
                // size has settled. Skipping the snapshot refresh here keeps a
                // live window drag from re-snapshotting every frame.
                let size = GridSize::new(columns, rows);
                self.viewport_sizes.insert(surface_id.clone(), size);
                self.pending_resizes.insert(surface_id, (size, self.tick));
                return;
            }
            ShellMessage::ViewportScrolled { surface_id, lines } => {
                self.scroll_surface(&surface_id, lines);
            }
            ShellMessage::ViewportScrollTo { surface_id, offset } => {
                self.scroll_surface_to(&surface_id, offset);
            }
            ShellMessage::ScrollPageFocused(direction) => {
                if let Some(surface_id) = self.active_surface_id() {
                    let amount = if direction < 0 {
                        ScrollAmount::PageUp
                    } else {
                        ScrollAmount::PageDown
                    };
                    self.scroll_surface_amount(&surface_id, amount);
                }
            }
            ShellMessage::SelectionStarted {
                surface_id,
                mode,
                line,
                col,
                right_half,
            } => {
                let id = surface_id.as_str();
                if self.remotes.has(id) {
                    self.remotes
                        .start_selection(id, mode, line, col, right_half);
                } else {
                    self.ptys.start_selection(id, mode, line, col, right_half);
                }
            }
            ShellMessage::SelectionUpdated {
                surface_id,
                line,
                col,
                right_half,
            } => {
                let id = surface_id.as_str();
                if self.remotes.has(id) {
                    self.remotes.update_selection(id, line, col, right_half);
                } else {
                    self.ptys.update_selection(id, line, col, right_half);
                }
            }
            ShellMessage::SelectionFinished(_) => {
                // Selection state lives in the engine; reserved for a future
                // copy-on-select setting.
            }
            ShellMessage::ContextMenuRequested { surface_id, x, y } => {
                let id = surface_id.as_str();
                let has_selection = if self.remotes.has(id) {
                    self.remotes.has_selection(id)
                } else {
                    self.ptys.has_selection(id)
                };
                let pane_id = self.app_state.active_workspace().and_then(|workspace| {
                    pandamux_core::find_pane_id_for_surface(&workspace.split_tree, &surface_id)
                });
                self.context_menu = Some(ContextMenuViewState {
                    surface_id,
                    pane_id,
                    x,
                    y,
                    has_selection,
                });
            }
            ShellMessage::ContextMenuDismissed => {
                self.context_menu = None;
            }
            ShellMessage::ContextMenuAction(action) => {
                self.run_context_menu_action(action);
            }
            ShellMessage::CopyOrInterrupt => {
                // Overlay text inputs own their clipboard; do nothing there.
                if self.chrome.active_overlay == Overlay::None {
                    let copied = self
                        .active_surface_id()
                        .is_some_and(|surface_id| self.copy_surface_selection(&surface_id));
                    if !copied {
                        self.write_terminal_input(&[0x03]);
                    }
                }
            }
            ShellMessage::CopySelectionRequested => {
                if self.chrome.active_overlay == Overlay::None
                    && let Some(surface_id) = self.active_surface_id()
                {
                    self.copy_surface_selection(&surface_id);
                }
            }
            ShellMessage::PasteRequested => {
                if self.chrome.active_overlay == Overlay::None
                    && let Some(surface_id) = self.active_surface_id()
                {
                    self.paste_into_surface(&surface_id);
                }
            }
            ShellMessage::SelectAllRequested => {
                if self.chrome.active_overlay == Overlay::None
                    && let Some(surface_id) = self.active_surface_id()
                {
                    let id = surface_id.as_str();
                    if self.remotes.has(id) {
                        self.remotes.select_all(id);
                    } else {
                        self.ptys.select_all(id);
                    }
                }
            }
            ShellMessage::ClearBufferRequested => {
                if self.chrome.active_overlay == Overlay::None
                    && let Some(surface_id) = self.active_surface_id()
                {
                    self.clear_surface_buffer(&surface_id);
                }
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
            ShellMessage::HomeRequested => {
                self.chrome.main_view = MainView::Home;
            }
            ShellMessage::HomePaneFocused(home_pane_id) => {
                let _ = self
                    .app_state
                    .apply(AppIntent::Home(HomeIntent::Focus { home_pane_id }));
            }
            ShellMessage::HomeUnpin(home_pane_id) => {
                // Removes the pane from Home only; the session keeps running
                // and stays reachable from its project view (spec 2.5).
                let _ = self
                    .app_state
                    .apply(AppIntent::Home(HomeIntent::Unpin { home_pane_id }));
                if self.live_ptys {
                    let _ = self.store.save_session(&self.app_state);
                }
            }
            ShellMessage::HomeMove(home_pane_id, delta) => {
                let _ = self.app_state.apply(AppIntent::Home(HomeIntent::MoveBy {
                    home_pane_id,
                    delta,
                }));
                if self.live_ptys {
                    let _ = self.store.save_session(&self.app_state);
                }
            }
            ShellMessage::HomeRelaunch(home_pane_id) => {
                self.relaunch_home_pane(home_pane_id);
            }
            ShellMessage::HomeAssignRequested(home_pane_id) => {
                // Pick any open session for this pane (rail-menu style picker).
                let mut items = Vec::new();
                for workspace in &self.app_state.workspaces {
                    for surface in crate::backend::terminal_surfaces(&workspace.split_tree) {
                        let session = surface.session.clone().unwrap_or_default();
                        items.push((
                            format!("{} \u{00b7} {}", workspace.title, session.label()),
                            RailMenuAction::AssignToHomePane {
                                home_pane_id: home_pane_id.clone(),
                                surface_id: surface.id,
                            },
                        ));
                    }
                }
                self.rail_menu = Some(RailMenuViewState {
                    title: "Assign a session".to_string(),
                    items,
                });
            }
            ShellMessage::SessionContextRequested {
                workspace_id,
                surface_id,
            } => {
                let project_id = self
                    .app_state
                    .workspace(&workspace_id)
                    .and_then(|workspace| workspace.project_id.clone());
                let mut items = vec![
                    (
                        "Rename session".to_string(),
                        RailMenuAction::RenameSession {
                            workspace_id: workspace_id.clone(),
                            surface_id: surface_id.clone(),
                        },
                    ),
                    (
                        "Close session".to_string(),
                        RailMenuAction::CloseSession {
                            workspace_id: workspace_id.clone(),
                            surface_id: surface_id.clone(),
                        },
                    ),
                ];
                // Detach makes sense only when the project has other workspaces
                // to stay behind (the manual undo for a wrong merge).
                let siblings = self
                    .app_state
                    .workspaces
                    .iter()
                    .filter(|workspace| project_id.is_some() && workspace.project_id == project_id)
                    .count();
                if siblings > 1 {
                    items.insert(
                        1,
                        (
                            "Detach into its own project".to_string(),
                            RailMenuAction::DetachSession {
                                workspace_id: workspace_id.clone(),
                            },
                        ),
                    );
                }
                items.push((
                    "Pin to Home".to_string(),
                    RailMenuAction::PinToHome {
                        workspace_id: workspace_id.clone(),
                        surface_id: surface_id.clone(),
                    },
                ));
                // "Pin this configuration" (spec 2.3): project + session type.
                if let Some(project_id) = project_id {
                    let session = self
                        .app_state
                        .workspace(&workspace_id)
                        .and_then(|workspace| {
                            crate::backend::terminal_surfaces(&workspace.split_tree)
                                .into_iter()
                                .find(|surface| surface.id == surface_id)
                        })
                        .and_then(|surface| surface.session)
                        .unwrap_or_default();
                    let config = LaunchConfig {
                        project_id,
                        session,
                    };
                    let label = if self.launcher_prefs.is_favorite(&config) {
                        "Unpin this configuration"
                    } else {
                        "Pin this configuration"
                    };
                    items.push((
                        label.to_string(),
                        RailMenuAction::PinConfiguration { config },
                    ));
                }
                self.rail_menu = Some(RailMenuViewState {
                    title: "Session".to_string(),
                    items,
                });
            }
            ShellMessage::ProjectContextRequested(project_id) => {
                let name = self
                    .app_state
                    .projects
                    .iter()
                    .find(|record| record.id == project_id)
                    .map(|record| record.name.clone())
                    .unwrap_or_else(|| "Project".to_string());
                let mut items = vec![(
                    "Rename project".to_string(),
                    RailMenuAction::RenameProject {
                        project_id: project_id.clone(),
                    },
                )];
                for record in self
                    .app_state
                    .projects
                    .iter()
                    .filter(|record| record.id != project_id)
                    .take(6)
                {
                    items.push((
                        format!("Merge into {}", record.name),
                        RailMenuAction::MergeProject {
                            source: project_id.clone(),
                            target: record.id.clone(),
                        },
                    ));
                }
                items.push((
                    "Close all sessions in project".to_string(),
                    RailMenuAction::CloseAllInProject { project_id },
                ));
                self.rail_menu = Some(RailMenuViewState { title: name, items });
            }
            ShellMessage::RailMenuDismissed => {
                self.rail_menu = None;
            }
            ShellMessage::RailMenuAction(action) => {
                self.rail_menu = None;
                self.run_rail_menu_action(action);
            }
            ShellMessage::SessionRenameEdited(value) => {
                if let Some((_, _, text)) = &mut self.session_rename {
                    *text = value;
                }
            }
            ShellMessage::SessionRenameCommitted => {
                if let Some((workspace_id, surface_id, value)) = self.session_rename.take() {
                    let name = Some(value).filter(|value| !value.trim().is_empty());
                    let _ = self
                        .app_state
                        .apply(AppIntent::Surface(SurfaceIntent::Rename {
                            workspace_id: Some(workspace_id),
                            surface_id,
                            name,
                        }));
                    if self.live_ptys {
                        let _ = self.store.save_session(&self.app_state);
                    }
                }
            }
            ShellMessage::ProjectRenameEdited(value) => {
                if let Some((_, text)) = &mut self.project_rename {
                    *text = value;
                }
            }
            ShellMessage::ProjectRenameCommitted => {
                if let Some((project_id, value)) = self.project_rename.take() {
                    let name = value.trim().to_string();
                    if !name.is_empty() {
                        let _ = self
                            .app_state
                            .apply(AppIntent::Project(ProjectIntent::Rename {
                                project_id,
                                name,
                            }));
                        if self.live_ptys {
                            let _ = self.store.save_session(&self.app_state);
                        }
                    }
                }
            }
            ShellMessage::CloseAllRequested(project_id) => {
                // Destructive and possibly killing running work: confirm first
                // (spec 1.5).
                self.pending_confirm = Some(PendingConfirm::CloseAll { project_id });
                self.chrome.active_overlay = Overlay::Confirm;
            }
            ShellMessage::ConfirmAccepted => {
                self.chrome.active_overlay = Overlay::None;
                match self.pending_confirm.take() {
                    Some(PendingConfirm::CloseAll { project_id }) => {
                        let _ =
                            self.app_state
                                .apply(AppIntent::Workspace(WorkspaceIntent::CloseAll {
                                    project_id,
                                }));
                        if self.live_ptys {
                            let _ = self.store.save_session(&self.app_state);
                        }
                    }
                    None => {}
                }
            }
            ShellMessage::SessionGroupingChanged(grouping) => {
                self.chrome.session_grouping = grouping;
                self.chrome.main_view = MainView::Workspace;
            }
            ShellMessage::NewSessionRequested => self.open_overlay(Overlay::QuickLaunch),
            ShellMessage::ProjectSessionRequested(workspace_id) => {
                // The per-project plus opens the type chooser with the project
                // prefilled (spec 2.1/2.2): never a silent PowerShell clone.
                self.open_type_step_for_workspace(workspace_id, None);
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
                    LauncherStep::Folder | LauncherStep::Connection => {
                        self.populate_launcher_lists();
                        LauncherStep::Project
                    }
                    LauncherStep::SessionType => {
                        self.pending_type_launch = None;
                        self.pending_session_type = None;
                        self.populate_launcher_lists();
                        LauncherStep::Project
                    }
                    step => step,
                };
            }
            ShellMessage::LauncherFilterChanged(value) => {
                self.launcher.filter = value;
                self.launcher.selected = 0;
                self.populate_launcher_lists();
            }
            ShellMessage::LauncherProjectChosen(project_id) => {
                let Some(record) = self
                    .app_state
                    .projects
                    .iter()
                    .find(|record| record.id == project_id)
                else {
                    return;
                };
                let Some(location) = record.known_locations.first().cloned() else {
                    return;
                };
                self.launcher.target_name = record.name.clone();
                self.pending_type_launch = Some(PendingTypeLaunch::Location { location });
                self.populate_type_items();
                self.launcher.step = LauncherStep::SessionType;
            }
            ShellMessage::LauncherShortcutChosen(config) => {
                // One-click favorite/recent launch (spec 2.3).
                let location = self
                    .app_state
                    .projects
                    .iter()
                    .find(|record| record.id == config.project_id)
                    .and_then(|record| record.known_locations.first().cloned());
                let Some(location) = location else {
                    return;
                };
                self.pending_type_launch = Some(PendingTypeLaunch::Location { location });
                self.launch_pending(config.session);
            }
            ShellMessage::LauncherFavoriteToggled(config) => {
                self.launcher_prefs.toggle_favorite(config);
                self.save_launcher_prefs();
                self.populate_launcher_lists();
            }
            ShellMessage::LauncherNewSsh => {
                self.launcher.step = LauncherStep::Connection;
            }
            ShellMessage::LauncherTypeChosen(session) => {
                self.launch_pending(session);
            }
            ShellMessage::LauncherCustomCommandChanged(value) => {
                self.launcher.custom_command = value;
            }
            ShellMessage::LauncherCustomSubmitted => {
                let command = self.launcher.custom_command.trim().to_string();
                if !command.is_empty() {
                    self.launch_pending(SessionType::Custom { command });
                }
            }
            ShellMessage::TabAddRequested(pane_id) => {
                // The plus button never silently clones the current shell
                // (spec 2.1): it opens the type chooser targeting that pane.
                if let Some(workspace_id) = self.app_state.active_workspace_id.clone() {
                    self.open_type_step_for_workspace(workspace_id, Some(pane_id));
                }
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
                let had_menu = self.context_menu.is_some()
                    || self.rail_menu.is_some()
                    || self.session_rename.is_some()
                    || self.project_rename.is_some();
                self.chrome.active_overlay = Overlay::None;
                // Esc also cancels an in-flight drag, menus, renames, and any
                // parked destructive action.
                self.drag = None;
                self.context_menu = None;
                self.rail_menu = None;
                self.session_rename = None;
                self.project_rename = None;
                self.pending_confirm = None;
                // A bare Esc with nothing to dismiss goes to the terminal.
                if !had_overlay && !had_drag && !had_menu {
                    self.write_terminal_input(&[0x1b]);
                }
            }
            ShellMessage::TerminalInput(bytes) => {
                // Typing closes the context menu rather than being eaten by it.
                self.context_menu = None;
                // Suppressed while an overlay is open; its own text inputs consume
                // typing (the global key subscription still fires in parallel).
                if self.chrome.active_overlay == Overlay::None {
                    // Welcome chooser (spec 2.7): on a fresh bare terminal,
                    // 1-4 pick a tool; anything else dismisses the strip and
                    // reaches the shell normally. Never injects shell text.
                    let mut swallowed = false;
                    if let Some(surface_id) = self.active_surface_id()
                        && self.welcome_visible(&surface_id)
                    {
                        swallowed = true;
                        match bytes.as_slice() {
                            [b'1'] => {
                                self.convert_surface_session(&surface_id, SessionType::Claude)
                            }
                            [b'2'] => self.convert_surface_session(&surface_id, SessionType::Codex),
                            [b'3'] => {
                                self.convert_surface_session(&surface_id, SessionType::Gemini)
                            }
                            [b'4'] => self.open_type_step_for_surface(surface_id),
                            _ => {
                                self.welcome_dismissed.insert(surface_id);
                                swallowed = false;
                            }
                        }
                    }
                    if !swallowed {
                        self.write_terminal_input(&bytes);
                    }
                }
            }
            ShellMessage::WelcomeChosen {
                surface_id,
                session,
            } => {
                self.convert_surface_session(&surface_id, session);
            }
            ShellMessage::WelcomeCustomRequested(surface_id) => {
                self.open_type_step_for_surface(surface_id);
            }
            ShellMessage::WelcomeDismissed(surface_id) => {
                self.welcome_dismissed.insert(surface_id);
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
                // Launcher Project/SessionType steps navigate with arrows too
                // (keyboard-first, spec 2.2).
                if self.chrome.active_overlay == Overlay::QuickLaunch {
                    let count = match self.launcher.step {
                        LauncherStep::Project => self.launcher.items.len(),
                        LauncherStep::SessionType => self.launcher.type_items.len(),
                        _ => 0,
                    };
                    if count > 0 {
                        self.launcher.selected = (self.launcher.selected as i64 + delta as i64)
                            .rem_euclid(count as i64)
                            as usize;
                    }
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
                // Enter on the confirm modal accepts (spec 1.5).
                if self.chrome.active_overlay == Overlay::Confirm {
                    self.update_shell(ShellMessage::ConfirmAccepted);
                    return;
                }
                // Enter activates the highlighted launcher row on the
                // Project/SessionType steps (keyboard-first, spec 2.2).
                if self.chrome.active_overlay == Overlay::QuickLaunch {
                    let item = match self.launcher.step {
                        LauncherStep::Project => {
                            self.launcher.items.get(self.launcher.selected).cloned()
                        }
                        LauncherStep::SessionType => self
                            .launcher
                            .type_items
                            .get(self.launcher.selected)
                            .cloned(),
                        _ => None,
                    };
                    if let Some(item) = item {
                        self.update_shell(item.message);
                    }
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
            ShellMessage::SettingsSectionSelected(section) => {
                self.settings_section = section;
            }
            ShellMessage::AccentSelected(accent) => {
                self.chrome.accent = accent;
                self.sync_ui_settings();
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
                self.sync_ui_settings();
            }
            ShellMessage::ToggleTheme => {
                self.chrome.ui_theme = self.chrome.ui_theme.toggled();
                self.sync_ui_settings();
            }
            ShellMessage::CycleAccent => {
                self.chrome.accent = self.chrome.accent.next();
                self.sync_ui_settings();
            }
            ShellMessage::ScrollbackLinesChanged(value) => {
                self.scrollback_input = value.chars().filter(char::is_ascii_digit).collect();
                if let Ok(lines) = self.scrollback_input.parse::<u32>() {
                    let clamped = lines.clamp(
                        pandamux_core::settings::SCROLLBACK_LINES_MIN,
                        pandamux_core::settings::SCROLLBACK_LINES_MAX,
                    );
                    if self.settings.terminal.scrollback_lines != clamped {
                        self.settings.terminal.scrollback_lines = clamped;
                        self.apply_scrollback_setting();
                        self.mark_settings_dirty();
                    }
                }
            }
            ShellMessage::TerminalSettingToggled(toggle) => {
                let terminal = &mut self.settings.terminal;
                match toggle {
                    TerminalToggle::WelcomePrompt => {
                        terminal.welcome_prompt_enabled = !terminal.welcome_prompt_enabled;
                    }
                    TerminalToggle::RightClickPaste => {
                        terminal.right_click_paste_optin = !terminal.right_click_paste_optin;
                    }
                    TerminalToggle::ConfirmClose => {
                        terminal.confirm_close_on_running = !terminal.confirm_close_on_running;
                    }
                }
                self.mark_settings_dirty();
            }
            ShellMessage::SettingsSaved(result) => {
                if let Err(error) = result {
                    self.last_error = Some(format!("save settings: {error}"));
                }
            }
            ShellMessage::GitRemoteDiscovered { project_id, url } => {
                // Best-effort identity hint: attach the normalized remote as a
                // matcher (may auto-merge machine-created duplicates).
                if let Some(url) = url.as_deref().and_then(pandamux_core::normalize_git_remote) {
                    let applied =
                        self.app_state
                            .apply(AppIntent::Project(ProjectIntent::AttachMatcher {
                                project_id,
                                matcher: ProjectMatcher::GitRemote { url },
                            }));
                    if applied.is_ok() && self.live_ptys {
                        let _ = self.store.save_session(&self.app_state);
                    }
                }
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
                    settings: &mut self.settings,
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
                    if request.method == "config.set" {
                        // Live-apply the mutated settings and schedule a save.
                        self.apply_settings();
                        self.mark_settings_dirty();
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
        let Some(workspace_id) =
            workspace_id.or_else(|| self.app_state.active_workspace_id.clone())
        else {
            return;
        };
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
        } else {
            // The last tab of the last workspace closes too: the app lands on
            // the "All sessions ended" empty state (spec 1.5).
            AppIntent::Workspace(WorkspaceIntent::Close { workspace_id })
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
        // Home panes track live sessions; when one closes (any path: tab X,
        // close-all, workspace close) the pane degrades to a placeholder.
        let alive: HashSet<SurfaceId> = self
            .app_state
            .workspaces
            .iter()
            .flat_map(|workspace| {
                crate::backend::terminal_surfaces(&workspace.split_tree)
                    .into_iter()
                    .map(|surface| surface.id)
            })
            .collect();
        self.app_state
            .home
            .release_dead_surfaces(&|surface_id| alive.contains(surface_id));
        // Trim session-local welcome bookkeeping to live surfaces.
        self.touched_surfaces
            .retain(|surface_id| alive.contains(surface_id));
        self.welcome_dismissed
            .retain(|surface_id| alive.contains(surface_id));
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
            &mut self.timings,
            (self.chrome.main_view == MainView::Home).then(|| {
                self.app_state
                    .home
                    .panes
                    .iter()
                    .filter_map(|pane| pane.surface_id.clone())
                    .collect()
            }),
        )
        .unwrap_or_else(|error| {
            self.last_error = Some(error);
            fallback_terminal_snapshots(&self.app_state)
        });
        // Stamp the welcome chooser onto fresh, untouched bare terminals
        // (spec 2.7). Two passes to keep the borrow checker happy.
        let welcome: HashSet<SurfaceId> = self
            .terminals
            .iter()
            .map(|snapshot| snapshot.surface_id.clone())
            .filter(|surface_id| self.welcome_visible(surface_id))
            .collect();
        for snapshot in &mut self.terminals {
            snapshot.show_welcome = welcome.contains(&snapshot.surface_id);
        }
        self.recompute_find_matches();
        self.rebuild_chrome();
        let active_surface_id = self.active_surface_id();
        let pending_projects = self
            .pending_remote_launch
            .as_ref()
            .map(|pending| HashSet::from([pending.target.workspace_id.clone()]))
            .unwrap_or_default();
        let mut sessions = project_sessions_with_profiles(
            &self.app_state,
            &self.ssh_profiles,
            self.chrome.session_grouping,
            self.chrome.session_panel_open,
            active_surface_id.as_ref(),
            &pending_projects,
        );
        sessions.home_active = self.chrome.main_view == MainView::Home;
        sessions.rename = self
            .session_rename
            .as_ref()
            .map(|(_, surface_id, value)| (surface_id.clone(), value.clone()));
        sessions.project_rename = self.project_rename.clone();
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
            terminal: self.settings.terminal.clone(),
            scrollback_input: self.scrollback_input.clone(),
            ..SettingsViewState::default()
        };
        self.view_model = ShellViewModel {
            projection: self
                .app_state
                .active_workspace()
                .map(project_workspace_shell)
                .unwrap_or_else(pandamux_ui::ShellProjection::empty),
            terminals: self.terminals.clone(),
            chrome: self.chrome.clone(),
            cursor_on: self.cursor_on(),
            find: self.find.clone(),
            notifications: self.notifications_view(),
            copy_mode: self.copy_mode,
            sessions,
            palette: self.palette.clone(),
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
            context_menu: self.context_menu.clone(),
            rail_menu: self.rail_menu.clone(),
            confirm: self.confirm_view(),
            home: self.home_view_state(),
        };
    }

    /// Project the Home layout into renderable pane entries (spec 2.5).
    fn home_view_state(&self) -> pandamux_ui::HomeViewState {
        let panes = self
            .app_state
            .home
            .panes
            .iter()
            .map(|pane| {
                let live = pane.surface_id.as_ref().filter(|surface_id| {
                    self.app_state.workspaces.iter().any(|workspace| {
                        find_pane_id_for_surface(&workspace.split_tree, surface_id).is_some()
                    })
                });
                let title = self.home_pane_title(pane);
                pandamux_ui::HomePaneEntry {
                    pane_id: pane.id.clone(),
                    title,
                    surface_id: live.cloned(),
                    can_relaunch: pane.pinned.is_some(),
                    is_focused: self.app_state.home.focused_pane_id.as_ref() == Some(&pane.id),
                }
            })
            .collect();
        pandamux_ui::HomeViewState { panes }
    }

    fn home_pane_title(&self, pane: &pandamux_core::HomePane) -> String {
        // A live session shows its rail name; otherwise the pinned config.
        if let Some(surface_id) = &pane.surface_id {
            for workspace in &self.app_state.workspaces {
                if let Some(surface) = crate::backend::terminal_surfaces(&workspace.split_tree)
                    .into_iter()
                    .find(|surface| &surface.id == surface_id)
                {
                    if let Some(name) = surface.name {
                        return name;
                    }
                    let project = workspace
                        .project_id
                        .as_ref()
                        .and_then(|id| {
                            self.app_state
                                .projects
                                .iter()
                                .find(|record| &record.id == id)
                        })
                        .map(|record| record.name.clone())
                        .unwrap_or_else(|| workspace.title.clone());
                    let session = surface.session.unwrap_or_default();
                    return format!("{project} \u{00b7} {}", session.label());
                }
            }
        }
        pane.pinned
            .as_ref()
            .map(|config| {
                let project = self
                    .app_state
                    .projects
                    .iter()
                    .find(|record| record.id == config.project_id)
                    .map(|record| record.name.clone())
                    .unwrap_or_else(|| "Project".to_string());
                format!("{project} \u{00b7} {}", config.session.label())
            })
            .unwrap_or_else(|| "Unassigned".to_string())
    }

    /// The confirm modal's content for whatever destructive action is parked.
    fn confirm_view(&self) -> Option<pandamux_ui::ConfirmViewState> {
        match self.pending_confirm.as_ref()? {
            PendingConfirm::CloseAll { project_id } => {
                let scope = project_id
                    .as_ref()
                    .and_then(|id| {
                        self.app_state
                            .projects
                            .iter()
                            .find(|record| &record.id == id)
                    })
                    .map(|record| format!("every session in {}", record.name))
                    .unwrap_or_else(|| "every open session".to_string());
                Some(pandamux_ui::ConfirmViewState {
                    title: "Close all sessions?".to_string(),
                    body: format!("This closes {scope} and may end running work."),
                    action_label: "Close all".to_string(),
                })
            }
        }
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
                "\u{00d7}",
                "Close all sessions",
                None,
                ShellMessage::CloseAllRequested(None),
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
    /// The focused pane's active surface (the "active session"), if any. On
    /// the Home dashboard this is the focused Home pane's session, so all
    /// keyboard input routes there (spec 2.5): one branch point for focus.
    fn active_surface_id(&self) -> Option<SurfaceId> {
        if self.chrome.main_view == MainView::Home {
            let focused = self.app_state.home.focused_pane_id.as_ref()?;
            return self
                .app_state
                .home
                .pane(focused)
                .and_then(|pane| pane.surface_id.clone());
        }
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
        if bytes.is_empty() {
            return;
        }
        let Some(surface_id) = self.active_surface_id() else {
            return;
        };
        // First keystroke retires the welcome chooser for good (spec 2.7).
        self.touched_surfaces.insert(surface_id.clone());
        if !self.live_ptys {
            return;
        }
        // Typing snaps the view back to the tail (spec 1.2).
        self.scroll_surface_amount(&surface_id, ScrollAmount::Bottom);
        self.write_surface_input(&surface_id, bytes);
    }

    /// Write bytes to a specific surface's PTY or SSH channel.
    fn write_surface_input(&mut self, surface_id: &SurfaceId, bytes: &[u8]) {
        if bytes.is_empty() || !self.live_ptys {
            return;
        }
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

    /// Route a wheel gesture: engine scrollback normally; arrow-key translation
    /// on the alternate screen when the app opted into AlternateScroll (the
    /// Windows Terminal behavior); nothing while the app owns the mouse (full
    /// SGR mouse forwarding is a documented fast-follow).
    fn scroll_surface(&mut self, surface_id: &SurfaceId, lines: i32) {
        let id = surface_id.as_str();
        let modes = if self.remotes.has(id) {
            self.remotes.modes(id)
        } else {
            self.ptys.modes(id)
        };
        if modes.mouse_reporting {
            return;
        }
        if modes.alt_screen {
            if modes.alternate_scroll {
                let sequence: &[u8] = match (lines > 0, modes.app_cursor) {
                    (true, true) => b"\x1bOA",
                    (true, false) => b"\x1b[A",
                    (false, true) => b"\x1bOB",
                    (false, false) => b"\x1b[B",
                };
                let count = lines.unsigned_abs().min(120) as usize;
                let mut bytes = Vec::with_capacity(count * sequence.len());
                for _ in 0..count {
                    bytes.extend_from_slice(sequence);
                }
                self.write_surface_input(surface_id, &bytes);
            }
            return;
        }
        self.scroll_surface_amount(surface_id, ScrollAmount::Lines(lines));
    }

    /// Copy a surface's selection to the OS clipboard. Returns true when a
    /// non-empty selection was copied (the highlight clears afterwards).
    fn copy_surface_selection(&mut self, surface_id: &SurfaceId) -> bool {
        let id = surface_id.as_str();
        let text = if self.remotes.has(id) {
            self.remotes.selection_text(id)
        } else {
            self.ptys.selection_text(id)
        };
        let Some(text) = text.filter(|text| !text.is_empty()) else {
            return false;
        };
        if let Err(error) = crate::clipboard_os::set_text(&text) {
            self.last_error = Some(error);
            return false;
        }
        if self.remotes.has(id) {
            self.remotes.clear_selection(id);
        } else {
            self.ptys.clear_selection(id);
        }
        true
    }

    /// Paste the OS clipboard into a surface, bracket-wrapped when the app has
    /// requested bracketed paste (multi-line text arrives intact); otherwise
    /// newlines normalize to carriage returns as a raw terminal expects.
    fn paste_into_surface(&mut self, surface_id: &SurfaceId) {
        let text = match crate::clipboard_os::get_text() {
            Ok(text) if !text.is_empty() => text,
            // An empty or non-text clipboard is a quiet no-op, not an error.
            _ => return,
        };
        const MAX_PASTE_BYTES: usize = 8 * 1024 * 1024;
        if text.len() > MAX_PASTE_BYTES {
            self.last_error = Some("clipboard text exceeds the 8 MiB paste cap".to_string());
            return;
        }
        let id = surface_id.as_str();
        let bracketed = if self.remotes.has(id) {
            self.remotes.bracketed_paste_active(id)
        } else {
            self.ptys.bracketed_paste_active(id)
        };
        let payload = paste_payload(&text, bracketed);
        self.scroll_surface_amount(surface_id, ScrollAmount::Bottom);
        self.write_surface_input(surface_id, &payload);
    }

    /// Clear a surface's scrollback, then nudge the shell to repaint its
    /// prompt (Ctrl+L semantics, matching Windows Terminal's Clear Buffer).
    fn clear_surface_buffer(&mut self, surface_id: &SurfaceId) {
        let id = surface_id.as_str();
        if self.remotes.has(id) {
            self.remotes.clear_buffer(id);
        } else {
            self.ptys.clear_buffer(id);
        }
        self.write_surface_input(surface_id, &[0x0c]);
    }

    /// Dispatch a rail-menu action (session rows and project headers).
    fn run_rail_menu_action(&mut self, action: RailMenuAction) {
        match action {
            RailMenuAction::RenameSession {
                workspace_id,
                surface_id,
            } => {
                // Prefill with the existing custom name (empty when derived).
                let current = self
                    .app_state
                    .workspace(&workspace_id)
                    .map(|workspace| {
                        crate::backend::terminal_surfaces(&workspace.split_tree)
                            .into_iter()
                            .find(|surface| surface.id == surface_id)
                            .and_then(|surface| surface.name)
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                self.session_rename = Some((workspace_id, surface_id, current));
            }
            RailMenuAction::DetachSession { workspace_id } => {
                let _ = self
                    .app_state
                    .apply(AppIntent::Project(ProjectIntent::Split { workspace_id }));
                if self.live_ptys {
                    let _ = self.store.save_session(&self.app_state);
                }
            }
            RailMenuAction::CloseSession {
                workspace_id,
                surface_id,
            } => self.close_session(Some(workspace_id), surface_id),
            RailMenuAction::RenameProject { project_id } => {
                let current = self
                    .app_state
                    .projects
                    .iter()
                    .find(|record| record.id == project_id)
                    .map(|record| record.name.clone())
                    .unwrap_or_default();
                self.project_rename = Some((project_id, current));
            }
            RailMenuAction::MergeProject { source, target } => {
                match self
                    .app_state
                    .apply(AppIntent::Project(ProjectIntent::Merge { source, target }))
                {
                    Ok(_) => {
                        if self.live_ptys {
                            let _ = self.store.save_session(&self.app_state);
                        }
                    }
                    Err(error) => self.last_error = Some(error),
                }
            }
            RailMenuAction::CloseAllInProject { project_id } => {
                self.update_shell(ShellMessage::CloseAllRequested(Some(project_id)));
            }
            RailMenuAction::PinConfiguration { config } => {
                let pinned = self.launcher_prefs.toggle_favorite(config);
                self.save_launcher_prefs();
                let _ = pinned;
            }
            RailMenuAction::PinToHome {
                workspace_id,
                surface_id,
            } => {
                let pinned = self.launch_config_for(&workspace_id, &surface_id);
                let _ = self
                    .app_state
                    .apply(AppIntent::Home(HomeIntent::Pin { surface_id, pinned }));
                self.chrome.main_view = MainView::Home;
                if self.live_ptys {
                    let _ = self.store.save_session(&self.app_state);
                }
            }
            RailMenuAction::AssignToHomePane {
                home_pane_id,
                surface_id,
            } => {
                let pinned = self
                    .app_state
                    .workspaces
                    .iter()
                    .find(|workspace| {
                        find_pane_id_for_surface(&workspace.split_tree, &surface_id).is_some()
                    })
                    .map(|workspace| workspace.id.clone())
                    .and_then(|workspace_id| self.launch_config_for(&workspace_id, &surface_id));
                let _ = self.app_state.apply(AppIntent::Home(HomeIntent::Assign {
                    home_pane_id,
                    surface_id,
                    pinned,
                }));
                if self.live_ptys {
                    let _ = self.store.save_session(&self.app_state);
                }
            }
        }
    }

    /// The pinned configuration (project + type) behind a live session, when
    /// its workspace has a registry identity.
    fn launch_config_for(
        &self,
        workspace_id: &WorkspaceId,
        surface_id: &SurfaceId,
    ) -> Option<LaunchConfig> {
        let workspace = self.app_state.workspace(workspace_id)?;
        let project_id = workspace.project_id.clone()?;
        let session = crate::backend::terminal_surfaces(&workspace.split_tree)
            .into_iter()
            .find(|surface| &surface.id == surface_id)
            .and_then(|surface| surface.session)
            .unwrap_or_default();
        Some(LaunchConfig {
            project_id,
            session,
        })
    }

    /// Relaunch a dead Home pane from its pinned configuration and point the
    /// pane at the new session once the launch completes.
    fn relaunch_home_pane(&mut self, home_pane_id: PaneId) {
        let Some(config) = self
            .app_state
            .home
            .pane(&home_pane_id)
            .and_then(|pane| pane.pinned.clone())
        else {
            return;
        };
        let Some(location) = self
            .app_state
            .projects
            .iter()
            .find(|record| record.id == config.project_id)
            .and_then(|record| record.known_locations.first().cloned())
        else {
            self.last_error = Some("no recorded folder to relaunch from".to_string());
            return;
        };
        self.pending_home_assign = Some(home_pane_id);
        self.pending_type_launch = Some(PendingTypeLaunch::Location { location });
        self.launch_pending(config.session);
    }

    /// After a launch completes, point the waiting Home pane (if any) at the
    /// fresh session and return to the dashboard.
    fn complete_home_assign(&mut self, surface_id: &SurfaceId) {
        let Some(home_pane_id) = self.pending_home_assign.take() else {
            return;
        };
        let _ = self.app_state.apply(AppIntent::Home(HomeIntent::Assign {
            home_pane_id,
            surface_id: surface_id.clone(),
            pinned: None,
        }));
        self.chrome.main_view = MainView::Home;
        if self.live_ptys {
            let _ = self.store.save_session(&self.app_state);
        }
    }

    /// Dispatch a context-menu item against the surface the menu targeted.
    fn run_context_menu_action(&mut self, action: ContextMenuAction) {
        let Some(menu) = self.context_menu.take() else {
            return;
        };
        match action {
            ContextMenuAction::Copy => {
                self.copy_surface_selection(&menu.surface_id);
            }
            ContextMenuAction::Paste => self.paste_into_surface(&menu.surface_id),
            ContextMenuAction::SelectAll => {
                let id = menu.surface_id.as_str();
                if self.remotes.has(id) {
                    self.remotes.select_all(id);
                } else {
                    self.ptys.select_all(id);
                }
            }
            ContextMenuAction::ClearBuffer => self.clear_surface_buffer(&menu.surface_id),
            ContextMenuAction::Find => self.update_shell(ShellMessage::FindOpened),
            ContextMenuAction::SplitRight => {
                if let Some(pane_id) = menu.pane_id {
                    self.update_shell(ShellMessage::PaneSplit {
                        pane_id,
                        direction: SplitDirection::Horizontal,
                    });
                }
            }
            ContextMenuAction::SplitDown => {
                if let Some(pane_id) = menu.pane_id {
                    self.update_shell(ShellMessage::PaneSplit {
                        pane_id,
                        direction: SplitDirection::Vertical,
                    });
                }
            }
            ContextMenuAction::CloseTab => {
                self.update_shell(ShellMessage::SurfaceClosed(menu.surface_id));
            }
        }
    }

    fn scroll_surface_amount(&mut self, surface_id: &SurfaceId, amount: ScrollAmount) {
        let id = surface_id.as_str();
        if self.remotes.has(id) {
            self.remotes.scroll_display(id, amount);
        } else {
            self.ptys.scroll_display(id, amount);
        }
    }

    /// Scroll a surface to an absolute history offset (scrollbar drag / pill).
    fn scroll_surface_to(&mut self, surface_id: &SurfaceId, offset: usize) {
        let id = surface_id.as_str();
        let current = if self.remotes.has(id) {
            self.remotes.display_offset(id)
        } else {
            self.ptys.display_offset(id)
        };
        let delta = (offset as i64 - current as i64).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        if delta != 0 {
            self.scroll_surface_amount(surface_id, ScrollAmount::Lines(delta));
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

    /// Push the loaded settings into live state: chrome preferences and the
    /// engine scrollback limits. Called at startup and after `config.set`.
    fn apply_settings(&mut self) {
        self.chrome.ui_theme = theme_from_setting(&self.settings.ui.theme);
        self.chrome.accent = accent_from_setting(&self.settings.ui.accent);
        self.chrome.show_status_bar = self.settings.ui.show_status_bar;
        self.scrollback_input = self.settings.terminal.scrollback_lines.to_string();
        self.apply_scrollback_setting();
    }

    fn apply_scrollback_setting(&mut self) {
        let lines = self.settings.terminal.scrollback_lines as usize;
        self.ptys.set_scrollback_lines(lines);
        self.remotes.set_scrollback_lines(lines);
    }

    /// Mirror the chrome preferences into the settings and schedule a save.
    fn sync_ui_settings(&mut self) {
        self.settings.ui.theme = theme_to_setting(self.chrome.ui_theme).to_string();
        self.settings.ui.accent = accent_to_setting(self.chrome.accent).to_string();
        self.settings.ui.show_status_bar = self.chrome.show_status_bar;
        self.mark_settings_dirty();
    }

    fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
        self.settings_dirty_since = self.tick;
    }

    /// Debounced async settings save: runs once changes have been quiet for
    /// [`SETTINGS_SAVE_SETTLE_TICKS`] (~500ms). File IO happens on a blocking
    /// worker, never the async executor.
    fn settings_flush_task(&mut self) -> Task<ShellMessage> {
        if !self.settings_dirty
            || !self.live_ptys
            || !self.settings_store_available
            || self.tick.wrapping_sub(self.settings_dirty_since) < SETTINGS_SAVE_SETTLE_TICKS
        {
            return Task::none();
        }
        self.settings_dirty = false;
        let dir = self.settings_dir.clone();
        let snapshot = self.settings.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    crate::persistence::SettingsStore::new(dir)
                        .save(&snapshot)
                        .map_err(|error| error.to_string())
                })
                .await
                .unwrap_or_else(|error| Err(error.to_string()))
            },
            ShellMessage::SettingsSaved,
        )
    }

    /// Turn queued git-remote probes into async tasks (spec 1.4). Each task
    /// reads `.git/config` locally (tokio::fs) or over SFTP with a 2s timeout,
    /// and reports back as `GitRemoteDiscovered`.
    fn drain_git_hint_tasks(&mut self) -> Task<ShellMessage> {
        if self.pending_git_hints.is_empty() {
            return Task::none();
        }
        let pending = std::mem::take(&mut self.pending_git_hints);
        let tasks: Vec<Task<ShellMessage>> = pending
            .into_iter()
            .map(|(project_id, location, config)| {
                Task::perform(fetch_git_remote(location, config), move |url| {
                    ShellMessage::GitRemoteDiscovered {
                        project_id: project_id.clone(),
                        url,
                    }
                })
            })
            .collect();
        Task::batch(tasks)
    }

    /// Synchronous flush used on window close (the async task would race the
    /// process exit).
    fn save_settings_now(&mut self) {
        if !self.settings_dirty || !self.live_ptys || !self.settings_store_available {
            return;
        }
        self.settings_dirty = false;
        let _ = crate::persistence::SettingsStore::new(&self.settings_dir).save(&self.settings);
    }

    /// Apply viewport-driven resizes that have settled for
    /// [`RESIZE_SETTLE_TICKS`], collapsing a live window drag into one
    /// engine + PTY/SSH resize.
    fn flush_pending_resizes(&mut self) {
        if self.pending_resizes.is_empty() {
            return;
        }
        let due: Vec<SurfaceId> = self
            .pending_resizes
            .iter()
            .filter(|(_, (_, recorded))| self.tick.wrapping_sub(*recorded) >= RESIZE_SETTLE_TICKS)
            .map(|(surface_id, _)| surface_id.clone())
            .collect();
        for surface_id in due {
            let Some((size, _)) = self.pending_resizes.remove(&surface_id) else {
                continue;
            };
            let id = surface_id.to_string();
            if self.remotes.has(&id) {
                let _ = self.remotes.resize(&id, size);
            } else if self.ptys.has(&id) {
                let _ = self.ptys.resize(&id, size);
            }
        }
    }

    /// The grid size for a new or restored session: the surface's last
    /// viewport-derived size, else the focused pane's, else the default. A
    /// brand-new surface corrects itself on its first redraw.
    fn spawn_size(&self, surface_id: &SurfaceId) -> GridSize {
        if let Some(size) = self.viewport_sizes.get(surface_id) {
            return *size;
        }
        self.focused_viewport_size()
    }

    /// The focused pane's viewport size, or the default before any pane has
    /// reported one. Used as the first guess for surfaces that do not exist in
    /// the layout yet (launcher flows).
    fn focused_viewport_size(&self) -> GridSize {
        self.active_surface_id()
            .and_then(|surface_id| self.viewport_sizes.get(&surface_id))
            .copied()
            .unwrap_or(DEFAULT_GRID_SIZE)
    }

    fn sync_terminal_sessions(&mut self) -> Result<(), String> {
        if !self.live_ptys {
            return Ok(());
        }

        let mut expected_session_ids = HashSet::new();
        for workspace in &self.app_state.workspaces {
            for surface in crate::backend::terminal_surfaces(&workspace.split_tree) {
                let surface_id = surface.id.clone();
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
                                let size = self.spawn_size(&surface_id);
                                self.timings.insert(
                                    surface_id.clone(),
                                    crate::latency::LaunchTimeline::start(format!(
                                        "ssh {}",
                                        config.host
                                    )),
                                );
                                self.remotes
                                    .connect(session_id.clone(), config.clone(), size)?;
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
                // Session-type-aware respawn: a restored Claude tab comes back
                // as Claude, not a bare shell (spec 2.2).
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
                let size = self.spawn_size(&surface_id);
                self.timings.insert(
                    surface_id.clone(),
                    crate::latency::LaunchTimeline::start(shell_label(&command.program)),
                );
                self.ptys
                    .spawn(session_id, &command, size)
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
        // Shift+PageUp/PageDown scroll the viewport (Windows Terminal
        // convention); unshifted stays terminal input.
        Named::PageUp => {
            if shift {
                ShellMessage::ScrollPageFocused(-1)
            } else {
                ShellMessage::TerminalInput(b"\x1b[5~".to_vec())
            }
        }
        Named::PageDown => {
            if shift {
                ShellMessage::ScrollPageFocused(1)
            } else {
                ShellMessage::TerminalInput(b"\x1b[6~".to_vec())
            }
        }
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
        // Clipboard (spec 1.3). Ctrl+C stays SIGINT when nothing is selected;
        // Ctrl+V pastes instead of sending a literal 0x16.
        (false, "c") => ShellMessage::CopyOrInterrupt,
        (true, "c") => ShellMessage::CopySelectionRequested,
        (false, "v") | (true, "v") => ShellMessage::PasteRequested,
        _ => ShellMessage::Noop,
    }
}

/// Build the paste byte payload: bracketed pastes pass the text through inside
/// the markers; unbracketed pastes normalize newlines to carriage returns so
/// multi-line text arrives as a raw shell expects.
fn paste_payload(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        return wrap_paste(text.as_bytes(), true);
    }
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

fn theme_iced_shell(state: &NativeShellRuntime) -> Theme {
    match state.chrome.ui_theme {
        UiTheme::Dark => Theme::Dark,
        UiTheme::Light => Theme::Light,
    }
}

// Settings string <-> UI enum mapping. Unknown values fall back to defaults so
// a hand-edited settings file never breaks startup.
fn theme_from_setting(value: &str) -> UiTheme {
    match value {
        "light" => UiTheme::Light,
        _ => UiTheme::Dark,
    }
}

fn theme_to_setting(theme: UiTheme) -> &'static str {
    match theme {
        UiTheme::Dark => "dark",
        UiTheme::Light => "light",
    }
}

fn accent_from_setting(value: &str) -> Accent {
    match value {
        "gold" => Accent::Gold,
        "blue" => Accent::Blue,
        "mauve" => Accent::Mauve,
        _ => Accent::Teal,
    }
}

fn accent_to_setting(accent: Accent) -> &'static str {
    match accent {
        Accent::Teal => "teal",
        Accent::Gold => "gold",
        Accent::Blue => "blue",
        Accent::Mauve => "mauve",
    }
}

fn terminal_snapshots(
    app_state: &AppState,
    ptys: &mut PtySessionManager,
    remotes: &mut RemoteSessionManager,
    remote_configs: &HashMap<SurfaceId, SshConfig>,
    live_ptys: bool,
    timings: &mut HashMap<SurfaceId, crate::latency::LaunchTimeline>,
    home_scope: Option<Vec<SurfaceId>>,
) -> Result<Vec<TerminalSnapshot>, String> {
    // What is visible: the active workspace's pane surfaces, or (on the Home
    // dashboard) the surfaces its panes reference from ANY project (spec 2.5).
    let surface_ids: Vec<SurfaceId> = match home_scope {
        Some(ids) => ids,
        None => {
            let Some(workspace) = app_state.active_workspace() else {
                return Ok(Vec::new());
            };
            project_workspace_shell(workspace)
                .visible_panes
                .into_iter()
                .filter_map(|pane| {
                    let surface_id = pane.active_surface_id?;
                    let is_terminal = pane.surfaces.iter().any(|surface| {
                        surface.id == surface_id && surface.surface_type == SurfaceType::Terminal
                    });
                    is_terminal.then_some(surface_id)
                })
                .collect()
        }
    };
    let snapshots = surface_ids
        .into_iter()
        .filter_map(|surface_id| {
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
            let default_view = || (0_usize, 0_usize, Vec::new(), true, TermModes::default());
            let (view, lines, cells, cursor, columns, rows) = match screen {
                Some(screen) => {
                    let lines = cells_to_lines(&screen.rows);
                    let columns = screen.rows.iter().map(|row| row.len()).max().unwrap_or(120);
                    let rows = screen.rows.len();
                    (
                        (
                            screen.display_offset,
                            screen.history_size,
                            screen.selection,
                            screen.cursor_visible,
                            screen.modes,
                        ),
                        lines,
                        screen.rows,
                        screen.cursor,
                        columns,
                        rows,
                    )
                }
                None if is_remote => (default_view(), Vec::new(), Vec::new(), (0, 0), 120, 30),
                None => (
                    default_view(),
                    fallback_lines(),
                    Vec::new(),
                    (0, 0),
                    120,
                    30,
                ),
            };
            let (display_offset, history_size, selection, cursor_visible, modes) = view;

            // Launch timing: the first real engine output completes and reports
            // the surface's timeline (checked before any synthesized text).
            let has_output = lines.iter().any(|line| !line.trim().is_empty());
            if has_output && let Some(mut timeline) = timings.remove(&surface_id) {
                timeline.mark("first_output");
                timeline.report();
            }
            // Connecting feedback (spec 1.6): a blank pane shows an honest
            // status line within the first frame instead of dead black.
            let (lines, cells) = if !has_output && live_ptys {
                let status = if is_remote {
                    let host = remote_configs
                        .get(&surface_id)
                        .map(|config| config.host.as_str())
                        .unwrap_or("host");
                    match remotes.status(surface_id.as_str()) {
                        Some(RemoteStatus::Connecting) => Some(format!("Connecting to {host}...")),
                        Some(RemoteStatus::Retrying) | Some(RemoteStatus::Disconnected) => {
                            Some(format!("Reconnecting to {host}..."))
                        }
                        Some(RemoteStatus::Failed) => Some(format!("Connection to {host} failed")),
                        _ => None,
                    }
                } else {
                    timings
                        .get(&surface_id)
                        .filter(|timeline| timeline.age() < Duration::from_secs(5))
                        .map(|timeline| format!("Starting {}...", timeline.label()))
                };
                match status {
                    Some(line) => (vec![line], Vec::new()),
                    None => (lines, cells),
                }
            } else {
                (lines, cells)
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
                display_offset,
                history_size,
                selection,
                cursor_visible,
                modes,
                // Stamped by refresh_terminal_snapshots from runtime-local
                // touch/dismiss state (spec 2.7).
                show_welcome: false,
            })
        })
        .collect();
    Ok(snapshots)
}

/// A minimal view model for construction time; overwritten by the first
/// `refresh_terminal_snapshots` call.
fn initial_view_model(app_state: &AppState, chrome: &ChromeState) -> ShellViewModel {
    ShellViewModel {
        projection: app_state
            .active_workspace()
            .map(project_workspace_shell)
            .unwrap_or_else(pandamux_ui::ShellProjection::empty),
        terminals: Vec::new(),
        chrome: chrome.clone(),
        cursor_on: true,
        find: FindViewState::default(),
        notifications: NotificationsViewState::default(),
        copy_mode: false,
        sessions: SessionsViewState::default(),
        palette: PaletteViewState::default(),
        launcher: SessionLauncherViewState::default(),
        settings: SettingsViewState::default(),
        surface_contents: HashMap::new(),
        drag: None,
        term_scheme: TermScheme::default(),
        surface_term_schemes: HashMap::new(),
        context_menu: None,
        rail_menu: None,
        confirm: None,
        home: pandamux_ui::HomeViewState::default(),
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

/// Short display detail for a project location (launcher rows).
fn location_detail(location: Option<&ProjectLocation>) -> String {
    match location {
        Some(ProjectLocation::Local { cwd, .. }) => cwd.clone(),
        Some(ProjectLocation::Ssh { remote_cwd, .. }) => format!("SSH \u{00b7} {remote_cwd}"),
        Some(ProjectLocation::Legacy) | None => "No folder recorded".to_string(),
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
        &mut HashMap::new(),
        None,
    )
    .unwrap_or_default()
}

/// Read a project's git remote URL for identity matching (spec 1.4). Local
/// checkouts read `.git/config` via tokio::fs; SSH checkouts read it over a
/// fresh SFTP session with a 2 second budget. Always best-effort.
async fn fetch_git_remote(location: ProjectLocation, config: Option<SshConfig>) -> Option<String> {
    match location {
        ProjectLocation::Local { cwd, .. } => {
            let path = std::path::Path::new(&cwd).join(".git").join("config");
            let text = tokio::fs::read_to_string(path).await.ok()?;
            pandamux_core::parse_git_remote_url(&text)
        }
        ProjectLocation::Ssh { remote_cwd, .. } => {
            let config = config?;
            let path = format!("{}/.git/config", remote_cwd.trim_end_matches('/'));
            let text = tokio::time::timeout(
                Duration::from_secs(2),
                pandamux_term::read_remote_file(config, path, 64 * 1024),
            )
            .await
            .ok()?
            .ok()?;
            pandamux_core::parse_git_remote_url(&text)
        }
        ProjectLocation::Legacy => None,
    }
}

/// Short display name for a shell program path ("C:\\...\\pwsh.exe" -> "pwsh").
fn shell_label(program: &str) -> String {
    program
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(program)
        .trim_end_matches(".exe")
        .to_string()
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
        let default_ws = runtime
            .app_state
            .active_workspace_id
            .clone()
            .expect("default workspace");
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
        let original_ws = runtime
            .app_state
            .active_workspace_id
            .clone()
            .expect("default workspace");
        assert_eq!(runtime.view_model().sessions.total, 1);

        // A second workspace switches the active session context.
        runtime
            .app_state
            .apply(AppIntent::Workspace(WorkspaceIntent::Create {
                title: Some("PowerShell 7".to_string()),
                shell: Some("pwsh".to_string()),
            }))
            .expect("create workspace");
        runtime.update_shell(ShellMessage::Tick);
        assert_ne!(
            runtime.app_state.active_workspace_id,
            Some(original_ws.clone())
        );
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
        assert_eq!(
            runtime.app_state.active_workspace_id,
            Some(original_ws.clone())
        );
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

        // New-session opens the launcher on the Project step with rows for
        // the new-project entry points; Escape dismisses it.
        runtime.update_shell(ShellMessage::NewSessionRequested);
        assert_eq!(
            runtime.view_model().chrome.active_overlay,
            Overlay::QuickLaunch
        );
        assert_eq!(runtime.view_model().launcher.step, LauncherStep::Project);
        assert!(
            runtime
                .view_model()
                .launcher
                .items
                .iter()
                .any(|item| item.label == "New local folder")
        );
        runtime.update_shell(ShellMessage::OverlayDismissed);
        assert_eq!(runtime.view_model().chrome.active_overlay, Overlay::None);

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
    fn shift_page_keys_scroll_while_plain_page_keys_stay_terminal_input() {
        use keyboard::key::Named;
        assert_eq!(
            map_named_key(Named::PageUp, false, true),
            ShellMessage::ScrollPageFocused(-1)
        );
        assert_eq!(
            map_named_key(Named::PageDown, false, true),
            ShellMessage::ScrollPageFocused(1)
        );
        assert_eq!(
            map_named_key(Named::PageUp, false, false),
            ShellMessage::TerminalInput(b"\x1b[5~".to_vec())
        );
        assert_eq!(
            map_named_key(Named::PageDown, false, false),
            ShellMessage::TerminalInput(b"\x1b[6~".to_vec())
        );
    }

    #[test]
    fn clipboard_shortcuts_map_copy_and_paste() {
        assert_eq!(
            shortcut_for(true, false, "c"),
            ShellMessage::CopyOrInterrupt
        );
        assert_eq!(
            shortcut_for(true, true, "c"),
            ShellMessage::CopySelectionRequested
        );
        assert_eq!(shortcut_for(true, false, "v"), ShellMessage::PasteRequested);
        assert_eq!(shortcut_for(true, true, "v"), ShellMessage::PasteRequested);
    }

    #[test]
    fn paste_payload_normalizes_newlines_only_when_unbracketed() {
        // Raw shells get carriage returns for every newline flavor.
        assert_eq!(paste_payload("a\r\nb\nc", false), b"a\rb\rc".to_vec());
        // Bracketed pastes keep the bytes intact inside the markers.
        let wrapped = paste_payload("a\r\nb\nc", true);
        assert!(wrapped.starts_with(b"\x1b[200~"));
        assert!(wrapped.ends_with(b"\x1b[201~"));
        let inner = &wrapped[6..wrapped.len() - 6];
        assert_eq!(inner, b"a\r\nb\nc");
    }

    #[test]
    fn context_menu_opens_targets_surface_and_dismisses() {
        let mut runtime = NativeShellRuntime::default();
        let surface_id = runtime.active_surface_id().expect("focused surface");

        runtime.update_shell(ShellMessage::ContextMenuRequested {
            surface_id: surface_id.clone(),
            x: 120.0,
            y: 90.0,
        });
        let menu = runtime
            .view_model()
            .context_menu
            .clone()
            .expect("menu open");
        assert_eq!(menu.surface_id, surface_id);
        assert!(menu.pane_id.is_some(), "pane resolved for split items");
        assert!(!menu.has_selection, "no selection without a live session");

        // Esc closes the menu without sending ESC to the terminal.
        runtime.update_shell(ShellMessage::OverlayDismissed);
        assert!(runtime.view_model().context_menu.is_none());
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    fn menu_split_action_splits_the_pane() {
        let mut runtime = NativeShellRuntime::default();
        let surface_id = runtime.active_surface_id().expect("focused surface");
        runtime.update_shell(ShellMessage::ContextMenuRequested {
            surface_id,
            x: 0.0,
            y: 0.0,
        });
        assert_eq!(runtime.view_model().projection.visible_panes.len(), 1);
        runtime.update_shell(ShellMessage::ContextMenuAction(
            ContextMenuAction::SplitRight,
        ));
        assert!(runtime.view_model().context_menu.is_none());
        assert_eq!(runtime.view_model().projection.visible_panes.len(), 2);
    }

    #[test]
    fn ui_preference_changes_mark_settings_dirty() {
        let mut runtime = NativeShellRuntime::default();
        assert!(!runtime.settings_dirty);
        runtime.update_shell(ShellMessage::ToggleTheme);
        assert!(runtime.settings_dirty);
        assert_eq!(runtime.settings.ui.theme, "light");
        runtime.update_shell(ShellMessage::ToggleStatusBar);
        assert!(!runtime.settings.ui.show_status_bar);
    }

    #[test]
    fn scrollback_input_filters_digits_and_clamps() {
        let mut runtime = NativeShellRuntime::default();
        runtime.update_shell(ShellMessage::ScrollbackLinesChanged("50k00".to_string()));
        assert_eq!(runtime.scrollback_input, "5000");
        assert_eq!(runtime.settings.terminal.scrollback_lines, 5_000);
        runtime.update_shell(ShellMessage::ScrollbackLinesChanged("5".to_string()));
        assert_eq!(
            runtime.settings.terminal.scrollback_lines,
            pandamux_core::settings::SCROLLBACK_LINES_MIN
        );
        assert!(runtime.settings_dirty);
    }

    #[test]
    fn terminal_toggles_flip_settings() {
        let mut runtime = NativeShellRuntime::default();
        runtime.update_shell(ShellMessage::TerminalSettingToggled(
            TerminalToggle::WelcomePrompt,
        ));
        assert!(!runtime.settings.terminal.welcome_prompt_enabled);
        runtime.update_shell(ShellMessage::TerminalSettingToggled(
            TerminalToggle::RightClickPaste,
        ));
        assert!(runtime.settings.terminal.right_click_paste_optin);
        runtime.update_shell(ShellMessage::TerminalSettingToggled(
            TerminalToggle::ConfirmClose,
        ));
        assert!(!runtime.settings.terminal.confirm_close_on_running);
    }

    #[test]
    fn home_switcher_toggles_main_view() {
        let mut runtime = NativeShellRuntime::default();
        assert_eq!(runtime.view_model().chrome.main_view, MainView::Workspace);
        runtime.update_shell(ShellMessage::HomeRequested);
        assert_eq!(runtime.view_model().chrome.main_view, MainView::Home);
        assert!(runtime.view_model().sessions.home_active);
        // Picking a grouping returns to the workspace view.
        runtime.update_shell(ShellMessage::SessionGroupingChanged(
            pandamux_ui::SessionGrouping::Type,
        ));
        assert_eq!(runtime.view_model().chrome.main_view, MainView::Workspace);
        assert!(!runtime.view_model().sessions.home_active);
        // The Home view still builds.
        runtime.update_shell(ShellMessage::HomeRequested);
        let _view = app_view(runtime.view_model());
    }

    #[test]
    fn session_rename_flows_through_the_rail_menu() {
        let mut runtime = NativeShellRuntime::default();
        let surface_id = runtime.active_surface_id().expect("focused surface");
        let workspace_id = runtime
            .app_state
            .active_workspace_id
            .clone()
            .expect("default workspace");

        runtime.update_shell(ShellMessage::SessionContextRequested {
            workspace_id: workspace_id.clone(),
            surface_id: surface_id.clone(),
        });
        assert!(runtime.view_model().rail_menu.is_some());
        runtime.update_shell(ShellMessage::RailMenuAction(
            RailMenuAction::RenameSession {
                workspace_id: workspace_id.clone(),
                surface_id: surface_id.clone(),
            },
        ));
        assert!(runtime.view_model().rail_menu.is_none());
        runtime.update_shell(ShellMessage::SessionRenameEdited(
            "Claude: auth refactor".to_string(),
        ));
        runtime.update_shell(ShellMessage::SessionRenameCommitted);

        let workspace = runtime.app_state.workspace(&workspace_id).unwrap();
        let renamed = crate::backend::terminal_surfaces(&workspace.split_tree)
            .into_iter()
            .find(|surface| surface.id == surface_id)
            .and_then(|surface| surface.name);
        assert_eq!(renamed.as_deref(), Some("Claude: auth refactor"));
        // The rail entry shows the custom name.
        let entry_name = runtime
            .view_model()
            .sessions
            .groups
            .iter()
            .flat_map(|group| &group.entries)
            .find(|entry| entry.surface_id == surface_id)
            .map(|entry| entry.name.clone());
        assert_eq!(entry_name.as_deref(), Some("Claude: auth refactor"));
    }

    #[test]
    fn close_all_confirms_then_lands_on_the_empty_state() {
        let mut runtime = NativeShellRuntime::default();
        // The request parks behind the confirm modal.
        runtime.update_shell(ShellMessage::CloseAllRequested(None));
        assert_eq!(runtime.view_model().chrome.active_overlay, Overlay::Confirm);
        assert!(runtime.view_model().confirm.is_some());
        assert!(!runtime.app_state.workspaces.is_empty());

        // Esc cancels without closing anything.
        runtime.update_shell(ShellMessage::OverlayDismissed);
        assert!(!runtime.app_state.workspaces.is_empty());

        // Confirming closes everything: the valid empty state (spec 1.5).
        runtime.update_shell(ShellMessage::CloseAllRequested(None));
        runtime.update_shell(ShellMessage::ConfirmAccepted);
        assert!(runtime.app_state.workspaces.is_empty());
        assert_eq!(runtime.app_state.active_workspace_id, None);
        assert!(runtime.view_model().projection.visible_panes.is_empty());
        // The empty-state view builds, and its CTA reopens the launcher.
        {
            let _view = app_view(runtime.view_model());
        }
        runtime.update_shell(ShellMessage::NewSessionRequested);
        assert_eq!(
            runtime.view_model().chrome.active_overlay,
            Overlay::QuickLaunch
        );
        assert_eq!(runtime.last_error(), None);
    }

    #[test]
    fn closing_the_last_tab_reaches_the_empty_state() {
        let mut runtime = NativeShellRuntime::default();
        let surface_id = runtime.active_surface_id().expect("focused surface");
        runtime.update_shell(ShellMessage::SurfaceClosed(surface_id));
        assert!(runtime.app_state.workspaces.is_empty());
        assert!(runtime.view_model().projection.visible_panes.is_empty());
        assert_eq!(runtime.last_error(), None);
        let _view = app_view(runtime.view_model());
    }

    #[test]
    fn launcher_keyboard_navigation_and_type_step_flow() {
        let mut runtime = NativeShellRuntime::default();
        // Give the default workspace a project identity so it lists.
        runtime.app_state.workspaces[0].project.location = ProjectLocation::Local {
            cwd: "C:\\Dev\\Repo".to_string(),
            shell: "pwsh".to_string(),
        };
        pandamux_core::ensure_project_registry(&mut runtime.app_state, 1);

        runtime.update_shell(ShellMessage::NewSessionRequested);
        assert_eq!(runtime.launcher.step, LauncherStep::Project);
        assert!(
            runtime
                .launcher
                .items
                .iter()
                .any(|item| item.tag == "PROJ" && item.label == "Repo")
        );

        // Arrow keys walk the rows; typing filters them.
        runtime.update_shell(ShellMessage::PaletteMoveSelection(1));
        assert_eq!(runtime.launcher.selected, 1);
        runtime.update_shell(ShellMessage::LauncherFilterChanged("repo".to_string()));
        assert!(
            runtime
                .launcher
                .items
                .iter()
                .any(|item| item.label == "Repo")
        );

        // Choosing the project advances to the type step with its name shown.
        let project_id = runtime.app_state.projects[0].id.clone();
        runtime.update_shell(ShellMessage::LauncherProjectChosen(project_id));
        assert_eq!(runtime.launcher.step, LauncherStep::SessionType);
        assert_eq!(runtime.launcher.target_name, "Repo");
        assert!(
            runtime
                .launcher
                .type_items
                .iter()
                .any(|item| item.label == "Claude")
        );
        // Back returns to the Project step.
        runtime.update_shell(ShellMessage::LauncherBack);
        assert_eq!(runtime.launcher.step, LauncherStep::Project);
    }

    #[test]
    fn favorites_pin_and_surface_in_the_launcher() {
        let mut runtime = NativeShellRuntime::default();
        runtime.app_state.workspaces[0].project.location = ProjectLocation::Local {
            cwd: "C:\\Dev\\Repo".to_string(),
            shell: "pwsh".to_string(),
        };
        pandamux_core::ensure_project_registry(&mut runtime.app_state, 1);
        let project_id = runtime.app_state.projects[0].id.clone();

        let config = LaunchConfig {
            project_id,
            session: SessionType::Claude,
        };
        assert!(runtime.launcher_prefs.toggle_favorite(config.clone()));
        runtime.update_shell(ShellMessage::NewSessionRequested);
        let pin = runtime
            .launcher
            .items
            .iter()
            .find(|item| item.tag == "PIN")
            .expect("pinned row");
        assert_eq!(pin.label, "Repo: Claude");
        // Unpinning through the launcher star removes the row.
        runtime.update_shell(ShellMessage::LauncherFavoriteToggled(config));
        assert!(!runtime.launcher.items.iter().any(|item| item.tag == "PIN"));
    }

    #[test]
    fn tab_plus_routes_through_the_type_chooser() {
        let mut runtime = NativeShellRuntime::default();
        let pane_id = runtime.view_model().projection.visible_panes[0].id.clone();
        runtime.update_shell(ShellMessage::TabAddRequested(pane_id));
        assert_eq!(
            runtime.view_model().chrome.active_overlay,
            Overlay::QuickLaunch
        );
        assert_eq!(runtime.launcher.step, LauncherStep::SessionType);

        // Choosing Claude creates the tab in that pane with the type set.
        runtime.update_shell(ShellMessage::LauncherTypeChosen(SessionType::Claude));
        assert_eq!(runtime.view_model().chrome.active_overlay, Overlay::None);
        let workspace = runtime.app_state.active_workspace().unwrap();
        let surfaces = crate::backend::terminal_surfaces(&workspace.split_tree);
        assert_eq!(surfaces.len(), 2);
        assert_eq!(surfaces[1].session, Some(SessionType::Claude));
    }

    #[test]
    fn scroll_messages_are_safe_without_live_sessions() {
        let mut runtime = NativeShellRuntime::default();
        let surface_id = runtime.active_surface_id().expect("focused surface");
        runtime.update_shell(ShellMessage::ViewportScrolled {
            surface_id: surface_id.clone(),
            lines: 5,
        });
        runtime.update_shell(ShellMessage::ViewportScrollTo {
            surface_id,
            offset: 0,
        });
        runtime.update_shell(ShellMessage::ScrollPageFocused(-1));
        assert_eq!(runtime.last_error(), None);
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
        // Item 1 is "Close all sessions": activating it runs the action,
        // which parks behind its confirm modal.
        assert_eq!(runtime.view_model().chrome.active_overlay, Overlay::Confirm);
        runtime.update_shell(ShellMessage::OverlayDismissed);
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
    fn viewport_resize_records_size_and_debounces_the_flush() {
        let mut runtime = NativeShellRuntime::default();
        let surface_id = runtime
            .active_surface_id()
            .expect("default workspace has a focused surface");

        runtime.update_shell(ShellMessage::ViewportResized {
            surface_id: surface_id.clone(),
            columns: 200,
            rows: 48,
        });
        assert_eq!(
            runtime.viewport_sizes.get(&surface_id),
            Some(&GridSize::new(200, 48))
        );
        assert!(runtime.pending_resizes.contains_key(&surface_id));
        // The recorded size drives the next spawn for this surface.
        assert_eq!(runtime.spawn_size(&surface_id), GridSize::new(200, 48));

        // Two settled ticks flush the pending entry (no live session exists in
        // the hermetic runtime, so the flush is a clean no-op on the managers).
        runtime.update_shell(ShellMessage::Tick);
        assert!(runtime.pending_resizes.contains_key(&surface_id));
        runtime.update_shell(ShellMessage::Tick);
        runtime.update_shell(ShellMessage::Tick);
        assert!(runtime.pending_resizes.is_empty());
        // The last known size survives the flush for future spawns.
        assert_eq!(runtime.spawn_size(&surface_id), GridSize::new(200, 48));
    }

    #[test]
    fn spawn_size_falls_back_to_focused_pane_then_default() {
        let mut runtime = NativeShellRuntime::default();
        let focused = runtime
            .active_surface_id()
            .expect("default workspace has a focused surface");
        let unknown = SurfaceId::generate();

        // No viewport reported yet: everything falls back to the default.
        assert_eq!(runtime.spawn_size(&unknown), DEFAULT_GRID_SIZE);

        // Once the focused pane has a real size, new surfaces inherit it as
        // their first guess.
        runtime.update_shell(ShellMessage::ViewportResized {
            surface_id: focused,
            columns: 132,
            rows: 40,
        });
        assert_eq!(runtime.spawn_size(&unknown), GridSize::new(132, 40));
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
        // Ctrl+C maps to the conditional copy shortcut; its handler falls back
        // to sending SIGINT (0x03) when no selection exists (spec 1.3).
        assert_eq!(
            decode_character("c", None, true, false),
            ShellMessage::CopyOrInterrupt
        );
        // Unmapped Ctrl+letters still reach the terminal as control codes.
        assert_eq!(
            decode_character("l", None, true, false),
            ShellMessage::TerminalInput(vec![0x0c])
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
    fn welcome_chooser_converts_the_bare_terminal_on_a_number_key() {
        let mut runtime = NativeShellRuntime::default();
        runtime.refresh_terminal_snapshots();
        assert!(runtime.view_model().terminals[0].show_welcome);

        // Key 1 converts the bare terminal to Claude instead of typing.
        runtime.update_shell(ShellMessage::TerminalInput(vec![b'1']));
        let workspace = runtime.app_state.active_workspace().unwrap();
        let session = crate::backend::terminal_surfaces(&workspace.split_tree)[0]
            .session
            .clone();
        assert_eq!(session, Some(SessionType::Claude));
        assert!(!runtime.view_model().terminals[0].show_welcome);
    }

    #[test]
    fn welcome_chooser_dismisses_on_any_other_key() {
        let mut runtime = NativeShellRuntime::default();
        runtime.refresh_terminal_snapshots();
        assert!(runtime.view_model().terminals[0].show_welcome);

        runtime.update_shell(ShellMessage::TerminalInput(vec![b'l']));
        let workspace = runtime.app_state.active_workspace().unwrap();
        let session = crate::backend::terminal_surfaces(&workspace.split_tree)[0]
            .session
            .clone();
        // The keystroke reached the shell path, not a conversion.
        assert_eq!(session, None);
        assert!(!runtime.view_model().terminals[0].show_welcome);
    }

    #[test]
    fn welcome_chooser_respects_the_setting() {
        let mut runtime = NativeShellRuntime::default();
        runtime.settings.terminal.welcome_prompt_enabled = false;
        runtime.refresh_terminal_snapshots();
        assert!(!runtime.view_model().terminals[0].show_welcome);
    }

    #[test]
    fn welcome_custom_opens_the_type_chooser_and_converts_in_place() {
        let mut runtime = NativeShellRuntime::default();
        runtime.refresh_terminal_snapshots();
        runtime.update_shell(ShellMessage::TerminalInput(vec![b'4']));
        assert_eq!(runtime.chrome.active_overlay, Overlay::QuickLaunch);
        assert_eq!(runtime.launcher.step, LauncherStep::SessionType);

        runtime.update_shell(ShellMessage::LauncherTypeChosen(SessionType::Codex));
        let workspace = runtime.app_state.active_workspace().unwrap();
        let session = crate::backend::terminal_surfaces(&workspace.split_tree)[0]
            .session
            .clone();
        assert_eq!(session, Some(SessionType::Codex));
        assert_eq!(runtime.chrome.active_overlay, Overlay::None);
    }

    #[test]
    fn home_pin_focus_and_unpin_round_trip() {
        let mut runtime = NativeShellRuntime::default();
        let workspace = runtime.app_state.active_workspace().unwrap();
        let workspace_id = workspace.id.clone();
        let surface_id = crate::backend::terminal_surfaces(&workspace.split_tree)[0]
            .id
            .clone();

        runtime.update_shell(ShellMessage::RailMenuAction(RailMenuAction::PinToHome {
            workspace_id,
            surface_id: surface_id.clone(),
        }));

        assert_eq!(runtime.chrome.main_view, MainView::Home);
        assert_eq!(runtime.app_state.home.panes.len(), 1);
        let pane_id = runtime.app_state.home.panes[0].id.clone();
        assert_eq!(
            runtime.app_state.home.focused_pane_id,
            Some(pane_id.clone())
        );
        // Home focus drives keyboard routing (spec 2.5): the focused Home
        // pane's session is the active session while the dashboard is up.
        assert_eq!(runtime.active_surface_id(), Some(surface_id));

        // Unpin removes the pane from Home only; the session lives on.
        runtime.update_shell(ShellMessage::HomeUnpin(pane_id));
        assert!(runtime.app_state.home.panes.is_empty());
        assert_eq!(runtime.app_state.workspaces.len(), 1);
    }

    #[test]
    fn dead_home_pane_becomes_placeholder_then_reassigns() {
        let mut runtime = NativeShellRuntime::default();
        // A restored Home pane whose session did not survive (spec 2.5).
        let pane_id = runtime
            .app_state
            .home
            .pin(SurfaceId::from("surf-gone"), None);
        runtime.refresh_terminal_snapshots();
        assert_eq!(
            runtime.app_state.home.pane(&pane_id).unwrap().surface_id,
            None
        );

        // The next launch completion routes into the waiting pane. Assign
        // validates the session exists, so mint a real one first.
        runtime.pending_home_assign = Some(pane_id.clone());
        let default_pane = runtime.view_model().projection.visible_panes[0].id.clone();
        runtime.update_shell(ShellMessage::TerminalSurfaceCreated(default_pane));
        let workspace = runtime.app_state.active_workspace().unwrap();
        let fresh = crate::backend::terminal_surfaces(&workspace.split_tree)
            .last()
            .unwrap()
            .id
            .clone();
        runtime.complete_home_assign(&fresh);
        assert_eq!(
            runtime.app_state.home.pane(&pane_id).unwrap().surface_id,
            Some(fresh)
        );
        assert_eq!(runtime.chrome.main_view, MainView::Home);
        assert_eq!(runtime.pending_home_assign, None);
    }

    #[test]
    fn home_assign_picker_lists_and_assigns_open_sessions() {
        let mut runtime = NativeShellRuntime::default();
        let pane_id = runtime
            .app_state
            .home
            .pin(SurfaceId::from("surf-gone"), None);
        runtime.app_state.home.panes[0].surface_id = None;

        runtime.update_shell(ShellMessage::HomeAssignRequested(pane_id.clone()));
        let menu = runtime.rail_menu.clone().expect("assign picker opens");
        assert_eq!(menu.items.len(), 1);

        let (_, action) = menu.items[0].clone();
        runtime.update_shell(ShellMessage::RailMenuAction(action));
        assert!(
            runtime
                .app_state
                .home
                .pane(&pane_id)
                .unwrap()
                .surface_id
                .is_some()
        );
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
