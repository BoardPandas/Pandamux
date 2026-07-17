//! Stable project identity across hosts and transports (spec 1.4).
//!
//! A [`ProjectRecord`] is the explicit identity the spec recommends, held as a
//! config entry in [`crate::AppState`] (never as a marker file inside user
//! repos). Sessions ALWAYS group by project; the host a session runs on is a
//! badge, never an identity input. A record accumulates [`ProjectMatcher`]s:
//! the exact locations it was opened from, its git remote URL when one could
//! be read cheaply, and its normalized folder name as the heuristic fallback,
//! with precedence Location > GitRemote > FolderName. Manual merge/split/
//! rename set `manual`, which stops heuristics from ever overriding a human
//! decision.

use crate::ids::ProjectId;
use crate::project::{ProjectKey, ProjectLocation, project_title};
use crate::state::AppState;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub id: ProjectId,
    /// Display name, user-renameable.
    pub name: String,
    #[serde(default)]
    pub matchers: Vec<ProjectMatcher>,
    /// Locations this project has been opened from, most recent first.
    #[serde(default)]
    pub known_locations: Vec<ProjectLocation>,
    #[serde(default)]
    pub created_at_ms: u64,
    /// Set by user merge/split/rename: heuristics never auto-merge or split a
    /// record the user shaped by hand.
    #[serde(default)]
    pub manual: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProjectMatcher {
    /// Exact ProjectKey string ("local:..." / "ssh:<profile>:...").
    Location { key: String },
    /// Normalized git remote URL ("github.com/org/repo").
    GitRemote { url: String },
    /// Normalized last path segment ("supportforge").
    FolderName { name: String },
}

/// How a location resolved against the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectResolution {
    Existing(ProjectId),
    New(ProjectRecord),
}

/// Normalize a git remote URL so the same repository matches across schemes
/// and hosts spellings: lowercase, credentials stripped, scp-like syntax
/// (`git@host:org/repo.git`) and URL syntax (`https://host/org/repo`) both
/// become `host/org/repo`.
pub fn normalize_git_remote(url: &str) -> Option<String> {
    let trimmed = url.trim().to_lowercase();
    if trimmed.is_empty() {
        return None;
    }
    // Strip a scheme when present.
    let rest = trimmed
        .split_once("://")
        .map(|(_, rest)| rest.to_string())
        .unwrap_or(trimmed);
    // scp-like: user@host:path (no scheme). Convert the first ':' to '/'.
    let rest = match (rest.contains('@'), rest.split_once(':')) {
        (true, Some((host_part, path))) if !path.starts_with("//") => {
            format!("{host_part}/{path}")
        }
        _ => rest,
    };
    // Strip credentials before the host.
    let rest = rest
        .rsplit_once('@')
        .map(|(_, host_and_path)| host_and_path.to_string())
        .unwrap_or(rest);
    let cleaned = rest
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/')
        .to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Normalized last path segment of a project folder ("C:\\Dev\\SupportForge"
/// and "/home/x/supportforge" both become "supportforge").
pub fn normalize_folder_name(location: &ProjectLocation) -> Option<String> {
    let path = match location {
        ProjectLocation::Local { cwd, .. } => cwd.as_str(),
        ProjectLocation::Ssh { remote_cwd, .. } => remote_cwd.as_str(),
        ProjectLocation::Legacy => return None,
    };
    let name = path
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if name.is_empty() || name.ends_with(':') {
        None
    } else {
        Some(name)
    }
}

/// Parse the `origin` remote URL (or the first remote) out of a raw
/// `.git/config` file. A tiny hand parser: IO stays in pandamux-app.
pub fn parse_git_remote_url(config_text: &str) -> Option<String> {
    let mut current_section = String::new();
    let mut origin_url: Option<String> = None;
    let mut first_url: Option<String> = None;
    for line in config_text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            current_section = line.to_string();
            continue;
        }
        if !current_section.starts_with("[remote ") {
            continue;
        }
        if let Some(value) = line.strip_prefix("url") {
            let value = value.trim_start().strip_prefix('=')?.trim();
            if value.is_empty() {
                continue;
            }
            if first_url.is_none() {
                first_url = Some(value.to_string());
            }
            if current_section.contains("\"origin\"") && origin_url.is_none() {
                origin_url = Some(value.to_string());
            }
        }
    }
    origin_url.or(first_url)
}

