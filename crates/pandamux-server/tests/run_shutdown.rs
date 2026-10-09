use pandamux_core::{
    AttemptBoundary, CleanupState, OrchestrationEffectId, ProviderInstanceId, RoutingDecision,
    RunAttemptId, RunId, RunTaskId, TaskRouteDecision, ThreadId, TurnAttemptId, TurnId,
};
use pandamux_server::{
    AttemptDispatchRecord, AttemptDispatchState, AttemptLifecycleControl, AttemptWatchdog,
    LifecycleFuture, NonProgressSignal, ProgressObservation, ProgressObservationOutcome,
    RunExecutionService, Store, WatchdogConfig, WatchdogDecision, cancel_owned_attempt,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct ScriptedControl {
    log: Arc<Mutex<Vec<&'static str>>>,
    drained: bool,
    death_verified: bool,
    join_ok: bool,
    delay_ms: u64,
    fail_at: Option<&'static str>,
}

impl ScriptedControl {
    fn new(drained: bool, death_verified: bool, join_ok: bool) -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            drained,
            death_verified,
            join_ok,
            delay_ms: 0,
            fail_at: None,
        }
    }

    fn delayed(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    fn failing(mut self, phase: &'static str) -> Self {
        self.fail_at = Some(phase);
        self
    }

    fn push(&self, value: &'static str) {
        self.log.lock().expect("lifecycle log").push(value);
    }

    fn entries(&self) -> Vec<&'static str> {
        self.log.lock().expect("lifecycle log").clone()
    }

    fn phase_result(&self, phase: &'static str) -> Result<(), String> {
        if self.fail_at == Some(phase) {
            Err(format!("{phase} failed"))
        } else {
            Ok(())
        }
    }
}

impl AttemptLifecycleControl for ScriptedControl {
    fn record_cancel_intent<'a>(&'a self, _at_ms: u64) -> LifecycleFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push("intent");
            self.phase_result("intent")
        })
    }

    fn interrupt<'a>(&'a self) -> LifecycleFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push("interrupt");
            if self.delay_ms != 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            self.phase_result("interrupt")
        })
    }

    fn drain<'a>(&'a self) -> LifecycleFuture<'a, Result<bool, String>> {
        Box::pin(async move {
            self.push("drain");
            self.phase_result("drain")?;
            Ok(self.drained)
        })
    }

    fn escalate<'a>(&'a self) -> LifecycleFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push("escalate");
            self.phase_result("escalate")
        })
    }

    fn verify_death<'a>(&'a self) -> LifecycleFuture<'a, Result<bool, String>> {
        Box::pin(async move {
            self.push("verify_death");
            self.phase_result("verify_death")?;
            Ok(self.death_verified)
        })
    }

    fn settle_verified<'a>(&'a self, _at_ms: u64) -> LifecycleFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push("settle");
            self.phase_result("settle")
        })
    }

    fn mark_cleanup_verified<'a>(&'a self, _at_ms: u64) -> LifecycleFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push("cleanup_verified");
            self.phase_result("cleanup_verified")
        })
    }

    fn quarantine<'a>(
        &'a self,
        _reason_code: &'static str,
        _at_ms: u64,
    ) -> LifecycleFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push("quarantine");
            self.phase_result("quarantine")
        })
    }

    fn release_leases<'a>(&'a self) -> LifecycleFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push("release_leases");
            self.phase_result("release_leases")
        })
    }

    fn stop_timers<'a>(&'a self) -> LifecycleFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push("stop_timers");
            self.phase_result("stop_timers")
        })
    }

    fn close_senders<'a>(&'a self) -> LifecycleFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push("close_senders");
            self.phase_result("close_senders")
        })
    }

    fn join_writers<'a>(&'a self) -> LifecycleFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push("join_writers");
            self.phase_result("join_writers")?;
            self.join_ok
                .then_some(())
                .ok_or_else(|| "writer did not join".to_string())
        })
    }
}

fn fast_config() -> WatchdogConfig {
    WatchdogConfig {
        poll_ms: 1,
        stall_threshold_ms: 50,
        interrupt_timeout_ms: 50,
        drain_timeout_ms: 50,
        escalation_timeout_ms: 50,
        shutdown_timeout_ms: 250,
    }
}

#[tokio::test]
async fn life_04_05_and_crash_10_persist_intent_before_cooperative_drain() {
    let control = ScriptedControl::new(true, true, true);
    let report = cancel_owned_attempt(&control, fast_config(), 10)
        .await
        .expect("cooperative cancellation");
    assert_eq!(report.cleanup_state, CleanupState::Verified);
    assert!(report.drained);
    assert!(!report.escalated);
    assert_eq!(
        control.entries(),
        vec![
            "intent",
            "interrupt",
            "drain",
            "verify_death",
            "settle",
            "release_leases",
            "stop_timers",
            "close_senders",
            "join_writers",
            "cleanup_verified",
        ]
    );
}

