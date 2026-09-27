pub mod acp;
pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod error;
pub mod mock;
pub mod models;
pub mod profiles;
pub mod shim_resolver;
pub mod supervision;
pub mod traits;

pub use acp::{
    AcpInitializeResult, AcpMode, AcpPermissionRequest, AcpSession, build_initialize_request,
    build_session_new_request, map_access_mode_to_acp, validate_antigravity_initialize,
};
pub use antigravity::{
    AntigravityConcurrencyLimiter, AntigravityDriver, ManagedBundleManifest, build_antigravity_env,
    extract_auth_url_from_output, probe_antigravity_health_offline, resolve_antigravity_binary,
    sweep_orphan_temp_dirs, validate_callback_query, validate_google_oauth_url,
    validate_sanitized_environment, verify_bundle_manifest,
};
pub use claude::{ClaudeDriver, ClaudeSession, build_claude_launch_args};
pub use codex::{CodexDriver, CodexSession};
pub use error::ProviderError;
pub use mock::{MockProviderDriver, MockProviderSession};
pub use models::{
    InjectedInstructions, ModelInfo, ProviderAuthStatus, ProviderEvent, ProviderHealth,
    ProviderMetadata, ProviderSnapshot, RateLimitEntry, SessionSpec, ToolPolicy, UsageLimits,
};
pub use profiles::{
    ENV_CLAUDE_CONFIG_DIR, ENV_CODEX_HOME, ENV_GEMINI_HOME, apply_profile_environment,
    ensure_profile_dir, resolve_profile_dir,
};
pub use shim_resolver::{ResolvedCommand, ShimType, resolve_command_shim};
pub use supervision::{CREATE_NO_WINDOW, SupervisedChild, create_supervised_command};
pub use traits::{BoxFuture, ProviderDriver, ProviderSession};
