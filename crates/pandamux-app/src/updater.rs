//! In-app update check (plan Phase 7). On launch and on an interval the GUI asks
//! GitHub for the latest release, semver-compares it to the running version, and
//! (past a quarantine window) raises the update banner. The Settings "Check for
//! updates" button runs the same check on demand, ignoring the quarantine. When
//! the user hits Install we download the signed `Setup.exe` asset and launch it.
//! Because the app discovers updates itself, the GitHub Release only needs the
//! single signed `Setup.exe` asset (no Velopack feed / `.nupkg`).
//!
//! This module keeps the decision logic (API parse, version compare, quarantine
//! gate) pure and hermetically unit-tested; the network fetch, the installer
//! download, and launching it are the only side effects, all gated behind the
//! `iced-runtime` feature so the headless build and the unit tests never touch
//! the network.

use serde::Deserialize;

/// The GitHub Releases "latest" endpoint for this repo.
pub const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/BoardPandas/Pandamux/releases/latest";

/// Default quarantine window: don't offer a release until it has been public for
/// this long, so a just-published broken build is not pushed to everyone
/// immediately (ports `updater.ts`'s quarantine intent).
pub const DEFAULT_QUARANTINE_SECS: u64 = 6 * 60 * 60;

/// A published release, distilled from the GitHub API response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseInfo {
    /// Version with any leading `v` stripped (e.g. `0.34.0`).
    pub version: String,
    /// The original tag (e.g. `v0.34.0`).
    pub tag: String,
    /// Download URL of the `.exe` installer asset, if present.
    pub installer_url: Option<String>,
    /// RFC 3339 publish timestamp (`YYYY-MM-DDTHH:MM:SSZ`).
    pub published_at: String,
    /// Release notes (the GitHub release body).
    pub notes: String,
}

// The subset of the GitHub API shape we read.
#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    published_at: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
}

/// Parse the GitHub `releases/latest` JSON into a [`ReleaseInfo`]. Drafts and
/// prereleases return `None` (never offered as updates).
pub fn parse_latest_release(json: &str) -> Option<ReleaseInfo> {
    let release: ApiRelease = serde_json::from_str(json).ok()?;
    if release.draft || release.prerelease {
        return None;
    }
    let installer_url = release
        .assets
        .iter()
        .find(|asset| asset.name.to_ascii_lowercase().ends_with(".exe"))
        .map(|asset| asset.browser_download_url.clone());
    Some(ReleaseInfo {
        version: normalize_version(&release.tag_name),
        tag: release.tag_name,
        installer_url,
        published_at: release.published_at,
        notes: release.body,
    })
}

fn normalize_version(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
}

/// Whether `candidate` is a strictly newer semver than `current`. Both may carry
/// a leading `v`; pre-release/build suffixes are ignored; missing or non-numeric
/// segments compare as 0.
pub fn is_newer(current: &str, candidate: &str) -> bool {
    semver_key(candidate) > semver_key(current)
}

fn semver_key(version: &str) -> (u64, u64, u64) {
    let normalized = normalize_version(version);
    let core = normalized.split(['-', '+']).next().unwrap_or("");
    let mut parts = core.split('.').map(|part| part.parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// The full offer decision: `candidate` is newer than `current` AND it has been
/// published for at least `quarantine_secs`. `now_unix` and the release's
/// `published_at` are compared in seconds since the Unix epoch.
pub fn should_offer(
    current: &str,
    release: &ReleaseInfo,
    now_unix: u64,
    quarantine_secs: u64,
) -> bool {
    if !is_newer(current, &release.version) {
        return false;
    }
    match rfc3339_to_unix(&release.published_at) {
        // Offer once it is at least `quarantine_secs` old.
        Some(published) => now_unix.saturating_sub(published) >= quarantine_secs,
        // No/!unparseable timestamp: fall back to offering (better than hiding a
        // real update because the field was missing).
        None => true,
    }
}

/// Parse a GitHub `YYYY-MM-DDTHH:MM:SSZ` timestamp into seconds since the Unix
/// epoch. Only the fixed UTC (`Z`) form GitHub emits is supported.
pub fn rfc3339_to_unix(ts: &str) -> Option<u64> {
    let ts = ts.trim();
    let (date, rest) = ts.split_once('T')?;
    let time = rest.strip_suffix('Z').unwrap_or(rest);
    // Drop any fractional seconds.
    let time = time.split('.').next().unwrap_or(time);

    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;

    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next().unwrap_or("0").parse().ok()?;

    let days = days_from_civil(year, month, day);
    let total = days * 86_400 + hour * 3_600 + minute * 60 + second;
    u64::try_from(total).ok()
}

/// Days since the Unix epoch for a civil (proleptic Gregorian) date. Howard
/// Hinnant's `days_from_civil` algorithm; exact, no leap-year special-casing at
/// the call site.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400; // [0, 399]
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Fetch the latest release and, if it is worth offering, return it. Network
/// side effect; only compiled for the GUI build. Any error (offline, rate
/// limited, parse failure) yields `None` so the caller simply skips this cycle.
#[cfg(feature = "iced-runtime")]
pub async fn check_for_update(
    current_version: String,
    now_unix: u64,
    quarantine_secs: u64,
) -> Option<ReleaseInfo> {
    let json = fetch_latest_release_json().await.ok()?;
    let release = parse_latest_release(&json)?;
    should_offer(&current_version, &release, now_unix, quarantine_secs).then_some(release)
}

/// Outcome of an on-demand ("Check for updates") check. Unlike the periodic
/// check ([`check_for_update`]) this ignores the quarantine window: a user who
/// explicitly asks should see any newer release right away.
#[cfg(feature = "iced-runtime")]
pub enum ManualOutcome {
    /// The running version is current (or the only release is a draft/prerelease).
    UpToDate,
    /// A strictly newer release was found.
    Newer(ReleaseInfo),
    /// The check could not complete (offline, rate limited, transport error).
    Failed(String),
}

/// Fetch the latest release and compare it to `current_version`, ignoring the
/// quarantine window. Network side effect; GUI build only. A transport failure
/// is surfaced as [`ManualOutcome::Failed`]; a payload we cannot read as a
/// release (no releases yet, a draft, or a rate-limit body) is treated as
/// "up to date" rather than a scary error.
#[cfg(feature = "iced-runtime")]
pub async fn check_latest(current_version: String) -> ManualOutcome {
    let json = match fetch_latest_release_json().await {
        Ok(json) => json,
        Err(error) => return ManualOutcome::Failed(error),
    };
    match parse_latest_release(&json) {
        Some(release) if is_newer(&current_version, &release.version) => {
            ManualOutcome::Newer(release)
        }
        _ => ManualOutcome::UpToDate,
    }
}

/// Download the installer at `url` to a temp file and launch it. Returns once the
/// installer process has started; the caller then closes the app so the NSIS
/// installer can replace the running files. GUI build only.
#[cfg(feature = "iced-runtime")]
pub async fn download_and_launch_installer(url: String) -> Result<(), String> {
    let user_agent = concat!("pandamux/", env!("CARGO_PKG_VERSION"));
    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "download failed: HTTP {}",
            response.status().as_u16()
        ));
    }
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;

    // Land the installer in the OS temp dir under a stable name.
    let mut path = std::env::temp_dir();
    path.push("PandaMUX-Setup.exe");
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|error| format!("write installer: {error}"))?;

    // Spawn it detached; on Windows this is the signed NSIS Setup.exe, a GUI
    // process, so no console window appears. Dropping the handle does not kill it.
    std::process::Command::new(&path)
        .spawn()
        .map(|_child| ())
        .map_err(|error| format!("launch installer: {error}"))
}

