use crate::environment_router::{EnvironmentRouter, EnvironmentTransportState};
use crate::store::{Store, StoreError};
use pandamux_core::EnvironmentId;
use pandamux_protocol::{
    RpcError, RpcRequest, RpcResponse, UsageAckParams, UsageBackfillCursor, UsageBackfillParams,
    UsageBackfillResult, UsageExportParams, UsageExportResult,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

#[derive(Clone, Debug)]
pub struct UsageSyncRuntimeConfig {
    pub page_limit: u32,
    pub max_pages_per_environment_pass: usize,
    pub trigger_capacity: usize,
    pub periodic_interval: Duration,
    pub shutdown_timeout: Duration,
}

impl Default for UsageSyncRuntimeConfig {
    fn default() -> Self {
        Self {
            page_limit: 250,
            max_pages_per_environment_pass: 16,
            trigger_capacity: 64,
            periodic_interval: Duration::from_secs(60),
            shutdown_timeout: Duration::from_secs(5),
        }
    }
}

impl UsageSyncRuntimeConfig {
    fn validate(&self) -> Result<(), std::io::Error> {
        if self.page_limit == 0
            || self.page_limit > pandamux_protocol::MAX_USAGE_PAGE_SIZE
            || self.max_pages_per_environment_pass == 0
            || self.trigger_capacity == 0
            || self.periodic_interval.is_zero()
            || self.shutdown_timeout.is_zero()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "usage sync runtime limits must be positive and bounded",
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct UsageSyncHandle {
    triggers: mpsc::Sender<EnvironmentId>,
}

impl UsageSyncHandle {
    pub fn trigger_environment(&self, environment_id: EnvironmentId) -> bool {
        self.triggers.try_send(environment_id).is_ok()
    }
}

pub struct UsageHubSyncService {
    handle: UsageSyncHandle,
    shutdown_tx: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
    shutdown_timeout: Duration,
}

impl UsageHubSyncService {
    pub fn start(
        store: Store,
        environment_router: Arc<EnvironmentRouter>,
        config: UsageSyncRuntimeConfig,
    ) -> Result<Self, std::io::Error> {
        config.validate()?;
        let (triggers, mut trigger_rx) = mpsc::channel(config.trigger_capacity);
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let task_router = environment_router.clone();
        let task_config = config.clone();
        let task = tokio::spawn(async move {
            let synchronizer = UsageHubSynchronizer::new(store, task_router.clone());
            let mut interval = tokio::time::interval(task_config.periodic_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            break;
                        }
                    }
                    environment_id = trigger_rx.recv() => {
                        let Some(environment_id) = environment_id else {
                            break;
                        };
                        let _ = synchronizer
                            .sync_environment(
                                &environment_id,
                                &task_config,
                            )
                            .await;
                    }
                    _ = interval.tick() => {
                        for environment_id in task_router.registered_environment_ids().await {
                            if *shutdown_rx.borrow() {
                                break;
                            }
                            let _ = synchronizer
                                .sync_environment(
                                    &environment_id,
                                    &task_config,
                                )
                                .await;
                        }
                    }
                }
            }
        });
        Ok(Self {
            handle: UsageSyncHandle { triggers },
            shutdown_tx,
            task: Some(task),
            shutdown_timeout: config.shutdown_timeout,
        })
    }

    pub fn handle(&self) -> UsageSyncHandle {
        self.handle.clone()
    }

    pub async fn shutdown(mut self) -> Result<(), std::io::Error> {
        let _ = self.shutdown_tx.send(true);
        if let Some(mut task) = self.task.take()
            && tokio::time::timeout(self.shutdown_timeout, &mut task)
                .await
                .is_err()
        {
            task.abort();
            let _ = task.await;
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "usage synchronization did not stop within its shutdown budget",
            ));
        }
        Ok(())
    }
}

