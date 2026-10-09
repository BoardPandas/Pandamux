use crate::{
    BundleActivationFailpoint, BundleDistributionRecord, BundleMaterializationInput, BundleStore,
    BundleStoreError, EnvironmentRouter, Store, StoreError,
};
use pandamux_core::{
    BundleActivationId, BundleActivationKind, BundleActivationRecord, BundleDistributionReceipt,
    BundleDistributionState, BundleTransferId, ConfigurationAuditId, ConfigurationAuditRecord,
    EnvironmentId, InstructionCandidateId, InstructionSourceId,
};
use pandamux_protocol::{
    BUNDLE_PUSH_SCHEMA_VERSION, BundleDiscardUploadParams, BundlePrepareUploadParams,
    BundlePrepareUploadResult, BundlePushParams, BundlePushResult, RpcRequest,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BundleSyncReport {
    pub delivered: usize,
    pub remaining: usize,
    pub last_error_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleSyncError {
    pub reason: &'static str,
    pub safe_message: String,
    pub retryable: bool,
}

impl std::fmt::Display for BundleSyncError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.safe_message)
    }
}

impl std::error::Error for BundleSyncError {}

impl From<StoreError> for BundleSyncError {
    fn from(_: StoreError) -> Self {
        state_error()
    }
}

impl From<BundleStoreError> for BundleSyncError {
    fn from(error: BundleStoreError) -> Self {
        Self {
            reason: error.reason,
            safe_message: error.safe_message,
            retryable: error.retryable,
        }
    }
}

#[derive(Clone)]
pub struct BundleSyncService {
    store: Store,
    bundle_store: BundleStore,
    environment_router: Arc<EnvironmentRouter>,
    local_environment_id: EnvironmentId,
}

