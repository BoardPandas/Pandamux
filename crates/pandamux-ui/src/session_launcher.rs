//! Staged Local and SSH Project session launcher.
//!
//! Every step renders inside the shared overlay chrome (scrim, card, shadow)
//! from [`crate::command_palette`], and every control consumes [`crate::theme`]
//! tokens so the launcher matches the rest of the shell in both themes.

use crate::command_palette::{modal, overlay_card_style};
use crate::iced_shell::ShellMessage;
use crate::icons::{Icon, icon};
use crate::theme::{self, Palette, UiTheme};
use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Color, Element, Length, Padding};
use pandamux_core::{FolderListing, ProjectError, SshAuthConfig, SshHostProfile, SshProfileId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LauncherStep {
    /// Step 1 (spec 2.2): pinned favorites, recents, existing projects, and
    /// the new-project entry points.
    #[default]
    Project,
    /// Step 2 (spec 2.2/2.7): what to open in the chosen project.
    SessionType,
    /// SSH connection management (was the old first step).
    Connection,
    ProfileForm,
    Credential,
    HostConfirmation,
    Folder,
    Launching,
}

/// One activatable row on the Project step, in display order. The runtime
/// builds this list (it owns the registry and prefs); Up/Down/Enter walk it,
/// so keyboard and mouse hit exactly the same actions.
#[derive(Clone, Debug, PartialEq)]
pub struct LauncherItem {
    /// "PIN" | "NEW" | recency text | project detail.
    pub tag: String,
    pub label: String,
    pub detail: String,
    pub message: ShellMessage,
    /// A pinned configuration renders a lit star (toggles off); a plain
    /// project renders none.
    pub pin: Option<ShellMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshProfileForm {
    pub id: Option<SshProfileId>,
    pub name: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub auth: SshAuthConfig,
    pub identity_file: String,
    pub error: Option<String>,
}

impl Default for SshProfileForm {
    fn default() -> Self {
        Self {
            id: None,
            name: String::new(),
            host: String::new(),
            port: "22".to_string(),
            user: String::new(),
            auth: SshAuthConfig::Agent,
            identity_file: String::new(),
            error: None,
        }
    }
}

impl SshProfileForm {
    pub fn from_profile(profile: &SshHostProfile) -> Self {
        let identity_file = match &profile.auth {
            SshAuthConfig::KeyFile { path } => path.clone(),
            _ => String::new(),
        };
        Self {
            id: Some(profile.id.clone()),
            name: profile.name.clone(),
            host: format!("{}@{}", profile.user, profile.host),
            port: profile.port.to_string(),
            user: profile.user.clone(),
            auth: profile.auth.clone(),
            identity_file,
            error: None,
        }
    }

    pub fn is_valid(&self) -> bool {
        !self.name.trim().is_empty()
            && !self.host.trim().is_empty()
            && self.port.parse::<u16>().is_ok_and(|port| port > 0)
            && (!matches!(self.auth, SshAuthConfig::KeyFile { .. })
                || !self.identity_file.trim().is_empty())
            && self.error.is_none()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionLauncherViewState {
    pub step: LauncherStep,
    pub profiles: Vec<SshHostProfile>,
    pub selected_profile_id: Option<SshProfileId>,
    pub form: SshProfileForm,
    pub credential: String,
    pub fingerprint: Option<String>,
    pub remote: bool,
    pub path: String,
    pub listing: Option<FolderListing>,
    pub loading: bool,
    pub launching: bool,
    pub error: Option<ProjectError>,
    /// Type-to-filter text on the Project step (keyboard-first, spec 2.2).
    pub filter: String,
    /// Project-step rows in display order (favorites, recents, projects, new).
    pub items: Vec<LauncherItem>,
    /// SessionType-step rows in display order.
    pub type_items: Vec<LauncherItem>,
    /// Keyboard selection index into the active step's items.
    pub selected: usize,
    /// The project name shown on the SessionType step header.
    pub target_name: String,
    /// Custom-command input on the SessionType step.
    pub custom_command: String,
}

pub fn session_launcher(
    state: &SessionLauncherViewState,
    palette: Palette,
) -> Element<'_, ShellMessage> {
    let content = match state.step {
        LauncherStep::Project => project_step(state, palette),
        LauncherStep::SessionType => type_step(state, palette),
        LauncherStep::Connection => connection_step(state, palette),
        LauncherStep::ProfileForm => profile_form_step(state, palette),
        LauncherStep::Credential => credential_step(state, palette),
        LauncherStep::HostConfirmation => host_confirmation_step(state, palette),
        LauncherStep::Folder => folder_step(state, palette),
        LauncherStep::Launching => launching_step(state, palette),
    };
    let card = container(container(content).padding(16.0).width(Length::Fixed(600.0)))
        .width(Length::Fixed(600.0))
        .max_height(640.0)
        .style(move |_theme| overlay_card_style(palette));
    modal(card, palette, Alignment::Center)
}

/// One activatable launcher row: tag chip, label + detail, optional pin star,
/// highlighted when keyboard-selected.
fn launcher_item_row<'a>(
    item: &'a LauncherItem,
    selected: bool,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let tag = container(
        text(item.tag.clone())
            .size(theme::SIZE_METADATA)
            .font(theme::mono(iced::font::Weight::Semibold))
            .color(palette.t3),
    )
    .width(Length::Fixed(44.0))
    .align_x(Alignment::Center)
    .padding(Padding::from([2.0, 4.0]))
    .style(move |_theme| container::Style {
        background: Some(palette.ov(0.05).into()),
        border: theme::border(palette.ov(0.08), 1.0, theme::RADIUS_CHIP),
        ..Default::default()
    });
    let body = column![
        text(item.label.clone())
            .size(theme::SIZE_BODY)
            .color(palette.t1),
        text(item.detail.clone())
            .size(theme::SIZE_METADATA)
            .font(theme::mono(iced::font::Weight::Normal))
            .color(palette.t4),
    ]
    .spacing(2)
    .width(Length::Fill);
    let content = row![tag, body].spacing(10).align_y(Alignment::Center);
    let main = button(content.width(Length::Fill))
        .padding(Padding::from([7.0, 8.0]))
        .width(Length::Fill)
        .on_press(item.message.clone())
        .style(move |_theme, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(
                    if selected {
                        palette.accent_alpha(0.1)
                    } else if hovered {
                        palette.ov(0.05)
                    } else {
                        Color::TRANSPARENT
                    }
                    .into(),
                ),
                text_color: palette.t1,
                border: theme::border(
                    if selected {
                        palette.accent_alpha(0.25)
                    } else {
                        Color::TRANSPARENT
                    },
                    1.0,
                    theme::RADIUS_ROW,
                ),
                ..Default::default()
            }
        });
    match &item.pin {
        Some(toggle) => {
            let star = button(
                text("\u{2605}")
                    .size(theme::SIZE_BODY)
                    .color(palette.accent),
            )
            .padding(Padding::from([3.0, 6.0]))
            .on_press(toggle.clone())
            .style(move |_theme, status| button::Style {
                background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                    .then(|| palette.ov(0.08).into()),
                text_color: palette.accent,
                border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_ROW),
                ..Default::default()
            });
            row![main, star]
                .spacing(2)
                .align_y(Alignment::Center)
                .into()
        }
        None => main.into(),
    }
}