impl Drop for UsageHubSyncService {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageSyncReport {
    pub pages: u64,
    pub observations_inserted: u64,
    pub observations_updated: u64,
    pub observations_duplicated: u64,
    pub acknowledged_through_seq: u64,
    pub needs_follow_up: bool,
}

#[derive(Debug)]
pub struct UsageSyncError {
    safe_code: &'static str,
    unreachable: bool,
    detail: String,
}

impl UsageSyncError {
    fn new(safe_code: &'static str, unreachable: bool, detail: impl Into<String>) -> Self {
        Self {
            safe_code,
            unreachable,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for UsageSyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.safe_code, self.detail)
    }
}

impl std::error::Error for UsageSyncError {}

impl From<StoreError> for UsageSyncError {
    fn from(error: StoreError) -> Self {
        Self::new("usage_sync_store_error", false, error.to_string())
    }
}

pub struct UsageHubSynchronizer {
    store: Store,
    environment_router: Arc<EnvironmentRouter>,
    next_request_id: AtomicU64,
}

impl UsageHubSynchronizer {
    pub fn new(store: Store, environment_router: Arc<EnvironmentRouter>) -> Self {
        Self {
            store,
            environment_router,
            next_request_id: AtomicU64::new(1),
        }
    }

    pub async fn sync_environment(
        &self,
        environment_id: &EnvironmentId,
        config: &UsageSyncRuntimeConfig,
    ) -> Result<UsageSyncReport, UsageSyncError> {
        let result = self.sync_environment_inner(environment_id, config).await;
        if let Err(error) = &result {
            let _ = self.store.record_usage_sync_failure(
                environment_id,
                error.unreachable,
                error.safe_code,
            );
        }
        result
    }

    async fn sync_environment_inner(
        &self,
        environment_id: &EnvironmentId,
        config: &UsageSyncRuntimeConfig,
    ) -> Result<UsageSyncReport, UsageSyncError> {
        if self
            .environment_router
            .transport_state(environment_id)
            .await
            != EnvironmentTransportState::Connected
        {
            return Err(UsageSyncError::new(
                "usage_sync_unreachable",
                true,
                "environment transport is not connected",
            ));
        }
        let mut report = UsageSyncReport::default();
        let state = self.store.usage_hub_sync_state(environment_id)?;
        if state
            .as_ref()
            .is_some_and(|state| state.backfill_run_id.is_some())
        {
            self.resume_backfill(environment_id, config, &mut report)
                .await?;
            if report.needs_follow_up {
                return Ok(report);
            }
        }

        for _ in 0..config.max_pages_per_environment_pass {
            let high_water = self
                .store
                .usage_hub_sync_state(environment_id)?
                .map_or(0, |state| state.high_water_seq);
            let page: UsageExportResult = self
                .call_remote(
                    environment_id,
                    "usage.export",
                    &UsageExportParams {
                        after_seq: high_water,
                        limit: config.page_limit,
                    },
                )
                .await?;
            report.pages = checked_report_add(report.pages, 1, "usage sync page count")?;
            let current = self.store.usage_hub_sync_state(environment_id)?;
            let store_changed = current
                .as_ref()
                .is_some_and(|state| state.node_store_id != page.node_store_id);
            if store_changed || page.coverage_gap.is_some() {
                self.store.begin_usage_backfill(
                    environment_id,
                    &page.node_store_id,
                    page.latest_seq,
                    page.coverage_gap.as_ref(),
                )?;
                self.resume_backfill(environment_id, config, &mut report)
                    .await?;
                if report.needs_follow_up {
                    return Ok(report);
                }
                continue;
            }

            let outcome = self.store.ingest_usage_export_page(environment_id, &page)?;
            accumulate_outcome(&mut report, &outcome)?;
            self.acknowledge_remote(environment_id, outcome.high_water_seq)
                .await?;
            report.acknowledged_through_seq = outcome.high_water_seq;
            if !page.has_more {
                return Ok(report);
            }
        }
        report.needs_follow_up = true;
        Ok(report)
    }

