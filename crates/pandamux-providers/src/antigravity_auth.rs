//! Antigravity profile isolation and Google OAuth ownership.
//!
//! Ported to Rust from T3 Code's `antigravityAuthSupport.ts`,
//! `antigravityCallback.ts`, and `AntigravityAuth.ts`. The source material is
//! covered by the MIT notice in the repository root `THIRD_PARTY_NOTICES.md`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use pandamux_core::ProviderInstanceId;
use pandamux_core::event::{ApprovalDecision, ApprovalKind, TurnOutcome};
use pandamux_core::provider_config::ProviderInstanceConfig;
use pandamux_core::thread::{AccessMode, TurnInput};
use serde_json::Value;
use sha2::Digest;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, Notify, mpsc};
use tokio::task::JoinHandle;
use url::Url;

use crate::acp::{
    AcpInitializeResult, build_initialize_request, map_access_mode_to_acp, parse_acp_line,
    validate_antigravity_initialize,
};
use crate::antigravity::{ConcurrencyGuard, shared_antigravity_process_limiter};
use crate::antigravity_release::{
    current_bundle_manifest, platform_arch, preflight_disk_space, resolve_antigravity_executable,
};
use crate::error::ProviderError;
use crate::models::{ModelInfo, ProviderEvent, SessionSpec};
use crate::supervision::{SupervisedChild, create_supervised_command};
use crate::traits::{BoxFuture, ProviderSession};

pub const ANTIGRAVITY_AUTH_STDOUT_PREFIX: &str =
    "Open the following link to authenticate the ACP server: ";
pub const ANTIGRAVITY_AUTH_BROWSER_MARKER: &str = "__PANDAMUX_ANTIGRAVITY_AUTH_URL__";
pub const MIN_FREE_DISK_BYTES: u64 = 2 * 1024 * 1024 * 1024;

const MAX_AUTHORIZATION_URL_LENGTH: usize = 16_384;
const MAX_PROTOCOL_LINE_BYTES: usize = 16 * 1024 * 1024;
const MAX_BROWSER_LINE_BYTES: usize = MAX_AUTHORIZATION_URL_LENGTH + 128;
const AUTH_FLOW_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const RPC_TIMEOUT: Duration = Duration::from_secs(90);
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(10);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CLIENT_FILE_BYTES: u64 = 4 * 1024 * 1024;

fn protocol_error(message: impl Into<String>) -> ProviderError {
    ProviderError::ProtocolError {
        provider: "antigravity".to_string(),
        message: message.into(),
    }
}

fn auth_error(message: impl Into<String>) -> ProviderError {
    ProviderError::AuthRequired {
        provider: "antigravity".to_string(),
        message: message.into(),
    }
}

#[derive(Debug, Clone)]
pub struct AntigravityProcessEnv {
    pub vars: HashMap<String, String>,
    pub gemini_home: PathBuf,
    pub scratch_dir: PathBuf,
    pub harness_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedOAuthUrl {
    pub raw_url: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub redirect_port: u16,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AntigravityAuthStart {
    pub provider_instance_id: ProviderInstanceId,
    pub flow_id: String,
    pub auth_url: String,
    pub redirect_port: u16,
    pub state: String,
}

fn is_removed_environment_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    matches!(
        key.as_str(),
        "GEMINI_API_KEY"
            | "GOOGLE_API_KEY"
            | "GOOGLE_APPLICATION_CREDENTIALS"
            | "GOOGLE_CLOUD_PROJECT"
            | "GOOGLE_CLOUD_LOCATION"
            | "GOOGLE_CLOUD_QUOTA_PROJECT"
            | "GOOGLE_GENAI_USE_VERTEXAI"
            | "GCLOUD_PROJECT"
            | "CLOUDSDK_CORE_PROJECT"
            | "GEMINI_HOME"
            | "AGY_ACP_FORCE_FILE_STORAGE"
            | "ANTIGRAVITY_HARNESS_PATH"
            | "BROWSER"
            | "PYTHONUNBUFFERED"
            | "ELECTRON_RUN_AS_NODE"
    ) || key.starts_with("AGY_ACP_")
        || key.starts_with("GOOGLE_CLOUD_")
}

fn isolated_process_environment(
    gemini_home: &Path,
    scratch_dir: &Path,
    harness_path: &Path,
    browser_helper_cmd: &str,
    overrides: Option<&BTreeMap<String, String>>,
) -> HashMap<String, String> {
    let mut vars = std::env::vars()
        .filter(|(key, _)| !is_removed_environment_key(key))
        .collect::<HashMap<_, _>>();
    if let Some(overrides) = overrides {
        for (key, value) in overrides {
            if !is_removed_environment_key(key) {
                vars.insert(key.clone(), value.clone());
            }
        }
    }
    vars.insert(
        "GEMINI_HOME".to_string(),
        gemini_home.to_string_lossy().to_string(),
    );
    vars.insert(
        "ANTIGRAVITY_HARNESS_PATH".to_string(),
        harness_path.to_string_lossy().to_string(),
    );
    vars.insert("AGY_ACP_FORCE_FILE_STORAGE".to_string(), "1".to_string());
    vars.insert("PYTHONUNBUFFERED".to_string(), "1".to_string());
    vars.insert("ELECTRON_RUN_AS_NODE".to_string(), "1".to_string());
    vars.insert("BROWSER".to_string(), browser_helper_cmd.to_string());
    let scratch = scratch_dir.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        vars.insert("TEMP".to_string(), scratch.clone());
        vars.insert("TMP".to_string(), scratch);
    }
    #[cfg(not(windows))]
    {
        vars.insert("TMPDIR".to_string(), scratch);
    }
    vars
}

/// Builds a replaced process environment with a private profile and scratch directory.
pub fn build_antigravity_env(
    instance_id: &str,
    app_data_base: &Path,
    harness_path: &Path,
    browser_helper_cmd: &str,
) -> AntigravityProcessEnv {
    let gemini_home = app_data_base.join("profiles/antigravity").join(instance_id);
    let scratch_dir = app_data_base.join("scratch/antigravity").join(instance_id);
    let vars = isolated_process_environment(
        &gemini_home,
        &scratch_dir,
        harness_path,
        browser_helper_cmd,
        None,
    );
    AntigravityProcessEnv {
        vars,
        gemini_home,
        scratch_dir,
        harness_path: harness_path.to_path_buf(),
    }
}

pub fn validate_sanitized_environment(vars: &HashMap<String, String>) -> bool {
    vars.keys().all(|key| {
        !is_removed_environment_key(key)
            || matches!(
                key.to_ascii_uppercase().as_str(),
                "GEMINI_HOME"
                    | "ANTIGRAVITY_HARNESS_PATH"
                    | "AGY_ACP_FORCE_FILE_STORAGE"
                    | "BROWSER"
                    | "PYTHONUNBUFFERED"
                    | "ELECTRON_RUN_AS_NODE"
            )
    })
}

