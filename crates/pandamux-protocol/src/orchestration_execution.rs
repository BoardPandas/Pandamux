use pandamux_core::{
    AccessMode, AgentId, ArchiveSha256, AttemptBoundary, AttemptStatus, AttemptWorktreeId,
    BundleSha, ContentSha256, EnvironmentId, NodeStoreId, OrchestrationEffectId, OrganizationId,
    ProjectRepositoryId, ProviderInstanceId, RunAttemptId, RunId, RunTaskId, SourceCommitSha,
    TaskRouteDecision, ThreadId, TurnAttemptId, TurnId,
};
use serde::{Deserialize, Serialize};

pub const ORCHESTRATION_EXECUTION_SCHEMA_VERSION: u32 = 1;

/// Stable authority and replay identity shared by every private Hub/node request.
/// The authenticated request context supplies the environment; it is deliberately
/// absent from this wire shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationExecutionIdentity {
    pub schema_version: u32,
    pub run_id: RunId,
    pub task_id: RunTaskId,
    pub attempt_id: RunAttemptId,
    pub effect_id: OrchestrationEffectId,
    pub payload_sha256: ContentSha256,
    pub repository_id: ProjectRepositoryId,
    pub baseline_commit: SourceCommitSha,
    pub bundle_sha: BundleSha,
    pub route: TaskRouteDecision,
    pub lease_generation: u64,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationRepositoryPrepareParams {
    pub identity: OrchestrationExecutionIdentity,
    pub baseline_bundle_sha256: ArchiveSha256,
    pub baseline_bundle_bytes_sha256: ContentSha256,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_bundle_bytes_base64: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationRepositoryPrepareResult {
    pub schema_version: u32,
    pub request_sha256: ContentSha256,
    pub receipt_sha256: ContentSha256,
    pub node_store_id: NodeStoreId,
    pub environment_id: EnvironmentId,
    pub identity_sha256: ContentSha256,
    pub repository_key_sha256: ContentSha256,
    pub repository_id: ProjectRepositoryId,
    pub baseline_commit: SourceCommitSha,
    pub baseline_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_artifact_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationAttemptPrepareParams {
    pub identity: OrchestrationExecutionIdentity,
    pub worktree_id: AttemptWorktreeId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub turn_attempt_id: TurnAttemptId,
    pub agent_id: AgentId,
    pub access_ceiling: AccessMode,
    pub compiled_instructions_sha256: ContentSha256,
    pub tool_policy_sha256: ContentSha256,
    pub prompt_sha256: ContentSha256,
    pub task_prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_contract: Option<String>,
    pub prepared_at_ms: u64,
    pub attempt_ordinal: u32,
    pub run_concurrency_limit: u32,
}

/// Exact provider-neutral execution material retained by the node before a
/// dispatch marker can be written. The node independently reconstructs role,
/// knowledge, and tool policy from the pinned bundle and verifies the hashes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationPreparedExecution {
    pub worktree_id: AttemptWorktreeId,
    pub agent_id: AgentId,
    pub organization_id: OrganizationId,
    pub provider_instance_id: ProviderInstanceId,
    pub access_ceiling: AccessMode,
    pub compiled_instructions_sha256: ContentSha256,
    pub tool_policy_sha256: ContentSha256,
    pub prompt_sha256: ContentSha256,
    pub task_prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_contract: Option<String>,
    pub prepared_at_ms: u64,
    pub attempt_ordinal: u32,
    pub run_concurrency_limit: u32,
    pub agent_concurrency_limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationAttemptPrepareResult {
    pub receipt: OrchestrationAttemptReceipt,
    pub prepared_execution: OrchestrationPreparedExecution,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationAttemptDispatchParams {
    pub identity: OrchestrationExecutionIdentity,
    pub prepare_receipt_sha256: ContentSha256,
    pub dispatch_identity: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationAttemptStatusParams {
    pub identity: OrchestrationExecutionIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationAttemptCancelParams {
    pub identity: OrchestrationExecutionIdentity,
    pub reason_code: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationAttemptEventsParams {
    pub identity: OrchestrationExecutionIdentity,
    pub after_sequence: u64,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationAttemptAckParams {
    pub identity: OrchestrationExecutionIdentity,
    pub through_sequence: u64,
    pub events_sha256: ContentSha256,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationNodeAttemptState {
    Prepared,
    DispatchStarted,
    Accepted,
    NotStarted,
    Ambiguous,
    Terminal,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationAttemptReceipt {
    pub schema_version: u32,
    pub request_sha256: ContentSha256,
    pub receipt_sha256: ContentSha256,
    pub node_store_id: NodeStoreId,
    pub environment_id: EnvironmentId,
    pub identity_sha256: ContentSha256,
    pub prepared_execution_sha256: ContentSha256,
    pub run_id: RunId,
    pub task_id: RunTaskId,
    pub attempt_id: RunAttemptId,
    pub effect_id: OrchestrationEffectId,
    pub state: OrchestrationNodeAttemptState,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub turn_attempt_id: TurnAttemptId,
    pub boundary: AttemptBoundary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_status: Option<AttemptStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_sha256: Option<ContentSha256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_error_code: Option<String>,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationAttemptStatusResult {
    pub receipt: OrchestrationAttemptReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationAttemptCancelResult {
    pub receipt: OrchestrationAttemptReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationNodeEvent {
    pub sequence: u64,
    pub at_ms: u64,
    pub payload_sha256: ContentSha256,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationAttemptEventsResult {
    pub schema_version: u32,
    pub request_sha256: ContentSha256,
    pub events_sha256: ContentSha256,
    pub node_store_id: NodeStoreId,
    pub environment_id: EnvironmentId,
    pub identity_sha256: ContentSha256,
    pub events: Vec<OrchestrationNodeEvent>,
    pub terminal_receipt: Option<OrchestrationAttemptReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationAttemptAckResult {
    pub schema_version: u32,
    pub request_sha256: ContentSha256,
    pub receipt_sha256: ContentSha256,
    pub node_store_id: NodeStoreId,
    pub environment_id: EnvironmentId,
    pub identity_sha256: ContentSha256,
    pub acknowledged_through_sequence: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{ProviderInstanceId, RoutingDecision};
    use serde_json::json;

    fn digest(value: char) -> ContentSha256 {
        ContentSha256::parse(value.to_string().repeat(64)).expect("digest")
    }

    fn identity() -> OrchestrationExecutionIdentity {
        OrchestrationExecutionIdentity {
            schema_version: ORCHESTRATION_EXECUTION_SCHEMA_VERSION,
            run_id: RunId::new("run-private-protocol"),
            task_id: RunTaskId::new("task-private-protocol"),
            attempt_id: RunAttemptId::new("run-attempt-private-protocol"),
            effect_id: OrchestrationEffectId::new("effect-private-protocol"),
            payload_sha256: digest('a'),
            repository_id: ProjectRepositoryId::new("project-repository-private-protocol"),
            baseline_commit: SourceCommitSha::parse("b".repeat(40)).expect("commit"),
            bundle_sha: BundleSha::parse("c".repeat(64)).expect("bundle"),
            route: TaskRouteDecision {
                task_id: RunTaskId::new("task-private-protocol"),
                selected_agent_id: Some(AgentId::new("agent-private-protocol")),
                agent_evaluations: Vec::new(),
                provider: RoutingDecision {
                    selected_provider_instance_id: Some(ProviderInstanceId::new(
                        "provider-private-protocol",
                    )),
                    selected_subscription_key: None,
                    selected_model: Some("model-private-protocol".to_string()),
                    evaluations: Vec::new(),
                },
            },
            lease_generation: 7,
            idempotency_key: "prepare-private-protocol".to_string(),
        }
    }

    #[test]
    fn private_attempt_request_round_trips_without_environment_authority() {
        let params = OrchestrationAttemptPrepareParams {
            identity: identity(),
            worktree_id: AttemptWorktreeId::new("attempt-worktree-private-protocol"),
            thread_id: ThreadId::new("thread-private-protocol"),
            turn_id: TurnId::new("turn-private-protocol"),
            turn_attempt_id: TurnAttemptId::new("attempt-private-protocol"),
            agent_id: AgentId::new("agent-private-protocol"),
            access_ceiling: AccessMode::WorkspaceOnly,
            compiled_instructions_sha256: digest('d'),
            tool_policy_sha256: digest('e'),
            prompt_sha256: digest('f'),
            task_prompt: "Execute the authenticated remote fixture".to_string(),
            output_contract: Some("{\"type\":\"object\"}".to_string()),
            prepared_at_ms: 1_700_000_000_000,
            attempt_ordinal: 1,
            run_concurrency_limit: 2,
        };
        let bytes = serde_json::to_vec(&params).expect("serialize private request");
        assert!(!String::from_utf8_lossy(&bytes).contains("environmentId"));
        assert_eq!(
            serde_json::from_slice::<OrchestrationAttemptPrepareParams>(&bytes)
                .expect("deserialize private request"),
            params
        );
    }

    #[test]
    fn request_json_cannot_inject_an_environment() {
        let mut value = serde_json::to_value(OrchestrationAttemptStatusParams {
            identity: identity(),
        })
        .expect("request value");
        value
            .as_object_mut()
            .expect("request object")
            .insert("environmentId".to_string(), json!("env-attacker"));
        serde_json::from_value::<OrchestrationAttemptStatusParams>(value)
            .expect_err("private request rejects context authority in JSON");
    }
}