    async fn resume_backfill(
        &self,
        environment_id: &EnvironmentId,
        config: &UsageSyncRuntimeConfig,
        report: &mut UsageSyncReport,
    ) -> Result<(), UsageSyncError> {
        let durable = self
            .store
            .usage_hub_sync_state(environment_id)?
            .ok_or_else(|| {
                UsageSyncError::new(
                    "usage_sync_state_missing",
                    false,
                    "durable backfill state disappeared",
                )
            })?;
        let probe: UsageExportResult = self
            .call_remote(
                environment_id,
                "usage.export",
                &UsageExportParams {
                    after_seq: 0,
                    limit: 1,
                },
            )
            .await?;
        report.pages = checked_report_add(report.pages, 1, "usage sync page count")?;
        if probe.node_store_id != durable.node_store_id
            || durable
                .backfill_through_seq
                .is_some_and(|through| through > probe.latest_seq)
        {
            self.store.begin_usage_backfill(
                environment_id,
                &probe.node_store_id,
                probe.latest_seq,
                probe.coverage_gap.as_ref(),
            )?;
        }
        for _ in 0..config.max_pages_per_environment_pass {
            let state = self
                .store
                .usage_hub_sync_state(environment_id)?
                .ok_or_else(|| {
                    UsageSyncError::new(
                        "usage_sync_state_missing",
                        false,
                        "durable backfill state disappeared",
                    )
                })?;
            let snapshot_through_seq = state.backfill_through_seq.ok_or_else(|| {
                UsageSyncError::new(
                    "usage_sync_state_invalid",
                    false,
                    "backfill snapshot is missing",
                )
            })?;
            let page: UsageBackfillResult = self
                .call_remote(
                    environment_id,
                    "usage.backfill",
                    &UsageBackfillParams {
                        snapshot_through_seq: Some(snapshot_through_seq),
                        after: state.backfill_after_observation_id.map(|value| {
                            UsageBackfillCursor {
                                observation_id: value.into(),
                            }
                        }),
                        limit: config.page_limit,
                    },
                )
                .await?;
            report.pages = checked_report_add(report.pages, 1, "usage sync page count")?;
            if page.node_store_id != state.node_store_id {
                self.store.begin_usage_backfill(
                    environment_id,
                    &page.node_store_id,
                    page.snapshot_through_seq,
                    None,
                )?;
                continue;
            }
            let outcome = self
                .store
                .ingest_usage_backfill_page(environment_id, &page)?;
            accumulate_outcome(report, &outcome)?;
            if !page.has_more {
                self.acknowledge_remote(environment_id, page.snapshot_through_seq)
                    .await?;
                report.acknowledged_through_seq = page.snapshot_through_seq;
                return Ok(());
            }
        }
        report.needs_follow_up = true;
        Ok(())
    }

    async fn acknowledge_remote(
        &self,
        environment_id: &EnvironmentId,
        through_seq: u64,
    ) -> Result<(), UsageSyncError> {
        let _: pandamux_protocol::UsageAckResult = self
            .call_remote(environment_id, "usage.ack", &UsageAckParams { through_seq })
            .await?;
        Ok(())
    }

    async fn call_remote<P, R>(
        &self,
        environment_id: &EnvironmentId,
        method: &str,
        params: &P,
    ) -> Result<R, UsageSyncError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let params = serde_json::to_value(params).map_err(|error| {
            UsageSyncError::new("usage_sync_encode_error", false, error.to_string())
        })?;
        let request = RpcRequest::new(
            i64::try_from(self.next_request_id.fetch_add(1, Ordering::Relaxed)).unwrap_or(i64::MAX),
            method,
            Some(params),
        );
        let response = self
            .environment_router
            .forward_request(environment_id, &request)
            .await
            .map_err(map_transport_error)?;
        decode_response(response)
    }
}

