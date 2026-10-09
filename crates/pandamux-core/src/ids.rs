use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! prefixed_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn generate() -> Self {
                Self(format!("{}{}", $prefix, uuid::Uuid::new_v4()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

prefixed_id!(WorkspaceId, "ws-");
prefixed_id!(PaneId, "pane-");
prefixed_id!(SurfaceId, "surf-");
prefixed_id!(WindowId, "win-");
prefixed_id!(SshProfileId, "ssh-");
prefixed_id!(ProjectId, "proj-");
prefixed_id!(ThreadId, "thread-");
prefixed_id!(TurnId, "turn-");
prefixed_id!(TurnAttemptId, "attempt-");
prefixed_id!(ContinuationId, "continuation-");
prefixed_id!(EnvironmentId, "env-");
prefixed_id!(ProviderInstanceId, "prov-");
prefixed_id!(ScheduleId, "sch-");
prefixed_id!(RunId, "run-");
prefixed_id!(RunTaskId, "task-");
prefixed_id!(RunAttemptId, "run-attempt-");
prefixed_id!(RunEventId, "run-event-");
prefixed_id!(OrchestrationEffectId, "effect-");
prefixed_id!(ExecutionLeaseId, "execution-lease-");
prefixed_id!(ServerIncarnationId, "server-incarnation-");
prefixed_id!(AttemptWorktreeId, "attempt-worktree-");
prefixed_id!(AttemptProgressId, "attempt-progress-");
prefixed_id!(CheckpointId, "checkpoint-");
prefixed_id!(CleanupRecordId, "cleanup-");
prefixed_id!(RecoveryReviewId, "recovery-review-");
prefixed_id!(VerificationEvidenceId, "verification-evidence-");
prefixed_id!(BudgetGrantId, "budget-grant-");
prefixed_id!(ArtifactKey, "artifact-");
prefixed_id!(DependencyEdgeId, "edge-");
prefixed_id!(ApprovalId, "approval-");
prefixed_id!(BlackboardEntryId, "blackboard-");
prefixed_id!(BudgetReservationId, "budget-");
prefixed_id!(AgentId, "agent-");
prefixed_id!(OrganizationId, "org-");
prefixed_id!(SubscriptionKey, "subkey-");
prefixed_id!(FailoverPoolId, "pool-");
prefixed_id!(UsageObservationId, "usage-");
prefixed_id!(NodeStoreId, "store-");
prefixed_id!(PricingVersionId, "price-");
prefixed_id!(InstructionSourceId, "instruction-source-");
prefixed_id!(InstructionCandidateId, "instruction-candidate-");
prefixed_id!(ConfigurationAuditId, "configuration-audit-");
prefixed_id!(BundleActivationId, "bundle-activation-");
prefixed_id!(BundleTransferId, "bundle-transfer-");
prefixed_id!(ProjectRepositoryId, "project-repository-");
prefixed_id!(ProjectLayerTrustId, "project-trust-");
prefixed_id!(MutationProposalId, "mutation-proposal-");
prefixed_id!(MutationAuditId, "mutation-audit-");
prefixed_id!(PersonalEditId, "personal-edit-");
