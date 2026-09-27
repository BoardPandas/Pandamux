use std::future::Future;
use std::pin::Pin;
use pandamux_core::{
    event::ApprovalDecision,
    provider_config::ProviderInstanceConfig,
    thread::TurnInput,
};
use tokio::sync::mpsc;

use crate::error::ProviderError;
use crate::models::{
    ModelInfo, ProviderMetadata, ProviderSnapshot, SessionSpec, UsageLimits, ProviderEvent,
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Driver for an agent provider backend (Codex, Claude, Antigravity, etc.).
pub trait ProviderDriver: Send + Sync {
    /// Static metadata describing the driver capabilities and supported models.
    fn metadata(&self) -> ProviderMetadata;

    /// Side-effect free health and authentication probe.
    /// MUST NOT create sessions, start logins, or refresh tokens.
    fn probe<'a>(&'a self, cfg: &'a ProviderInstanceConfig) -> BoxFuture<'a, ProviderSnapshot>;

    /// Lists supported models for this provider instance.
    fn list_models<'a>(
        &'a self,
        cfg: &'a ProviderInstanceConfig,
    ) -> BoxFuture<'a, Result<Vec<ModelInfo>, ProviderError>>;

    /// Queries quota and rate limits if supported by the provider.
    fn usage_limits<'a>(&'a self, cfg: &'a ProviderInstanceConfig) -> BoxFuture<'a, Option<UsageLimits>>;

    /// Starts an active supervised session for a thread.
    fn start_session<'a>(
        &'a self,
        cfg: &'a ProviderInstanceConfig,
        spec: SessionSpec,
    ) -> BoxFuture<'a, Result<Box<dyn ProviderSession>, ProviderError>>;
}

/// An active, supervised session running turns with an agent provider.
pub trait ProviderSession: Send {
    /// Sends a prompt and attachments to initiate a turn.
    fn send_turn<'a>(&'a mut self, input: TurnInput) -> BoxFuture<'a, Result<(), ProviderError>>;

    /// Returns the receiver for streamed provider events.
    fn events(&mut self) -> &mut mpsc::Receiver<ProviderEvent>;

    /// Answers a pending approval request (e.g. bash execution, file write).
    fn respond_approval<'a>(
        &'a mut self,
        request_id: String,
        decision: ApprovalDecision,
    ) -> BoxFuture<'a, Result<(), ProviderError>>;

    /// Interrupts the currently executing turn without killing the session.
    fn interrupt<'a>(&'a mut self) -> BoxFuture<'a, Result<(), ProviderError>>;

    /// Returns a resumption token if the provider supports session continuation.
    fn resume_token(&self) -> Option<String>;

    /// Gracefully shuts down the session and terminates child processes.
    fn shutdown(self: Box<Self>) -> BoxFuture<'static, ()>;
}