#[cfg(feature = "iced-runtime")]
async fn fetch_latest_release_json() -> Result<String, String> {
    // GitHub requires a User-Agent; identify by app + version.
    let user_agent = concat!("pandamux/", env!("CARGO_PKG_VERSION"));
    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(RELEASES_LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    response.text().await.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "tag_name": "v0.34.0",
        "body": "New stuff",
        "published_at": "2026-07-08T12:00:00Z",
        "draft": false,
        "prerelease": false,
        "assets": [
            { "name": "notes.txt", "browser_download_url": "https://x/notes.txt" },
            { "name": "PandaMUX-Setup.exe", "browser_download_url": "https://x/Setup.exe" }
        ]
    }"#;

    #[test]
    fn parses_latest_release_and_picks_the_exe_asset() {
        let release = parse_latest_release(SAMPLE).expect("parses");
        assert_eq!(release.version, "0.34.0");
        assert_eq!(release.tag, "v0.34.0");
        assert_eq!(
            release.installer_url.as_deref(),
            Some("https://x/Setup.exe")
        );
        assert_eq!(release.notes, "New stuff");
    }

    #[test]
    fn drafts_and_prereleases_are_not_offered() {
        let draft = SAMPLE.replace("\"draft\": false", "\"draft\": true");
        assert!(parse_latest_release(&draft).is_none());
        let pre = SAMPLE.replace("\"prerelease\": false", "\"prerelease\": true");
        assert!(parse_latest_release(&pre).is_none());
    }

    #[test]
    fn semver_compare_handles_v_prefix_and_segments() {
        assert!(is_newer("0.33.0", "0.34.0"));
        assert!(is_newer("v0.33.1", "v0.34.0"));
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(!is_newer("0.34.0", "0.34.0"));
        assert!(!is_newer("0.34.0", "0.33.9"));
        // Pre-release/build suffixes are ignored for the core comparison.
        assert!(!is_newer("1.2.3", "1.2.3-rc1"));
    }

    #[test]
    fn rfc3339_parses_github_timestamps() {
        // 2026-07-08T12:00:00Z. Sanity: it round-trips within a day of a known
        // reference and is monotonic.
        let a = rfc3339_to_unix("2026-07-08T12:00:00Z").expect("parses");
        let b = rfc3339_to_unix("2026-07-08T13:00:00Z").expect("parses");
        assert_eq!(b - a, 3600);
        // The Unix epoch itself.
        assert_eq!(rfc3339_to_unix("1970-01-01T00:00:00Z"), Some(0));
        // A known date: 2000-01-01T00:00:00Z = 946684800.
        assert_eq!(rfc3339_to_unix("2000-01-01T00:00:00Z"), Some(946_684_800));
        assert!(rfc3339_to_unix("not-a-date").is_none());
    }

    #[test]
    fn should_offer_respects_newer_and_quarantine() {
        let release = parse_latest_release(SAMPLE).expect("parses");
        let published = rfc3339_to_unix(&release.published_at).unwrap();

        // Older-or-equal current: never offered.
        assert!(!should_offer("0.34.0", &release, published + 10_000, 3600));
        // Newer but still inside the quarantine window: not yet.
        assert!(!should_offer("0.33.0", &release, published + 100, 3600));
        // Newer and past quarantine: offered.
        assert!(should_offer("0.33.0", &release, published + 7200, 3600));
    }

    #[test]
    fn should_offer_falls_back_when_timestamp_missing() {
        let mut release = parse_latest_release(SAMPLE).expect("parses");
        release.published_at = String::new();
        assert!(should_offer("0.33.0", &release, 0, DEFAULT_QUARANTINE_SECS));
    }
}