fn map_transport_error(error: RpcError) -> UsageSyncError {
    let safe_code = error
        .data
        .as_ref()
        .and_then(|value| value.get("proxyCode"))
        .and_then(|value| value.as_str())
        .filter(|value| value.len() <= 64)
        .unwrap_or("usage_sync_transport_error");
    let unreachable = matches!(
        safe_code,
        "proxy_disconnected" | "proxy_stale" | "proxy_closed"
    );
    UsageSyncError::new(
        if unreachable {
            "usage_sync_unreachable"
        } else {
            "usage_sync_transport_error"
        },
        unreachable,
        error.message,
    )
}

fn decode_response<R: DeserializeOwned>(response: RpcResponse) -> Result<R, UsageSyncError> {
    if let Some(error) = response.error {
        return Err(UsageSyncError::new(
            "usage_sync_remote_error",
            false,
            error.message,
        ));
    }
    let result = response.result.ok_or_else(|| {
        UsageSyncError::new(
            "usage_sync_response_missing",
            false,
            "remote response omitted its result",
        )
    })?;
    serde_json::from_value(result)
        .map_err(|error| UsageSyncError::new("usage_sync_decode_error", false, error.to_string()))
}

fn accumulate_outcome(
    report: &mut UsageSyncReport,
    outcome: &crate::store::UsageHubIngestOutcome,
) -> Result<(), UsageSyncError> {
    report.observations_inserted = checked_report_add(
        report.observations_inserted,
        outcome.inserted,
        "usage sync insert count",
    )?;
    report.observations_updated = checked_report_add(
        report.observations_updated,
        outcome.updated,
        "usage sync update count",
    )?;
    report.observations_duplicated = checked_report_add(
        report.observations_duplicated,
        outcome.duplicates,
        "usage sync duplicate count",
    )?;
    Ok(())
}

