//! Staged Local and SSH Project session launcher.

use crate::iced_shell::ShellMessage;
use crate::theme::{self, Palette};
use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Color, Element, Length, Padding};
use pandamux_core::{FolderListing, ProjectError, SshAuthConfig, SshHostProfile, SshProfileId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LauncherStep {
    #[default]
    Connection,
    ProfileForm,
    Credential,
    HostConfirmation,
    Folder,
    Launching,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
}

pub fn session_launcher(
    state: &SessionLauncherViewState,
    palette: Palette,
) -> Element<'_, ShellMessage> {
    let content = match state.step {
        LauncherStep::Connection => connection_step(state, palette),
        LauncherStep::ProfileForm => profile_form_step(state, palette),
        LauncherStep::Credential => credential_step(state, palette),
        LauncherStep::HostConfirmation => host_confirmation_step(state, palette),
        LauncherStep::Folder => folder_step(state, palette),
        LauncherStep::Launching => launching_step(state, palette),
    };
    let modal = container(content)
        .width(Length::Fixed(620.0))
        .max_height(650.0)
        .padding(24.0)
        .style(move |_theme| container::Style {
            background: Some(palette.panel2.into()),
            border: theme::border(palette.ov(0.12), 1.0, theme::RADIUS_OVERLAY),
            shadow: theme::overlay_shadow(),
            ..Default::default()
        });
    container(modal)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_theme| container::Style {
            background: Some(palette.scrim.into()),
            ..Default::default()
        })
        .into()
}

fn connection_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let mut choices = column![launcher_title(
        "New Session",
        "Choose a connection",
        palette
    )]
    .spacing(10);
    choices = choices.push(wide_button(
        "Local",
        "PowerShell on this computer",
        ShellMessage::LauncherLocalSelected,
        palette,
    ));
    for profile in &state.profiles {
        let detail = if profile.jump.is_some() {
            "ProxyJump is not supported".to_string()
        } else {
            format!("{}@{}:{}", profile.user, profile.host, profile.port)
        };
        let mut select = button(
            column![
                text(&profile.name).color(palette.t1),
                text(detail).size(theme::SIZE_METADATA).color(palette.t3)
            ]
            .spacing(2),
        )
        .padding(12.0)
        .width(Length::Fill);
        if profile.jump.is_none() {
            select = select.on_press(ShellMessage::LauncherProfileSelected(profile.id.clone()));
        }
        let edit = button(text("Edit").color(palette.t3))
            .on_press(ShellMessage::LauncherProfileEdit(profile.id.clone()));
        let delete = button(text("Delete").color(danger()))
            .on_press(ShellMessage::LauncherProfileDelete(profile.id.clone()));
        choices = choices.push(
            row![select, edit, delete]
                .spacing(6)
                .align_y(Alignment::Center),
        );
    }
    choices = choices.push(
        row![
            button("Add SSH Connection").on_press(ShellMessage::LauncherProfileAdd),
            button("Import SSH Config").on_press(ShellMessage::LauncherProfileImport),
            Space::new().width(Length::Fill),
            button("Cancel").on_press(ShellMessage::OverlayDismissed),
        ]
        .spacing(8),
    );
    choices.into()
}

fn profile_form_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let form = &state.form;
    let mut content = column![
        launcher_title("SSH Connection", "Save connection details", palette),
        labeled_input("Name", &form.name, ShellMessage::LauncherProfileNameChanged),
        labeled_input(
            "SSH Host",
            &form.host,
            ShellMessage::LauncherProfileHostChanged
        ),
        labeled_input(
            "SSH Port",
            &form.port,
            ShellMessage::LauncherProfilePortChanged
        ),
        text("Authentication").color(palette.t3),
        row![
            button("Windows OpenSSH agent").on_press(ShellMessage::LauncherProfileAuthChanged(
                SshAuthConfig::Agent
            )),
            button("Identity File").on_press(ShellMessage::LauncherProfileAuthChanged(
                SshAuthConfig::KeyFile {
                    path: form.identity_file.clone()
                }
            )),
            button("Password Prompt").on_press(ShellMessage::LauncherProfileAuthChanged(
                SshAuthConfig::Password
            )),
        ]
        .spacing(6),
    ]
    .spacing(9);
    if matches!(form.auth, SshAuthConfig::KeyFile { .. }) {
        content = content.push(labeled_input(
            "Identity File",
            &form.identity_file,
            ShellMessage::LauncherIdentityFileChanged,
        ));
    }
    if let Some(error) = &form.error {
        content = content.push(text(error).color(danger()));
    }
    let mut save = button("Save");
    if form.is_valid() {
        save = save.on_press(ShellMessage::LauncherProfileSave);
    }
    content = content.push(
        row![
            button("Back").on_press(ShellMessage::LauncherBack),
            Space::new().width(Length::Fill),
            save,
        ]
        .spacing(8),
    );
    content.into()
}

fn credential_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let mut continue_button = button("Continue");
    if !state.credential.is_empty() {
        continue_button = continue_button.on_press(ShellMessage::LauncherCredentialSubmit);
    }
    column![
        launcher_title(
            "Credentials Required",
            "Credentials stay in memory until exit",
            palette
        ),
        text_input("Password or key passphrase", &state.credential)
            .secure(true)
            .on_input(ShellMessage::LauncherCredentialChanged),
        error_line(state, palette),
        row![
            button("Back").on_press(ShellMessage::LauncherBack),
            Space::new().width(Length::Fill),
            continue_button,
        ],
    ]
    .spacing(12)
    .into()
}

