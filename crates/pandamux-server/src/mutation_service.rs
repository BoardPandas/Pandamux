use crate::personal_repo::{PersonalRepoError, PersonalRepoService};
use crate::request_context::{PeerRole, RequestContext};
use crate::store::{MutationApplyFailpoint, MutationProposalSave, Store, StoreError};
use pandamux_core::{
    ConfigurationRevision, ContentSha256, MutationCaller, MutationImpact, MutationProposal,
    MutationProposalId, MutationResolution, MutationTarget, SourceCommitSha,
};
use pandamux_orchestrator::{canonical_json_bytes, canonical_json_sha256};
use pandamux_protocol::{MutationListParams, MutationListResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const DEFAULT_MUTATION_TTL_MS: u64 = 10 * 60 * 1_000;
pub const MUTATION_SWEEP_INTERVAL: Duration = Duration::from_secs(30);
const MAX_MUTATION_JSON_BYTES: usize = 1024 * 1024;
const MAX_MUTATION_TEXT_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct MutationProposalRequest {
    pub request_id: String,
    pub tool_name: String,
    pub target: MutationTarget,
    pub expected_revision: ConfigurationRevision,
    pub old_json: Value,
    pub new_json: Value,
    pub impact: MutationImpact,
    pub created_at_ms: u64,
    pub ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MutationProposalOutcome {
    pub proposal: MutationProposal,
    pub created: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMutationState {
    pub head_source_commit_sha: SourceCommitSha,
    pub target_source_commit_sha: SourceCommitSha,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutationServiceError {
    pub reason: &'static str,
    pub safe_message: String,
    pub retryable: bool,
}

impl std::fmt::Display for MutationServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.safe_message)
    }
}

impl std::error::Error for MutationServiceError {}

impl From<StoreError> for MutationServiceError {
    fn from(error: StoreError) -> Self {
        let text = error.to_string();
        if text.contains("request identity conflicts") {
            return Self::new(
                "request_identity_conflict",
                "This caller already used the request identity for different proposal content",
                false,
            );
        }
        if text.contains("terminal resolution") || text.contains("is not pending") {
            return Self::new(
                "proposal_already_resolved",
                "The mutation proposal already has a terminal result",
                false,
            );
        }
        Self::new(
            "mutation_store_failed",
            "The mutation proposal could not be persisted or resolved",
            true,
        )
    }
}

impl From<PersonalRepoError> for MutationServiceError {
    fn from(error: PersonalRepoError) -> Self {
        Self::new(error.reason, error.safe_message, error.retryable)
    }
}

impl MutationServiceError {
    fn new(reason: &'static str, safe_message: impl Into<String>, retryable: bool) -> Self {
        Self {
            reason,
            safe_message: safe_message.into(),
            retryable,
        }
    }
}

#[derive(Clone)]
pub struct MutationService {
    store: Store,
    personal: PersonalRepoService,
}

impl MutationService {
    pub fn new(store: Store, personal: PersonalRepoService) -> Self {
        Self { store, personal }
    }

    pub fn caller(&self, context: &RequestContext) -> MutationCaller {
        caller_projection(context)
    }

    pub fn propose(
        &self,
        context: &RequestContext,
        request: MutationProposalRequest,
    ) -> Result<MutationProposalOutcome, MutationServiceError> {
        authorize_proposer(context)?;
        validate_text(&request.request_id, "request ID")?;
        validate_text(&request.tool_name, "tool name")?;
        validate_json(&request.old_json)?;
        validate_json(&request.new_json)?;
        let ttl_ms = request.ttl_ms.unwrap_or(DEFAULT_MUTATION_TTL_MS);
        if ttl_ms == 0 || ttl_ms > DEFAULT_MUTATION_TTL_MS {
            return Err(MutationServiceError::new(
                "mutation_ttl_invalid",
                "Mutation proposal expiry must be within ten minutes",
                false,
            ));
        }
        let old_json = canonical_value(&request.old_json)?;
        let new_json = canonical_value(&request.new_json)?;
        let caller = caller_projection(context);
        let caller_identity = caller_identity(context);
        let proposal = MutationProposal {
            id: MutationProposalId::generate(),
            request_id: request.request_id,
            caller,
            tool_name: request.tool_name,
            environment_id: context.authenticated_environment_id.clone(),
            target: request.target,
            expected_revision: request.expected_revision,
            old_digest: content_digest(&old_json)?,
            new_digest: content_digest(&new_json)?,
            old_json,
            new_json,
            impact: request.impact,
            created_at_ms: request.created_at_ms,
            expires_at_ms: request.created_at_ms.saturating_add(ttl_ms),
            resolution: MutationResolution::Pending,
            audit_id: None,
        };
        let MutationProposalSave { proposal, created } = self
            .store
            .save_mutation_proposal(&proposal, &caller_identity)?;
        Ok(MutationProposalOutcome { proposal, created })
    }

    pub fn get(
        &self,
        context: &RequestContext,
        proposal_id: &MutationProposalId,
        now_ms: u64,
    ) -> Result<MutationProposal, MutationServiceError> {
        authorize_local_resolver(context)?;
        self.sweep(now_ms)?;
        self.store
            .get_mutation_proposal(proposal_id)?
            .ok_or_else(|| {
                MutationServiceError::new(
                    "mutation_proposal_not_found",
                    "The mutation proposal was not found",
                    false,
                )
            })
    }

    pub fn list(
        &self,
        context: &RequestContext,
        params: &MutationListParams,
        now_ms: u64,
    ) -> Result<MutationListResult, MutationServiceError> {
        authorize_local_resolver(context)?;
        self.sweep(now_ms)?;
        let (proposals, has_more) = self.store.list_mutation_proposals(
            params.include_terminal,
            params.before_created_at_ms,
            params.limit.unwrap_or(50),
        )?;
        Ok(MutationListResult {
            proposals,
            has_more,
        })
    }

    pub fn approve_once(
        &self,
        context: &RequestContext,
        proposal_id: &MutationProposalId,
        applied_at_ms: u64,
        failpoint: Option<MutationApplyFailpoint>,
    ) -> Result<MutationProposal, MutationServiceError> {
        authorize_local_resolver(context)?;
        let resolver = caller_projection(context);
        let proposal = self
            .store
            .get_mutation_proposal(proposal_id)?
            .ok_or_else(|| {
                MutationServiceError::new(
                    "mutation_proposal_not_found",
                    "The mutation proposal was not found",
                    false,
                )
            })?;
        if proposal.resolution == MutationResolution::ApprovedApplied {
            return Ok(proposal);
        }
        if applied_at_ms >= proposal.expires_at_ms {
            self.sweep(applied_at_ms)?;
            return self
                .store
                .get_mutation_proposal(proposal_id)?
                .ok_or_else(|| {
                    MutationServiceError::new(
                        "mutation_proposal_not_found",
                        "The mutation proposal was not found",
                        false,
                    )
                });
        }
        if proposal.target != MutationTarget::PersonalConfiguration {
            return self
                .store
                .apply_db_mutation_proposal(proposal_id, &resolver, applied_at_ms, failpoint)
                .map_err(mutation_apply_error);
        }

        let old: PersonalMutationState = serde_json::from_value(proposal.old_json.clone())
            .map_err(|_| invalid_personal_state())?;
        let new: PersonalMutationState = serde_json::from_value(proposal.new_json.clone())
            .map_err(|_| invalid_personal_state())?;
        if old.head_source_commit_sha != new.head_source_commit_sha
            || old.target_source_commit_sha != old.head_source_commit_sha
            || new.target_source_commit_sha == new.head_source_commit_sha
        {
            return Err(invalid_personal_state());
        }
        let result = self.personal.revert(
            proposal.id.as_str().to_string(),
            resolver.client_id.clone(),
            applied_at_ms,
            None,
            new.head_source_commit_sha,
            new.target_source_commit_sha,
        )?;
        self.store
            .finalize_personal_mutation_proposal(
                proposal_id,
                &resolver,
                result.revision,
                applied_at_ms,
                failpoint,
            )
            .map_err(mutation_apply_error)
    }

    pub fn reject(
        &self,
        context: &RequestContext,
        proposal_id: &MutationProposalId,
        now_ms: u64,
    ) -> Result<MutationProposal, MutationServiceError> {
        authorize_local_resolver(context)?;
        self.store
            .reject_mutation_proposal(proposal_id, now_ms)
            .map_err(Into::into)
    }

    pub fn cancel_owned(
        &self,
        context: &RequestContext,
        now_ms: u64,
    ) -> Result<usize, MutationServiceError> {
        if context.connection_id.is_empty() {
            return Ok(0);
        }
        self.store
            .cancel_mutation_proposals_for_caller(&caller_identity(context), now_ms)
            .map_err(Into::into)
    }

    pub fn sweep(&self, now_ms: u64) -> Result<usize, MutationServiceError> {
        self.store
            .expire_mutation_proposals(now_ms)
            .map_err(Into::into)
    }
}

pub struct MutationSweepService {
    state: Arc<(Mutex<bool>, Condvar)>,
    owner: Option<JoinHandle<()>>,
}

impl MutationSweepService {
    pub fn start(service: MutationService) -> std::io::Result<Self> {
        let state = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_state = state.clone();
        let owner = thread::Builder::new()
            .name("pandamux-mutation-sweep".to_string())
            .spawn(move || {
                let (lock, wake) = &*worker_state;
                let mut stopping = lock.lock().expect("mutation sweep lock");
                loop {
                    let waited = wake
                        .wait_timeout(stopping, MUTATION_SWEEP_INTERVAL)
                        .expect("mutation sweep wait");
                    stopping = waited.0;
                    if *stopping {
                        break;
                    }
                    drop(stopping);
                    let _ = service.sweep(now_ms());
                    stopping = lock.lock().expect("mutation sweep lock");
                }
            })?;
        Ok(Self {
            state,
            owner: Some(owner),
        })
    }

    pub fn shutdown(mut self) -> std::io::Result<()> {
        let (lock, wake) = &*self.state;
        *lock.lock().expect("mutation sweep lock") = true;
        wake.notify_all();
        if let Some(owner) = self.owner.take() {
            owner.join().map_err(|_| {
                std::io::Error::other("mutation sweep owner thread panicked during shutdown")
            })?;
        }
        Ok(())
    }
}

impl Drop for MutationSweepService {
    fn drop(&mut self) {
        let (lock, wake) = &*self.state;
        *lock.lock().expect("mutation sweep lock") = true;
        wake.notify_all();
        if let Some(owner) = self.owner.take() {
            let _ = owner.join();
        }
    }
}

fn authorize_proposer(context: &RequestContext) -> Result<(), MutationServiceError> {
    if matches!(
        context.peer_role,
        PeerRole::Internal | PeerRole::LocalControl | PeerRole::McpAdapter
    ) {
        Ok(())
    } else {
        Err(MutationServiceError::new(
            "capability_scope_denied",
            "This authenticated peer cannot create mutation proposals",
            false,
        ))
    }
}

fn authorize_local_resolver(context: &RequestContext) -> Result<(), MutationServiceError> {
    if context.peer_role == PeerRole::LocalControl {
        Ok(())
    } else {
        Err(MutationServiceError::new(
            "capability_scope_denied",
            "Mutation proposals can only be resolved by authenticated local control",
            false,
        ))
    }
}

fn caller_projection(context: &RequestContext) -> MutationCaller {
    let role = match context.peer_role {
        PeerRole::Internal => "internal",
        PeerRole::LocalControl => "local_control",
        PeerRole::McpAdapter => "mcp_adapter",
        PeerRole::ProviderChild => "provider_child",
        PeerRole::NodeProxy => "node_proxy",
    };
    let display_name = bounded_display_name(
        context.client_identity.description.trim(),
        match context.peer_role {
            PeerRole::Internal => "PandaMUX server",
            PeerRole::LocalControl => "Local PandaMUX control",
            PeerRole::McpAdapter => "Local MCP adapter",
            PeerRole::ProviderChild => "Provider child",
            PeerRole::NodeProxy => "Authenticated node proxy",
        },
    );
    MutationCaller {
        client_id: caller_identity(context),
        peer_role: role.to_string(),
        display_name,
    }
}

fn caller_identity(context: &RequestContext) -> String {
    let mut hasher = Sha256::new();
    hasher.update(match context.peer_role {
        PeerRole::Internal => b"internal".as_slice(),
        PeerRole::LocalControl => b"local_control".as_slice(),
        PeerRole::McpAdapter => b"mcp_adapter".as_slice(),
        PeerRole::ProviderChild => b"provider_child".as_slice(),
        PeerRole::NodeProxy => b"node_proxy".as_slice(),
    });
    hasher.update([0]);
    hasher.update(context.authenticated_environment_id.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(context.connection_id.as_bytes());
    let digest = hasher.finalize();
    format!("caller-{}", hex_lower(&digest))
}

fn bounded_display_name(candidate: &str, fallback: &str) -> String {
    let value = if candidate.is_empty() {
        fallback
    } else {
        candidate
    };
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(80)
        .collect()
}

fn validate_text(value: &str, label: &str) -> Result<(), MutationServiceError> {
    if value.trim().is_empty()
        || value.len() > MAX_MUTATION_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(MutationServiceError::new(
            "mutation_metadata_invalid",
            format!("Mutation {label} is invalid"),
            false,
        ));
    }
    Ok(())
}

fn validate_json(value: &Value) -> Result<(), MutationServiceError> {
    if canonical_json_bytes(value).len() > MAX_MUTATION_JSON_BYTES {
        return Err(MutationServiceError::new(
            "mutation_content_too_large",
            "Mutation proposal content exceeds the supported size",
            false,
        ));
    }
    Ok(())
}

fn canonical_value(value: &Value) -> Result<Value, MutationServiceError> {
    serde_json::from_slice(&canonical_json_bytes(value)).map_err(|_| {
        MutationServiceError::new(
            "mutation_content_invalid",
            "Mutation proposal content is not canonical JSON",
            false,
        )
    })
}

fn content_digest(value: &Value) -> Result<ContentSha256, MutationServiceError> {
    ContentSha256::parse(canonical_json_sha256(value)).map_err(|_| {
        MutationServiceError::new(
            "mutation_content_invalid",
            "Mutation proposal content digest is invalid",
            false,
        )
    })
}

fn invalid_personal_state() -> MutationServiceError {
    MutationServiceError::new(
        "personal_mutation_invalid",
        "The personal configuration proposal has invalid revision state",
        false,
    )
}

fn mutation_apply_error(error: StoreError) -> MutationServiceError {
    let mapped = MutationServiceError::from(error);
    if mapped.reason == "mutation_store_failed" {
        MutationServiceError::new(
            "proposal_apply_failed",
            "The reviewed mutation could not be applied atomically",
            true,
        )
    } else {
        mapped
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_context::{CapabilityScope, ClientIdentity};
    use crate::{DataPaths, PersonalEditRequest, PersonalRepoService};
    use pandamux_core::{AgentId, EnvironmentId, TrustedSettingsMutationState};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Barrier};

    struct Fixture {
        service: MutationService,
        personal: PersonalRepoService,
        store: Store,
        _temp: tempfile::TempDir,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.personal.shutdown();
        }
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().expect("temporary mutation fixture");
        let paths = DataPaths::from_root(temp.path().join("app")).expect("data paths");
        paths.ensure_private_roots().expect("private roots");
        let store = Store::in_memory().expect("mutation store");
        let personal = PersonalRepoService::new(paths, store.clone());
        let service = MutationService::new(store.clone(), personal.clone());
        Fixture {
            service,
            personal,
            store,
            _temp: temp,
        }
    }

    fn context(role: PeerRole, connection_id: &str) -> RequestContext {
        let capability_scope = match role {
            PeerRole::Internal => CapabilityScope::Internal,
            PeerRole::LocalControl => CapabilityScope::Control,
            PeerRole::McpAdapter => CapabilityScope::Mcp,
            PeerRole::ProviderChild => CapabilityScope::ProviderChild {
                thread_id: pandamux_core::ThreadId::new("thread-mutation-test"),
                methods: Default::default(),
                expires_at_ms: u64::MAX,
            },
            PeerRole::NodeProxy => CapabilityScope::NodeProxy,
        };
        RequestContext {
            peer_role: role,
            authenticated_environment_id: EnvironmentId::new("env-local"),
            client_identity: ClientIdentity {
                description: "Mutation test caller".to_string(),
                ..ClientIdentity::default()
            },
            capability_scope,
            connection_id: connection_id.to_string(),
        }
    }

    fn settings_request(
        store: &Store,
        request_id: &str,
        agent_id: &str,
        created_at_ms: u64,
        ttl_ms: Option<u64>,
    ) -> MutationProposalRequest {
        let current = store
            .get_or_initialize_settings(created_at_ms)
            .expect("settings state");
        let old = TrustedSettingsMutationState {
            trusted_configuration: current.settings.trusted_configuration.clone(),
        };
        let mut new = old.clone();
        new.trusted_configuration
            .full_access_agent_ids
            .insert(AgentId::new(agent_id));
        let mut impact = MutationImpact::default();
        impact.agent_ids.insert(AgentId::new(agent_id));
        impact
            .full_access_summaries
            .insert("Grant one exact agent FullAccess".to_string());
        MutationProposalRequest {
            request_id: request_id.to_string(),
            tool_name: "settings.set.trustedConfiguration".to_string(),
            target: MutationTarget::Settings,
            expected_revision: current.revision,
            old_json: serde_json::to_value(old).expect("old settings projection"),
            new_json: serde_json::to_value(new).expect("new settings projection"),
            impact,
            created_at_ms,
            ttl_ms,
        }
    }

    #[test]
    fn canonical_identity_deduplicates_key_order_and_conflicts_fail_closed() {
        let fixture = fixture();
        let caller = context(PeerRole::McpAdapter, "mcp-canonical");
        let mut first = settings_request(
            &fixture.store,
            "request-canonical",
            "agent-canonical",
            1_000,
            None,
        );
        first.old_json = serde_json::from_str(
            r#"{"trustedConfiguration":{"disabledAgentIds":[],"fullAccessAgentIds":[]}}"#,
        )
        .expect("ordered old JSON");
        first.new_json = serde_json::from_str(
            r#"{"trustedConfiguration":{"disabledAgentIds":[],"fullAccessAgentIds":["agent-canonical"]}}"#,
        )
        .expect("ordered new JSON");
        let created = fixture
            .service
            .propose(&caller, first.clone())
            .expect("first proposal");
        assert!(created.created);

        let mut reordered = first.clone();
        reordered.created_at_ms = 1_001;
        reordered.old_json = serde_json::from_str(
            r#"{"trustedConfiguration":{"fullAccessAgentIds":[],"disabledAgentIds":[]}}"#,
        )
        .expect("reordered old JSON");
        reordered.new_json = serde_json::from_str(
            r#"{"trustedConfiguration":{"fullAccessAgentIds":["agent-canonical"],"disabledAgentIds":[]}}"#,
        )
        .expect("reordered new JSON");
        let duplicate = fixture
            .service
            .propose(&caller, reordered.clone())
            .expect("canonical duplicate");
        assert!(!duplicate.created);
        assert_eq!(duplicate.proposal.id, created.proposal.id);
        assert_eq!(duplicate.proposal.old_digest, created.proposal.old_digest);
        assert_eq!(duplicate.proposal.new_digest, created.proposal.new_digest);

        reordered.new_json = json!({
            "trustedConfiguration": {
                "disabledAgentIds": [],
                "fullAccessAgentIds": ["agent-conflict"]
            }
        });
        let conflict = fixture
            .service
            .propose(&caller, reordered)
            .expect_err("changed bytes must conflict");
        assert_eq!(conflict.reason, "request_identity_conflict");
        assert_eq!(
            fixture
                .store
                .list_mutation_proposals(true, None, 100)
                .expect("stored proposals")
                .0
                .len(),
            1
        );
    }

    #[test]
    fn rejection_expiry_disconnect_and_revision_drift_never_mutate() {
        let fixture = fixture();
        let local = context(PeerRole::LocalControl, "local-lifecycle");
        let mcp = context(PeerRole::McpAdapter, "mcp-lifecycle");
        let baseline = fixture
            .store
            .get_or_initialize_settings(1_000)
            .expect("baseline settings");

        let rejected = fixture
            .service
            .propose(
                &mcp,
                settings_request(
                    &fixture.store,
                    "request-reject",
                    "agent-reject",
                    1_000,
                    None,
                ),
            )
            .expect("rejection proposal")
            .proposal;
        let first_reject = fixture
            .service
            .reject(&local, &rejected.id, 1_010)
            .expect("reject proposal");
        let retry_reject = fixture
            .service
            .reject(&local, &rejected.id, 1_011)
            .expect("retry rejection");
        assert_eq!(first_reject.resolution, MutationResolution::Rejected);
        assert_eq!(retry_reject.resolution, MutationResolution::Rejected);
        assert_eq!(
            fixture
                .service
                .approve_once(&local, &rejected.id, 1_012, None)
                .expect_err("rejected proposal cannot apply")
                .reason,
            "proposal_already_resolved"
        );

        let expired = fixture
            .service
            .propose(
                &mcp,
                settings_request(
                    &fixture.store,
                    "request-expire",
                    "agent-expire",
                    2_000,
                    Some(10),
                ),
            )
            .expect("expiry proposal")
            .proposal;
        let expired = fixture
            .service
            .approve_once(&local, &expired.id, 2_010, None)
            .expect("expiry settles safely");
        assert_eq!(expired.resolution, MutationResolution::Expired);

        let disconnected = fixture
            .service
            .propose(
                &mcp,
                settings_request(
                    &fixture.store,
                    "request-disconnect",
                    "agent-disconnect",
                    3_000,
                    None,
                ),
            )
            .expect("disconnect proposal")
            .proposal;
        assert_eq!(
            fixture.service.cancel_owned(&mcp, 3_010).expect("cancel"),
            1
        );
        assert_eq!(
            fixture
                .service
                .get(&local, &disconnected.id, 3_011)
                .expect("cancelled proposal")
                .resolution,
            MutationResolution::CancelledDisconnected
        );
        assert_eq!(
            fixture
                .service
                .approve_once(&local, &disconnected.id, 3_012, None)
                .expect_err("disconnected proposal cannot apply")
                .reason,
            "proposal_already_resolved"
        );

        let stale = fixture
            .service
            .propose(
                &local,
                settings_request(&fixture.store, "request-stale", "agent-stale", 4_000, None),
            )
            .expect("stale proposal")
            .proposal;
        let mut intervening = baseline.settings.clone();
        intervening.ui.show_status_bar = !intervening.ui.show_status_bar;
        let intervening_revision = fixture
            .store
            .save_settings_cas(&intervening, baseline.revision, 4_010)
            .expect("intervening settings change");
        let superseded = fixture
            .service
            .approve_once(&local, &stale.id, 4_020, None)
            .expect("stale proposal settles safely");
        assert_eq!(
            superseded.resolution,
            MutationResolution::SupersededRevision
        );

        let current = fixture
            .store
            .get_or_initialize_settings(4_030)
            .expect("current settings");
        assert_eq!(current.revision, intervening_revision);
        assert!(
            current
                .settings
                .trusted_configuration
                .full_access_agent_ids
                .is_empty()
        );
        for proposal in [rejected, expired, disconnected, superseded] {
            assert!(
                fixture
                    .store
                    .get_mutation_audit_for_proposal(&proposal.id)
                    .expect("audit query")
                    .is_none()
            );
        }
    }

    #[test]
    fn apply_is_atomic_redacted_and_response_retry_is_idempotent() {
        let fixture = fixture();
        let local = context(PeerRole::LocalControl, "local-atomic");
        let mut record = fixture
            .store
            .get_or_initialize_settings(1_000)
            .expect("settings state");
        record.settings.providers[0].env_overrides.insert(
            "TEST_SECRET".to_string(),
            "mutation-secret-sentinel".to_string(),
        );
        record.revision = fixture
            .store
            .save_settings_cas(&record.settings, record.revision, 1_001)
            .expect("seed secret settings");

        let proposal = fixture
            .service
            .propose(
                &local,
                settings_request(
                    &fixture.store,
                    "request-atomic",
                    "agent-atomic",
                    2_000,
                    None,
                ),
            )
            .expect("atomic proposal")
            .proposal;
        let stored_json = serde_json::to_string(&proposal).expect("proposal JSON");
        assert!(!stored_json.contains("mutation-secret-sentinel"));
        assert!(!stored_json.contains("TEST_SECRET"));

        let failure = fixture
            .service
            .approve_once(
                &local,
                &proposal.id,
                2_010,
                Some(MutationApplyFailpoint::BeforeCommit),
            )
            .expect_err("failpoint must roll back");
        assert_eq!(failure.reason, "proposal_apply_failed");
        let after_failure = fixture
            .store
            .get_or_initialize_settings(2_011)
            .expect("settings after rollback");
        assert_eq!(after_failure.revision, record.revision);
        assert!(
            after_failure
                .settings
                .trusted_configuration
                .full_access_agent_ids
                .is_empty()
        );
        assert_eq!(
            fixture
                .store
                .get_mutation_proposal(&proposal.id)
                .expect("proposal after rollback")
                .expect("proposal remains")
                .resolution,
            MutationResolution::Pending
        );
        assert!(
            fixture
                .store
                .get_mutation_audit_for_proposal(&proposal.id)
                .expect("audit after rollback")
                .is_none()
        );

        let applied = fixture
            .service
            .approve_once(&local, &proposal.id, 2_020, None)
            .expect("apply proposal");
        assert_eq!(applied.resolution, MutationResolution::ApprovedApplied);
        let audit_id = applied.audit_id.clone().expect("durable audit identity");
        let applied_settings = fixture
            .store
            .get_or_initialize_settings(2_021)
            .expect("applied settings");
        assert_eq!(
            applied_settings.revision,
            record.revision.checked_next().expect("next revision")
        );
        assert!(
            applied_settings
                .settings
                .trusted_configuration
                .full_access_agent_ids
                .contains(&AgentId::new("agent-atomic"))
        );
        let audit = fixture
            .store
            .get_mutation_audit_for_proposal(&proposal.id)
            .expect("audit query")
            .expect("audit record");
        assert_eq!(audit.id, audit_id);
        assert_eq!(audit.old_digest, proposal.old_digest);
        assert_eq!(audit.new_digest, proposal.new_digest);

        let replay = fixture
            .service
            .approve_once(&local, &proposal.id, 2_030, None)
            .expect("lost response retry");
        assert_eq!(replay.audit_id, Some(audit_id));
        assert_eq!(
            fixture
                .store
                .get_or_initialize_settings(2_031)
                .expect("settings after replay")
                .revision,
            applied_settings.revision
        );
    }

    #[test]
    fn git_backed_revert_reconciles_a_lost_audit_response_once() {
        let fixture = fixture();
        let local = context(PeerRole::LocalControl, "local-personal-revert");
        let base = fixture
            .personal
            .ensure_initialized()
            .expect("initialize personal repository");
        let agent = b"---\nschemaVersion: 1\nkind: pandamux.agent\nid: agent-personal\nname: Personal\ndescription: Personal mutation fixture\nspecialties: []\ntier: fast\neffort: low\naccessMode: read_only\nrequiredCapabilities: []\nproviders: { prefer: [], avoid: [] }\nenvironments: [local]\nknowledge: []\ntools: { allow: [], deny: [] }\noutput: { kind: change_set }\nverification: []\nconcurrency: 1\nbudget: { maxTokens: 1000, maxCostMicroUsd: 1000, maxRuntimeMs: 1000 }\n---\nBody\n".to_vec();
        let edited = fixture
            .personal
            .apply_edit(&PersonalEditRequest {
                request_id: "seed-personal-edit".to_string(),
                caller: "mutation-test".to_string(),
                accepted_at_ms: 2_000,
                originating_run_id: None,
                expected_head: base.clone(),
                files: BTreeMap::from([("agents/agent-personal.md".to_string(), agent)]),
            })
            .expect("seed personal edit");
        let state = fixture
            .store
            .get_personal_repo_state()
            .expect("personal state query")
            .expect("personal state");
        assert_eq!(state.head_source_commit_sha, edited.source_commit_sha);

        let proposal = fixture
            .service
            .propose(
                &local,
                MutationProposalRequest {
                    request_id: "personal-revert-proposal".to_string(),
                    tool_name: "personalConfig.revert".to_string(),
                    target: MutationTarget::PersonalConfiguration,
                    expected_revision: state.revision,
                    old_json: serde_json::to_value(PersonalMutationState {
                        head_source_commit_sha: state.head_source_commit_sha.clone(),
                        target_source_commit_sha: state.head_source_commit_sha.clone(),
                    })
                    .expect("old personal state"),
                    new_json: serde_json::to_value(PersonalMutationState {
                        head_source_commit_sha: state.head_source_commit_sha.clone(),
                        target_source_commit_sha: base,
                    })
                    .expect("new personal state"),
                    impact: MutationImpact::default(),
                    created_at_ms: 3_000,
                    ttl_ms: None,
                },
            )
            .expect("personal revert proposal")
            .proposal;
        fixture
            .service
            .approve_once(
                &local,
                &proposal.id,
                3_010,
                Some(MutationApplyFailpoint::BeforeCommit),
            )
            .expect_err("injected audit failure");
        let after_ref = fixture
            .store
            .get_personal_repo_state()
            .expect("personal state after ref")
            .expect("personal state after ref");
        assert_eq!(
            after_ref.revision,
            state
                .revision
                .checked_next()
                .expect("next personal revision")
        );
        assert_eq!(
            fixture
                .store
                .get_mutation_proposal(&proposal.id)
                .expect("proposal query")
                .expect("proposal")
                .resolution,
            MutationResolution::Pending
        );
        assert!(
            fixture
                .store
                .get_mutation_audit_for_proposal(&proposal.id)
                .expect("audit query")
                .is_none()
        );

        let reconciled = fixture
            .service
            .approve_once(&local, &proposal.id, 3_020, None)
            .expect("reconcile personal result");
        assert_eq!(reconciled.resolution, MutationResolution::ApprovedApplied);
        let (head, documents) = fixture
            .personal
            .current_documents()
            .expect("reverted personal documents");
        assert_eq!(head, after_ref.head_source_commit_sha);
        assert!(documents.is_empty());
        let replay = fixture
            .service
            .approve_once(&local, &proposal.id, 3_030, None)
            .expect("personal response replay");
        assert_eq!(replay.audit_id, reconciled.audit_id);
        assert_eq!(
            fixture
                .store
                .get_personal_repo_state()
                .expect("personal state after retry")
                .expect("personal state after retry")
                .revision,
            after_ref.revision
        );
    }

    #[test]
    fn approve_reject_race_uses_independent_sqlite_connections() {
        let temp = tempfile::tempdir().expect("temporary race fixture");
        let paths = DataPaths::from_root(temp.path().join("app")).expect("data paths");
        paths.ensure_private_roots().expect("private roots");
        let left_store = Store::open(paths.database()).expect("left store connection");
        left_store
            .get_or_initialize_settings(1_000)
            .expect("initialize settings");
        let right_store = Store::open(paths.database()).expect("right store connection");
        let left_personal = PersonalRepoService::new(paths.clone(), left_store.clone());
        let right_personal = PersonalRepoService::new(paths, right_store.clone());
        let left = MutationService::new(left_store.clone(), left_personal.clone());
        let right = MutationService::new(right_store.clone(), right_personal.clone());
        let proposal = left
            .propose(
                &context(PeerRole::LocalControl, "race-proposer"),
                settings_request(&left_store, "request-race", "agent-race", 2_000, None),
            )
            .expect("race proposal")
            .proposal;
        let barrier = Arc::new(Barrier::new(3));
        let approve_barrier = barrier.clone();
        let approve_id = proposal.id.clone();
        let approve = std::thread::spawn(move || {
            approve_barrier.wait();
            left.approve_once(
                &context(PeerRole::LocalControl, "race-approver"),
                &approve_id,
                2_010,
                None,
            )
        });
        let reject_barrier = barrier.clone();
        let reject_id = proposal.id.clone();
        let reject = std::thread::spawn(move || {
            reject_barrier.wait();
            right.reject(
                &context(PeerRole::LocalControl, "race-rejector"),
                &reject_id,
                2_010,
            )
        });
        barrier.wait();
        let approve = approve.join().expect("approve thread");
        let reject = reject.join().expect("reject thread");
        assert_eq!(
            usize::from(approve.is_ok()) + usize::from(reject.is_ok()),
            1
        );

        let final_proposal = left_store
            .get_mutation_proposal(&proposal.id)
            .expect("final proposal query")
            .expect("final proposal");
        assert!(matches!(
            final_proposal.resolution,
            MutationResolution::ApprovedApplied | MutationResolution::Rejected
        ));
        let audit = left_store
            .get_mutation_audit_for_proposal(&proposal.id)
            .expect("race audit query");
        assert_eq!(
            audit.is_some(),
            final_proposal.resolution == MutationResolution::ApprovedApplied
        );
        let settings = left_store
            .get_or_initialize_settings(2_020)
            .expect("race settings");
        assert_eq!(
            settings
                .settings
                .trusted_configuration
                .full_access_agent_ids
                .contains(&AgentId::new("agent-race")),
            final_proposal.resolution == MutationResolution::ApprovedApplied
        );
        left_personal.shutdown();
        right_personal.shutdown();
    }

    #[test]
    fn only_authenticated_local_control_can_resolve() {
        let fixture = fixture();
        let mcp = context(PeerRole::McpAdapter, "mcp-auth");
        let proposal = fixture
            .service
            .propose(
                &mcp,
                settings_request(&fixture.store, "request-auth", "agent-auth", 1_000, None),
            )
            .expect("MCP proposal")
            .proposal;
        for role in [
            PeerRole::Internal,
            PeerRole::McpAdapter,
            PeerRole::ProviderChild,
            PeerRole::NodeProxy,
        ] {
            let peer = context(role, "unauthorized-resolver");
            assert_eq!(
                fixture
                    .service
                    .approve_once(&peer, &proposal.id, 1_010, None)
                    .expect_err("nonlocal resolver must fail")
                    .reason,
                "capability_scope_denied"
            );
        }
        for role in [PeerRole::ProviderChild, PeerRole::NodeProxy] {
            assert_eq!(
                fixture
                    .service
                    .propose(
                        &context(role, "unauthorized-proposer"),
                        settings_request(
                            &fixture.store,
                            "request-denied",
                            "agent-denied",
                            1_020,
                            None,
                        ),
                    )
                    .expect_err("peer must not propose")
                    .reason,
                "capability_scope_denied"
            );
        }
        let applied = fixture
            .service
            .approve_once(
                &context(PeerRole::LocalControl, "authorized-resolver"),
                &proposal.id,
                1_030,
                None,
            )
            .expect("local resolver");
        assert_eq!(applied.resolution, MutationResolution::ApprovedApplied);
    }
}