fn checked_report_add(left: u64, right: u64, field: &str) -> Result<u64, UsageSyncError> {
    left.checked_add(right)
        .ok_or_else(|| UsageSyncError::new("usage_sync_overflow", false, field))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment_router::MockEnvironmentTransport;
    use crate::store::UsageOutboxRetention;
    use pandamux_core::{
        ProviderInstanceId, SubscriptionKey, TokenCounts, UsageObservation, UsageObservationId,
        UsageProvider, UsageSourceKind, UsageSubscription,
    };
    use pandamux_protocol::{UsageAckResult, UsageQueryParams};
    use serde_json::Value;
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    use tempfile::NamedTempFile;

    fn observation(id: &str, input: u64) -> UsageObservation {
        UsageObservation {
            id: UsageObservationId::new(id),
            source_kind: UsageSourceKind::CodexTranscript,
            parser_version: 1,
            provider_native_session_id: Some("session-sync".to_string()),
            provider_native_turn_id: Some(format!("turn-{id}")),
            occurred_at_ms: Some(1_000 + input),
            provider: UsageProvider::Codex,
            provider_instance_id: ProviderInstanceId::new("codex-sync"),
            subscription_key: SubscriptionKey::new("subkey-sync"),
            model: Some("gpt-sync".to_string()),
            tokens: TokenCounts {
                input,
                output: 1,
                ..TokenCounts::default()
            },
            unknown_fields: BTreeSet::new(),
            provider_reported_micro_usd: None,
            source_record_count: 1,
            thread_id: None,
            turn_id: None,
            agent_id: None,
            schedule_id: None,
        }
    }

    fn node_store(environment_id: &EnvironmentId, ids: &[(&str, u64)]) -> Store {
        let store = Store::in_memory().expect("open node store");
        store
            .save_usage_subscription(
                &UsageSubscription {
                    key: SubscriptionKey::new("subkey-sync"),
                    provider: UsageProvider::Codex,
                    display_name: "Sync subscription".to_string(),
                    masked_account_label: None,
                },
                1,
            )
            .expect("save node subscription");
        store
            .map_provider_subscription(
                environment_id,
                &ProviderInstanceId::new("codex-sync"),
                &SubscriptionKey::new("subkey-sync"),
                1,
            )
            .expect("map node subscription");
        for (id, input) in ids {
            store
                .record_usage_batch(environment_id, &[observation(id, *input)], None)
                .expect("record node usage");
        }
        store
    }

    fn mock_node_transport(store: Arc<Mutex<Store>>) -> Arc<MockEnvironmentTransport> {
        Arc::new(MockEnvironmentTransport::new(move |request| {
            let store = store.lock().expect("mock node store lock");
            let result = match request.method.as_str() {
                "usage.export" => {
                    let params: UsageExportParams = parse_params(request)?;
                    serde_json::to_value(
                        store
                            .export_usage(&params)
                            .map_err(|error| RpcError::internal_error(error.to_string()))?,
                    )
                }
                "usage.backfill" => {
                    let params: UsageBackfillParams = parse_params(request)?;
                    serde_json::to_value(
                        store
                            .export_usage_backfill(&params)
                            .map_err(|error| RpcError::internal_error(error.to_string()))?,
                    )
                }
                "usage.ack" => {
                    let params: UsageAckParams = parse_params(request)?;
                    let outcome = store
                        .acknowledge_and_compact_usage(
                            params.through_seq,
                            UsageOutboxRetention::default(),
                        )
                        .map_err(|error| RpcError::internal_error(error.to_string()))?;
                    serde_json::to_value(UsageAckResult {
                        acknowledged: outcome.acknowledged,
                        through_seq: params.through_seq,
                        compacted: outcome.compacted,
                        retained_from_seq: outcome.retained_from_seq,
                    })
                }
                other => return Err(RpcError::method_not_found(other)),
            }
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
            Ok(RpcResponse {
                jsonrpc: pandamux_protocol::JSONRPC_VERSION.to_string(),
                id: request.id.clone(),
                result: Some(result),
                error: None,
            })
        }))
    }

    fn parse_params<T: DeserializeOwned>(request: &RpcRequest) -> Result<T, RpcError> {
        serde_json::from_value(request.params.clone().unwrap_or(Value::Null))
            .map_err(|error| RpcError::invalid_params(error.to_string()))
    }

    fn config() -> UsageSyncRuntimeConfig {
        UsageSyncRuntimeConfig {
            page_limit: 1,
            max_pages_per_environment_pass: 16,
            trigger_capacity: 4,
            periodic_interval: Duration::from_secs(60),
            shutdown_timeout: Duration::from_secs(1),
        }
    }

    #[tokio::test]
    async fn synchronizer_pages_acknowledges_and_replays_without_duplicate_totals() {
        let environment_id = EnvironmentId::new("env-node");
        let node = Arc::new(Mutex::new(node_store(
            &environment_id,
            &[("usage-one", 1), ("usage-two", 2), ("usage-three", 3)],
        )));
        let hub = Store::in_memory().expect("open hub store");
        let router = Arc::new(EnvironmentRouter::new(EnvironmentId::new("env-hub")));
        router
            .register_transport(environment_id.clone(), mock_node_transport(node), None)
            .await;
        let synchronizer = UsageHubSynchronizer::new(hub.clone(), router.clone());
        let first = synchronizer
            .sync_environment(&environment_id, &config())
            .await
            .expect("synchronize pages");
        assert_eq!(first.observations_inserted, 3);
        assert_eq!(first.acknowledged_through_seq, 3);

        let replay = synchronizer
            .sync_environment(&environment_id, &config())
            .await
            .expect("replay completed synchronization");
        assert_eq!(replay.observations_inserted, 0);
        let query = hub
            .query_usage(&UsageQueryParams {
                environment_id: Some(environment_id.clone()),
                ..UsageQueryParams::default()
            })
            .expect("query synchronized totals");
        assert_eq!(query.aggregate.observation_count, 3);
        assert_eq!(query.aggregate.tokens.input, 6);
        router.unregister_transport(&environment_id).await;
    }

    #[tokio::test]
    async fn changed_node_store_reconciles_snapshot_and_removes_old_facts() {
        let environment_id = EnvironmentId::new("env-node-reset");
        let active_node = Arc::new(Mutex::new(node_store(
            &environment_id,
            &[("usage-old", 10)],
        )));
        let hub = Store::in_memory().expect("open hub store");
        let router = Arc::new(EnvironmentRouter::new(EnvironmentId::new("env-hub")));
        router
            .register_transport(
                environment_id.clone(),
                mock_node_transport(active_node.clone()),
                None,
            )
            .await;
        let synchronizer = UsageHubSynchronizer::new(hub.clone(), router.clone());
        synchronizer
            .sync_environment(&environment_id, &config())
            .await
            .expect("synchronize old node store");

        *active_node.lock().expect("replace node store") =
            node_store(&environment_id, &[("usage-new", 20)]);
        synchronizer
            .sync_environment(&environment_id, &config())
            .await
            .expect("reconcile changed node store");
        let query = hub
            .query_usage(&UsageQueryParams {
                environment_id: Some(environment_id.clone()),
                ..UsageQueryParams::default()
            })
            .expect("query reset node totals");
        assert_eq!(query.aggregate.observation_count, 1);
        assert_eq!(query.items[0].observation.id.as_str(), "usage-new");
        assert_eq!(query.aggregate.tokens.input, 20);
        router.unregister_transport(&environment_id).await;
    }

    #[tokio::test]
    async fn reconnect_after_hub_restart_resumes_from_committed_high_water() {
        let environment_id = EnvironmentId::new("env-reconnect");
        let node = Arc::new(Mutex::new(node_store(
            &environment_id,
            &[("usage-one", 1), ("usage-two", 2)],
        )));
        let file = NamedTempFile::new().expect("temporary hub database");
        let path = file.path().to_path_buf();
        {
            let hub = Store::open(&path).expect("open first hub process");
            let router = Arc::new(EnvironmentRouter::new(EnvironmentId::new("env-hub")));
            router
                .register_transport(
                    environment_id.clone(),
                    mock_node_transport(node.clone()),
                    None,
                )
                .await;
            let synchronizer = UsageHubSynchronizer::new(hub, router.clone());
            let mut one_page = config();
            one_page.max_pages_per_environment_pass = 1;
            let report = synchronizer
                .sync_environment(&environment_id, &one_page)
                .await
                .expect("synchronize first process page");
            assert!(report.needs_follow_up);
            assert_eq!(report.acknowledged_through_seq, 1);
            router.unregister_transport(&environment_id).await;
        }

        let hub = Store::open(&path).expect("reopen hub process");
        let router = Arc::new(EnvironmentRouter::new(EnvironmentId::new("env-hub")));
        router
            .register_transport(environment_id.clone(), mock_node_transport(node), None)
            .await;
        let synchronizer = UsageHubSynchronizer::new(hub.clone(), router.clone());
        synchronizer
            .sync_environment(&environment_id, &config())
            .await
            .expect("resume after reconnect");
        let query = hub
            .query_usage(&UsageQueryParams {
                environment_id: Some(environment_id.clone()),
                ..UsageQueryParams::default()
            })
            .expect("query resumed totals");
        assert_eq!(query.aggregate.observation_count, 2);
        assert_eq!(query.aggregate.tokens.input, 3);
        router.unregister_transport(&environment_id).await;
    }

    #[tokio::test]
    async fn synchronization_service_shutdown_joins_its_owned_task() {
        let service = UsageHubSyncService::start(
            Store::in_memory().expect("open service hub"),
            Arc::new(EnvironmentRouter::new(EnvironmentId::new("env-hub"))),
            config(),
        )
        .expect("start usage synchronization service");
        service.shutdown().await.expect("join usage sync service");
    }
}
