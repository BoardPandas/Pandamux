use crate::store::{
    AttemptDispatchRecord, AttemptDispatchState, AttemptProgressOutcome, Store, StoreError,
};
use pandamux_core::{
    AttemptProgressKind, AttemptProgressRecord, CleanupState, RunAttemptId, RunId, RunTaskId,
};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub const DEFAULT_PROGRESS_POLL_MS: u64 = 1_000;
pub const DEFAULT_STALL_THRESHOLD_MS: u64 = 10 * 60_000;
pub const DEFAULT_INTERRUPT_TIMEOUT_MS: u64 = 10_000;
pub const DEFAULT_DRAIN_TIMEOUT_MS: u64 = 20_000;
pub const DEFAULT_ESCALATION_TIMEOUT_MS: u64 = 10_000;
pub const DEFAULT_SHUTDOWN_TIMEOUT_MS: u64 = 60_000;
pub const DEFAULT_LEASE_TTL_MS: u64 = 60_000;
pub const DEFAULT_LEASE_RENEWAL_MS: u64 = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchdogConfig {
    pub poll_ms: u64,
    pub stall_threshold_ms: u64,
    pub interrupt_timeout_ms: u64,
    pub drain_timeout_ms: u64,
    pub escalation_timeout_ms: u64,
    pub shutdown_timeout_ms: u64,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            poll_ms: DEFAULT_PROGRESS_POLL_MS,
            stall_threshold_ms: DEFAULT_STALL_THRESHOLD_MS,
            interrupt_timeout_ms: DEFAULT_INTERRUPT_TIMEOUT_MS,
            drain_timeout_ms: DEFAULT_DRAIN_TIMEOUT_MS,
            escalation_timeout_ms: DEFAULT_ESCALATION_TIMEOUT_MS,
            shutdown_timeout_ms: DEFAULT_SHUTDOWN_TIMEOUT_MS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchdogDecision {
    Continue,
    Terminal,
    StallDeadline,
    AbsoluteRuntimeDeadline,
    ClockAnomaly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NonProgressSignal {
    Heartbeat,
    DuplicateEvent,
    IdenticalDigest,
    QueuePoll,
    TimerTick,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgressObservation {
    Meaningful(AttemptProgressRecord),
    NonProgress {
        run_id: RunId,
        task_id: RunTaskId,
        attempt_id: RunAttemptId,
        signal: NonProgressSignal,
        at_ms: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgressObservationOutcome {
    Recorded(AttemptProgressOutcome),
    Ignored(NonProgressSignal),
}

#[derive(Clone)]
pub struct AttemptWatchdog {
    store: Store,
    config: WatchdogConfig,
}

impl AttemptWatchdog {
    pub fn new(store: Store) -> Self {
        Self::with_config(store, WatchdogConfig::default())
    }

    pub fn with_config(store: Store, config: WatchdogConfig) -> Self {
        Self { store, config }
    }

    pub const fn config(&self) -> WatchdogConfig {
        self.config
    }

    pub fn observe(
        &self,
        observation: ProgressObservation,
    ) -> Result<ProgressObservationOutcome, StoreError> {
        match observation {
            ProgressObservation::Meaningful(progress) => self
                .store
                .record_attempt_progress(&progress)
                .map(ProgressObservationOutcome::Recorded),
            ProgressObservation::NonProgress { signal, .. } => {
                Ok(ProgressObservationOutcome::Ignored(signal))
            }
        }
    }

    pub fn evaluate(
        &self,
        dispatch: &AttemptDispatchRecord,
        absolute_runtime_ms: u64,
        now_ms: u64,
    ) -> WatchdogDecision {
        if matches!(
            dispatch.state,
            AttemptDispatchState::NotStarted
                | AttemptDispatchState::Ambiguous
                | AttemptDispatchState::Terminal
                | AttemptDispatchState::Cancelled
        ) {
            return WatchdogDecision::Terminal;
        }
        if now_ms < dispatch.created_at_ms || now_ms < dispatch.last_progress_at_ms {
            return WatchdogDecision::ClockAnomaly;
        }
        if now_ms.saturating_sub(dispatch.created_at_ms) >= absolute_runtime_ms {
            return WatchdogDecision::AbsoluteRuntimeDeadline;
        }
        if now_ms.saturating_sub(dispatch.last_progress_at_ms) >= self.config.stall_threshold_ms {
            return WatchdogDecision::StallDeadline;
        }
        WatchdogDecision::Continue
    }
}

pub type LifecycleFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Control surface for one attempt owner. Implementations keep the provider
/// session and process permit inside their existing owner; the watchdog only
/// sequences bounded lifecycle calls.
pub trait AttemptLifecycleControl: Send + Sync {
    fn record_cancel_intent<'a>(&'a self, at_ms: u64) -> LifecycleFuture<'a, Result<(), String>>;
    fn interrupt<'a>(&'a self) -> LifecycleFuture<'a, Result<(), String>>;
    fn drain<'a>(&'a self) -> LifecycleFuture<'a, Result<bool, String>>;
    fn escalate<'a>(&'a self) -> LifecycleFuture<'a, Result<(), String>>;
    fn verify_death<'a>(&'a self) -> LifecycleFuture<'a, Result<bool, String>>;
    fn settle_verified<'a>(&'a self, at_ms: u64) -> LifecycleFuture<'a, Result<(), String>>;
    fn mark_cleanup_verified<'a>(&'a self, at_ms: u64) -> LifecycleFuture<'a, Result<(), String>>;
    fn quarantine<'a>(
        &'a self,
        reason_code: &'static str,
        at_ms: u64,
    ) -> LifecycleFuture<'a, Result<(), String>>;
    fn release_leases<'a>(&'a self) -> LifecycleFuture<'a, Result<(), String>>;
    fn stop_timers<'a>(&'a self) -> LifecycleFuture<'a, Result<(), String>>;
    fn close_senders<'a>(&'a self) -> LifecycleFuture<'a, Result<(), String>>;
    fn join_writers<'a>(&'a self) -> LifecycleFuture<'a, Result<(), String>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancellationReport {
    pub cleanup_state: CleanupState,
    pub interrupt_completed: bool,
    pub drained: bool,
    pub escalated: bool,
    pub death_verified: bool,
    pub leases_released: bool,
    pub writers_joined: bool,
    pub reason_code: Option<&'static str>,
}