/// The matchers a brand-new record starts with for `location`.
fn initial_matchers(location: &ProjectLocation, git_remote: Option<&str>) -> Vec<ProjectMatcher> {
    let mut matchers = Vec::new();
    if let Ok(Some(key)) = ProjectKey::from_location(location) {
        matchers.push(ProjectMatcher::Location {
            key: key.as_str().to_string(),
        });
    }
    if let Some(url) = git_remote.and_then(normalize_git_remote) {
        matchers.push(ProjectMatcher::GitRemote { url });
    }
    if let Some(name) = normalize_folder_name(location) {
        matchers.push(ProjectMatcher::FolderName { name });
    }
    matchers
}

/// Resolve a location against the registry: exact location match first, then
/// git remote, then folder name. Hosts never participate. When nothing
/// matches, a new record (with initial matchers) is proposed.
pub fn resolve_project_id(
    projects: &[ProjectRecord],
    location: &ProjectLocation,
    git_remote: Option<&str>,
    now_ms: u64,
) -> Option<ProjectResolution> {
    if matches!(location, ProjectLocation::Legacy) {
        return None;
    }
    let location_key = ProjectKey::from_location(location)
        .ok()
        .flatten()
        .map(|key| key.as_str().to_string());
    if let Some(key) = &location_key
        && let Some(record) = projects.iter().find(|record| {
            record
                .matchers
                .iter()
                .any(|matcher| matches!(matcher, ProjectMatcher::Location { key: k } if k == key))
        })
    {
        return Some(ProjectResolution::Existing(record.id.clone()));
    }
    if let Some(url) = git_remote.and_then(normalize_git_remote)
        && let Some(record) = projects.iter().find(|record| {
            record
                .matchers
                .iter()
                .any(|matcher| matches!(matcher, ProjectMatcher::GitRemote { url: u } if *u == url))
        })
    {
        return Some(ProjectResolution::Existing(record.id.clone()));
    }
    if let Some(name) = normalize_folder_name(location)
        && let Some(record) = projects.iter().find(|record| {
            record.matchers.iter().any(
                |matcher| matches!(matcher, ProjectMatcher::FolderName { name: n } if *n == name),
            )
        })
    {
        return Some(ProjectResolution::Existing(record.id.clone()));
    }
    Some(ProjectResolution::New(ProjectRecord {
        id: ProjectId::generate(),
        name: project_title(location),
        matchers: initial_matchers(location, git_remote),
        known_locations: vec![location.clone()],
        created_at_ms: now_ms,
        manual: false,
    }))
}

/// Assign a `project_id` to every workspace that lacks one, creating or
/// reusing registry records. Folder-name matching collapses the historical
/// per-host duplicates (the 1.4 migration). Legacy workspaces (no folder)
/// keep `project_id: None` and group per-workspace as before. Returns true
/// when anything changed.
pub fn ensure_project_registry(app: &mut AppState, now_ms: u64) -> bool {
    let mut changed = false;
    for index in 0..app.workspaces.len() {
        if app.workspaces[index].project_id.is_some() {
            continue;
        }
        let location = app.workspaces[index].project.location.clone();
        match resolve_project_id(&app.projects, &location, None, now_ms) {
            Some(ProjectResolution::Existing(project_id)) => {
                record_location(&mut app.projects, &project_id, &location);
                app.workspaces[index].project_id = Some(project_id);
                changed = true;
            }
            Some(ProjectResolution::New(record)) => {
                let project_id = record.id.clone();
                app.projects.push(record);
                app.workspaces[index].project_id = Some(project_id);
                changed = true;
            }
            None => {}
        }
    }
    // Drop records no workspace references anymore (keeps the registry from
    // accumulating ghosts as projects close; favorites/recents validate their
    // ids against this registry at load).
    let referenced: Vec<ProjectId> = app
        .workspaces
        .iter()
        .filter_map(|workspace| workspace.project_id.clone())
        .collect();
    let before = app.projects.len();
    app.projects
        .retain(|record| referenced.contains(&record.id) || record.manual);
    changed |= app.projects.len() != before;
    changed
}