/// Step 1: favorites + recents + projects + new-project entry points, filtered
/// by the type-to-filter input and fully keyboard-navigable (spec 2.2/2.3).
fn project_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let filter = text_input("Type to filter, arrows + Enter to launch", &state.filter)
        .on_input(ShellMessage::LauncherFilterChanged)
        .size(theme::SIZE_BODY)
        .width(Length::Fill);

    let mut list = column![].spacing(2).width(Length::Fill);
    if state.items.is_empty() {
        list = list.push(
            text("No matches. Clear the filter or create a new project below.")
                .size(theme::SIZE_SECONDARY)
                .color(palette.t3),
        );
    }
    for (index, item) in state.items.iter().enumerate() {
        list = list.push(launcher_item_row(item, index == state.selected, palette));
    }

    column![
        header("New Session", "Which project?", palette),
        filter,
        scrollable(list)
            .height(Length::Fixed(360.0))
            .width(Length::Fill),
        row![
            Space::new().width(Length::Fill),
            ghost_button("Cancel", Some(ShellMessage::OverlayDismissed), palette),
        ]
        .spacing(8),
    ]
    .spacing(12)
    .into()
}

/// Step 2: what to open in the chosen project (spec 2.2/2.7).
fn type_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let mut list = column![].spacing(2).width(Length::Fill);
    for (index, item) in state.type_items.iter().enumerate() {
        list = list.push(launcher_item_row(item, index == state.selected, palette));
    }
    let custom = row![
        text_input("Custom command (e.g. npm run dev)", &state.custom_command)
            .on_input(ShellMessage::LauncherCustomCommandChanged)
            .on_submit(ShellMessage::LauncherCustomSubmitted)
            .size(theme::SIZE_BODY)
            .width(Length::Fill),
        ghost_button(
            "Run",
            (!state.custom_command.trim().is_empty())
                .then_some(ShellMessage::LauncherCustomSubmitted),
            palette
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    column![
        header("New Session", "What are you opening?", palette),
        container(
            text(state.target_name.clone())
                .size(theme::SIZE_SECONDARY)
                .color(palette.accent)
        )
        .padding(Padding::from([0.0, 2.0])),
        list,
        custom,
        row![
            ghost_button("Back", Some(ShellMessage::LauncherBack), palette),
            Space::new().width(Length::Fill),
            ghost_button("Cancel", Some(ShellMessage::OverlayDismissed), palette),
        ]
        .spacing(8),
    ]
    .spacing(12)
    .into()
}

fn connection_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let mut list = column![].spacing(2).width(Length::Fill);
    list = list.push(connection_row(
        "PS",
        palette.shell_powershell,
        "Local",
        "PowerShell on this computer",
        Some(ShellMessage::LauncherLocalSelected),
        palette,
    ));
    for profile in &state.profiles {
        let unsupported = profile.jump.is_some();
        let detail = if unsupported {
            "ProxyJump is not supported".to_string()
        } else {
            format!("{}@{}:{}", profile.user, profile.host, profile.port)
        };
        let select = connection_row_owned(
            "SSH",
            palette.shell_ssh,
            profile.name.clone(),
            detail,
            (!unsupported).then(|| ShellMessage::LauncherProfileSelected(profile.id.clone())),
            palette,
        );
        let edit = small_button(
            "Edit",
            palette.t3,
            ShellMessage::LauncherProfileEdit(profile.id.clone()),
        );
        let delete = small_button(
            "Delete",
            danger(palette),
            ShellMessage::LauncherProfileDelete(profile.id.clone()),
        );
        list = list.push(
            row![select, edit, delete]
                .spacing(4)
                .align_y(Alignment::Center),
        );
    }

    column![
        header("New Session", "Choose a connection", palette),
        list,
        row![
            ghost_button(
                "Add SSH Connection",
                Some(ShellMessage::LauncherProfileAdd),
                palette
            ),
            ghost_button(
                "Import SSH Config",
                Some(ShellMessage::LauncherProfileImport),
                palette
            ),
            Space::new().width(Length::Fill),
            ghost_button("Cancel", Some(ShellMessage::OverlayDismissed), palette),
        ]
        .spacing(8),
    ]
    .spacing(12)
    .into()
}