impl CancellationReport {
    fn pending() -> Self {
        Self {
            cleanup_state: CleanupState::Pending,
            interrupt_completed: false,
            drained: false,
            escalated: false,
            death_verified: false,
            leases_released: false,
            writers_joined: false,
            reason_code: None,
        }
    }
}

/// Runs the contractually ordered cancellation sequence. Cancellation intent
/// is durable before interrupt, and timeout paths always quarantine rather
/// than treating elapsed time as proof of process death.
pub async fn cancel_owned_attempt(
    control: &dyn AttemptLifecycleControl,
    config: WatchdogConfig,
    at_ms: u64,
) -> Result<CancellationReport, String> {
    let mut report = CancellationReport::pending();
    control.record_cancel_intent(at_ms).await?;

    report.interrupt_completed = matches!(
        tokio::time::timeout(
            Duration::from_millis(config.interrupt_timeout_ms),
            control.interrupt(),
        )
        .await,
        Ok(Ok(()))
    );

    report.drained = matches!(
        tokio::time::timeout(
            Duration::from_millis(config.drain_timeout_ms),
            control.drain(),
        )
        .await,
        Ok(Ok(true))
    );

    if !report.drained {
        report.escalated = matches!(
            tokio::time::timeout(
                Duration::from_millis(config.escalation_timeout_ms),
                control.escalate(),
            )
            .await,
            Ok(Ok(()))
        );
    }

    report.death_verified = matches!(
        tokio::time::timeout(
            Duration::from_millis(config.escalation_timeout_ms),
            control.verify_death(),
        )
        .await,
        Ok(Ok(true))
    );

    if !report.death_verified {
        let reason = if report.escalated {
            "attempt_death_unverified"
        } else {
            "attempt_cleanup_pending"
        };
        quarantine_with_bound(control, config, reason, at_ms).await?;
        report.cleanup_state = CleanupState::Quarantined;
        report.reason_code = Some(reason);
        return Ok(report);
    }

    if !matches!(
        tokio::time::timeout(
            Duration::from_millis(config.drain_timeout_ms),
            control.settle_verified(at_ms),
        )
        .await,
        Ok(Ok(()))
    ) {
        quarantine_with_bound(control, config, "attempt_settlement_failed", at_ms).await?;
        report.cleanup_state = CleanupState::Quarantined;
        report.reason_code = Some("attempt_settlement_failed");
        return Ok(report);
    }

    if !matches!(
        tokio::time::timeout(
            Duration::from_millis(config.escalation_timeout_ms),
            control.release_leases(),
        )
        .await,
        Ok(Ok(()))
    ) {
        quarantine_with_bound(control, config, "attempt_lease_release_failed", at_ms).await?;
        report.cleanup_state = CleanupState::Quarantined;
        report.reason_code = Some("attempt_lease_release_failed");
        return Ok(report);
    }
    let timers_stopped = matches!(
        tokio::time::timeout(
            Duration::from_millis(config.escalation_timeout_ms),
            control.stop_timers(),
        )
        .await,
        Ok(Ok(()))
    );
    let senders_closed = matches!(
        tokio::time::timeout(
            Duration::from_millis(config.escalation_timeout_ms),
            control.close_senders(),
        )
        .await,
        Ok(Ok(()))
    );
    let writers_joined = matches!(
        tokio::time::timeout(
            Duration::from_millis(config.drain_timeout_ms),
            control.join_writers(),
        )
        .await,
        Ok(Ok(()))
    );
    if !timers_stopped || !senders_closed || !writers_joined {
        quarantine_with_bound(control, config, "attempt_writer_join_failed", at_ms).await?;
        report.cleanup_state = CleanupState::Quarantined;
        report.reason_code = Some("attempt_writer_join_failed");
        return Ok(report);
    }
    report.writers_joined = true;

    if !matches!(
        tokio::time::timeout(
            Duration::from_millis(config.escalation_timeout_ms),
            control.mark_cleanup_verified(at_ms),
        )
        .await,
        Ok(Ok(()))
    ) {
        quarantine_with_bound(control, config, "attempt_cleanup_receipt_failed", at_ms).await?;
        report.cleanup_state = CleanupState::Quarantined;
        report.reason_code = Some("attempt_cleanup_receipt_failed");
        return Ok(report);
    }
    report.leases_released = true;
    report.cleanup_state = CleanupState::Verified;
    Ok(report)
}