fn private_directory(path: &Path) -> Result<(), ProviderError> {
    std::fs::create_dir_all(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(protocol_error(format!(
            "Refusing unsafe Antigravity profile directory {}",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn prepare_profile(gemini_home: &Path, scratch_dir: &Path) -> Result<(), ProviderError> {
    let acp_directory = gemini_home.join("antigravity-acp");
    for directory in [gemini_home, acp_directory.as_path(), scratch_dir] {
        private_directory(directory)?;
    }
    let settings_path = acp_directory.join("settings.json");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut settings = options.open(settings_path)?;
    use std::io::Write;
    settings.write_all(b"{\"auth\":{\"type\":\"oauth-personal\"}}\n")?;
    settings.sync_all()?;
    Ok(())
}

fn quote_browser_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub fn browser_helper_command(executable: &Path) -> Result<String, ProviderError> {
    let helper = executable.to_string_lossy().replace('\\', "/");
    let forbidden_separator = if cfg!(windows) { ';' } else { ':' };
    if helper.contains(forbidden_separator)
        || helper.contains(['\r', '\n', '\0'])
        || helper.contains("%s")
    {
        return Err(protocol_error(
            "The PandaMUX server path cannot safely suppress Antigravity browser launches",
        ));
    }
    Ok([
        quote_browser_argument(&helper),
        quote_browser_argument("antigravity-browser-helper"),
        quote_browser_argument("%s"),
    ]
    .join(" "))
}

pub async fn preflight_browser_helper(executable: &Path) -> Result<(), ProviderError> {
    let preflight_url = "https://example.invalid/pandamux-antigravity-browser-preflight";
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(executable)
            .arg("antigravity-browser-helper")
            .arg(preflight_url)
            .output(),
    )
    .await
    .map_err(|_| protocol_error("Antigravity browser suppression preflight timed out"))?
    .map_err(|error| {
        protocol_error(format!(
            "Could not start the Antigravity browser suppression helper: {error}"
        ))
    })?;
    let expected = format!(
        "{ANTIGRAVITY_AUTH_BROWSER_MARKER}{}\n",
        serde_json::to_string(preflight_url)?
    );
    if !output.status.success() || !output.stdout.is_empty() || output.stderr != expected.as_bytes()
    {
        return Err(protocol_error(
            "Antigravity browser suppression preflight returned unexpected output",
        ));
    }
    Ok(())
}

pub fn extract_auth_url_from_output(line: &str) -> Option<String> {
    if let Some(value) = line.strip_prefix(ANTIGRAVITY_AUTH_STDOUT_PREFIX) {
        return Some(value.trim().to_string());
    }
    if let Some(value) = line.strip_prefix(ANTIGRAVITY_AUTH_BROWSER_MARKER) {
        return serde_json::from_str::<String>(value.trim()).ok();
    }
    if let Some(value) = line.strip_prefix("[BROWSER] ") {
        return Some(value.trim().to_string());
    }
    None
}

fn exactly_one_query_value(url: &Url, name: &str) -> Option<String> {
    let values = url
        .query_pairs()
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    (values.len() == 1).then(|| values[0].clone())
}

/// Strictly validates the authorization URL emitted by this ACP process.
pub fn validate_google_oauth_url(url_str: &str) -> Result<ValidatedOAuthUrl, ProviderError> {
    if url_str.len() > MAX_AUTHORIZATION_URL_LENGTH || url_str.chars().any(char::is_whitespace) {
        return Err(protocol_error(
            "Antigravity returned an invalid Google sign-in URL",
        ));
    }
    let url = Url::parse(url_str)
        .map_err(|_| protocol_error("Antigravity returned an invalid Google sign-in URL"))?;
    let state = exactly_one_query_value(&url, "state")
        .filter(|value| {
            !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_whitespace)
        })
        .ok_or_else(|| protocol_error("Antigravity returned an invalid Google sign-in URL"))?;
    let redirect_uri = exactly_one_query_value(&url, "redirect_uri")
        .ok_or_else(|| protocol_error("Antigravity returned an invalid Google sign-in URL"))?;
    let response_type = exactly_one_query_value(&url, "response_type");
    let client_id = exactly_one_query_value(&url, "client_id")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| protocol_error("Antigravity returned an invalid Google sign-in URL"))?;
    let redirect = Url::parse(&redirect_uri)
        .map_err(|_| protocol_error("Antigravity returned an invalid Google sign-in URL"))?;
    let redirect_port = redirect
        .port()
        .filter(|port| *port >= 1024)
        .ok_or_else(|| protocol_error("Antigravity returned an invalid Google sign-in URL"))?;

    if url.scheme() != "https"
        || url.host_str() != Some("accounts.google.com")
        || url.port_or_known_default() != Some(443)
        || url.path() != "/o/oauth2/v2/auth"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || response_type.as_deref() != Some("code")
        || redirect.scheme() != "http"
        || redirect.host_str() != Some("127.0.0.1")
        || redirect.path() != "/"
        || !redirect.username().is_empty()
        || redirect.password().is_some()
        || redirect.query().is_some()
        || redirect.fragment().is_some()
    {
        return Err(protocol_error(
            "Antigravity returned an invalid Google sign-in URL",
        ));
    }

    Ok(ValidatedOAuthUrl {
        raw_url: url_str.to_string(),
        client_id,
        redirect_uri,
        redirect_port,
        state,
    })
}

fn validate_callback_url(
    callback_url: &str,
    pending: &ValidatedOAuthUrl,
) -> Result<Url, ProviderError> {
    if callback_url.len() > MAX_AUTHORIZATION_URL_LENGTH {
        return Err(protocol_error("The sign-in response URL is too long"));
    }
    let callback = Url::parse(callback_url)
        .map_err(|_| protocol_error("Paste the complete Google redirect URL"))?;
    let expected = Url::parse(&pending.redirect_uri)
        .map_err(|_| protocol_error("The pending Google redirect URI is invalid"))?;
    let states = callback
        .query_pairs()
        .filter(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    let codes = callback
        .query_pairs()
        .filter(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    let errors = callback
        .query_pairs()
        .filter(|(key, _)| key == "error")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    let issuers = callback
        .query_pairs()
        .filter(|(key, _)| key == "iss")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    let one_response = (codes.len() == 1 && !codes[0].is_empty() && errors.is_empty())
        || (errors.len() == 1 && !errors[0].is_empty() && codes.is_empty());

    if callback.scheme() != "http"
        || callback.host_str() != Some("127.0.0.1")
        || callback.origin() != expected.origin()
        || callback.path() != expected.path()
        || !callback.username().is_empty()
        || callback.password().is_some()
        || callback.fragment().is_some()
        || states.as_slice() != [pending.state.as_str()]
        || !one_response
        || issuers.len() > 1
        || issuers
            .first()
            .is_some_and(|issuer| issuer != "https://accounts.google.com")
    {
        return Err(protocol_error(
            "This redirect URL does not belong to the current Google sign-in",
        ));
    }
    Ok(callback)
}

/// Compatibility helper for fixtures that carry only a callback query.
pub fn validate_callback_query(
    query_str: &str,
    expected_state: &str,
) -> Result<String, ProviderError> {
    let callback = if query_str.contains("://") {
        Url::parse(query_str)
    } else {
        Url::parse(&format!(
            "http://127.0.0.1:8085/{}",
            if query_str.starts_with('?') {
                query_str.to_string()
            } else {
                format!("?{query_str}")
            }
        ))
    }
    .map_err(|_| protocol_error("Invalid OAuth callback query"))?;
    let states = callback
        .query_pairs()
        .filter(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    let codes = callback
        .query_pairs()
        .filter(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    if states.as_slice() != [expected_state] || codes.len() != 1 || codes[0].is_empty() {
        return Err(protocol_error("Invalid OAuth callback state or code"));
    }
    Ok(codes[0].clone())
}

async fn forward_callback(callback: &Url) -> Result<(), ProviderError> {
    let port = callback
        .port()
        .ok_or_else(|| protocol_error("Google callback has no loopback port"))?;
    let request_path = match callback.query() {
        Some(query) => format!("{}?{query}", callback.path()),
        None => callback.path().to_string(),
    };
    let operation = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|_| auth_error("Could not deliver the Google sign-in response"))?;
        let request = format!(
            "GET {request_path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await?;
        stream.flush().await?;
        let mut response = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            if response.len().saturating_add(read) > 64 * 1024 {
                return Err(protocol_error("Google callback response exceeded 64 KiB"));
            }
            response.extend_from_slice(&buffer[..read]);
        }
        let status_line = response
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default();
        let status_line = String::from_utf8_lossy(status_line);
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(0);
        if !(200..300).contains(&status) {
            return Err(auth_error(
                "Antigravity rejected the Google sign-in response",
            ));
        }
        Ok(())
    };
    tokio::time::timeout(CALLBACK_TIMEOUT, operation)
        .await
        .map_err(|_| auth_error("Google sign-in response delivery timed out"))?
}

pub async fn execute_oauth_relay_callback(
    callback_url: &str,
    expected_state: &str,
    redirect_port: u16,
) -> Result<String, ProviderError> {
    let pending = ValidatedOAuthUrl {
        raw_url: String::new(),
        client_id: String::new(),
        redirect_uri: format!("http://127.0.0.1:{redirect_port}/"),
        redirect_port,
        state: expected_state.to_string(),
    };
    let callback = validate_callback_url(callback_url, &pending)?;
    forward_callback(&callback).await?;
    Ok(format!(
        "Relayed the Google sign-in response to Antigravity on port {redirect_port}"
    ))
}

fn configure_antigravity_command(
    executable: &Path,
    harness: &Path,
    gemini_home: &Path,
    scratch_dir: &Path,
    browser_command: &str,
    cwd: &Path,
    overrides: Option<&BTreeMap<String, String>>,
) -> tokio::process::Command {
    let environment = isolated_process_environment(
        gemini_home,
        scratch_dir,
        harness,
        browser_command,
        overrides,
    );
    let mut command = create_supervised_command(executable);
    command.env_clear();
    command.envs(environment);
    command.current_dir(cwd);
    #[cfg(target_os = "linux")]
    command.arg("--uid=");
    command
}

enum ProcessMessage {
    Json(Value),
    AuthorizationUrl(ValidatedOAuthUrl),
    Failure(String),
    StdoutClosed,
}

async fn read_process_lines<R: AsyncRead + Unpin>(
    reader: R,
    stdout: bool,
    sender: mpsc::Sender<ProcessMessage>,
) {
    let mut reader = BufReader::new(reader);
    let max = if stdout {
        MAX_PROTOCOL_LINE_BYTES
    } else {
        MAX_BROWSER_LINE_BYTES
    };
    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line).await {
            Ok(0) => {
                if stdout {
                    let _ = sender.send(ProcessMessage::StdoutClosed).await;
                }
                break;
            }
            Ok(_) if line.len() > max => {
                let _ = sender
                    .send(ProcessMessage::Failure(format!(
                        "Antigravity sent a line larger than {max} bytes"
                    )))
                    .await;
                break;
            }
            Ok(_) => {
                let text = match std::str::from_utf8(&line) {
                    Ok(value) => value.trim_end_matches(['\r', '\n']),
                    Err(_) => {
                        if stdout {
                            let _ = sender
                                .send(ProcessMessage::Failure(
                                    "Antigravity sent non-UTF-8 protocol output".to_string(),
                                ))
                                .await;
                            break;
                        }
                        continue;
                    }
                };
                if let Some(url) = extract_auth_url_from_output(text) {
                    match validate_google_oauth_url(&url) {
                        Ok(validated) => {
                            if sender
                                .send(ProcessMessage::AuthorizationUrl(validated))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) if stdout => {
                            let _ = sender
                                .send(ProcessMessage::Failure(error.to_string()))
                                .await;
                            break;
                        }
                        Err(_) => {}
                    }
                    continue;
                }
                if stdout
                    && let Ok(value) = serde_json::from_str::<Value>(text)
                    && sender.send(ProcessMessage::Json(value)).await.is_err()
                {
                    break;
                }
            }
            Err(error) => {
                if stdout {
                    let _ = sender
                        .send(ProcessMessage::Failure(format!(
                            "Could not read Antigravity protocol output: {error}"
                        )))
                        .await;
                }
                break;
            }
        }
    }
}

struct RunningProcess {
    child: SupervisedChild,
    stdin: tokio::process::ChildStdin,
    messages: mpsc::Receiver<ProcessMessage>,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
}

impl RunningProcess {
    async fn stop(mut self) {
        let _ = self.child.kill().await;
        self.stdout_task.abort();
        self.stderr_task.abort();
        let _ = self.stdout_task.await;
        let _ = self.stderr_task.await;
    }
}

fn spawn_process(command: tokio::process::Command) -> Result<RunningProcess, ProviderError> {
    let mut child = SupervisedChild::spawn(command)?;
    let stdin = child
        .take_stdin()
        .ok_or_else(|| protocol_error("Antigravity child stdin was unavailable"))?;
    let stdout = child
        .take_stdout()
        .ok_or_else(|| protocol_error("Antigravity child stdout was unavailable"))?;
    let stderr = child
        .take_stderr()
        .ok_or_else(|| protocol_error("Antigravity child stderr was unavailable"))?;
    let (sender, messages) = mpsc::channel(128);
    let stdout_task = tokio::spawn(read_process_lines(stdout, true, sender.clone()));
    let stderr_task = tokio::spawn(read_process_lines(stderr, false, sender));
    Ok(RunningProcess {
        child,
        stdin,
        messages,
        stdout_task,
        stderr_task,
    })
}

async fn send_request(
    stdin: &mut tokio::process::ChildStdin,
    request: &Value,
) -> Result<(), ProviderError> {
    let mut line = serde_json::to_vec(request)?;
    line.push(b'\n');
    stdin.write_all(&line).await?;
    stdin.flush().await?;
    Ok(())
}

fn response_id(value: &Value) -> Option<u64> {
    value.get("id").and_then(Value::as_u64)
}

fn response_result(value: Value, method: &str) -> Result<Value, ProviderError> {
    if let Some(error) = value.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Antigravity returned an unknown ACP error");
        if code == -32000 {
            return Err(auth_error("Sign in to Antigravity before continuing"));
        }
        if code == -32603 && method == "session/new" {
            return Err(auth_error(
                "Antigravity authenticated but could not initialize a session or load models",
            ));
        }
        return Err(protocol_error(format!(
            "Antigravity {method} failed with ACP error {code}: {message}"
        )));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| protocol_error(format!("Antigravity {method} returned no result")))
}

async fn wait_for_response(
    messages: &mut mpsc::Receiver<ProcessMessage>,
    id: u64,
    method: &str,
    reject_authorization_url: bool,
) -> Result<Value, ProviderError> {
    let wait = async {
        loop {
            match messages.recv().await {
                Some(ProcessMessage::Json(value)) if response_id(&value) == Some(id) => {
                    return response_result(value, method);
                }
                Some(ProcessMessage::Failure(message)) => return Err(protocol_error(message)),
                Some(ProcessMessage::AuthorizationUrl(_)) if reject_authorization_url => {
                    return Err(auth_error(
                        "Sign in to Antigravity in Settings before continuing",
                    ));
                }
                Some(ProcessMessage::StdoutClosed) | None => {
                    return Err(protocol_error(format!(
                        "Antigravity exited before {method} completed"
                    )));
                }
                Some(_) => {}
            }
        }
    };
    tokio::time::timeout(RPC_TIMEOUT, wait)
        .await
        .map_err(|_| ProviderError::Timeout {
            timeout_secs: RPC_TIMEOUT.as_secs(),
        })?
}

async fn wait_for_authorization_url(
    messages: &mut mpsc::Receiver<ProcessMessage>,
    request_id: u64,
) -> Result<ValidatedOAuthUrl, ProviderError> {
    let wait = async {
        loop {
            match messages.recv().await {
                Some(ProcessMessage::AuthorizationUrl(url)) => return Ok(url),
                Some(ProcessMessage::Json(value)) if response_id(&value) == Some(request_id) => {
                    response_result(value, "authenticate")?;
                    return Err(protocol_error(
                        "Antigravity completed authentication without emitting a Google sign-in URL",
                    ));
                }
                Some(ProcessMessage::Failure(message)) => return Err(protocol_error(message)),
                Some(ProcessMessage::StdoutClosed) | None => {
                    return Err(protocol_error(
                        "Antigravity exited before emitting a Google sign-in URL",
                    ));
                }
                Some(_) => {}
            }
        }
    };
    tokio::time::timeout(RPC_TIMEOUT, wait)
        .await
        .map_err(|_| ProviderError::Timeout {
            timeout_secs: RPC_TIMEOUT.as_secs(),
        })?
}

async fn initialize_process(
    process: &mut RunningProcess,
    client_file_system: bool,
) -> Result<AcpInitializeResult, ProviderError> {
    let mut request = build_initialize_request(1);
    request["params"]["clientCapabilities"]["fs"]["readTextFile"] = Value::Bool(client_file_system);
    request["params"]["clientCapabilities"]["fs"]["writeTextFile"] =
        Value::Bool(client_file_system);
    send_request(&mut process.stdin, &request).await?;
    let result = wait_for_response(&mut process.messages, 1, "initialize", true).await?;
    let initialized: AcpInitializeResult = serde_json::from_value(result)?;
    validate_antigravity_initialize(&initialized)?;
    Ok(initialized)
}

/// Launches the extracted runtime once and requires the exact ACP capabilities.
pub async fn validate_antigravity_runtime(
    executable: &Path,
    harness: &Path,
    expected_version: &str,
    state_base_dir: &Path,
) -> Result<(), ProviderError> {
    let validation_root = state_base_dir
        .join("scratch/antigravity")
        .join(format!("validation-{}", uuid::Uuid::new_v4()));
    let gemini_home = validation_root.join("profile");
    let scratch_dir = validation_root.join("runtime");
    private_directory(&validation_root)?;
    prepare_profile(&gemini_home, &scratch_dir)?;
    let helper = std::env::current_exe()?;
    preflight_browser_helper(&helper).await?;
    let browser = browser_helper_command(&helper)?;
    let command = configure_antigravity_command(
        executable,
        harness,
        &gemini_home,
        &scratch_dir,
        &browser,
        &validation_root,
        None,
    );
    let mut process = spawn_process(command)?;
    let validation = async {
        let initialized = initialize_process(&mut process, false).await?;
        if initialized.agent_info.version != expected_version {
            return Err(protocol_error(format!(
                "Antigravity identified as version {}, expected {expected_version}",
                initialized.agent_info.version
            )));
        }
        Ok(())
    }
    .await;
    process.stop().await;
    let _ = tokio::fs::remove_dir_all(&validation_root).await;
    validation
}

fn config_option<'a>(session: &'a Value, id: &str) -> Option<&'a Value> {
    session
        .get("configOptions")
        .and_then(Value::as_array)?
        .iter()
        .find(|option| option.get("id").and_then(Value::as_str) == Some(id))
}

