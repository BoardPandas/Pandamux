pub mod acp;
pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod error;
pub mod models;
pub mod profiles;
pub mod shim_resolver;
pub mod supervision;
pub mod traits;

pub use acp::{
    build_initialize_request, build_session_new_request, map_access_mode_to_acp,
    validate_antigravity_initialize, AcpInitializeResult, AcpMode, AcpPermissionRequest,
    AcpSession,
};
pub use antigravity::{
    build_antigravity_env, extract_auth_url_from_output, probe_antigravity_health_offline,
    resolve_antigravity_binary, sweep_orphan_temp_dirs, validate_callback_query,
    validate_google_oauth_url, validate_sanitized_environment, verify_bundle_manifest,
    AntigravityConcurrencyLimiter, AntigravityDriver, ManagedBundleManifest,
};
pub use claude::{build_claude_launch_args, ClaudeDriver, ClaudeSession};
pub use codex::{CodexDriver, CodexSession};
pub use error::ProviderError;
pub use models::{
    InjectedInstructions, ModelInfo, ProviderAuthStatus, ProviderEvent, ProviderHealth,
    ProviderMetadata, ProviderSnapshot, RateLimitEntry, SessionSpec, ToolPolicy, UsageLimits,
};
pub use profiles::{
    apply_profile_environment, ensure_profile_dir, resolve_profile_dir, ENV_CLAUDE_CONFIG_DIR,
    ENV_CODEX_HOME, ENV_GEMINI_HOME,
};
pub use shim_resolver::{resolve_command_shim, ResolvedCommand, ShimType};
pub use supervision::{create_supervised_command, SupervisedChild, CREATE_NO_WINDOW};
pub use traits::{BoxFuture, ProviderDriver, ProviderSession};