impl BundleSyncService {
    pub fn new(
        store: Store,
        bundle_store: BundleStore,
        environment_router: Arc<EnvironmentRouter>,
        local_environment_id: EnvironmentId,
    ) -> Self {
        Self {
            store,
            bundle_store,
            environment_router,
            local_environment_id,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn activate(
        &self,
        materialization: &BundleMaterializationInput,
        source_id: Option<&InstructionSourceId>,
        candidate_id: Option<&InstructionCandidateId>,
        kind: BundleActivationKind,
        actor: &str,
        target_environments: &[EnvironmentId],
        activated_at_ms: u64,
        failpoint: Option<BundleActivationFailpoint>,
    ) -> Result<BundleActivationRecord, BundleSyncError> {
        validate_actor(actor)?;
        let materialized = self
            .bundle_store
            .materialize(materialization, activated_at_ms)?;
        if let Some(current) = self.store.current_bundle_activation()?
            && current.bundle_sha == materialized.object.bundle_sha
        {
            return Ok(current);
        }
        let prior = self
            .store
            .current_bundle_activation()?
            .map(|activation| activation.bundle_sha);
        let activation = BundleActivationRecord {
            id: BundleActivationId::generate(),
            bundle_sha: materialized.object.bundle_sha.clone(),
            source_manifest: materialized.object.source_manifest.clone(),
            activated_at_ms,
            kind,
            actor: actor.to_string(),
            prior_bundle_sha: prior.clone(),
        };
        let mut targets = target_environments
            .iter()
            .filter(|environment| *environment != &self.local_environment_id)
            .cloned()
            .collect::<BTreeSet<_>>();
        targets.remove(&self.local_environment_id);
        let distributions = targets
            .into_iter()
            .map(|environment_id| BundleDistributionRecord {
                transfer_id: BundleTransferId::generate(),
                environment_id,
                bundle_sha: materialized.object.bundle_sha.clone(),
                archive_sha256: materialized.object.archive_sha256.clone(),
                expected_prior_bundle_sha: prior.clone(),
                state: BundleDistributionState::Pending,
                receipt: None,
                created_at_ms: activated_at_ms,
                updated_at_ms: activated_at_ms,
            })
            .collect::<Vec<_>>();
        let audit = ConfigurationAuditRecord {
            id: ConfigurationAuditId::generate(),
            kind: "bundle_activated".to_string(),
            actor: actor.to_string(),
            source_id: source_id.cloned(),
            candidate_id: candidate_id.cloned(),
            bundle_sha: Some(materialized.object.bundle_sha.clone()),
            details: json!({
                "activationId": activation.id,
                "priorBundleSha": prior,
                "targetEnvironmentCount": distributions.len(),
            }),
            created_at_ms: activated_at_ms,
        };
        self.store.activate_bundle_with_outbox(
            &activation,
            candidate_id,
            &audit,
            &distributions,
            failpoint,
        )?;
        Ok(activation)
    }

    pub async fn drain_environment(
        &self,
        environment_id: &EnvironmentId,
        now_ms: u64,
    ) -> BundleSyncReport {
        let pending = match self
            .store
            .list_incomplete_bundle_distributions(Some(environment_id))
        {
            Ok(pending) => pending,
            Err(_) => {
                return BundleSyncReport {
                    delivered: 0,
                    remaining: 0,
                    last_error_reason: Some("bundle_state_failed".to_string()),
                };
            }
        };
        let mut report = BundleSyncReport::default();
        for distribution in pending {
            match self.deliver(&distribution, now_ms).await {
                Ok(()) => report.delivered += 1,
                Err(error) => {
                    let _ = self
                        .store
                        .reset_bundle_distribution_pending(&distribution.transfer_id, now_ms);
                    report.last_error_reason = Some(error.reason.to_string());
                    break;
                }
            }
        }
        report.remaining = self
            .store
            .list_incomplete_bundle_distributions(Some(environment_id))
            .map(|pending| pending.len())
            .unwrap_or_default();
        report
    }

    async fn deliver(
        &self,
        distribution: &BundleDistributionRecord,
        now_ms: u64,
    ) -> Result<(), BundleSyncError> {
        if !self
            .store
            .mark_bundle_distribution_uploading(&distribution.transfer_id, now_ms)?
        {
            return Ok(());
        }
        let object = self
            .store
            .get_bundle_object(&distribution.bundle_sha)?
            .ok_or_else(|| BundleSyncError {
                reason: "bundle_object_missing",
                safe_message: "The pending bundle object is unavailable".to_string(),
                retryable: false,
            })?;
        let archive_path = self.bundle_store.archive_path(&object)?;
        let prepare = BundlePrepareUploadParams {
            schema_version: BUNDLE_PUSH_SCHEMA_VERSION,
            transfer_id: distribution.transfer_id.clone(),
            environment_id: distribution.environment_id.clone(),
            archive_sha256: distribution.archive_sha256.clone(),
            archive_bytes: object.archive_bytes,
        };
        let prepare_response = self
            .environment_router
            .forward_request(
                &distribution.environment_id,
                &RpcRequest::new(
                    now_ms as i64,
                    "bundle.prepareUpload",
                    Some(serde_json::to_value(prepare).map_err(|_| state_error())?),
                ),
            )
            .await
            .map_err(|_| transport_error())?;
        let prepared: BundlePrepareUploadResult = response_result(prepare_response)?;
        if prepared.transfer_id != distribution.transfer_id {
            return Err(integrity_error());
        }
        self.environment_router
            .upload_file(
                &distribution.environment_id,
                &archive_path,
                &prepared.private_artifact_ref,
            )
            .await
            .map_err(|_| transport_error())?;
        let push = BundlePushParams {
            schema_version: BUNDLE_PUSH_SCHEMA_VERSION,
            transfer_id: distribution.transfer_id.clone(),
            bundle_sha: distribution.bundle_sha.clone(),
            archive_sha256: distribution.archive_sha256.clone(),
            archive_bytes: object.archive_bytes,
            source_manifest: object.source_manifest.clone(),
            compiler_version: object.compiler_version.clone(),
            private_artifact_ref: prepared.private_artifact_ref.clone(),
            expected_prior_bundle_sha: distribution.expected_prior_bundle_sha.clone(),
        };
        let pushed = self
            .environment_router
            .forward_request(
                &distribution.environment_id,
                &RpcRequest::new(
                    now_ms.saturating_add(1) as i64,
                    "bundle.push",
                    Some(serde_json::to_value(push).map_err(|_| state_error())?),
                ),
            )
            .await
            .map_err(|_| transport_error());
        let pushed = match pushed {
            Ok(response) => response_result::<BundlePushResult>(response),
            Err(error) => Err(error),
        };
        let pushed = match pushed {
            Ok(pushed) => pushed,
            Err(error) => {
                let discard = BundleDiscardUploadParams {
                    transfer_id: distribution.transfer_id.clone(),
                    environment_id: distribution.environment_id.clone(),
                };
                let _ = self
                    .environment_router
                    .forward_request(
                        &distribution.environment_id,
                        &RpcRequest::new(
                            now_ms.saturating_add(2) as i64,
                            "bundle.discardUpload",
                            serde_json::to_value(discard).ok(),
                        ),
                    )
                    .await;
                return Err(error);
            }
        };
        if pushed.transfer_id != distribution.transfer_id
            || pushed.stored_bundle_sha != distribution.bundle_sha
            || pushed.active_bundle_sha != distribution.bundle_sha
            || pushed.prior_bundle_sha != distribution.expected_prior_bundle_sha
            || pushed.status != BundleDistributionState::Active
        {
            return Err(integrity_error());
        }
        self.store
            .complete_bundle_distribution(&BundleDistributionReceipt {
                transfer_id: pushed.transfer_id,
                environment_id: distribution.environment_id.clone(),
                bundle_sha: distribution.bundle_sha.clone(),
                archive_sha256: distribution.archive_sha256.clone(),
                stored_bundle_sha: pushed.stored_bundle_sha,
                active_bundle_sha: pushed.active_bundle_sha,
                prior_bundle_sha: pushed.prior_bundle_sha,
                state: pushed.status,
                received_at_ms: pushed.received_at_ms,
            })?;
        Ok(())
    }

    pub fn bundle_store(&self) -> &BundleStore {
        &self.bundle_store
    }
}

fn response_result<T: serde::de::DeserializeOwned>(
    response: pandamux_protocol::RpcResponse,
) -> Result<T, BundleSyncError> {
    if let Some(error) = response.error {
        let reason = error
            .data
            .as_ref()
            .and_then(|data| data.get("reason"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("node_bundle_rejected");
        return Err(BundleSyncError {
            reason: if reason == "bundle_integrity_failed" {
                "node_bundle_rejected"
            } else {
                "bundle_delivery_failed"
            },
            safe_message: "The node did not accept the exact bundle transfer".to_string(),
            retryable: true,
        });
    }
    serde_json::from_value(response.result.ok_or_else(integrity_error)?)
        .map_err(|_| integrity_error())
}

fn validate_actor(actor: &str) -> Result<(), BundleSyncError> {
    if actor.trim().is_empty() || actor.len() > 256 || actor.chars().any(char::is_control) {
        return Err(BundleSyncError {
            reason: "bundle_actor_invalid",
            safe_message: "The bundle activation actor is invalid".to_string(),
            retryable: false,
        });
    }
    Ok(())
}

fn state_error() -> BundleSyncError {
    BundleSyncError {
        reason: "bundle_state_failed",
        safe_message: "The bundle state could not be read or persisted".to_string(),
        retryable: true,
    }
}

fn transport_error() -> BundleSyncError {
    BundleSyncError {
        reason: "node_bundle_pending",
        safe_message: "The node is not ready to receive this bundle".to_string(),
        retryable: true,
    }
}

fn integrity_error() -> BundleSyncError {
    BundleSyncError {
        reason: "node_bundle_rejected",
        safe_message: "The node returned a bundle receipt with mismatched identities".to_string(),
        retryable: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle_store::tests::materialization_fixture;
    use crate::{DataPaths, MockEnvironmentTransport};
    use pandamux_core::{
        BudgetAmount, OrchestrationMode, OrganizationId, Run, RunId, RunSeed, RunTrigger,
    };
    use pandamux_protocol::{RpcError, RpcResponse};
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn offline_outbox_drains_once_and_existing_run_stays_pinned() {
        let temp = tempfile::tempdir().expect("temporary sync fixture");
        let input = materialization_fixture(temp.path());
        let hub_paths = DataPaths::from_root(temp.path().join("hub")).expect("hub paths");
        hub_paths.ensure_private_roots().expect("hub roots");
        let hub_store = Store::in_memory().expect("hub store");
        let hub_bundle_store = BundleStore::new(hub_paths, hub_store.clone());
        let local_environment = EnvironmentId::new("environment-hub");
        let remote_environment = EnvironmentId::new("environment-node");
        let environment_router = Arc::new(EnvironmentRouter::new(local_environment.clone()));
        let sync = BundleSyncService::new(
            hub_store.clone(),
            hub_bundle_store,
            environment_router.clone(),
            local_environment,
        );
        let activation = sync
            .activate(
                &input,
                None,
                None,
                BundleActivationKind::TrustedBranch,
                "local-control",
                std::slice::from_ref(&remote_environment),
                10,
                None,
            )
            .expect("activate hub bundle");
        let run = Run::draft(
            RunId::new("run-pinned-bundle"),
            RunSeed {
                trigger: RunTrigger::Manual {
                    client_id: "test".to_string(),
                },
                policy_revision: "settings:1".to_string(),
                bundle_sha: activation.bundle_sha.as_str().to_string(),
                organization_id: OrganizationId::new("organization-bundle-test"),
                mode: OrchestrationMode::ApproveFirst,
                budget_limit: BudgetAmount::default(),
                max_parallel_tasks: 1,
            },
            10,
        );
        let offline = sync.drain_environment(&remote_environment, 20).await;
        assert_eq!(offline.delivered, 0);
        assert_eq!(offline.remaining, 1);
        assert_eq!(
            offline.last_error_reason.as_deref(),
            Some("node_bundle_pending")
        );

        let node_paths = DataPaths::from_root(temp.path().join("node")).expect("node paths");
        node_paths.ensure_private_roots().expect("node roots");
        let node_store = Store::in_memory().expect("node store");
        let node_bundle_store = BundleStore::new(node_paths, node_store.clone());
        let uploads = Arc::new(AtomicUsize::new(0));
        let pushes = Arc::new(AtomicUsize::new(0));
        let responder_store = node_bundle_store.clone();
        let responder_environment = remote_environment.clone();
        let responder_pushes = pushes.clone();
        let responder = move |request: &RpcRequest| -> Result<RpcResponse, RpcError> {
            let id = request.id.clone().unwrap_or_else(|| 1.into());
            match request.method.as_str() {
                "bundle.prepareUpload" => {
                    let params: BundlePrepareUploadParams =
                        serde_json::from_value(request.params.clone().unwrap_or_default())
                            .map_err(|error| RpcError::invalid_params(error.to_string()))?;
                    let path = responder_store
                        .prepare_upload(&params.transfer_id)
                        .map_err(|error| RpcError::internal_error(error.to_string()))?;
                    Ok(RpcResponse::success(
                        id,
                        serde_json::to_value(BundlePrepareUploadResult {
                            transfer_id: params.transfer_id,
                            private_artifact_ref: path.to_string_lossy().into_owned(),
                        })
                        .expect("prepare result"),
                    ))
                }
                "bundle.push" => {
                    responder_pushes.fetch_add(1, Ordering::SeqCst);
                    let params: BundlePushParams =
                        serde_json::from_value(request.params.clone().unwrap_or_default())
                            .map_err(|error| RpcError::invalid_params(error.to_string()))?;
                    let result = responder_store
                        .receive_uploaded_bundle(&responder_environment, &params, 30)
                        .map_err(|error| RpcError::internal_error(error.to_string()))?;
                    Ok(RpcResponse::success(
                        id,
                        serde_json::to_value(result).expect("push result"),
                    ))
                }
                "bundle.discardUpload" => Ok(RpcResponse::success(
                    id,
                    serde_json::json!({ "discarded": true }),
                )),
                method => Err(RpcError::method_not_found(method)),
            }
        };
        let uploader_store = uploads.clone();
        let transport = MockEnvironmentTransport::new(responder).with_uploader(
            move |local_path, remote_path| {
                uploader_store.fetch_add(1, Ordering::SeqCst);
                fs::copy(local_path, remote_path)
                    .map(|_| ())
                    .map_err(|error| RpcError::internal_error(error.to_string()))
            },
        );
        environment_router
            .register_transport(remote_environment.clone(), Arc::new(transport), None)
            .await;

        let delivered = sync.drain_environment(&remote_environment, 30).await;
        assert_eq!(delivered.delivered, 1);
        assert_eq!(delivered.remaining, 0);
        assert_eq!(uploads.load(Ordering::SeqCst), 1);
        assert_eq!(pushes.load(Ordering::SeqCst), 1);
        assert_eq!(
            node_store
                .current_bundle_activation()
                .expect("node activation")
                .expect("active node bundle")
                .bundle_sha,
            activation.bundle_sha
        );

        let replay = sync.drain_environment(&remote_environment, 40).await;
        assert_eq!(replay.delivered, 0);
        assert_eq!(replay.remaining, 0);
        assert_eq!(uploads.load(Ordering::SeqCst), 1);
        assert_eq!(pushes.load(Ordering::SeqCst), 1);
        assert_eq!(run.bundle_sha, activation.bundle_sha.as_str());
    }
}