#[tokio::test]
async fn life_06_escalates_only_after_drain_refuses_completion() {
    let control = ScriptedControl::new(false, true, true);
    let report = cancel_owned_attempt(&control, fast_config(), 20)
        .await
        .expect("escalated cancellation");
    assert_eq!(report.cleanup_state, CleanupState::Verified);
    assert!(report.escalated);
    let entries = control.entries();
    assert!(
        entries.iter().position(|entry| *entry == "drain").unwrap()
            < entries
                .iter()
                .position(|entry| *entry == "escalate")
                .unwrap()
    );
    assert!(
        entries
            .iter()
            .position(|entry| *entry == "escalate")
            .unwrap()
            < entries
                .iter()
                .position(|entry| *entry == "verify_death")
                .unwrap()
    );
}

#[tokio::test]
async fn life_07_timeout_is_not_death_and_retains_capacity_debt() {
    let control = ScriptedControl::new(false, false, true);
    let report = cancel_owned_attempt(&control, fast_config(), 30)
        .await
        .expect("quarantined cancellation");
    assert_eq!(report.cleanup_state, CleanupState::Quarantined);
    assert!(!report.death_verified);
    assert!(!report.leases_released);
    let entries = control.entries();
    assert!(entries.contains(&"quarantine"));
    assert!(!entries.contains(&"settle"));
    assert!(!entries.contains(&"release_leases"));
}

#[tokio::test]
async fn life_08_cancellation_owners_drain_concurrently() {
    let first = ScriptedControl::new(true, true, true).delayed(100);
    let second = ScriptedControl::new(true, true, true).delayed(100);
    let mut config = fast_config();
    config.interrupt_timeout_ms = 250;
    let started = Instant::now();
    let (first_report, second_report) = tokio::join!(
        cancel_owned_attempt(&first, config, 40),
        cancel_owned_attempt(&second, config, 40),
    );
    assert_eq!(
        first_report.expect("first").cleanup_state,
        CleanupState::Verified
    );
    assert_eq!(
        second_report.expect("second").cleanup_state,
        CleanupState::Verified
    );
    assert!(started.elapsed() < Duration::from_millis(175));
}

#[tokio::test]
async fn life_09_closes_senders_before_join_and_quarantines_join_failure() {
    let control = ScriptedControl::new(true, true, false);
    let report = cancel_owned_attempt(&control, fast_config(), 50)
        .await
        .expect("join failure is quarantined");
    assert_eq!(report.cleanup_state, CleanupState::Quarantined);
    assert!(!report.leases_released);
    let entries = control.entries();
    assert!(
        entries
            .iter()
            .position(|entry| *entry == "close_senders")
            .unwrap()
            < entries
                .iter()
                .position(|entry| *entry == "join_writers")
                .unwrap()
    );
    assert!(!entries.contains(&"cleanup_verified"));
}

#[tokio::test]
async fn cancellation_phase_failures_never_report_verified_cleanup() {
    let intent = ScriptedControl::new(true, true, true).failing("intent");
    assert!(
        cancel_owned_attempt(&intent, fast_config(), 60)
            .await
            .is_err()
    );
    assert_eq!(intent.entries(), vec!["intent"]);

    let interrupt = ScriptedControl::new(true, true, true).failing("interrupt");
    let report = cancel_owned_attempt(&interrupt, fast_config(), 61)
        .await
        .expect("failed interrupt can still drain and verify death");
    assert_eq!(report.cleanup_state, CleanupState::Verified);
    assert!(!report.interrupt_completed);

    for (phase, reason) in [
        ("settle", "attempt_settlement_failed"),
        ("release_leases", "attempt_lease_release_failed"),
        ("stop_timers", "attempt_writer_join_failed"),
        ("close_senders", "attempt_writer_join_failed"),
        ("join_writers", "attempt_writer_join_failed"),
        ("cleanup_verified", "attempt_cleanup_receipt_failed"),
    ] {
        let control = ScriptedControl::new(true, true, true).failing(phase);
        let report = cancel_owned_attempt(&control, fast_config(), 62)
            .await
            .expect("phase failure is durably quarantined");
        assert_eq!(report.cleanup_state, CleanupState::Quarantined, "{phase}");
        assert_eq!(report.reason_code, Some(reason), "{phase}");
        assert!(!report.leases_released, "{phase}");
        assert!(control.entries().contains(&"quarantine"), "{phase}");
    }

    let drain = ScriptedControl::new(true, true, true).failing("drain");
    let report = cancel_owned_attempt(&drain, fast_config(), 63)
        .await
        .expect("failed drain escalates");
    assert!(report.escalated);
    assert_eq!(report.cleanup_state, CleanupState::Verified);

    let death = ScriptedControl::new(false, true, true).failing("verify_death");
    let report = cancel_owned_attempt(&death, fast_config(), 64)
        .await
        .expect("unverified death is quarantined");
    assert_eq!(report.cleanup_state, CleanupState::Quarantined);
    assert_eq!(report.reason_code, Some("attempt_death_unverified"));
    assert!(!report.leases_released);
}