fn collect_option_values(value: &Value, values: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_option_values(item, values);
            }
        }
        Value::Object(object) => {
            if let Some(value) = object.get("value").and_then(Value::as_str) {
                values.push(value.to_string());
            }
            if let Some(options) = object.get("options") {
                collect_option_values(options, values);
            }
        }
        _ => {}
    }
}

fn collect_model_options(value: &Value, models: &mut Vec<ModelInfo>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_model_options(item, models);
            }
        }
        Value::Object(object) => {
            if let Some(id) = object.get("value").and_then(Value::as_str) {
                let display_name = object
                    .get("name")
                    .or_else(|| object.get("label"))
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(id);
                if !models.iter().any(|model| model.id == id) {
                    models.push(ModelInfo {
                        id: id.to_string(),
                        display_name: display_name.to_string(),
                        context_window: None,
                        max_output_tokens: None,
                        supports_reasoning: true,
                    });
                }
            }
            if let Some(options) = object.get("options") {
                collect_model_options(options, models);
            }
        }
        _ => {}
    }
}

async fn set_session_option(
    process: &mut RunningProcess,
    request_id: u64,
    session_id: &str,
    config_id: &str,
    value: &str,
) -> Result<(), ProviderError> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "session/set_config_option",
        "params": {
            "sessionId": session_id,
            "configId": config_id,
            "value": value
        }
    });
    send_request(&mut process.stdin, &request).await?;
    wait_for_response(
        &mut process.messages,
        request_id,
        "session/set_config_option",
        true,
    )
    .await?;
    Ok(())
}

