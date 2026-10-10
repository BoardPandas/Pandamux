use crate::data_paths::DataPaths;
use crate::store::{AssignedRunEvent, RunTransitionRows, Store, StoreError};
use pandamux_core::{
    ArtifactProvenance, AttemptStatus, BlackboardArtifact, BlackboardEntry, BlackboardEntryId,
    CheckpointId, CheckpointKind, OutputContract, ProviderInstanceId, Run, RunAttemptId,
    RunCommand, RunEventId, RunEventKind, RunId, RunTaskId, ThreadEventKind, ThreadId,
    ToolLedgerStatus, TurnAttemptId, UsageObservationId, VerificationEvidence,
    VerificationEvidenceId, VerificationEvidenceKind, VerificationEvidenceOutcome,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;
const MAX_TASK_SUMMARY_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactProvenanceSource {
    Checkpoint { checkpoint_id: CheckpointId },
    Command { evidence_id: VerificationEvidenceId },
    ToolResult { tool_item_id: String },
    Usage { observation_id: UsageObservationId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactSubmission {
    pub output_contract: OutputContract,
    pub version: u64,
    pub previous_entry_id: Option<BlackboardEntryId>,
    pub value: Value,
    pub provenance: Option<ArtifactProvenanceSource>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteTaskArtifactsRequest {
    pub run_id: RunId,
    pub task_id: RunTaskId,
    pub attempt_id: RunAttemptId,
    pub artifacts: Vec<ArtifactSubmission>,
    pub evidence_ids: BTreeSet<VerificationEvidenceId>,
    pub summary: Option<String>,
    pub idempotency_key: String,
    pub completed_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerificationEvidenceSource {
    Command { event_seq: u64 },
    Checkpoint { checkpoint_id: CheckpointId },
    ToolResult { tool_item_id: String },
    Usage { observation_id: UsageObservationId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordVerificationEvidenceRequest {
    pub run_id: RunId,
    pub attempt_id: RunAttemptId,
    pub source: VerificationEvidenceSource,
    pub idempotency_key: String,
    pub recorded_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactCacheRecord {
    pub entry_id: BlackboardEntryId,
    pub content_sha256: String,
    pub path: PathBuf,
    pub created: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedBlackboardTransition {
    pub run_id: RunId,
    pub command: RunCommand,
    pub event: AssignedRunEvent,
    pub event_kind: RunEventKind,
    pub rows: RunTransitionRows,
}

#[derive(Clone)]
pub struct BlackboardStore {
    store: Store,
    data_paths: DataPaths,
}

#[derive(Debug)]
pub enum BlackboardError {
    Store(StoreError),
    Io(io::Error),
    Invalid(&'static str),
    Conflict(&'static str),
}

impl BlackboardError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Store(_) => "blackboard_store_failed",
            Self::Io(_) => "blackboard_cache_io_failed",
            Self::Invalid(code) | Self::Conflict(code) => code,
        }
    }
}

impl std::fmt::Display for BlackboardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for BlackboardError {}

impl From<StoreError> for BlackboardError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<io::Error> for BlackboardError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl BlackboardStore {
    pub fn new(store: Store, data_paths: DataPaths) -> Result<Self, BlackboardError> {
        data_paths.ensure_private_roots()?;
        Ok(Self { store, data_paths })
    }

    pub(crate) fn prepare_task_completion(
        &self,
        run: &Run,
        request: &CompleteTaskArtifactsRequest,
    ) -> Result<PreparedBlackboardTransition, BlackboardError> {
        validate_request_identity(
            run,
            &request.run_id,
            &request.idempotency_key,
            request.completed_at_ms,
        )?;
        if request
            .summary
            .as_ref()
            .is_some_and(|summary| summary.len() > MAX_TASK_SUMMARY_BYTES)
        {
            return Err(BlackboardError::Invalid(
                "blackboard_task_summary_too_large",
            ));
        }
        let _task = run
            .tasks
            .get(&request.task_id)
            .ok_or(BlackboardError::Invalid("blackboard_task_missing"))?;
        let attempt = run
            .attempts
            .get(&request.attempt_id)
            .ok_or(BlackboardError::Invalid("blackboard_attempt_missing"))?;
        if attempt.task_id != request.task_id
            || attempt.status != AttemptStatus::Succeeded
            || !attempt.usage_reconciled
        {
            return Err(BlackboardError::Invalid("blackboard_attempt_not_complete"));
        }

        let event_id = stable_event_id(&request.run_id, &request.idempotency_key);
        let mut artifacts = Vec::with_capacity(request.artifacts.len());
        for submission in &request.artifacts {
            let canonical_json = pandamux_orchestrator::canonical_artifact_json(&submission.value);
            let content_sha256 = sha256_bytes(&canonical_json);
            let artifact_key =
                pandamux_orchestrator::artifact_key(&request.task_id, &submission.output_contract);
            let consumers = run
                .tasks
                .values()
                .filter(|consumer| {
                    consumer.inputs.iter().any(|input| {
                        input.producer_task_id.as_ref() == Some(&request.task_id)
                            && input.contract == submission.output_contract
                    })
                })
                .map(|consumer| consumer.id.clone())
                .collect::<BTreeSet<_>>();
            let provenance = submission
                .provenance
                .as_ref()
                .map(|source| self.resolve_artifact_provenance(run, attempt, source))
                .transpose()?;
            let entry_id = stable_artifact_id(
                &request.run_id,
                &request.task_id,
                &request.attempt_id,
                artifact_key.as_str(),
                submission.version,
                &request.idempotency_key,
            );
            artifacts.push(BlackboardArtifact {
                id: entry_id,
                run_id: request.run_id.clone(),
                artifact_key,
                version: submission.version,
                previous_entry_id: submission.previous_entry_id.clone(),
                producer: request.task_id.clone(),
                consumers,
                output_contract: submission.output_contract.clone(),
                schema_sha256: pandamux_orchestrator::output_contract_schema_sha256(
                    &submission.output_contract,
                ),
                byte_len: u64::try_from(canonical_json.len()).unwrap_or(u64::MAX),
                content_sha256,
                canonical_json,
                provenance,
            });
        }

        let command = RunCommand::CompleteTask {
            task_id: request.task_id.clone(),
            attempt_id: request.attempt_id.clone(),
            artifacts: artifacts.clone(),
            evidence_ids: request.evidence_ids.clone(),
            summary: request.summary.clone(),
        };
        let event_kind = RunEventKind::TaskSucceeded {
            task_id: request.task_id.clone(),
            attempt_id: request.attempt_id.clone(),
            artifacts: artifacts.clone(),
            evidence_ids: request.evidence_ids.clone(),
            summary: request.summary.clone(),
        };
        let rows = RunTransitionRows {
            artifacts: artifacts
                .into_iter()
                .map(|artifact| BlackboardEntry {
                    id: artifact.id,
                    run_id: artifact.run_id,
                    artifact_key: artifact.artifact_key,
                    version: artifact.version,
                    previous_entry_id: artifact.previous_entry_id,
                    producer: artifact.producer,
                    consumers: artifact.consumers,
                    output_contract: artifact.output_contract,
                    schema_sha256: artifact.schema_sha256,
                    canonical_json: artifact.canonical_json,
                    byte_len: artifact.byte_len,
                    content_sha256: artifact.content_sha256,
                    created_event_id: event_id.clone(),
                    provenance: artifact.provenance,
                })
                .collect(),
            ..RunTransitionRows::default()
        };
        Ok(PreparedBlackboardTransition {
            run_id: request.run_id.clone(),
            command,
            event: AssignedRunEvent {
                event_id,
                idempotency_key: request.idempotency_key.clone(),
                at_ms: request.completed_at_ms,
            },
            event_kind,
            rows,
        })
    }

    pub(crate) fn prepare_verification_evidence(
        &self,
        run: &Run,
        request: &RecordVerificationEvidenceRequest,
    ) -> Result<PreparedBlackboardTransition, BlackboardError> {
        validate_request_identity(
            run,
            &request.run_id,
            &request.idempotency_key,
            request.recorded_at_ms,
        )?;
        let attempt = run
            .attempts
            .get(&request.attempt_id)
            .ok_or(BlackboardError::Invalid("verification_attempt_missing"))?;
        if attempt.status != AttemptStatus::Succeeded || !attempt.usage_reconciled {
            return Err(BlackboardError::Invalid(
                "verification_attempt_not_complete",
            ));
        }
        let context = self.attempt_context(run, &request.attempt_id)?;
        let evidence = match &request.source {
            VerificationEvidenceSource::Command { event_seq } => {
                self.command_evidence(run, attempt, &context, *event_seq, request.recorded_at_ms)?
            }
            VerificationEvidenceSource::Checkpoint { checkpoint_id } => self.checkpoint_evidence(
                run,
                attempt,
                &context,
                checkpoint_id,
                request.recorded_at_ms,
            )?,
            VerificationEvidenceSource::ToolResult { tool_item_id } => {
                self.tool_evidence(run, attempt, &context, tool_item_id, request.recorded_at_ms)?
            }
            VerificationEvidenceSource::Usage { observation_id } => self.usage_evidence(
                run,
                attempt,
                &context,
                observation_id,
                request.recorded_at_ms,
            )?,
        };
        let command = RunCommand::RecordVerificationEvidence {
            evidence: evidence.clone(),
        };
        let event_kind = RunEventKind::VerificationEvidenceRecorded { evidence };
        Ok(PreparedBlackboardTransition {
            run_id: request.run_id.clone(),
            command,
            event: AssignedRunEvent {
                event_id: stable_event_id(&request.run_id, &request.idempotency_key),
                idempotency_key: request.idempotency_key.clone(),
                at_ms: request.recorded_at_ms,
            },
            event_kind,
            rows: RunTransitionRows::default(),
        })
    }

    pub fn materialize_artifact_cache(
        &self,
        run_id: &RunId,
        entry_id: &BlackboardEntryId,
        expected_content_sha256: &str,
    ) -> Result<ArtifactCacheRecord, BlackboardError> {
        if !is_sha256(expected_content_sha256) {
            return Err(BlackboardError::Invalid("blackboard_cache_digest_invalid"));
        }
        self.data_paths.ensure_private_roots()?;
        let record = self
            .store
            .load_verified_orchestration_run(run_id)?
            .ok_or(BlackboardError::Invalid("blackboard_run_missing"))?;
        let artifact = record
            .run
            .blackboard
            .get(entry_id)
            .ok_or(BlackboardError::Invalid("blackboard_artifact_missing"))?;
        if artifact.content_sha256 != expected_content_sha256
            || artifact.byte_len != u64::try_from(artifact.canonical_json.len()).unwrap_or(u64::MAX)
            || sha256_bytes(&artifact.canonical_json) != expected_content_sha256
        {
            return Err(BlackboardError::Conflict(
                "blackboard_cache_identity_conflict",
            ));
        }
        let shard = &expected_content_sha256[..2];
        let directory = self.data_paths.run_artifacts().join(shard);
        ensure_private_cache_directory(&directory, self.data_paths.run_artifacts())?;
        let path = directory.join(format!("{expected_content_sha256}.json"));
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                reject_link_or_non_file(&metadata)?;
                let bytes = fs::read(&path)?;
                if bytes != artifact.canonical_json
                    || sha256_bytes(&bytes) != expected_content_sha256
                {
                    return Err(BlackboardError::Conflict(
                        "blackboard_cache_identity_conflict",
                    ));
                }
                return Ok(ArtifactCacheRecord {
                    entry_id: entry_id.clone(),
                    content_sha256: expected_content_sha256.to_string(),
                    path,
                    created: false,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return self.materialize_artifact_cache(run_id, entry_id, expected_content_sha256);
            }
            Err(error) => return Err(error.into()),
        };
        if let Err(error) = file
            .write_all(&artifact.canonical_json)
            .and_then(|()| file.sync_all())
        {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error.into());
        }
        drop(file);
        reject_link_or_non_file(&fs::symlink_metadata(&path)?)?;
        Ok(ArtifactCacheRecord {
            entry_id: entry_id.clone(),
            content_sha256: expected_content_sha256.to_string(),
            path,
            created: true,
        })
    }

    fn resolve_artifact_provenance(
        &self,
        run: &Run,
        attempt: &pandamux_core::Attempt,
        source: &ArtifactProvenanceSource,
    ) -> Result<ArtifactProvenance, BlackboardError> {
        match source {
            ArtifactProvenanceSource::Checkpoint { checkpoint_id } => {
                let checkpoint =
                    run.checkpoints
                        .get(checkpoint_id)
                        .ok_or(BlackboardError::Invalid(
                            "artifact_checkpoint_provenance_missing",
                        ))?;
                if checkpoint.task_id != attempt.task_id || checkpoint.attempt_id != attempt.id {
                    return Err(BlackboardError::Invalid(
                        "artifact_checkpoint_provenance_mismatch",
                    ));
                }
                Ok(ArtifactProvenance::Checkpoint {
                    checkpoint_id: checkpoint_id.clone(),
                })
            }
            ArtifactProvenanceSource::Command { evidence_id } => {
                let evidence =
                    run.verification_evidence
                        .get(evidence_id)
                        .ok_or(BlackboardError::Invalid(
                            "artifact_command_provenance_missing",
                        ))?;
                if evidence.task_id != attempt.task_id
                    || evidence.attempt_id != attempt.id
                    || evidence.kind != VerificationEvidenceKind::Command
                {
                    return Err(BlackboardError::Invalid(
                        "artifact_command_provenance_mismatch",
                    ));
                }
                Ok(ArtifactProvenance::Command {
                    evidence_id: evidence_id.clone(),
                })
            }
            ArtifactProvenanceSource::ToolResult { tool_item_id } => {
                let context = self.attempt_context(run, &attempt.id)?;
                let ledger = self.store.list_tool_ledger(&context.turn_id)?;
                let entry = ledger
                    .iter()
                    .find(|entry| entry.provider_item_id == *tool_item_id)
                    .ok_or(BlackboardError::Invalid("artifact_tool_provenance_missing"))?;
                if entry.attempt_id != context.turn_attempt_id
                    || entry.status == ToolLedgerStatus::Started
                {
                    return Err(BlackboardError::Invalid(
                        "artifact_tool_provenance_mismatch",
                    ));
                }
                Ok(ArtifactProvenance::ToolResult {
                    thread_id: context.thread_id,
                    turn_attempt_id: context.turn_attempt_id,
                    tool_item_id: tool_item_id.clone(),
                })
            }
            ArtifactProvenanceSource::Usage { observation_id } => {
                let runtime = self.store.load_orchestration_runtime_records(&run.id)?;
                let found = runtime.budget_reservations.iter().any(|record| {
                    record.reservation.attempt_id == attempt.id
                        && record.reconciliation.as_ref().is_some_and(|evidence| {
                            evidence.usage_observation_ids.contains(observation_id)
                        })
                });
                if !found {
                    return Err(BlackboardError::Invalid(
                        "artifact_usage_provenance_missing",
                    ));
                }
                Ok(ArtifactProvenance::Usage {
                    observation_id: observation_id.clone(),
                })
            }
        }
    }

    fn attempt_context(
        &self,
        run: &Run,
        attempt_id: &RunAttemptId,
    ) -> Result<AttemptEvidenceContext, BlackboardError> {
        let attempt = run
            .attempts
            .get(attempt_id)
            .ok_or(BlackboardError::Invalid("verification_attempt_missing"))?;
        let thread_id = attempt
            .thread_id
            .clone()
            .ok_or(BlackboardError::Invalid("verification_thread_missing"))?;
        let turn_attempt_id = attempt
            .turn_attempt_id
            .clone()
            .ok_or(BlackboardError::Invalid(
                "verification_turn_attempt_missing",
            ))?;
        let provider_instance_id = attempt
            .route
            .provider
            .selected_provider_instance_id
            .clone()
            .ok_or(BlackboardError::Invalid("verification_provider_missing"))?;
        let turn_attempt =
            self.store
                .get_turn_attempt(&turn_attempt_id)?
                .ok_or(BlackboardError::Invalid(
                    "verification_turn_attempt_missing",
                ))?;
        if turn_attempt.thread_id != thread_id
            || turn_attempt.provider_instance_id != provider_instance_id
        {
            return Err(BlackboardError::Invalid(
                "verification_turn_attempt_mismatch",
            ));
        }
        let turn = self
            .store
            .get_turn(&turn_attempt.turn_id)?
            .ok_or(BlackboardError::Invalid("verification_turn_missing"))?;
        if turn.thread_id != thread_id || turn.provider_instance_id != provider_instance_id {
            return Err(BlackboardError::Invalid("verification_turn_mismatch"));
        }
        Ok(AttemptEvidenceContext {
            thread_id,
            turn_id: turn.id,
            turn_attempt_id,
            provider_instance_id,
        })
    }

    fn command_evidence(
        &self,
        run: &Run,
        attempt: &pandamux_core::Attempt,
        context: &AttemptEvidenceContext,
        event_seq: u64,
        recorded_at_ms: u64,
    ) -> Result<VerificationEvidence, BlackboardError> {
        let event = self
            .store
            .get_events(&context.thread_id, None)?
            .into_iter()
            .find(|event| event.seq == event_seq)
            .ok_or(BlackboardError::Invalid(
                "verification_command_event_missing",
            ))?;
        let ThreadEventKind::CommandRun {
            item_id,
            command,
            cwd,
            exit_code,
            output_tail,
        } = event.kind
        else {
            return Err(BlackboardError::Invalid(
                "verification_command_event_mismatch",
            ));
        };
        let exit_status = exit_code.ok_or(BlackboardError::Invalid(
            "verification_command_not_terminal",
        ))?;
        let command_digest = sha256_fields(&[
            context.turn_id.as_str().as_bytes(),
            event_seq.to_string().as_bytes(),
            item_id.as_bytes(),
            command.as_bytes(),
        ]);
        let arguments_digest = sha256_serialized(&(command.as_str(), cwd.as_str()))?;
        let bounded_output_digest = sha256_serialized(&output_tail)?;
        Ok(base_evidence(
            run,
            attempt,
            context,
            recorded_at_ms,
            EvidenceDerivation {
                kind: VerificationEvidenceKind::Command,
                outcome: if exit_status == 0 {
                    VerificationEvidenceOutcome::Passed
                } else {
                    VerificationEvidenceOutcome::Failed
                },
                source_digest: command_digest.clone(),
                fields: EvidenceFields {
                    command_digest: Some(command_digest),
                    arguments_digest: Some(arguments_digest),
                    exit_status: Some(exit_status),
                    bounded_output_digest,
                    ..EvidenceFields::default()
                },
            },
        ))
    }

    fn checkpoint_evidence(
        &self,
        run: &Run,
        attempt: &pandamux_core::Attempt,
        context: &AttemptEvidenceContext,
        checkpoint_id: &CheckpointId,
        recorded_at_ms: u64,
    ) -> Result<VerificationEvidence, BlackboardError> {
        let checkpoint = run
            .checkpoints
            .get(checkpoint_id)
            .ok_or(BlackboardError::Invalid("verification_checkpoint_missing"))?;
        if checkpoint.task_id != attempt.task_id
            || checkpoint.attempt_id != attempt.id
            || checkpoint.thread_id.as_ref() != Some(&context.thread_id)
            || checkpoint.kind != CheckpointKind::AfterAttempt
        {
            return Err(BlackboardError::Invalid("verification_checkpoint_mismatch"));
        }
        let runtime = self.store.load_orchestration_runtime_records(&run.id)?;
        let worktree = runtime
            .worktrees
            .iter()
            .find(|record| record.worktree.attempt_id == attempt.id)
            .ok_or(BlackboardError::Invalid(
                "verification_checkpoint_worktree_missing",
            ))?;
        let diff_digest = worktree
            .checkpoint_intent_sha256
            .get(checkpoint_id)
            .filter(|digest| is_sha256(digest))
            .cloned()
            .ok_or(BlackboardError::Invalid(
                "verification_checkpoint_receipt_missing",
            ))?;
        let bounded_output_digest = sha256_serialized(&(
            checkpoint_id,
            checkpoint.value.as_str(),
            worktree.worktree.id.as_str(),
            diff_digest.as_str(),
        ))?;
        Ok(base_evidence(
            run,
            attempt,
            context,
            recorded_at_ms,
            EvidenceDerivation {
                kind: VerificationEvidenceKind::Checkpoint,
                outcome: VerificationEvidenceOutcome::Passed,
                source_digest: diff_digest.clone(),
                fields: EvidenceFields {
                    bounded_output_digest,
                    checkpoint_id: Some(checkpoint_id.clone()),
                    checkpoint_diff_digest: Some(diff_digest),
                    ..EvidenceFields::default()
                },
            },
        ))
    }

    fn tool_evidence(
        &self,
        run: &Run,
        attempt: &pandamux_core::Attempt,
        context: &AttemptEvidenceContext,
        tool_item_id: &str,
        recorded_at_ms: u64,
    ) -> Result<VerificationEvidence, BlackboardError> {
        if tool_item_id.trim().is_empty() || tool_item_id.len() > MAX_IDEMPOTENCY_KEY_BYTES {
            return Err(BlackboardError::Invalid(
                "verification_tool_identity_invalid",
            ));
        }
        let ledger = self.store.list_tool_ledger(&context.turn_id)?;
        let entry = ledger
            .iter()
            .find(|entry| entry.provider_item_id == tool_item_id)
            .ok_or(BlackboardError::Invalid("verification_tool_missing"))?;
        if entry.attempt_id != context.turn_attempt_id
            || entry.turn_id != context.turn_id
            || entry.status == ToolLedgerStatus::Started
        {
            return Err(BlackboardError::Invalid("verification_tool_mismatch"));
        }
        let source_digest = sha256_serialized(entry)?;
        Ok(base_evidence(
            run,
            attempt,
            context,
            recorded_at_ms,
            EvidenceDerivation {
                kind: VerificationEvidenceKind::ToolResult,
                outcome: if entry.status == ToolLedgerStatus::Succeeded {
                    VerificationEvidenceOutcome::Passed
                } else {
                    VerificationEvidenceOutcome::Failed
                },
                source_digest: source_digest.clone(),
                fields: EvidenceFields {
                    bounded_output_digest: source_digest,
                    tool_item_id: Some(tool_item_id.to_string()),
                    ..EvidenceFields::default()
                },
            },
        ))
    }

    fn usage_evidence(
        &self,
        run: &Run,
        attempt: &pandamux_core::Attempt,
        context: &AttemptEvidenceContext,
        observation_id: &UsageObservationId,
        recorded_at_ms: u64,
    ) -> Result<VerificationEvidence, BlackboardError> {
        let runtime = self.store.load_orchestration_runtime_records(&run.id)?;
        let reconciliation = runtime
            .budget_reservations
            .iter()
            .find(|record| record.reservation.attempt_id == attempt.id)
            .and_then(|record| record.reconciliation.as_ref())
            .ok_or(BlackboardError::Invalid(
                "verification_usage_receipt_missing",
            ))?;
        if reconciliation.turn_id != context.turn_id
            || reconciliation.turn_attempt_id != context.turn_attempt_id
            || !reconciliation
                .usage_observation_ids
                .contains(observation_id)
            || !is_sha256(&reconciliation.source_sha256)
        {
            return Err(BlackboardError::Invalid(
                "verification_usage_receipt_mismatch",
            ));
        }
        Ok(base_evidence(
            run,
            attempt,
            context,
            recorded_at_ms,
            EvidenceDerivation {
                kind: VerificationEvidenceKind::Usage,
                outcome: VerificationEvidenceOutcome::Passed,
                source_digest: reconciliation.source_sha256.clone(),
                fields: EvidenceFields {
                    bounded_output_digest: reconciliation.source_sha256.clone(),
                    usage_observation_id: Some(observation_id.clone()),
                    ..EvidenceFields::default()
                },
            },
        ))
    }
}

#[derive(Clone, Debug)]
struct AttemptEvidenceContext {
    thread_id: ThreadId,
    turn_id: pandamux_core::TurnId,
    turn_attempt_id: TurnAttemptId,
    provider_instance_id: ProviderInstanceId,
}

#[derive(Default)]
struct EvidenceFields {
    command_digest: Option<String>,
    arguments_digest: Option<String>,
    exit_status: Option<i32>,
    bounded_output_digest: String,
    checkpoint_id: Option<CheckpointId>,
    checkpoint_diff_digest: Option<String>,
    tool_item_id: Option<String>,
    usage_observation_id: Option<UsageObservationId>,
}

struct EvidenceDerivation {
    kind: VerificationEvidenceKind,
    outcome: VerificationEvidenceOutcome,
    source_digest: String,
    fields: EvidenceFields,
}

fn base_evidence(
    run: &Run,
    attempt: &pandamux_core::Attempt,
    context: &AttemptEvidenceContext,
    recorded_at_ms: u64,
    derivation: EvidenceDerivation,
) -> VerificationEvidence {
    let evidence_id = VerificationEvidenceId::new(format!(
        "verification-evidence-{}",
        &sha256_fields(&[
            run.id.as_str().as_bytes(),
            attempt.task_id.as_str().as_bytes(),
            attempt.id.as_str().as_bytes(),
            evidence_kind_name(derivation.kind).as_bytes(),
            derivation.source_digest.as_bytes(),
        ])[..32]
    ));
    let fields = derivation.fields;
    VerificationEvidence {
        id: evidence_id,
        run_id: run.id.clone(),
        task_id: attempt.task_id.clone(),
        attempt_id: attempt.id.clone(),
        kind: derivation.kind,
        outcome: derivation.outcome,
        command_digest: fields.command_digest,
        arguments_digest: fields.arguments_digest,
        exit_status: fields.exit_status,
        bounded_output_digest: fields.bounded_output_digest,
        checkpoint_id: fields.checkpoint_id,
        checkpoint_diff_digest: fields.checkpoint_diff_digest,
        tool_item_id: fields.tool_item_id,
        usage_observation_id: fields.usage_observation_id,
        provider_instance_id: context.provider_instance_id.clone(),
        thread_id: context.thread_id.clone(),
        turn_attempt_id: context.turn_attempt_id.clone(),
        recorded_at_ms,
    }
}

fn evidence_kind_name(kind: VerificationEvidenceKind) -> &'static str {
    match kind {
        VerificationEvidenceKind::Command => "command",
        VerificationEvidenceKind::Checkpoint => "checkpoint",
        VerificationEvidenceKind::ToolResult => "tool_result",
        VerificationEvidenceKind::Usage => "usage",
    }
}

fn validate_request_identity(
    run: &Run,
    run_id: &RunId,
    idempotency_key: &str,
    at_ms: u64,
) -> Result<(), BlackboardError> {
    if run.id != *run_id
        || idempotency_key.trim().is_empty()
        || idempotency_key.len() > MAX_IDEMPOTENCY_KEY_BYTES
        || (at_ms < run.updated_at_ms && !run.idempotency_receipts.contains_key(idempotency_key))
    {
        return Err(BlackboardError::Invalid(
            "blackboard_transition_identity_invalid",
        ));
    }
    Ok(())
}

fn stable_event_id(run_id: &RunId, idempotency_key: &str) -> RunEventId {
    RunEventId::new(format!(
        "run-event-blackboard-{}",
        &sha256_fields(&[run_id.as_str().as_bytes(), idempotency_key.as_bytes()])[..32]
    ))
}

fn stable_artifact_id(
    run_id: &RunId,
    task_id: &RunTaskId,
    attempt_id: &RunAttemptId,
    artifact_key: &str,
    version: u64,
    idempotency_key: &str,
) -> BlackboardEntryId {
    BlackboardEntryId::new(format!(
        "blackboard-{}",
        &sha256_fields(&[
            run_id.as_str().as_bytes(),
            task_id.as_str().as_bytes(),
            attempt_id.as_str().as_bytes(),
            artifact_key.as_bytes(),
            &version.to_be_bytes(),
            idempotency_key.as_bytes(),
        ])[..32]
    ))
}

fn sha256_fields(fields: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field);
    }
    format!("{:x}", hasher.finalize())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_serialized(value: &impl Serialize) -> Result<String, BlackboardError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| BlackboardError::Invalid("blackboard_digest_encoding_failed"))?;
    Ok(sha256_bytes(&bytes))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn ensure_private_cache_directory(path: &Path, owner_root: &Path) -> Result<(), BlackboardError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => reject_link_or_non_directory(&metadata)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path)?;
            reject_link_or_non_directory(&fs::symlink_metadata(path)?)?;
        }
        Err(error) => return Err(error.into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let root = fs::symlink_metadata(owner_root)?;
        let metadata = fs::symlink_metadata(path)?;
        if root.uid() != metadata.uid() {
            return Err(BlackboardError::Invalid("blackboard_cache_owner_mismatch"));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = owner_root;
    Ok(())
}

fn reject_link_or_non_directory(metadata: &fs::Metadata) -> Result<(), BlackboardError> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() || is_reparse_point(metadata) {
        return Err(BlackboardError::Invalid(
            "blackboard_cache_directory_unsafe",
        ));
    }
    Ok(())
}

fn reject_link_or_non_file(metadata: &fs::Metadata) -> Result<(), BlackboardError> {
    if metadata.file_type().is_symlink() || !metadata.is_file() || is_reparse_point(metadata) {
        return Err(BlackboardError::Invalid("blackboard_cache_file_unsafe"));
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}