fn profile_form_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let form = &state.form;
    let auth_pills = row![
        auth_pill(
            "OpenSSH agent",
            matches!(form.auth, SshAuthConfig::Agent),
            ShellMessage::LauncherProfileAuthChanged(SshAuthConfig::Agent),
            palette,
        ),
        auth_pill(
            "Identity file",
            matches!(form.auth, SshAuthConfig::KeyFile { .. }),
            ShellMessage::LauncherProfileAuthChanged(SshAuthConfig::KeyFile {
                path: form.identity_file.clone(),
            }),
            palette,
        ),
        auth_pill(
            "Password",
            matches!(form.auth, SshAuthConfig::Password),
            ShellMessage::LauncherProfileAuthChanged(SshAuthConfig::Password),
            palette,
        ),
    ]
    .spacing(6);

    let mut content = column![
        header("SSH Connection", "Save connection details", palette),
        labeled_input(
            "Name",
            "galahad",
            &form.name,
            ShellMessage::LauncherProfileNameChanged,
            palette
        ),
        row![
            container(labeled_input(
                "SSH Host",
                "user@host",
                &form.host,
                ShellMessage::LauncherProfileHostChanged,
                palette
            ))
            .width(Length::Fill),
            container(labeled_input(
                "Port",
                "22",
                &form.port,
                ShellMessage::LauncherProfilePortChanged,
                palette
            ))
            .width(Length::Fixed(90.0)),
        ]
        .spacing(8),
        column![field_label("Authentication", palette), auth_pills].spacing(4),
    ]
    .spacing(10);
    if matches!(form.auth, SshAuthConfig::KeyFile { .. }) {
        content = content.push(labeled_input(
            "Identity File",
            "C:\\Users\\you\\.ssh\\id_ed25519",
            &form.identity_file,
            ShellMessage::LauncherIdentityFileChanged,
            palette,
        ));
    }
    if let Some(error) = &form.error {
        content = content.push(error_banner(error, palette));
    }
    content = content.push(
        row![
            ghost_button("Back", Some(ShellMessage::LauncherBack), palette),
            Space::new().width(Length::Fill),
            primary_button(
                "Save",
                form.is_valid().then_some(ShellMessage::LauncherProfileSave),
                palette
            ),
        ]
        .spacing(8),
    );
    content.into()
}