#[derive(Debug)]
enum PendingClientRequest {
    Permission { options: Vec<(String, String)> },
    WriteFile { path: PathBuf, content: String },
}

fn request_id_key(id: &Value) -> Result<String, ProviderError> {
    if id.is_null() || (!id.is_string() && !id.is_number()) {
        return Err(protocol_error(
            "Antigravity sent an invalid JSON-RPC request ID",
        ));
    }
    serde_json::to_string(id).map_err(ProviderError::from)
}

async fn send_shared_json(
    stdin: &Arc<Mutex<tokio::process::ChildStdin>>,
    value: &Value,
) -> Result<(), ProviderError> {
    let mut line = serde_json::to_vec(value)?;
    if line.len() > MAX_PROTOCOL_LINE_BYTES {
        return Err(protocol_error(
            "PandaMUX refused to send an oversized Antigravity protocol line",
        ));
    }
    line.push(b'\n');
    let mut stdin = stdin.lock().await;
    stdin.write_all(&line).await?;
    stdin.flush().await?;
    Ok(())
}

fn rpc_result(id: Value, result: Value) -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() }
    })
}

fn canonical_workspace_path(cwd: &Path, requested: &Path) -> Result<PathBuf, ProviderError> {
    if !requested.is_absolute() {
        return Err(protocol_error(
            "Antigravity requested a workspace file with a relative path",
        ));
    }
    let workspace = std::fs::canonicalize(cwd)?;
    let requested_metadata = std::fs::symlink_metadata(requested)?;
    if requested_metadata.file_type().is_symlink() || !requested_metadata.is_file() {
        return Err(protocol_error(
            "Antigravity requested a workspace path that is not a regular file",
        ));
    }
    let canonical = std::fs::canonicalize(requested)?;
    if !canonical.starts_with(&workspace) {
        return Err(protocol_error(
            "Antigravity requested a file outside the thread workspace",
        ));
    }
    Ok(canonical)
}

fn writable_workspace_path(cwd: &Path, requested: &Path) -> Result<PathBuf, ProviderError> {
    if !requested.is_absolute() {
        return Err(protocol_error(
            "Antigravity requested a workspace write with a relative path",
        ));
    }
    let workspace = std::fs::canonicalize(cwd)?;
    if requested.exists() {
        return canonical_workspace_path(cwd, requested);
    }
    let parent = requested
        .parent()
        .ok_or_else(|| protocol_error("Antigravity requested an invalid workspace path"))?;
    let parent = std::fs::canonicalize(parent)?;
    if !parent.starts_with(&workspace) {
        return Err(protocol_error(
            "Antigravity requested a write outside the thread workspace",
        ));
    }
    let name = requested
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| protocol_error("Antigravity requested an invalid workspace file name"))?;
    Ok(parent.join(name))
}

