//! Canonical Project location metadata and platform-specific folder helpers.
//!
//! A workspace is the durable Project owner. Names are presentation only;
//! [`ProjectKey`] derives identity from the normalized saved location.

use crate::SshProfileId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProjectLocation {
    #[default]
    Legacy,
    Local {
        cwd: String,
        shell: String,
    },
    Ssh {
        profile_id: SshProfileId,
        remote_cwd: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSpec {
    pub location: ProjectLocation,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectKey(String);

impl ProjectKey {
    pub fn from_location(location: &ProjectLocation) -> Result<Option<Self>, String> {
        match location {
            ProjectLocation::Legacy => Ok(None),
            ProjectLocation::Local { cwd, .. } => Ok(Some(Self(format!(
                "local:{}",
                normalize_windows_path(cwd)?
            )))),
            ProjectLocation::Ssh {
                profile_id,
                remote_cwd,
            } => Ok(Some(Self(format!(
                "ssh:{}:{}",
                profile_id,
                normalize_posix_path(remote_cwd)?
            )))),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderBreadcrumb {
    pub label: String,
    pub canonical_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderEntry {
    pub name: String,
    pub canonical_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderListing {
    pub canonical_path: String,
    pub parent_path: Option<String>,
    pub breadcrumbs: Vec<FolderBreadcrumb>,
    pub directories: Vec<FolderEntry>,
    /// Ready local drive roots (e.g. `C:\`), for the Windows drive switcher.
    /// Empty for remote listings and on non-Windows hosts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drives: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectErrorCategory {
    Validation,
    Filesystem,
    Connection,
    HostKeyUnknown,
    HostKeyChanged,
    Authentication,
    RemotePath,
    PtyStart,
    ProfileMissing,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectError {
    pub code: String,
    pub category: ProjectErrorCategory,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known_hosts_line: Option<usize>,
}

impl ProjectError {
    pub fn new(
        code: impl Into<String>,
        category: ProjectErrorCategory,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            code: code.into(),
            category,
            message: message.into(),
            retryable,
            fingerprint: None,
            known_hosts_line: None,
        }
    }
}

pub fn normalize_windows_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("local path is empty".to_string());
    }
    let replaced = trimmed.replace('/', "\\");
    let is_unc = replaced.starts_with("\\\\");
    let is_drive = replaced.as_bytes().get(1) == Some(&b':')
        && replaced
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic);
    if !is_unc && !is_drive {
        return Err("local path must be an absolute drive or UNC path".to_string());
    }

    let prefix_len = if is_unc { 2 } else { 0 };
    let mut normalized = String::from(&replaced[..prefix_len]);
    let mut previous_separator = false;
    for character in replaced[prefix_len..].chars() {
        if character == '\\' {
            if !previous_separator {
                normalized.push(character);
            }
            previous_separator = true;
        } else {
            normalized.push(character);
            previous_separator = false;
        }
    }

    let root_len = windows_root_len(&normalized);
    while normalized.len() > root_len && normalized.ends_with('\\') {
        normalized.pop();
    }
    Ok(normalized.to_lowercase())
}

pub fn normalize_posix_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if !trimmed.starts_with('/') {
        return Err("remote path must be absolute".to_string());
    }
    let mut components = Vec::new();
    for component in trimmed.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            value => components.push(value),
        }
    }
    if components.is_empty() {
        Ok("/".to_string())
    } else {
        Ok(format!("/{}", components.join("/")))
    }
}

pub fn project_title(location: &ProjectLocation) -> String {
    let path = match location {
        ProjectLocation::Legacy => return "Project".to_string(),
        ProjectLocation::Local { cwd, .. } => cwd.trim_end_matches(['\\', '/']),
        ProjectLocation::Ssh { remote_cwd, .. } => remote_cwd.trim_end_matches('/'),
    };
    path.rsplit(['\\', '/'])
        .find(|part| !part.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match location {
            ProjectLocation::Local { cwd, .. } => local_root_label(cwd),
            ProjectLocation::Ssh { .. } => "/".to_string(),
            ProjectLocation::Legacy => "Project".to_string(),
        })
}

pub fn local_parent(path: &str) -> Option<String> {
    let display = normalize_windows_display(path).ok()?;
    let root_len = windows_root_len(&display);
    if display.len() <= root_len {
        return None;
    }
    let trimmed = display.trim_end_matches('\\');
    let index = trimmed.rfind('\\')?;
    let parent = if index < root_len {
        &display[..root_len]
    } else {
        &trimmed[..index]
    };
    Some(parent.to_string())
}

pub fn posix_parent(path: &str) -> Option<String> {
    let normalized = normalize_posix_path(path).ok()?;
    if normalized == "/" {
        return None;
    }
    let parent = normalized
        .rsplit_once('/')
        .map(|(value, _)| value)
        .unwrap_or("");
    Some(if parent.is_empty() { "/" } else { parent }.to_string())
}

pub fn local_breadcrumbs(path: &str) -> Vec<FolderBreadcrumb> {
    let Ok(display) = normalize_windows_display(path) else {
        return Vec::new();
    };
    let root_len = windows_root_len(&display);
    let root = display[..root_len].to_string();
    let mut crumbs = vec![FolderBreadcrumb {
        label: local_root_label(&root),
        canonical_path: root.clone(),
    }];
    let mut current = root.trim_end_matches('\\').to_string();
    for component in display[root_len..]
        .split('\\')
        .filter(|part| !part.is_empty())
    {
        if !current.ends_with('\\') {
            current.push('\\');
        }
        current.push_str(component);
        crumbs.push(FolderBreadcrumb {
            label: component.to_string(),
            canonical_path: current.clone(),
        });
    }
    crumbs
}

pub fn posix_breadcrumbs(path: &str) -> Vec<FolderBreadcrumb> {
    let Ok(normalized) = normalize_posix_path(path) else {
        return Vec::new();
    };
    let mut crumbs = vec![FolderBreadcrumb {
        label: "/".to_string(),
        canonical_path: "/".to_string(),
    }];
    let mut current = String::new();
    for component in normalized.split('/').filter(|part| !part.is_empty()) {
        current.push('/');
        current.push_str(component);
        crumbs.push(FolderBreadcrumb {
            label: component.to_string(),
            canonical_path: current.clone(),
        });
    }
    crumbs
}

/// Strip the Windows verbatim (`\\?\`) prefix `std::fs::canonicalize` adds, so
/// stored and displayed local paths read like Explorer paths (`D:\Dev`, not
/// `\\?\D:\Dev`). UNC verbatim paths (`\\?\UNC\server\share`) collapse back to
/// `\\server\share`. Non-verbatim paths pass through unchanged.
pub fn strip_windows_verbatim(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = path.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else {
        path.to_string()
    }
}

pub fn sort_directories(directories: &mut [FolderEntry]) {
    directories.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn normalize_windows_display(path: &str) -> Result<String, String> {
    let key = normalize_windows_path(path)?;
    let original = path.trim().replace('/', "\\");
    let mut display = String::new();
    let prefix_len = if original.starts_with("\\\\") { 2 } else { 0 };
    display.push_str(&original[..prefix_len]);
    let mut previous_separator = false;
    for character in original[prefix_len..].chars() {
        if character == '\\' {
            if !previous_separator {
                display.push(character);
            }
            previous_separator = true;
        } else {
            display.push(character);
            previous_separator = false;
        }
    }
    let root_len = windows_root_len(&display);
    while display.len() > root_len && display.ends_with('\\') {
        display.pop();
    }
    debug_assert_eq!(
        normalize_windows_path(&display).ok().as_deref(),
        Some(key.as_str())
    );
    Ok(display)
}

fn windows_root_len(path: &str) -> usize {
    if path.starts_with("\\\\") {
        let mut separators = path.match_indices('\\').map(|(index, _)| index).skip(2);
        separators.nth(1).map_or(path.len(), |index| index + 1)
    } else if path.as_bytes().get(1) == Some(&b':') {
        if path.as_bytes().get(2) == Some(&b'\\') {
            3
        } else {
            2
        }
    } else {
        0
    }
}

fn local_root_label(path: &str) -> String {
    let trimmed = path.trim_end_matches('\\');
    if trimmed.is_empty() {
        "\\\\".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_keys_are_location_based() {
        let local_a = ProjectLocation::Local {
            cwd: "C:/Dev/Repo/".to_string(),
            shell: "pwsh.exe".to_string(),
        };
        let local_b = ProjectLocation::Local {
            cwd: "c:\\dev\\repo".to_string(),
            shell: "powershell.exe".to_string(),
        };
        assert_eq!(
            ProjectKey::from_location(&local_a),
            ProjectKey::from_location(&local_b)
        );

        let ssh_a = ProjectLocation::Ssh {
            profile_id: SshProfileId::from("ssh-a"),
            remote_cwd: "/srv/./app/".to_string(),
        };
        let ssh_b = ProjectLocation::Ssh {
            profile_id: SshProfileId::from("ssh-b"),
            remote_cwd: "/srv/app".to_string(),
        };
        assert_ne!(
            ProjectKey::from_location(&ssh_a),
            ProjectKey::from_location(&ssh_b)
        );
    }

    #[test]
    fn folder_helpers_cover_roots_unc_and_unicode() {
        assert_eq!(local_parent("C:\\"), None);
        assert_eq!(local_parent("C:\\Dev\\Repo"), Some("C:\\Dev".to_string()));
        assert_eq!(
            local_parent("\\\\server\\share\\folder"),
            Some("\\\\server\\share\\".to_string())
        );
        assert_eq!(posix_parent("/home/chaz"), Some("/home".to_string()));
        assert_eq!(posix_parent("/"), None);
        assert_eq!(
            project_title(&ProjectLocation::Local {
                cwd: "D:\\開発\\Panda MUX".to_string(),
                shell: "pwsh.exe".to_string(),
            }),
            "Panda MUX"
        );
    }

    #[test]
    fn verbatim_prefixes_are_stripped_for_drive_and_unc_paths() {
        assert_eq!(
            strip_windows_verbatim("\\\\?\\D:\\Dev\\Repo"),
            "D:\\Dev\\Repo"
        );
        assert_eq!(strip_windows_verbatim("\\\\?\\C:\\"), "C:\\");
        assert_eq!(
            strip_windows_verbatim("\\\\?\\UNC\\server\\share\\folder"),
            "\\\\server\\share\\folder"
        );
        assert_eq!(strip_windows_verbatim("D:\\Dev"), "D:\\Dev");
        assert_eq!(strip_windows_verbatim("/home/chaz"), "/home/chaz");
    }

    #[test]
    fn breadcrumbs_keep_platform_semantics_separate() {
        let local = local_breadcrumbs("C:\\Dev\\Panda MUX");
        assert_eq!(
            local.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            vec!["C:", "Dev", "Panda MUX"]
        );
        let remote = posix_breadcrumbs("/home/chaz/Panda MUX");
        assert_eq!(
            remote.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            vec!["/", "home", "chaz", "Panda MUX"]
        );
    }
}