fn credential_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let mut input = text_input("Password or key passphrase", &state.credential)
        .secure(true)
        .on_input(ShellMessage::LauncherCredentialChanged)
        .size(theme::SIZE_BODY)
        .padding(Padding::from([7.0, 10.0]))
        .style(input_style(palette));
    if !state.credential.is_empty() {
        input = input.on_submit(ShellMessage::LauncherCredentialSubmit);
    }
    let mut content = column![
        header(
            "Credentials Required",
            "Credentials stay in memory until exit",
            palette
        ),
        input,
    ]
    .spacing(12);
    if let Some(error) = &state.error {
        content = content.push(error_banner(&error.message, palette));
    }
    content = content.push(
        row![
            ghost_button("Back", Some(ShellMessage::LauncherBack), palette),
            Space::new().width(Length::Fill),
            primary_button(
                "Continue",
                (!state.credential.is_empty()).then_some(ShellMessage::LauncherCredentialSubmit),
                palette
            ),
        ]
        .spacing(8),
    );
    content.into()
}

fn host_confirmation_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let fingerprint = container(
        text(
            state
                .fingerprint
                .as_deref()
                .unwrap_or("Fingerprint unavailable"),
        )
        .size(theme::SIZE_SECONDARY)
        .font(theme::mono(iced::font::Weight::Medium))
        .color(palette.t1),
    )
    .padding(Padding::from([8.0, 10.0]))
    .width(Length::Fill)
    .style(move |_theme| container::Style {
        background: Some(palette.ov(0.04).into()),
        border: theme::border(palette.ov(0.08), 1.0, theme::RADIUS_ROW),
        ..Default::default()
    });
    column![
        header(
            "Unknown Host Key",
            "Confirm the server fingerprint",
            palette
        ),
        fingerprint,
        row![
            ghost_button("Cancel", Some(ShellMessage::LauncherBack), palette),
            Space::new().width(Length::Fill),
            primary_button(
                "Trust and Continue",
                Some(ShellMessage::LauncherHostTrustConfirmed),
                palette
            ),
        ]
        .spacing(8),
    ]
    .spacing(12)
    .into()
}