async fn read_workspace_text(
    cwd: &Path,
    params: &Value,
    session_id: &str,
) -> Result<String, ProviderError> {
    if params.get("sessionId").and_then(Value::as_str) != Some(session_id) {
        return Err(protocol_error(
            "Antigravity requested a file for the wrong ACP session",
        ));
    }
    let requested = params
        .get("path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| protocol_error("Antigravity omitted the file path to read"))?;
    let path = canonical_workspace_path(cwd, &requested)?;
    let metadata = tokio::fs::metadata(&path).await?;
    if metadata.len() > MAX_CLIENT_FILE_BYTES {
        return Err(protocol_error(format!(
            "Antigravity requested a file larger than {MAX_CLIENT_FILE_BYTES} bytes"
        )));
    }
    let content = tokio::fs::read_to_string(path).await?;
    let first_line = params
        .get("line")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(usize::MAX);
    Ok(content
        .split_inclusive('\n')
        .skip(first_line - 1)
        .take(limit)
        .collect())
}

async fn write_workspace_text(cwd: &Path, path: &Path, content: &str) -> Result<(), ProviderError> {
    if content.len() as u64 > MAX_CLIENT_FILE_BYTES {
        return Err(protocol_error(format!(
            "Antigravity requested a write larger than {MAX_CLIENT_FILE_BYTES} bytes"
        )));
    }
    let path = writable_workspace_path(cwd, path)?;
    tokio::fs::write(path, content.as_bytes()).await?;
    Ok(())
}

fn permission_options(params: &Value) -> Vec<(String, String)> {
    params
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|option| {
            Some((
                option.get("optionId")?.as_str()?.to_string(),
                option.get("kind")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

fn selected_permission_result(options: &[(String, String)], allow: bool) -> Value {
    let wanted = if allow { "allow_once" } else { "reject_once" };
    if let Some((option_id, _)) = options.iter().find(|(_, kind)| kind == wanted) {
        serde_json::json!({
            "outcome": { "outcome": "selected", "optionId": option_id }
        })
    } else {
        serde_json::json!({ "outcome": { "outcome": "cancelled" } })
    }
}

async fn emit_terminal_failure(events: &mpsc::Sender<ProviderEvent>, message: impl Into<String>) {
    let _ = events
        .send(ProviderEvent::Error {
            message: message.into(),
            recoverable: false,
        })
        .await;
    let _ = events
        .send(ProviderEvent::TurnCompleted {
            outcome: TurnOutcome::Failed,
            usage: None,
        })
        .await;
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_process_messages(
    mut messages: mpsc::Receiver<ProcessMessage>,
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    pending: Arc<Mutex<HashMap<String, PendingClientRequest>>>,
    events: mpsc::Sender<ProviderEvent>,
    session_id: String,
    cwd: PathBuf,
    access: AccessMode,
    active_prompt_id: Arc<AtomicU64>,
    completed_prompt_id: Arc<AtomicU64>,
    prompt_completed: Arc<Notify>,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
) {
    while let Some(message) = messages.recv().await {
        match message {
            ProcessMessage::Json(value) => {
                let response_id = response_id(&value);
                let active_id = active_prompt_id.load(Ordering::SeqCst);
                if active_id != 0 && response_id == Some(active_id) {
                    completed_prompt_id.store(active_id, Ordering::SeqCst);
                    prompt_completed.notify_waiters();
                    if let Some(error) = value.get("error") {
                        let message = error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("Antigravity prompt failed");
                        emit_terminal_failure(&events, message).await;
                    } else if let Ok(line) = serde_json::to_string(&value) {
                        if let Some(event) = parse_acp_line(&line) {
                            let _ = crate::forward_provider_event(&events, event).await;
                        } else {
                            emit_terminal_failure(
                                &events,
                                "Antigravity prompt returned an invalid completion",
                            )
                            .await;
                        }
                    }
                    continue;
                }

                let Some(method) = value.get("method").and_then(Value::as_str) else {
                    continue;
                };
                let params = value.get("params").cloned().unwrap_or(Value::Null);
                match method {
                    "fs/read_text_file" => {
                        let Some(id) = value.get("id").cloned() else {
                            continue;
                        };
                        let response = match read_workspace_text(&cwd, &params, &session_id).await {
                            Ok(content) => {
                                rpc_result(id, serde_json::json!({ "content": content }))
                            }
                            Err(error) => rpc_error(id, -32001, error.to_string()),
                        };
                        if let Err(error) = send_shared_json(&stdin, &response).await {
                            emit_terminal_failure(&events, error.to_string()).await;
                            break;
                        }
                    }
                    "fs/write_text_file" => {
                        let Some(id) = value.get("id").cloned() else {
                            continue;
                        };
                        if params.get("sessionId").and_then(Value::as_str) != Some(&session_id) {
                            let _ = send_shared_json(
                                &stdin,
                                &rpc_error(id, -32001, "ACP session mismatch"),
                            )
                            .await;
                            continue;
                        }
                        let Some(path) = params
                            .get("path")
                            .and_then(Value::as_str)
                            .map(PathBuf::from)
                        else {
                            let _ = send_shared_json(
                                &stdin,
                                &rpc_error(id, -32602, "Missing file path"),
                            )
                            .await;
                            continue;
                        };
                        let Some(content) = params.get("content").and_then(Value::as_str) else {
                            let _ = send_shared_json(
                                &stdin,
                                &rpc_error(id, -32602, "Missing file content"),
                            )
                            .await;
                            continue;
                        };
                        match access {
                            AccessMode::ReadOnly => {
                                let _ = send_shared_json(
                                    &stdin,
                                    &rpc_error(id, -32001, "Read-only access denied this write"),
                                )
                                .await;
                            }
                            AccessMode::AutoEdit | AccessMode::FullAccess => {
                                let response =
                                    match write_workspace_text(&cwd, &path, content).await {
                                        Ok(()) => rpc_result(id, serde_json::json!({})),
                                        Err(error) => rpc_error(id, -32001, error.to_string()),
                                    };
                                let _ = send_shared_json(&stdin, &response).await;
                            }
                            AccessMode::Ask | AccessMode::WorkspaceOnly => {
                                let key = match request_id_key(&id) {
                                    Ok(key) => key,
                                    Err(error) => {
                                        let _ = send_shared_json(
                                            &stdin,
                                            &rpc_error(id, -32600, error.to_string()),
                                        )
                                        .await;
                                        continue;
                                    }
                                };
                                pending.lock().await.insert(
                                    key.clone(),
                                    PendingClientRequest::WriteFile {
                                        path: path.clone(),
                                        content: content.to_string(),
                                    },
                                );
                                let detail = serde_json::json!({
                                    "path": path,
                                    "size": content.len(),
                                    "sha256": hex::encode(sha2::Sha256::digest(content.as_bytes()))
                                });
                                let _ = crate::forward_provider_event(
                                    &events,
                                    ProviderEvent::ApprovalRequested {
                                        request_id: key,
                                        kind: ApprovalKind::FileEdit,
                                        detail,
                                        command: Some(path.to_string_lossy().to_string()),
                                    },
                                )
                                .await;
                            }
                        }
                    }
                    "session/request_permission" | "request_permission" => {
                        let Some(id) = value.get("id").cloned() else {
                            continue;
                        };
                        let options = permission_options(&params);
                        if access == AccessMode::ReadOnly {
                            let result = selected_permission_result(&options, false);
                            let _ = send_shared_json(&stdin, &rpc_result(id, result)).await;
                            continue;
                        }
                        let key = match request_id_key(&id) {
                            Ok(key) => key,
                            Err(error) => {
                                let _ = send_shared_json(
                                    &stdin,
                                    &rpc_error(id, -32600, error.to_string()),
                                )
                                .await;
                                continue;
                            }
                        };
                        pending
                            .lock()
                            .await
                            .insert(key, PendingClientRequest::Permission { options });
                        if let Ok(line) = serde_json::to_string(&value)
                            && let Some(event) = parse_acp_line(&line)
                        {
                            let _ = crate::forward_provider_event(&events, event).await;
                        }
                    }
                    "session/update"
                    | "session/promptUpdate"
                    | "session/toolCall"
                    | "session/completed" => {
                        if let Ok(line) = serde_json::to_string(&value)
                            && let Some(event) = parse_acp_line(&line)
                        {
                            let _ = crate::forward_provider_event(&events, event).await;
                        }
                    }
                    _ => {
                        if let Some(id) = value.get("id").cloned() {
                            let _ = send_shared_json(
                                &stdin,
                                &rpc_error(id, -32601, format!("Unsupported ACP method {method}")),
                            )
                            .await;
                        }
                    }
                }
            }
            ProcessMessage::AuthorizationUrl(_) => {
                emit_terminal_failure(
                    &events,
                    "Antigravity requested Google sign-in during an active thread",
                )
                .await;
                break;
            }
            ProcessMessage::Failure(message) => {
                emit_terminal_failure(&events, message).await;
                break;
            }
            ProcessMessage::StdoutClosed => {
                emit_terminal_failure(&events, "Antigravity process closed its protocol stream")
                    .await;
                break;
            }
        }
    }
    stdout_task.abort();
    stderr_task.abort();
    let _ = stdout_task.await;
    let _ = stderr_task.await;
}

pub struct AntigravitySession {
    child: SupervisedChild,
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    events_rx: Option<mpsc::Receiver<ProviderEvent>>,
    dispatcher_task: JoinHandle<()>,
    pending: Arc<Mutex<HashMap<String, PendingClientRequest>>>,
    next_id: AtomicU64,
    active_prompt_id: Arc<AtomicU64>,
    completed_prompt_id: Arc<AtomicU64>,
    prompt_completed: Arc<Notify>,
    session_id: String,
    cwd: PathBuf,
    access: AccessMode,
    preamble: Option<String>,
    _process_guard: ConcurrencyGuard,
}

impl AntigravitySession {
    pub fn with_preamble(mut self, preamble: Option<String>) -> Self {
        self.preamble = preamble;
        self
    }

    async fn cancel_pending(&self) {
        let pending = self.pending.lock().await.drain().collect::<Vec<_>>();
        for (key, request) in pending {
            let id = serde_json::from_str::<Value>(&key).unwrap_or(Value::String(key));
            let response = match request {
                PendingClientRequest::Permission { .. } => rpc_result(
                    id,
                    serde_json::json!({ "outcome": { "outcome": "cancelled" } }),
                ),
                PendingClientRequest::WriteFile { .. } => {
                    rpc_error(id, -32800, "The Antigravity turn was cancelled")
                }
            };
            let _ = send_shared_json(&self.stdin, &response).await;
        }
    }
}

impl ProviderSession for AntigravitySession {
    fn send_turn<'a>(&'a mut self, input: TurnInput) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let id = self.next_id.fetch_add(1, Ordering::SeqCst);
            self.active_prompt_id.store(id, Ordering::SeqCst);
            let mut prompt_text = String::new();
            if let Some(preamble) = self.preamble.take() {
                prompt_text.push_str(&preamble);
                prompt_text.push_str("\n\n");
            }
            prompt_text.push_str(&input.text);
            if !input.attachments.is_empty() {
                prompt_text.push_str(&pandamux_core::format_attachments_summary(
                    &input.attachments,
                ));
            }
            let mut prompt = vec![serde_json::json!({
                "type": "text",
                "text": prompt_text
            })];
            for attachment in &input.attachments {
                if !matches!(
                    attachment.mime_type.as_str(),
                    "image/bmp" | "image/jpeg" | "image/png" | "image/webp"
                ) {
                    continue;
                }
                let path = Path::new(&attachment.file_path);
                let metadata = tokio::fs::symlink_metadata(path).await?;
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || metadata.len() != attachment.size_bytes
                {
                    return Err(protocol_error(format!(
                        "Attachment {} changed before Antigravity could read it",
                        attachment.file_name
                    )));
                }
                let bytes = tokio::fs::read(path).await?;
                prompt.push(serde_json::json!({
                    "type": "image",
                    "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                    "mimeType": attachment.mime_type
                }));
            }
            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "session/prompt",
                "params": { "sessionId": self.session_id, "prompt": prompt }
            });
            send_shared_json(&self.stdin, &request).await
        })
    }

    fn events(&mut self) -> &mut mpsc::Receiver<ProviderEvent> {
        self.events_rx
            .as_mut()
            .expect("events receiver already taken")
    }

    fn take_event_receiver(&mut self) -> Option<mpsc::Receiver<ProviderEvent>> {
        self.events_rx.take()
    }

    fn respond_approval<'a>(
        &'a mut self,
        request_id: String,
        decision: ApprovalDecision,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let request = self
                .pending
                .lock()
                .await
                .remove(&request_id)
                .ok_or_else(|| protocol_error("Antigravity approval is no longer pending"))?;
            let id = serde_json::from_str::<Value>(&request_id)
                .unwrap_or(Value::String(request_id.clone()));
            let approved = matches!(decision, ApprovalDecision::Approved)
                && self.access != AccessMode::ReadOnly;
            let response = match request {
                PendingClientRequest::Permission { options } => {
                    rpc_result(id, selected_permission_result(&options, approved))
                }
                PendingClientRequest::WriteFile { path, content } if approved => {
                    match write_workspace_text(&self.cwd, &path, &content).await {
                        Ok(()) => rpc_result(id, serde_json::json!({})),
                        Err(error) => rpc_error(id, -32001, error.to_string()),
                    }
                }
                PendingClientRequest::WriteFile { .. } => {
                    rpc_error(id, -32001, "The workspace write was denied")
                }
            };
            send_shared_json(&self.stdin, &response).await
        })
    }

    fn interrupt<'a>(&'a mut self) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let prompt_id = self.active_prompt_id.load(Ordering::SeqCst);
            self.cancel_pending().await;
            if prompt_id == 0 {
                return Ok(());
            }
            send_shared_json(
                &self.stdin,
                &serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "session/cancel",
                    "params": { "sessionId": self.session_id }
                }),
            )
            .await?;
            let wait = async {
                loop {
                    if self.completed_prompt_id.load(Ordering::SeqCst) >= prompt_id {
                        return;
                    }
                    self.prompt_completed.notified().await;
                }
            };
            if tokio::time::timeout(CANCEL_TIMEOUT, wait).await.is_err() {
                self.child.kill().await?;
            }
            Ok(())
        })
    }

    fn resume_token(&self) -> Option<String> {
        Some(self.session_id.clone())
    }

    fn shutdown(mut self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            self.cancel_pending().await;
            let _ = self.child.kill().await;
            self.dispatcher_task.abort();
            let _ = self.dispatcher_task.await;
        })
    }
}

