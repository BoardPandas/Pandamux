use std::path::PathBuf;

/// The provider boundary at which a supervised dispatch was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderDispatchStage {
    StartSession,
    SendTurn,
}

/// Structured error type for provider drivers and sessions.
#[derive(Debug)]
pub enum ProviderError {
    /// A typed negative receipt from a provider adapter. Only the exact
    /// `prompt_accepted = false` plus `cleanup_verified = true` combination is
    /// safe for an owner to classify as not started. Every other combination
    /// remains externally ambiguous.
    DispatchRejected {
        stage: ProviderDispatchStage,
        code: String,
        message: String,
        prompt_accepted: bool,
        cleanup_verified: bool,
    },
    ProcessFailed {
        program: String,
        message: String,
    },
    AuthRequired {
        provider: String,
        message: String,
    },
    ProtocolError {
        provider: String,
        message: String,
    },
    SessionError {
        thread_id: String,
        message: String,
    },
    Timeout {
        timeout_secs: u64,
    },
    RateLimitExceeded {
        provider: String,
        details: String,
    },
    InstallationError {
        operation: String,
        code: String,
        message: String,
    },
    SupervisionError {
        message: String,
    },
    ProfileError {
        path: PathBuf,
        message: String,
    },
    Io(std::io::Error),
    Json(serde_json::Error),
    Other(String),
}

impl From<std::io::Error> for ProviderError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for ProviderError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DispatchRejected {
                stage,
                code,
                message,
                prompt_accepted,
                cleanup_verified,
            } => write!(
                f,
                "Provider dispatch was rejected at {stage:?} ({code}): {message} (prompt accepted: {prompt_accepted}, cleanup verified: {cleanup_verified})"
            ),
            Self::ProcessFailed { program, message } => {
                write!(f, "Process execution failed for '{program}': {message}")
            }
            Self::AuthRequired { provider, message } => {
                write!(
                    f,
                    "Authentication required for provider '{provider}': {message}"
                )
            }
            Self::ProtocolError { provider, message } => {
                write!(
                    f,
                    "Protocol error parsing output from '{provider}': {message}"
                )
            }
            Self::SessionError { thread_id, message } => {
                write!(f, "Session error in thread '{thread_id}': {message}")
            }
            Self::Timeout { timeout_secs } => {
                write!(f, "Operation timed out after {timeout_secs}s")
            }
            Self::RateLimitExceeded { provider, details } => {
                write!(
                    f,
                    "Rate limit exceeded for provider '{provider}': {details}"
                )
            }
            Self::InstallationError {
                operation,
                code,
                message,
            } => {
                write!(
                    f,
                    "Installation error during {operation} ({code}): {message}"
                )
            }
            Self::SupervisionError { message } => {
                write!(f, "Supervision error: {message}")
            }
            Self::ProfileError { path, message } => {
                write!(f, "Profile error for path '{}': {message}", path.display())
            }
            Self::Io(err) => write!(f, "IO error: {err}"),
            Self::Json(err) => write!(f, "Serialization error: {err}"),
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl ProviderError {
    /// Positive adapter evidence that neither a prompt nor an unjoined child
    /// can have produced an external side effect.
    pub fn verified_not_started(&self) -> bool {
        matches!(
            self,
            Self::DispatchRejected {
                prompt_accepted: false,
                cleanup_verified: true,
                ..
            }
        )
    }

    pub fn safe_dispatch_code(&self) -> Option<&str> {
        match self {
            Self::DispatchRejected { code, .. } => Some(code),
            _ => None,
        }
    }
}

impl std::error::Error for ProviderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Json(err) => Some(err),
            _ => None,
        }
    }
}