fn folder_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let title = if state.remote {
        "Select Remote Folder"
    } else {
        "Select Local Folder"
    };
    let path_row = row![
        text_input("Folder path", &state.path)
            .on_input(ShellMessage::LauncherPathChanged)
            .on_submit(ShellMessage::LauncherFolderGo)
            .size(theme::SIZE_BODY)
            .padding(Padding::from([7.0, 10.0]))
            .width(Length::Fill)
            .style(input_style(palette)),
        ghost_button(
            "Go",
            (!state.loading && !state.path.trim().is_empty())
                .then_some(ShellMessage::LauncherFolderGo),
            palette
        ),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let mut content =
        column![header(title, "Choose the Project root", palette), path_row].spacing(10);

    // Home shortcut plus (locally) one chip per ready drive, Explorer style.
    let mut places = row![nav_chip(
        Some(Icon::Home),
        "Home",
        false,
        ShellMessage::LauncherFolderHome,
        palette,
    )]
    .spacing(4)
    .align_y(Alignment::Center);
    if let Some(listing) = &state.listing {
        if !listing.drives.is_empty() {
            places = places.push(
                container(
                    Space::new()
                        .width(Length::Fixed(1.0))
                        .height(Length::Fixed(16.0)),
                )
                .style(move |_theme| container::Style {
                    background: Some(palette.ov(0.1).into()),
                    ..Default::default()
                }),
            );
        }
        for drive in &listing.drives {
            let active = listing
                .canonical_path
                .to_ascii_lowercase()
                .starts_with(&drive.to_ascii_lowercase());
            places = places.push(nav_chip_owned(
                Some(Icon::Drive),
                drive.trim_end_matches('\\').to_string(),
                active,
                ShellMessage::LauncherFolderNavigate(drive.clone()),
                palette,
            ));
        }
    }
    content = content.push(places);

    if let Some(listing) = &state.listing {
        let mut breadcrumbs = row![].spacing(2).align_y(Alignment::Center);
        for (index, crumb) in listing.breadcrumbs.iter().enumerate() {
            if index > 0 {
                breadcrumbs = breadcrumbs.push(
                    text("\u{203a}")
                        .size(theme::SIZE_SECONDARY)
                        .color(palette.t4),
                );
            }
            let last = index + 1 == listing.breadcrumbs.len();
            breadcrumbs = breadcrumbs.push(nav_chip_owned(
                None,
                crumb.label.clone(),
                last,
                ShellMessage::LauncherFolderNavigate(crumb.canonical_path.clone()),
                palette,
            ));
        }
        content = content.push(
            scrollable(breadcrumbs)
                .direction(scrollable::Direction::Horizontal(
                    scrollable::Scrollbar::new().width(2.0).scroller_width(2.0),
                ))
                .width(Length::Fill),
        );

        let mut folders = column![].spacing(1).width(Length::Fill);
        if let Some(parent) = &listing.parent_path {
            folders = folders.push(folder_row(
                "\u{2191}",
                "..".to_string(),
                palette.t3,
                ShellMessage::LauncherFolderNavigate(parent.clone()),
                palette,
            ));
        }
        for directory in &listing.directories {
            folders = folders.push(folder_icon_row(
                directory.name.clone(),
                ShellMessage::LauncherFolderNavigate(directory.canonical_path.clone()),
                palette,
            ));
        }
        if listing.directories.is_empty() {
            folders = folders.push(
                container(
                    text("No subfolders")
                        .size(theme::SIZE_SECONDARY)
                        .color(palette.t4),
                )
                .padding(Padding::from([6.0, 8.0])),
            );
        }
        content = content.push(
            container(
                scrollable(folders)
                    .height(Length::Fixed(260.0))
                    .width(Length::Fill),
            )
            .padding(4.0)
            .width(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(palette.ov(0.03).into()),
                border: theme::border(palette.ov(0.06), 1.0, theme::RADIUS_ROW),
                ..Default::default()
            }),
        );
    }
    if state.loading {
        content = content.push(
            text("Loading...")
                .size(theme::SIZE_SECONDARY)
                .color(palette.t3),
        );
    }
    if let Some(error) = &state.error {
        content = content.push(error_banner(&error.message, palette));
    }
    let can_select = state
        .listing
        .as_ref()
        .is_some_and(|listing| listing.canonical_path == state.path)
        && !state.loading
        && !state.launching;
    content = content.push(
        row![
            ghost_button("Back", Some(ShellMessage::LauncherBack), palette),
            Space::new().width(Length::Fill),
            primary_button(
                "Select Folder",
                can_select.then_some(ShellMessage::LauncherFolderSelected),
                palette
            ),
        ]
        .spacing(8),
    );
    content.into()
}

fn launching_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let mut content = column![
        header(
            "Launching",
            "Starting the terminal before saving the Project",
            palette
        ),
        text("Connecting...")
            .size(theme::SIZE_BODY)
            .color(palette.t3),
    ]
    .spacing(12);
    if let Some(error) = &state.error {
        content = content.push(error_banner(&error.message, palette));
    }
    content.into()
}

// ---------------------------------------------------------------------------
// Shared launcher chrome
// ---------------------------------------------------------------------------