/// Starts a fully initialized and authenticated Antigravity ACP session.
pub async fn start_antigravity_session(
    config: &ProviderInstanceConfig,
    state_base_dir: &Path,
    spec: &SessionSpec,
) -> Result<AntigravitySession, ProviderError> {
    let process_guard = shared_antigravity_process_limiter().try_acquire()?;
    let manifest = current_bundle_manifest().ok_or_else(|| {
        protocol_error("Antigravity is not supported on this platform and architecture")
    })?;
    let explicit = config.binary_path.as_deref().map(Path::new).or_else(|| {
        config
            .settings
            .get("binaryPath")
            .and_then(Value::as_str)
            .map(Path::new)
    });
    let resolved =
        resolve_antigravity_executable(explicit, state_base_dir, &platform_arch(&manifest))
            .ok_or_else(|| protocol_error("Install Antigravity before starting a thread"))?;
    let profile_dir = PathBuf::from(&config.profile_dir);
    let instance_hash = hex::encode(sha2::Sha256::digest(config.id.as_str().as_bytes()));
    let scratch_dir = state_base_dir
        .join("scratch/antigravity")
        .join(&instance_hash[..12]);
    prepare_profile(&profile_dir, &scratch_dir)?;
    preflight_disk_space(&scratch_dir, MIN_FREE_DISK_BYTES)?;
    let helper = std::env::current_exe()?;
    preflight_browser_helper(&helper).await?;
    let browser = browser_helper_command(&helper)?;
    let command = configure_antigravity_command(
        &resolved.executable_path,
        &resolved.harness_path,
        &profile_dir,
        &scratch_dir,
        &browser,
        &spec.cwd,
        Some(&config.env_overrides),
    );
    let mut process = spawn_process(command)?;

    let startup = async {
        initialize_process(&mut process, true).await?;
        let authenticate = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "authenticate",
            "params": { "methodId": "oauth-personal" }
        });
        send_request(&mut process.stdin, &authenticate).await?;
        wait_for_response(&mut process.messages, 2, "authenticate", true).await?;

        let (session_id, session) = if let Some(resume) = spec
            .resume
            .as_deref()
            .filter(|resume| !resume.trim().is_empty())
        {
            let resume_session = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "session/resume",
                "params": {
                    "sessionId": resume,
                    "cwd": spec.cwd.to_string_lossy(),
                    "mcpServers": []
                }
            });
            send_request(&mut process.stdin, &resume_session).await?;
            let session =
                wait_for_response(&mut process.messages, 3, "session/resume", true).await?;
            (resume.to_string(), session)
        } else {
            let create_session = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "session/new",
                "params": {
                    "cwd": spec.cwd.to_string_lossy(),
                    "mcpServers": []
                }
            });
            send_request(&mut process.stdin, &create_session).await?;
            let session = wait_for_response(&mut process.messages, 3, "session/new", true).await?;
            let session_id = session
                .get("sessionId")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| protocol_error("Antigravity session/new returned no session ID"))?
                .to_string();
            (session_id, session)
        };

        let permission_mode = map_access_mode_to_acp(spec.access).as_str();
        if config_option(&session, "mode").is_some() {
            set_session_option(&mut process, 4, &session_id, "mode", permission_mode).await?;
        } else if !matches!(
            spec.access,
            AccessMode::Ask | AccessMode::ReadOnly | AccessMode::WorkspaceOnly
        ) {
            return Err(protocol_error(
                "Antigravity did not expose the mode control required for this access ceiling",
            ));
        }

        if !spec.model.trim().is_empty() && spec.model != "default" {
            let model_option = config_option(&session, "model")
                .ok_or_else(|| protocol_error("Antigravity did not expose model configuration"))?;
            let mut allowed = Vec::new();
            collect_option_values(
                model_option.get("options").unwrap_or(&Value::Null),
                &mut allowed,
            );
            if !allowed.iter().any(|value| value == &spec.model) {
                return Err(protocol_error(format!(
                    "Antigravity model '{}' is unavailable for this account",
                    spec.model
                )));
            }
            set_session_option(&mut process, 5, &session_id, "model", &spec.model).await?;
        }
        Ok(session_id)
    }
    .await;

    let session_id = match startup {
        Ok(session_id) => session_id,
        Err(error) => {
            process.stop().await;
            return Err(error);
        }
    };

    let RunningProcess {
        child,
        stdin,
        messages,
        stdout_task,
        stderr_task,
    } = process;
    let (events_tx, events_rx) = mpsc::channel(128);
    let stdin = Arc::new(Mutex::new(stdin));
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let active_prompt_id = Arc::new(AtomicU64::new(0));
    let completed_prompt_id = Arc::new(AtomicU64::new(0));
    let prompt_completed = Arc::new(Notify::new());
    let dispatcher_task = tokio::spawn(dispatch_process_messages(
        messages,
        Arc::clone(&stdin),
        Arc::clone(&pending),
        events_tx,
        session_id.clone(),
        spec.cwd.clone(),
        spec.access,
        Arc::clone(&active_prompt_id),
        Arc::clone(&completed_prompt_id),
        Arc::clone(&prompt_completed),
        stdout_task,
        stderr_task,
    ));
    Ok(AntigravitySession {
        child,
        stdin,
        events_rx: Some(events_rx),
        dispatcher_task,
        pending,
        next_id: AtomicU64::new(10),
        active_prompt_id,
        completed_prompt_id,
        prompt_completed,
        session_id,
        cwd: spec.cwd.clone(),
        access: spec.access,
        preamble: None,
        _process_guard: process_guard,
    })
}