#[tokio::test]
async fn successful_service_shutdown_reports_zero_live_owners_and_sqlite_checkpoint() {
    let service = RunExecutionService::new(Store::in_memory().expect("shutdown store"));
    let report = service.shutdown(Duration::from_millis(250)).await;
    assert!(report.intake_closed);
    assert!(report.claims_stopped);
    assert_eq!(report.attempts_total, 0);
    assert_eq!(report.live_owned_tasks, 0);
    assert!(!report.timed_out);
    assert!(report.sqlite_checkpointed);
    assert_eq!(service.live_owned_attempts().await, 0);
}

#[test]
fn life_01_to_03_nonprogress_and_repeated_progress_never_extend_hard_runtime() {
    let watchdog = AttemptWatchdog::with_config(Store::in_memory().unwrap(), fast_config());
    let ignored = watchdog
        .observe(ProgressObservation::NonProgress {
            run_id: RunId::new("run-life"),
            task_id: RunTaskId::new("task-life"),
            attempt_id: RunAttemptId::new("attempt-life"),
            signal: NonProgressSignal::Heartbeat,
            at_ms: 1_049,
        })
        .expect("nonprogress is ignored");
    assert_eq!(
        ignored,
        ProgressObservationOutcome::Ignored(NonProgressSignal::Heartbeat)
    );

    let dispatch = AttemptDispatchRecord {
        attempt_id: RunAttemptId::new("attempt-life"),
        run_id: RunId::new("run-life"),
        task_id: RunTaskId::new("task-life"),
        effect_id: OrchestrationEffectId::new("effect-life"),
        thread_id: ThreadId::new("thread-life"),
        turn_id: TurnId::new("turn-life"),
        turn_attempt_id: TurnAttemptId::new("turn-attempt-life"),
        provider_instance_id: ProviderInstanceId::new("provider-life"),
        route: TaskRouteDecision {
            task_id: RunTaskId::new("task-life"),
            selected_agent_id: None,
            agent_evaluations: Vec::new(),
            provider: RoutingDecision {
                selected_provider_instance_id: Some(ProviderInstanceId::new("provider-life")),
                selected_subscription_key: None,
                selected_model: None,
                evaluations: Vec::new(),
            },
        },
        bundle_sha: "a".repeat(64),
        state: AttemptDispatchState::Accepted,
        last_boundary: AttemptBoundary::VisibleOutputBegun,
        last_progress_at_ms: 1_049,
        cancel_requested_at_ms: None,
        ambiguity_reason: None,
        outcome_digest: None,
        created_at_ms: 1_000,
        updated_at_ms: 1_049,
    };
    assert_eq!(
        watchdog.evaluate(&dispatch, 50, 1_050),
        WatchdogDecision::AbsoluteRuntimeDeadline
    );
    assert_eq!(
        watchdog.evaluate(&dispatch, 500, 999),
        WatchdogDecision::ClockAnomaly
    );
}

#[tokio::test]
async fn lifecycle_real_local_child_has_positive_death_evidence() {
    #[cfg(windows)]
    let mut child = tokio::process::Command::new("cmd")
        .args(["/C", "ping -n 30 127.0.0.1 >NUL"])
        .kill_on_drop(true)
        .spawn()
        .expect("spawn local Windows child");
    #[cfg(not(windows))]
    let mut child = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .expect("spawn local child");

    assert!(child.try_wait().expect("child status").is_none());
    child.start_kill().expect("start child termination");
    let status = tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .expect("bounded child death verification")
        .expect("wait for local child");
    assert!(!status.success());
    assert!(child.try_wait().expect("final child status").is_some());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn lifecycle_supervised_process_group_kills_its_descendant() {
    use pandamux_providers::{SupervisedChild, create_supervised_command};
    use std::path::Path;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut command = create_supervised_command(Path::new("/bin/sh"));
    command.args([
        "-c",
        "sleep 30 & descendant=$!; printf '%s\\n' \"$descendant\"; wait",
    ]);
    let mut supervised = SupervisedChild::spawn(command).expect("spawn supervised process group");
    let group_leader = supervised
        .child_mut()
        .and_then(|child| child.id())
        .expect("supervised group leader pid");
    let stdout = supervised.take_stdout().expect("supervised stdout");
    let mut lines = BufReader::new(stdout).lines();
    let descendant = tokio::time::timeout(Duration::from_secs(2), lines.next_line())
        .await
        .expect("descendant pid announcement timeout")
        .expect("read descendant pid")
        .expect("descendant pid announcement")
        .parse::<u32>()
        .expect("numeric descendant pid");
    assert!(linux_process_is_running(group_leader));
    assert!(linux_process_is_running(descendant));

    supervised
        .kill()
        .await
        .expect("kill supervised process group");
    tokio::time::timeout(Duration::from_secs(2), async {
        while linux_process_is_running(group_leader) || linux_process_is_running(descendant) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("process group descendants did not reach a dead or reaped state");
}

#[cfg(target_os = "linux")]
fn linux_process_is_running(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    stat.rsplit_once(") ")
        .and_then(|(_, fields)| fields.chars().next())
        .is_some_and(|state| state != 'Z' && state != 'X')
}