/// Title, subtitle, and a close (x) affordance, matching the settings modal.
fn header<'a>(title: &'a str, subtitle: &'a str, palette: Palette) -> Element<'a, ShellMessage> {
    row![
        column![
            text(title)
                .size(theme::SIZE_TITLE)
                .font(theme::ui(iced::font::Weight::Semibold))
                .color(palette.t1),
            text(subtitle).size(theme::SIZE_SECONDARY).color(palette.t3),
        ]
        .spacing(3),
        Space::new().width(Length::Fill),
        button(icon(Icon::Close, 12.0, palette.t3))
            .padding(Padding::from([4.0, 6.0]))
            .on_press(ShellMessage::OverlayDismissed)
            .style(move |_theme, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: hovered.then(|| palette.ov(0.08).into()),
                    text_color: palette.t2,
                    border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_CHIP),
                    ..Default::default()
                }
            }),
    ]
    .align_y(Alignment::Start)
    .into()
}

/// A full-width connection list row: shell badge, title, and metadata detail.
/// `message: None` renders the row disabled (dimmed, not pressable).
fn connection_row<'a>(
    badge: &'a str,
    badge_color: Color,
    title: &'a str,
    detail: &'a str,
    message: Option<ShellMessage>,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    connection_row_owned(
        badge,
        badge_color,
        title.to_string(),
        detail.to_string(),
        message,
        palette,
    )
}

fn connection_row_owned<'a>(
    badge: &'a str,
    badge_color: Color,
    title: String,
    detail: String,
    message: Option<ShellMessage>,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let enabled = message.is_some();
    let content = row![
        container(
            text(badge)
                .size(theme::SIZE_METADATA)
                .font(theme::mono(iced::font::Weight::Semibold))
                .color(badge_color),
        )
        .width(Length::Fixed(34.0))
        .height(Length::Fixed(24.0))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_theme| container::Style {
            background: Some(theme::with_alpha(badge_color, 0.1).into()),
            border: theme::border(theme::with_alpha(badge_color, 0.3), 1.0, 7.0),
            ..Default::default()
        }),
        column![
            text(title)
                .size(theme::SIZE_BODY)
                .color(if enabled { palette.t1 } else { palette.t4 }),
            text(detail)
                .size(theme::SIZE_METADATA)
                .font(theme::mono(iced::font::Weight::Normal))
                .color(palette.t4),
        ]
        .spacing(2),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    let mut select = button(content)
        .padding(Padding::from([7.0, 8.0]))
        .width(Length::Fill)
        .style(move |_theme, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: (enabled && hovered).then(|| palette.ov(0.05).into()),
                text_color: palette.t1,
                border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_ROW),
                ..Default::default()
            }
        });
    if let Some(message) = message {
        select = select.on_press(message);
    }
    select.into()
}

/// One folder row in the browser list, with a themed folder line icon.
fn folder_icon_row<'a>(
    name: String,
    message: ShellMessage,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    button(
        row![
            icon(Icon::Folder, 13.0, palette.t3),
            text(name).size(theme::SIZE_BODY).color(palette.t1),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([5.0, 8.0]))
    .width(Length::Fill)
    .on_press(message)
    .style(list_row_style(palette))
    .into()
}

/// The ".. parent" row, sharing the folder row look with a glyph marker.
fn folder_row<'a>(
    glyph: &'a str,
    name: String,
    color: Color,
    message: ShellMessage,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    button(
        row![
            container(text(glyph).size(theme::SIZE_BODY).color(color))
                .width(Length::Fixed(13.0))
                .align_x(Alignment::Center),
            text(name).size(theme::SIZE_BODY).color(color),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([5.0, 8.0]))
    .width(Length::Fill)
    .on_press(message)
    .style(list_row_style(palette))
    .into()
}

fn list_row_style(palette: Palette) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: hovered.then(|| palette.ov(0.06).into()),
            text_color: palette.t1,
            border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_CHIP),
            ..Default::default()
        }
    }
}