/// Refreshes account-specific Antigravity models through a disposable ACP session.
pub async fn list_antigravity_models(
    config: &ProviderInstanceConfig,
    state_base_dir: &Path,
) -> Result<Vec<ModelInfo>, ProviderError> {
    let _process_guard = shared_antigravity_process_limiter().try_acquire()?;
    let manifest = current_bundle_manifest().ok_or_else(|| {
        protocol_error("Antigravity is not supported on this platform and architecture")
    })?;
    let explicit = config.binary_path.as_deref().map(Path::new).or_else(|| {
        config
            .settings
            .get("binaryPath")
            .and_then(Value::as_str)
            .map(Path::new)
    });
    let resolved =
        resolve_antigravity_executable(explicit, state_base_dir, &platform_arch(&manifest))
            .ok_or_else(|| protocol_error("Install Antigravity before refreshing models"))?;
    let profile_dir = PathBuf::from(&config.profile_dir);
    let instance_hash = hex::encode(sha2::Sha256::digest(config.id.as_str().as_bytes()));
    let scratch_dir = state_base_dir
        .join("scratch/antigravity")
        .join(&instance_hash[..12]);
    prepare_profile(&profile_dir, &scratch_dir)?;
    preflight_disk_space(&scratch_dir, MIN_FREE_DISK_BYTES)?;
    let helper = std::env::current_exe()?;
    preflight_browser_helper(&helper).await?;
    let browser = browser_helper_command(&helper)?;
    let command = configure_antigravity_command(
        &resolved.executable_path,
        &resolved.harness_path,
        &profile_dir,
        &scratch_dir,
        &browser,
        &scratch_dir,
        Some(&config.env_overrides),
    );
    let mut process = spawn_process(command)?;
    let result = async {
        initialize_process(&mut process, false).await?;
        let authenticate = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "authenticate",
            "params": { "methodId": "oauth-personal" }
        });
        send_request(&mut process.stdin, &authenticate).await?;
        wait_for_response(&mut process.messages, 2, "authenticate", true).await?;
        let create_session = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "session/new",
            "params": {
                "cwd": scratch_dir.to_string_lossy(),
                "mcpServers": []
            }
        });
        send_request(&mut process.stdin, &create_session).await?;
        let session = wait_for_response(&mut process.messages, 3, "session/new", true).await?;
        let model_option = config_option(&session, "model")
            .ok_or_else(|| protocol_error("Antigravity returned no model configuration"))?;
        let mut models = Vec::new();
        collect_model_options(
            model_option.get("options").unwrap_or(&Value::Null),
            &mut models,
        );
        if models.is_empty() {
            return Err(protocol_error(
                "Antigravity returned no models for this Google account",
            ));
        }
        Ok(models)
    }
    .await;
    process.stop().await;
    result
}

struct PendingAuthFlow {
    provider_instance_id: ProviderInstanceId,
    flow_id: String,
    authorization: ValidatedOAuthUrl,
    process: RunningProcess,
    profile_dir: PathBuf,
    cwd: PathBuf,
    expires_at: Instant,
    _process_guard: ConcurrencyGuard,
}

#[derive(Clone)]
pub struct AntigravityAuthManager {
    flows: Arc<Mutex<HashMap<String, PendingAuthFlow>>>,
    limiter: Arc<crate::antigravity::AntigravityConcurrencyLimiter>,
}

impl Default for AntigravityAuthManager {
    fn default() -> Self {
        Self {
            flows: Arc::new(Mutex::new(HashMap::new())),
            limiter: shared_antigravity_process_limiter(),
        }
    }
}

impl AntigravityAuthManager {
    pub fn new() -> Self {
        Self::default()
    }