/// Remember a location on a record (most recent first, deduped) and make sure
/// its exact key matches next time.
pub fn record_location(
    projects: &mut [ProjectRecord],
    project_id: &ProjectId,
    location: &ProjectLocation,
) {
    let Some(record) = projects.iter_mut().find(|record| &record.id == project_id) else {
        return;
    };
    record.known_locations.retain(|known| known != location);
    record.known_locations.insert(0, location.clone());
    record.known_locations.truncate(8);
    if let Ok(Some(key)) = ProjectKey::from_location(location) {
        let key = key.as_str().to_string();
        let already = record
            .matchers
            .iter()
            .any(|matcher| matches!(matcher, ProjectMatcher::Location { key: k } if *k == key));
        if !already {
            record.matchers.push(ProjectMatcher::Location { key });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::SshProfileId;

    fn local(cwd: &str) -> ProjectLocation {
        ProjectLocation::Local {
            cwd: cwd.to_string(),
            shell: "pwsh".to_string(),
        }
    }

    fn ssh(profile: &str, cwd: &str) -> ProjectLocation {
        ProjectLocation::Ssh {
            profile_id: SshProfileId::from(profile),
            remote_cwd: cwd.to_string(),
        }
    }

    #[test]
    fn git_remote_urls_normalize_across_schemes() {
        assert_eq!(
            normalize_git_remote("git@github.com:Org/Repo.git"),
            Some("github.com/org/repo".to_string())
        );
        assert_eq!(
            normalize_git_remote("https://github.com/org/repo"),
            Some("github.com/org/repo".to_string())
        );
        assert_eq!(
            normalize_git_remote("ssh://git@github.com/org/repo.git/"),
            Some("github.com/org/repo".to_string())
        );
        assert_eq!(
            normalize_git_remote("https://user:token@github.com/org/repo.git"),
            Some("github.com/org/repo".to_string())
        );
        assert_eq!(normalize_git_remote("   "), None);
    }

    #[test]
    fn folder_names_normalize_across_platforms() {
        assert_eq!(
            normalize_folder_name(&local("C:\\Dev\\SupportForge")),
            Some("supportforge".to_string())
        );
        assert_eq!(
            normalize_folder_name(&ssh("ssh-a", "/home/chaz/supportforge/")),
            Some("supportforge".to_string())
        );
        assert_eq!(normalize_folder_name(&ProjectLocation::Legacy), None);
        assert_eq!(normalize_folder_name(&local("C:\\")), None);
    }

    #[test]
    fn parses_origin_url_from_git_config() {
        let config = "[core]\n\trepositoryformatversion = 0\n[remote \"upstream\"]\n\turl = https://github.com/other/fork.git\n[remote \"origin\"]\n\turl = git@github.com:org/repo.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n";
        assert_eq!(
            parse_git_remote_url(config),
            Some("git@github.com:org/repo.git".to_string())
        );
        // No origin: the first remote wins.
        let config = "[remote \"fork\"]\n\turl = https://example.com/x.git\n";
        assert_eq!(
            parse_git_remote_url(config),
            Some("https://example.com/x.git".to_string())
        );
        assert_eq!(parse_git_remote_url("[core]\n"), None);
    }

    #[test]
    fn resolution_prefers_location_then_remote_then_folder() {
        let location = local("C:\\Dev\\Repo");
        let Some(ProjectResolution::New(record)) =
            resolve_project_id(&[], &location, Some("git@github.com:org/repo.git"), 1)
        else {
            panic!("expected a new record");
        };
        let projects = vec![record.clone()];

        // Same exact location.
        assert_eq!(
            resolve_project_id(&projects, &location, None, 2),
            Some(ProjectResolution::Existing(record.id.clone()))
        );
        // Different transport, same git remote.
        assert_eq!(
            resolve_project_id(
                &projects,
                &ssh("ssh-x", "/srv/checkout"),
                Some("https://github.com/org/repo"),
                3
            ),
            Some(ProjectResolution::Existing(record.id.clone()))
        );
        // Different transport, same folder name (the heuristic fallback).
        assert_eq!(
            resolve_project_id(&projects, &ssh("ssh-y", "/home/x/repo"), None, 4),
            Some(ProjectResolution::Existing(record.id.clone()))
        );
        // Unrelated folder: a new record.
        assert!(matches!(
            resolve_project_id(&projects, &local("D:\\Other\\Thing"), None, 5),
            Some(ProjectResolution::New(_))
        ));
        // Legacy never resolves.
        assert_eq!(
            resolve_project_id(&projects, &ProjectLocation::Legacy, None, 6),
            None
        );
    }

    #[test]
    fn hosts_never_affect_grouping() {
        let a = ssh("ssh-host-one", "/home/a/supportforge");
        let b = ssh("ssh-host-two", "/data/supportforge");
        let Some(ProjectResolution::New(record)) = resolve_project_id(&[], &a, None, 1) else {
            panic!("expected new");
        };
        let projects = vec![record.clone()];
        assert_eq!(
            resolve_project_id(&projects, &b, None, 2),
            Some(ProjectResolution::Existing(record.id))
        );
    }
}