fn host_confirmation_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    column![
        launcher_title(
            "Unknown Host Key",
            "Confirm the server fingerprint",
            palette
        ),
        text(
            state
                .fingerprint
                .as_deref()
                .unwrap_or("Fingerprint unavailable")
        )
        .font(theme::mono(iced::font::Weight::Medium))
        .color(palette.t1),
        row![
            button("Cancel").on_press(ShellMessage::LauncherBack),
            Space::new().width(Length::Fill),
            button("Trust and Continue").on_press(ShellMessage::LauncherHostTrustConfirmed),
        ],
    ]
    .spacing(14)
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
    let mut go = button("Go");
    if !state.loading && !state.path.trim().is_empty() {
        go = go.on_press(ShellMessage::LauncherFolderGo);
    }
    let mut content = column![
        launcher_title(title, "Choose the Project root", palette),
        row![
            text_input("Folder path", &state.path)
                .on_input(ShellMessage::LauncherPathChanged)
                .on_submit(ShellMessage::LauncherFolderGo)
                .width(Length::Fill),
            go,
        ]
        .spacing(6),
    ]
    .spacing(10);
    if let Some(listing) = &state.listing {
        let mut breadcrumbs = row![].spacing(4);
        for crumb in &listing.breadcrumbs {
            breadcrumbs = breadcrumbs.push(
                button(text(&crumb.label).size(theme::SIZE_METADATA)).on_press(
                    ShellMessage::LauncherFolderNavigate(crumb.canonical_path.clone()),
                ),
            );
        }
        content = content.push(breadcrumbs);
        if let Some(parent) = &listing.parent_path {
            content = content.push(
                button(".. Parent").on_press(ShellMessage::LauncherFolderNavigate(parent.clone())),
            );
        }
        let mut folders = column![].spacing(4);
        for directory in &listing.directories {
            folders = folders.push(
                button(text(&directory.name).color(palette.t1))
                    .width(Length::Fill)
                    .on_press(ShellMessage::LauncherFolderNavigate(
                        directory.canonical_path.clone(),
                    )),
            );
        }
        if listing.directories.is_empty() {
            folders = folders.push(text("No subfolders").color(palette.t4));
        }
        content = content.push(scrollable(folders).height(Length::Fixed(300.0)));
    }
    if state.loading {
        content = content.push(text("Loading...").color(palette.t3));
    }
    content = content.push(error_line(state, palette));
    let mut select = button("Select Folder");
    if state
        .listing
        .as_ref()
        .is_some_and(|listing| listing.canonical_path == state.path)
        && !state.loading
        && !state.launching
    {
        select = select.on_press(ShellMessage::LauncherFolderSelected);
    }
    content = content.push(
        row![
            button("Back").on_press(ShellMessage::LauncherBack),
            Space::new().width(Length::Fill),
            select,
        ]
        .spacing(8),
    );
    content.into()
}

fn launching_step<'a>(
    state: &'a SessionLauncherViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    column![
        launcher_title(
            "Launching",
            "Starting the terminal before saving the Project",
            palette
        ),
        text("Connecting...").color(palette.t3),
        error_line(state, palette),
    ]
    .spacing(12)
    .into()
}

fn launcher_title<'a>(
    title: &'a str,
    subtitle: &'a str,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    column![
        text(title)
            .size(theme::SIZE_TITLE)
            .font(theme::ui(iced::font::Weight::Semibold))
            .color(palette.t1),
        text(subtitle).size(theme::SIZE_SECONDARY).color(palette.t3),
    ]
    .spacing(3)
    .into()
}

fn labeled_input<'a>(
    label: &'a str,
    value: &'a str,
    on_input: fn(String) -> ShellMessage,
) -> Element<'a, ShellMessage> {
    column![text(label), text_input(label, value).on_input(on_input)]
        .spacing(3)
        .into()
}

fn wide_button<'a>(
    title: &'a str,
    detail: &'a str,
    message: ShellMessage,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    button(
        column![
            text(title).color(palette.t1),
            text(detail).size(theme::SIZE_METADATA).color(palette.t3),
        ]
        .spacing(2),
    )
    .padding(Padding::from([10.0, 12.0]))
    .width(Length::Fill)
    .on_press(message)
    .into()
}

fn error_line<'a>(
    state: &'a SessionLauncherViewState,
    _palette: Palette,
) -> Element<'a, ShellMessage> {
    text(
        state
            .error
            .as_ref()
            .map(|error| error.message.as_str())
            .unwrap_or(""),
    )
    .color(danger())
    .into()
}

fn danger() -> Color {
    Color::from_rgb8(0xe0, 0x6c, 0x75)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn launcher_stages_connection_before_folder() {
        let mut state = SessionLauncherViewState::default();
        assert_eq!(state.step, LauncherStep::Connection);
        state.step = LauncherStep::Folder;
        state.remote = true;
        assert_eq!(state.step, LauncherStep::Folder);
        assert!(state.remote);
    }
}