async fn quarantine_with_bound(
    control: &dyn AttemptLifecycleControl,
    config: WatchdogConfig,
    reason: &'static str,
    at_ms: u64,
) -> Result<(), String> {
    match tokio::time::timeout(
        Duration::from_millis(config.escalation_timeout_ms),
        control.quarantine(reason, at_ms),
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => return Err("attempt quarantine exceeded its bounded cleanup stage".to_string()),
    }
    let _ = tokio::time::timeout(
        Duration::from_millis(config.escalation_timeout_ms),
        control.stop_timers(),
    )
    .await;
    let _ = tokio::time::timeout(
        Duration::from_millis(config.escalation_timeout_ms),
        control.close_senders(),
    )
    .await;
    let _ = tokio::time::timeout(
        Duration::from_millis(config.drain_timeout_ms),
        control.join_writers(),
    )
    .await;
    Ok(())
}

pub fn progress_kind_name(kind: AttemptProgressKind) -> &'static str {
    match kind {
        AttemptProgressKind::BoundaryAdvance => "boundary_advance",
        AttemptProgressKind::StreamBytes => "stream_bytes",
        AttemptProgressKind::ToolLifecycle => "tool_lifecycle",
        AttemptProgressKind::ApprovalTransition => "approval_transition",
        AttemptProgressKind::CheckpointCreated => "checkpoint_created",
        AttemptProgressKind::UsageReceipt => "usage_receipt",
        AttemptProgressKind::ArtifactPublished => "artifact_published",
        AttemptProgressKind::TerminalOutcome => "terminal_outcome",
    }
}