    async fn stop_instance_flows(&self, instance_id: &ProviderInstanceId) {
        let flows = {
            let mut active = self.flows.lock().await;
            let ids = active
                .iter()
                .filter(|(_, flow)| &flow.provider_instance_id == instance_id)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| active.remove(&id))
                .collect::<Vec<_>>()
        };
        for flow in flows {
            flow.process.stop().await;
        }
    }

    pub async fn start(
        &self,
        config: &ProviderInstanceConfig,
        state_base_dir: &Path,
    ) -> Result<AntigravityAuthStart, ProviderError> {
        self.stop_instance_flows(&config.id).await;
        let process_guard = self.limiter.try_acquire()?;
        let manifest = current_bundle_manifest().ok_or_else(|| {
            protocol_error("Antigravity is not supported on this platform and architecture")
        })?;
        let explicit = config.binary_path.as_deref().map(Path::new).or_else(|| {
            config
                .settings
                .get("binaryPath")
                .and_then(Value::as_str)
                .map(Path::new)
        });
        let resolved =
            resolve_antigravity_executable(explicit, state_base_dir, &platform_arch(&manifest))
                .ok_or_else(|| {
                    protocol_error("Install Antigravity before starting Google sign-in")
                })?;
        let profile_dir = PathBuf::from(&config.profile_dir);
        let instance_hash = hex::encode(sha2::Sha256::digest(config.id.as_str().as_bytes()));
        let scratch_dir = state_base_dir
            .join("scratch/antigravity")
            .join(&instance_hash[..12]);
        prepare_profile(&profile_dir, &scratch_dir)?;
        let helper = std::env::current_exe()?;
        preflight_browser_helper(&helper).await?;
        let browser = browser_helper_command(&helper)?;
        let command = configure_antigravity_command(
            &resolved.executable_path,
            &resolved.harness_path,
            &profile_dir,
            &scratch_dir,
            &browser,
            &scratch_dir,
            Some(&config.env_overrides),
        );
        let mut process = spawn_process(command)?;
        initialize_process(&mut process, false).await?;
        let authenticate = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "authenticate",
            "params": { "methodId": "oauth-personal" }
        });
        send_request(&mut process.stdin, &authenticate).await?;
        let authorization = wait_for_authorization_url(&mut process.messages, 2).await?;
        let flow_id = uuid::Uuid::new_v4().to_string();
        let result = AntigravityAuthStart {
            provider_instance_id: config.id.clone(),
            flow_id: flow_id.clone(),
            auth_url: authorization.raw_url.clone(),
            redirect_port: authorization.redirect_port,
            state: authorization.state.clone(),
        };
        self.flows.lock().await.insert(
            flow_id.clone(),
            PendingAuthFlow {
                provider_instance_id: config.id.clone(),
                flow_id: flow_id.clone(),
                authorization,
                process,
                profile_dir,
                cwd: scratch_dir,
                expires_at: Instant::now() + AUTH_FLOW_TIMEOUT,
                _process_guard: process_guard,
            },
        );

        let manager = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(AUTH_FLOW_TIMEOUT).await;
            let expired = manager.flows.lock().await.remove(&flow_id);
            if let Some(flow) = expired {
                flow.process.stop().await;
            }
        });
        Ok(result)
    }

    pub async fn complete(
        &self,
        instance_id: &ProviderInstanceId,
        flow_id: &str,
        callback_url: &str,
    ) -> Result<String, ProviderError> {
        let mut flow = self
            .flows
            .lock()
            .await
            .remove(flow_id)
            .ok_or_else(|| auth_error("This Google sign-in is no longer active"))?;
        let completion = async {
            if &flow.provider_instance_id != instance_id || flow.flow_id != flow_id {
                return Err(auth_error(
                    "This Google sign-in belongs to a different provider instance",
                ));
            }
            if Instant::now() >= flow.expires_at {
                return Err(auth_error("Google sign-in expired"));
            }
            let callback = validate_callback_url(callback_url, &flow.authorization)?;
            forward_callback(&callback).await?;
            wait_for_response(&mut flow.process.messages, 2, "authenticate", false).await?;

            let create_session = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "session/new",
                "params": {
                    "cwd": flow.cwd.to_string_lossy(),
                    "mcpServers": []
                }
            });
            send_request(&mut flow.process.stdin, &create_session).await?;
            let session =
                wait_for_response(&mut flow.process.messages, 3, "session/new", true).await?;
            let session_id = session
                .get("sessionId")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| protocol_error("Antigravity session/new returned no session ID"))?;
            let model_options = session
                .get("configOptions")
                .and_then(Value::as_array)
                .and_then(|options| {
                    options.iter().find(|option| {
                        option.get("id") == Some(&Value::String("model".to_string()))
                    })
                });
            if model_options.is_none() {
                return Err(auth_error(
                    "Antigravity authenticated but did not return model options",
                ));
            }
            let token_path = flow.profile_dir.join("antigravity-acp/acp_token.json");
            if !token_path.is_file() {
                return Err(auth_error(
                    "Antigravity authenticated but did not persist its private credential",
                ));
            }
            Ok(format!(
                "Google sign-in completed and Antigravity session {session_id} initialized"
            ))
        }
        .await;
        flow.process.stop().await;
        completion
    }

    pub async fn shutdown(&self) {
        let flows = self
            .flows
            .lock()
            .await
            .drain()
            .map(|(_, flow)| flow)
            .collect::<Vec<_>>();
        for flow in flows {
            flow.process.stop().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_authorization_url() -> String {
        "https://accounts.google.com/o/oauth2/v2/auth?client_id=123.apps.googleusercontent.com&redirect_uri=http%3A%2F%2F127.0.0.1%3A8085%2F&response_type=code&state=secure-state".to_string()
    }

    #[test]
    fn authorization_url_requires_exact_origin_path_and_single_parameters() {
        let parsed = validate_google_oauth_url(&valid_authorization_url()).expect("valid URL");
        assert_eq!(parsed.redirect_port, 8085);
        assert_eq!(parsed.state, "secure-state");

        for invalid in [
            valid_authorization_url()
                .replace("accounts.google.com", "accounts.google.com.evil.test"),
            valid_authorization_url().replace("/o/oauth2/v2/auth", "/other"),
            format!("{}&state=second", valid_authorization_url()),
            valid_authorization_url().replace("response_type=code", "response_type=token"),
            valid_authorization_url().replace("127.0.0.1%3A8085", "localhost%3A8085"),
            valid_authorization_url().replace("8085", "80"),
        ] {
            assert!(
                validate_google_oauth_url(&invalid).is_err(),
                "accepted {invalid}"
            );
        }
    }

    #[test]
    fn callback_must_match_the_exact_pending_loopback_request() {
        let pending = validate_google_oauth_url(&valid_authorization_url()).unwrap();
        let valid = "http://127.0.0.1:8085/?code=abc&state=secure-state&iss=https%3A%2F%2Faccounts.google.com";
        validate_callback_url(valid, &pending).expect("valid callback");
        for invalid in [
            "http://127.0.0.1:8086/?code=abc&state=secure-state",
            "http://127.0.0.1:8085/other?code=abc&state=secure-state",
            "http://127.0.0.1:8085/?code=abc&state=wrong",
            "http://127.0.0.1:8085/?code=abc&code=def&state=secure-state",
            "http://127.0.0.1:8085/?code=abc&error=denied&state=secure-state",
            "http://127.0.0.1:8085/?code=abc&state=secure-state#fragment",
        ] {
            assert!(
                validate_callback_url(invalid, &pending).is_err(),
                "accepted {invalid}"
            );
        }
    }

    #[test]
    fn environment_removes_ambient_google_credentials() {
        let env = build_antigravity_env(
            "instance",
            Path::new("/tmp/pandamux"),
            Path::new("/tmp/pandamux/harness"),
            "helper %s",
        );
        assert!(validate_sanitized_environment(&env.vars));
        assert_eq!(
            env.vars
                .get("AGY_ACP_FORCE_FILE_STORAGE")
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            env.vars.get("ELECTRON_RUN_AS_NODE").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn permission_responses_use_the_opaque_option_id_offered_by_the_agent() {
        let options = vec![
            ("remember-command".to_string(), "allow_always".to_string()),
            ("run-once-opaque".to_string(), "allow_once".to_string()),
            ("deny-opaque".to_string(), "reject_once".to_string()),
        ];
        assert_eq!(
            selected_permission_result(&options, true),
            serde_json::json!({
                "outcome": { "outcome": "selected", "optionId": "run-once-opaque" }
            })
        );
        assert_eq!(
            selected_permission_result(&options, false),
            serde_json::json!({
                "outcome": { "outcome": "selected", "optionId": "deny-opaque" }
            })
        );
    }

    #[tokio::test]
    async fn workspace_file_handlers_confine_reads_and_writes() {
        let root = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let source = root.path().join("source.txt");
        std::fs::write(&source, "one\ntwo\nthree\n").expect("source fixture");
        let read = read_workspace_text(
            root.path(),
            &serde_json::json!({
                "sessionId": "session-1",
                "path": source,
                "line": 2,
                "limit": 1
            }),
            "session-1",
        )
        .await
        .expect("confined read");
        assert_eq!(read, "two\n");

        let target = root.path().join("target.txt");
        write_workspace_text(root.path(), &target, "written")
            .await
            .expect("confined write");
        assert_eq!(std::fs::read_to_string(target).unwrap(), "written");

        let escaped = outside.path().join("escaped.txt");
        std::fs::write(&escaped, "outside").expect("outside fixture");
        assert!(
            read_workspace_text(
                root.path(),
                &serde_json::json!({
                    "sessionId": "session-1",
                    "path": escaped
                }),
                "session-1",
            )
            .await
            .is_err()
        );
        assert!(
            write_workspace_text(root.path(), &outside.path().join("new.txt"), "outside")
                .await
                .is_err()
        );
    }
}