/// Accent-tinted call-to-action. `message: None` renders it disabled.
fn primary_button<'a>(
    label: &'a str,
    message: Option<ShellMessage>,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let enabled = message.is_some();
    let mut cta = button(
        text(label)
            .size(theme::SIZE_BODY)
            .font(theme::ui(iced::font::Weight::Medium)),
    )
    .padding(Padding::from([6.0, 16.0]))
    .style(move |_theme, status| {
        let (background, text_color, border) = if !enabled {
            (palette.ov(0.04), palette.t4, palette.ov(0.06))
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            (
                palette.accent_alpha(0.22),
                palette.accent,
                palette.accent_alpha(0.45),
            )
        } else {
            (
                palette.accent_alpha(0.14),
                palette.accent,
                palette.accent_alpha(0.35),
            )
        };
        button::Style {
            background: Some(background.into()),
            text_color,
            border: theme::border(border, 1.0, theme::RADIUS_ROW),
            ..Default::default()
        }
    });
    if let Some(message) = message {
        cta = cta.on_press(message);
    }
    cta.into()
}

/// Neutral secondary action (Back, Cancel, Go, Add/Import).
fn ghost_button<'a>(
    label: &'a str,
    message: Option<ShellMessage>,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let enabled = message.is_some();
    let mut ghost = button(text(label).size(theme::SIZE_BODY))
        .padding(Padding::from([6.0, 14.0]))
        .style(move |_theme, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(
                    palette
                        .ov(if !enabled {
                            0.03
                        } else if hovered {
                            0.1
                        } else {
                            0.06
                        })
                        .into(),
                ),
                text_color: if enabled { palette.t1 } else { palette.t4 },
                border: theme::border(
                    palette.ov(if enabled { 0.1 } else { 0.05 }),
                    1.0,
                    theme::RADIUS_ROW,
                ),
                ..Default::default()
            }
        });
    if let Some(message) = message {
        ghost = ghost.on_press(message);
    }
    ghost.into()
}

/// Small inline text action (Edit / Delete on a connection row).
fn small_button<'a>(
    label: &'a str,
    color: Color,
    message: ShellMessage,
) -> Element<'a, ShellMessage> {
    button(text(label).size(theme::SIZE_SECONDARY).color(color))
        .padding(Padding::from([4.0, 8.0]))
        .on_press(message)
        .style(move |_theme, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: hovered.then(|| theme::with_alpha(color, 0.1).into()),
                text_color: color,
                border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_CHIP),
                ..Default::default()
            }
        })
        .into()
}

/// Segmented authentication choice pill; the active choice carries the accent.
fn auth_pill<'a>(
    label: &'a str,
    active: bool,
    message: ShellMessage,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    button(text(label).size(theme::SIZE_SECONDARY).color(if active {
        palette.accent
    } else {
        palette.t3
    }))
    .padding(Padding::from([4.0, 10.0]))
    .on_press(message)
    .style(chip_style(palette, active))
    .into()
}

/// Navigation chip (Home, drive letters, breadcrumb segments), optionally with
/// a leading line icon. `active` carries the accent highlight.
fn nav_chip<'a>(
    glyph: Option<Icon>,
    label: &'a str,
    active: bool,
    message: ShellMessage,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    nav_chip_owned(glyph, label.to_string(), active, message, palette)
}

fn nav_chip_owned<'a>(
    glyph: Option<Icon>,
    label: String,
    active: bool,
    message: ShellMessage,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let color = if active { palette.accent } else { palette.t3 };
    let mut content = row![].spacing(5).align_y(Alignment::Center);
    if let Some(kind) = glyph {
        content = content.push(icon(kind, 11.0, color));
    }
    content = content.push(text(label).size(theme::SIZE_SECONDARY).color(color));
    button(content)
        .padding(Padding::from([3.0, 8.0]))
        .on_press(message)
        .style(chip_style(palette, active))
        .into()
}

fn chip_style(
    palette: Palette,
    active: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let background = if active {
            palette.accent_alpha(0.14)
        } else if hovered {
            palette.ov(0.09)
        } else {
            palette.ov(0.04)
        };
        button::Style {
            background: Some(background.into()),
            text_color: if active { palette.accent } else { palette.t2 },
            border: theme::border(
                if active {
                    palette.accent_alpha(0.3)
                } else {
                    palette.ov(0.07)
                },
                1.0,
                theme::RADIUS_CHIP,
            ),
            ..Default::default()
        }
    }
}

fn field_label<'a>(label: &'a str, palette: Palette) -> Element<'a, ShellMessage> {
    text(label)
        .size(theme::SIZE_SECONDARY)
        .color(palette.t3)
        .into()
}

