//! Shared Project launch coordination and folder listing services.

use pandamux_core::{
    AppDelta, AppIntent, AppState, FolderEntry, FolderListing, PaneId, ProjectError,
    ProjectErrorCategory, ProjectKey, ProjectLocation, ProjectSpec, SshAuthConfig, SshHostProfile,
    SurfaceId, SurfaceIntent, SurfaceType, WorkspaceId, WorkspaceIntent, local_breadcrumbs,
    local_parent, posix_breadcrumbs, posix_parent, project_title, sort_directories,
    strip_windows_verbatim,
};
use pandamux_term::{
    GridSize, PtyCommand, PtySessionManager, RemoteSessionManager, SshAuth, SshConfig,
    SshErrorCategory, SshFailure, browse_remote_folders, resolve_powershell,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EphemeralCredential {
    Password(String),
    KeyPassphrase(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchTarget {
    pub workspace_id: WorkspaceId,
    pub pane_id: PaneId,
    pub surface_id: SurfaceId,
    pub existing_workspace: bool,
    pub title: String,
    pub location: ProjectLocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchSuccess {
    pub workspace_id: WorkspaceId,
    pub pane_id: PaneId,
    pub surface_id: SurfaceId,
    pub reused_project: bool,
}

pub fn prepare_launch(
    app: &AppState,
    location: ProjectLocation,
) -> Result<LaunchTarget, ProjectError> {
    let key = ProjectKey::from_location(&location)
        .map_err(validation_error)?
        .ok_or_else(|| validation_error("Legacy Projects need a selected folder"))?;
    if let Some(workspace) = app.workspace_by_project_key(&key) {
        let pane_id = workspace
            .focused_pane_id
            .clone()
            .or_else(|| {
                pandamux_core::get_all_pane_ids(&workspace.split_tree)
                    .into_iter()
                    .next()
            })
            .ok_or_else(|| validation_error("Project has no pane for a new session"))?;
        return Ok(LaunchTarget {
            workspace_id: workspace.id.clone(),
            pane_id,
            surface_id: SurfaceId::generate(),
            existing_workspace: true,
            title: workspace.title.clone(),
            location,
        });
    }
    Ok(LaunchTarget {
        workspace_id: WorkspaceId::generate(),
        pane_id: PaneId::generate(),
        surface_id: SurfaceId::generate(),
        existing_workspace: false,
        title: project_title(&location),
        location,
    })
}

pub fn commit_prestarted(
    app: &mut AppState,
    target: &LaunchTarget,
    shell: &str,
) -> Result<LaunchSuccess, ProjectError> {
    let delta = if target.existing_workspace {
        app.apply(AppIntent::Surface(SurfaceIntent::CreateWithId {
            workspace_id: target.workspace_id.clone(),
            pane_id: Some(target.pane_id.clone()),
            surface_id: target.surface_id.clone(),
            surface_type: SurfaceType::Terminal,
        }))
    } else {
        app.apply(AppIntent::Workspace(WorkspaceIntent::CreateProject {
            workspace_id: target.workspace_id.clone(),
            pane_id: target.pane_id.clone(),
            surface_id: target.surface_id.clone(),
            title: target.title.clone(),
            shell: shell.to_string(),
            project: ProjectSpec {
                location: target.location.clone(),
            },
        }))
    }
    .map_err(|message| {
        ProjectError::new(
            "project_commit_failed",
            ProjectErrorCategory::Validation,
            message,
            false,
        )
    })?;

    let (workspace_id, pane_id, surface_id) = match delta {
        AppDelta::WorkspaceCreated { workspace, tree } => {
            let pane_id = pandamux_core::get_all_pane_ids(&tree)
                .into_iter()
                .next()
                .expect("created Project has a pane");
            (workspace.id, pane_id, target.surface_id.clone())
        }
        AppDelta::SurfaceCreated {
            workspace_id,
            pane_id,
            surface,
        } => (workspace_id, pane_id, surface.id),
        _ => unreachable!("Project commit returned an unrelated delta"),
    };
    Ok(LaunchSuccess {
        workspace_id,
        pane_id,
        surface_id,
        reused_project: target.existing_workspace,
    })
}

pub fn launch_local(
    app: &mut AppState,
    ptys: &mut PtySessionManager,
    cwd: String,
    spawn_pty: bool,
) -> Result<LaunchSuccess, ProjectError> {
    let shell = resolve_powershell().ok_or_else(|| {
        ProjectError::new(
            "powershell_not_found",
            ProjectErrorCategory::PtyStart,
            "PowerShell 7 and Windows PowerShell are unavailable",
            false,
        )
    })?;
    let location = ProjectLocation::Local {
        cwd: cwd.clone(),
        shell: shell.clone(),
    };
    let target = prepare_launch(app, location)?;
    if spawn_pty {
        let command = PtyCommand::new(shell.clone()).with_cwd(Some(cwd)).with_env(
            crate::backend::pandamux_env(target.surface_id.as_str(), None),
        );
        ptys.spawn(
            target.surface_id.to_string(),
            &command,
            GridSize::new(120, 30),
        )
        .map_err(|error| {
            ProjectError::new(
                "local_pty_start_failed",
                ProjectErrorCategory::PtyStart,
                format!("start PowerShell Project: {error}"),
                true,
            )
        })?;
    }
    match commit_prestarted(app, &target, &shell) {
        Ok(success) => Ok(success),
        Err(error) => {
            if spawn_pty {
                let _ = ptys.kill(target.surface_id.as_str());
            }
            Err(error)
        }
    }
}

pub fn launch_remote_blocking(
    app: &mut AppState,
    remotes: &mut RemoteSessionManager,
    remote_configs: &mut HashMap<SurfaceId, SshConfig>,
    profile: &SshHostProfile,
    remote_cwd: String,
    credential: Option<&EphemeralCredential>,
    trust_unknown_host: bool,
    spawn_pty: bool,
) -> Result<LaunchSuccess, ProjectError> {
    let location = ProjectLocation::Ssh {
        profile_id: profile.id.clone(),
        remote_cwd: remote_cwd.clone(),
    };
    let target = prepare_launch(app, location)?;
    let config = ssh_config(profile, remote_cwd, credential, trust_unknown_host)?;
    if spawn_pty {
        remotes
            .connect_ready(
                target.surface_id.to_string(),
                config.clone(),
                GridSize::new(120, 30),
                Duration::from_secs(30),
            )
            .map_err(|message| {
                ProjectError::new(
                    "ssh_pty_start_failed",
                    ProjectErrorCategory::PtyStart,
                    message,
                    true,
                )
            })?;
    }
    match commit_prestarted(app, &target, "ssh") {
        Ok(success) => {
            remote_configs.insert(target.surface_id, config);
            Ok(success)
        }
        Err(error) => {
            if spawn_pty {
                let _ = remotes.kill(target.surface_id.as_str());
            }
            Err(error)
        }
    }
}

pub fn ssh_config(
    profile: &SshHostProfile,
    remote_cwd: String,
    credential: Option<&EphemeralCredential>,
    trust_unknown_host: bool,
) -> Result<SshConfig, ProjectError> {
    if profile.jump.is_some() {
        return Err(ProjectError::new(
            "ssh_proxy_jump_unsupported",
            ProjectErrorCategory::Unsupported,
            format!("{} uses ProxyJump, which is not supported", profile.name),
            false,
        ));
    }
    let auth = match &profile.auth {
        SshAuthConfig::Agent => SshAuth::Agent {
            pipe_path: r"\\.\pipe\openssh-ssh-agent".to_string(),
        },
        SshAuthConfig::KeyFile { path } => SshAuth::KeyFile {
            path: PathBuf::from(path),
            passphrase: match credential {
                Some(EphemeralCredential::KeyPassphrase(value)) => Some(value.clone()),
                _ => None,
            },
        },
        SshAuthConfig::Password => match credential {
            Some(EphemeralCredential::Password(password)) => SshAuth::Password {
                password: password.clone(),
            },
            _ => {
                return Err(ProjectError::new(
                    "ssh_credential_required",
                    ProjectErrorCategory::Authentication,
                    format!("{} requires a password", profile.name),
                    true,
                ));
            }
        },
    };
    Ok(
        SshConfig::new(profile.host.clone(), profile.user.clone(), auth)
            .with_port(profile.port)
            .with_remote_cwd(remote_cwd)
            .with_unknown_host_trust(trust_unknown_host),
    )
}

pub async fn list_local_folders(path: String) -> Result<FolderListing, ProjectError> {
    let input = PathBuf::from(&path);
    let canonical = tokio::fs::canonicalize(&input).await.map_err(|error| {
        ProjectError::new(
            "local_folder_missing",
            ProjectErrorCategory::Filesystem,
            format!("open local folder {path}: {error}"),
            true,
        )
    })?;
    let metadata = tokio::fs::metadata(&canonical).await.map_err(|error| {
        ProjectError::new(
            "local_folder_metadata_failed",
            ProjectErrorCategory::Filesystem,
            format!("inspect local folder {}: {error}", canonical.display()),
            true,
        )
    })?;
    if !metadata.is_dir() {
        return Err(ProjectError::new(
            "local_path_not_directory",
            ProjectErrorCategory::Validation,
            format!("{} is not a directory", canonical.display()),
            false,
        ));
    }
    // canonicalize returns Windows verbatim paths (`\\?\D:\...`); strip the
    // prefix so stored Project locations and the browser UI read naturally.
    let canonical_path = strip_windows_verbatim(&canonical.to_string_lossy());
    let mut reader = tokio::fs::read_dir(&canonical).await.map_err(|error| {
        ProjectError::new(
            "local_folder_access_denied",
            ProjectErrorCategory::Filesystem,
            format!("list local folder {canonical_path}: {error}"),
            true,
        )
    })?;
    let mut directories = Vec::new();
    while let Some(entry) = reader.next_entry().await.map_err(|error| {
        ProjectError::new(
            "local_folder_read_failed",
            ProjectErrorCategory::Filesystem,
            format!("read local folder {canonical_path}: {error}"),
            true,
        )
    })? {
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        if file_type.is_dir() {
            directories.push(FolderEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                canonical_path: strip_windows_verbatim(&entry.path().to_string_lossy()),
            });
        }
    }
    sort_directories(&mut directories);
    Ok(FolderListing {
        parent_path: local_parent(&canonical_path),
        breadcrumbs: local_breadcrumbs(&canonical_path),
        canonical_path,
        directories,
        drives: list_local_drives().await,
    })
}

/// The user's local home folder (`%USERPROFILE%` on Windows, `$HOME` off it),
/// used as the folder browser's starting point and Home shortcut.
pub fn local_home_folder() -> Option<String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// The local drive roots (`C:\`, `D:\`, ...) that are ready right now, probed
/// concurrently with a short timeout so an unplugged card reader or a stale
/// network mapping cannot stall the folder browser. Empty off Windows.
#[cfg(windows)]
async fn list_local_drives() -> Vec<String> {
    let mut probes = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        probes.push(tokio::spawn(async move {
            tokio::time::timeout(Duration::from_millis(400), tokio::fs::metadata(&root))
                .await
                .is_ok_and(|result| result.is_ok())
                .then_some(root)
        }));
    }
    let mut drives = Vec::new();
    for probe in probes {
        if let Ok(Some(root)) = probe.await {
            drives.push(root);
        }
    }
    drives
}

#[cfg(not(windows))]
async fn list_local_drives() -> Vec<String> {
    Vec::new()
}

pub async fn list_remote_folders(
    config: SshConfig,
    path: String,
) -> Result<FolderListing, ProjectError> {
    let listing = browse_remote_folders(config, path)
        .await
        .map_err(project_error_from_ssh)?;
    let mut directories = listing
        .directories
        .into_iter()
        .map(|entry| FolderEntry {
            name: entry.name,
            canonical_path: entry.canonical_path,
        })
        .collect::<Vec<_>>();
    sort_directories(&mut directories);
    Ok(FolderListing {
        parent_path: posix_parent(&listing.canonical_path),
        breadcrumbs: posix_breadcrumbs(&listing.canonical_path),
        canonical_path: listing.canonical_path,
        directories,
        drives: Vec::new(),
    })
}

pub fn project_error_from_ssh(failure: SshFailure) -> ProjectError {
    let category = match failure.category {
        SshErrorCategory::Connection => ProjectErrorCategory::Connection,
        SshErrorCategory::HostKeyUnknown => ProjectErrorCategory::HostKeyUnknown,
        SshErrorCategory::HostKeyChanged => ProjectErrorCategory::HostKeyChanged,
        SshErrorCategory::Authentication => ProjectErrorCategory::Authentication,
        SshErrorCategory::RemotePath => ProjectErrorCategory::RemotePath,
        SshErrorCategory::PtyStart => ProjectErrorCategory::PtyStart,
    };
    ProjectError {
        code: failure.code.to_string(),
        category,
        message: failure.message,
        retryable: failure.retryable,
        fingerprint: failure.fingerprint,
        known_hosts_line: failure.known_hosts_line,
    }
}

fn validation_error(message: impl Into<String>) -> ProjectError {
    ProjectError::new(
        "project_validation_failed",
        ProjectErrorCategory::Validation,
        message,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_location_reuses_project_after_rename() {
        let mut app = AppState::default();
        let cwd = "C:\\Dev\\PandaMUX".to_string();
        let target = prepare_launch(
            &app,
            ProjectLocation::Local {
                cwd: cwd.clone(),
                shell: "pwsh.exe".to_string(),
            },
        )
        .unwrap();
        commit_prestarted(&mut app, &target, "pwsh.exe").unwrap();
        app.apply(AppIntent::Workspace(WorkspaceIntent::Rename {
            workspace_id: target.workspace_id.clone(),
            title: "Edited".to_string(),
        }))
        .unwrap();
        let again = prepare_launch(
            &app,
            ProjectLocation::Local {
                cwd,
                shell: "powershell.exe".to_string(),
            },
        )
        .unwrap();
        assert!(again.existing_workspace);
        assert_eq!(again.workspace_id, target.workspace_id);
    }

    #[test]
    fn failed_commit_does_not_mutate_state() {
        let mut app = AppState::default();
        let before = app.clone();
        let target = LaunchTarget {
            workspace_id: app.active_workspace_id.clone(),
            pane_id: PaneId::generate(),
            surface_id: SurfaceId::generate(),
            existing_workspace: false,
            title: "Duplicate".to_string(),
            location: ProjectLocation::Local {
                cwd: "C:\\Dev".to_string(),
                shell: "pwsh.exe".to_string(),
            },
        };
        assert!(commit_prestarted(&mut app, &target, "pwsh.exe").is_err());
        assert_eq!(app, before);
    }

    #[test]
    fn profile_config_never_accepts_password_without_ephemeral_credential() {
        let mut profile = SshHostProfile::new("Server", "server", "chaz");
        profile.auth = SshAuthConfig::Password;
        let error = ssh_config(&profile, "/srv".to_string(), None, false).unwrap_err();
        assert_eq!(error.category, ProjectErrorCategory::Authentication);
    }
}