fn labeled_input<'a>(
    label: &'a str,
    placeholder: &'a str,
    value: &'a str,
    on_input: fn(String) -> ShellMessage,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    column![
        field_label(label, palette),
        text_input(placeholder, value)
            .on_input(on_input)
            .size(theme::SIZE_BODY)
            .padding(Padding::from([7.0, 10.0]))
            .style(input_style(palette)),
    ]
    .spacing(4)
    .into()
}

fn input_style(palette: Palette) -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style {
    move |_theme, status| {
        let focused = matches!(status, text_input::Status::Focused { .. });
        text_input::Style {
            background: palette.ov(0.05).into(),
            border: theme::border(
                if focused {
                    palette.accent_alpha(0.5)
                } else {
                    palette.ov(0.1)
                },
                1.0,
                theme::RADIUS_ROW,
            ),
            icon: palette.t3,
            placeholder: palette.t4,
            value: palette.t1,
            selection: palette.accent_alpha(0.35),
        }
    }
}

fn error_banner<'a>(message: &str, palette: Palette) -> Element<'a, ShellMessage> {
    let color = danger(palette);
    container(
        text(message.to_string())
            .size(theme::SIZE_SECONDARY)
            .color(color),
    )
    .padding(Padding::from([6.0, 10.0]))
    .width(Length::Fill)
    .style(move |_theme| container::Style {
        background: Some(theme::with_alpha(color, 0.08).into()),
        border: theme::border(theme::with_alpha(color, 0.25), 1.0, theme::RADIUS_ROW),
        ..Default::default()
    })
    .into()
}

fn danger(palette: Palette) -> Color {
    match palette.theme {
        UiTheme::Dark => Color::from_rgb8(0xe0, 0x6c, 0x75),
        UiTheme::Light => Color::from_rgb8(0xb8, 0x43, 0x4e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Accent;
    use pandamux_core::{FolderBreadcrumb, FolderEntry};

    fn palette() -> Palette {
        Palette::new(UiTheme::Dark, Accent::Teal)
    }

    #[test]
    fn profile_form_requires_fields_and_identity_path() {
        let mut form = SshProfileForm {
            name: "Server".to_string(),
            host: "chaz@server".to_string(),
            ..SshProfileForm::default()
        };
        assert!(form.is_valid());
        form.auth = SshAuthConfig::KeyFile {
            path: String::new(),
        };
        assert!(!form.is_valid());
        form.identity_file = "C:\\Users\\chaz\\.ssh\\id_ed25519".to_string();
        assert!(form.is_valid());
    }

    #[test]
    fn launcher_opens_on_the_project_step() {
        let mut state = SessionLauncherViewState::default();
        assert_eq!(state.step, LauncherStep::Project);
        state.step = LauncherStep::Folder;
        state.remote = true;
        assert_eq!(state.step, LauncherStep::Folder);
        assert!(state.remote);
    }

    #[test]
    fn every_step_builds_with_a_populated_listing() {
        let mut state = SessionLauncherViewState {
            profiles: vec![SshHostProfile::new("galahad", "10.55.88.48", "chaz")],
            path: "C:\\Dev".to_string(),
            listing: Some(FolderListing {
                canonical_path: "C:\\Dev".to_string(),
                parent_path: Some("C:\\".to_string()),
                breadcrumbs: vec![
                    FolderBreadcrumb {
                        label: "C:".to_string(),
                        canonical_path: "C:\\".to_string(),
                    },
                    FolderBreadcrumb {
                        label: "Dev".to_string(),
                        canonical_path: "C:\\Dev".to_string(),
                    },
                ],
                directories: vec![FolderEntry {
                    name: "Repos".to_string(),
                    canonical_path: "C:\\Dev\\Repos".to_string(),
                }],
                drives: vec!["C:\\".to_string(), "D:\\".to_string()],
            }),
            ..SessionLauncherViewState::default()
        };
        for step in [
            LauncherStep::Connection,
            LauncherStep::ProfileForm,
            LauncherStep::Credential,
            LauncherStep::HostConfirmation,
            LauncherStep::Folder,
            LauncherStep::Launching,
        ] {
            state.step = step;
            let _view = session_launcher(&state, palette());
        }
    }
}
