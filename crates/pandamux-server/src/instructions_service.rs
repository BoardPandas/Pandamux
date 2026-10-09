use crate::managed_git::{ManagedGitCommand, ManagedGitError, ManagedGitRunner};
use crate::{DataPaths, Store, StoreError};
use pandamux_core::{
    ConfigurationRevision, ContentSha256, InstructionCandidateId, InstructionCandidateRecord,
    InstructionCandidateState, InstructionSourceId, InstructionsSourceRecord,
    InstructionsUpdatePolicy, ScheduleTarget, SourceCommitSha, TrackingRef, ValueLayer,
    ValueOrigin,
};
use pandamux_orchestrator::{
    ConfigurationCandidateInspection, ConfigurationCandidateInspectionInput,
    ConfigurationDependencySnapshot, SourceDocument, canonical_json_sha256,
    inspect_configuration_candidate,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const INSTRUCTIONS_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const MIRROR_INIT_PREFIX: &str = ".mirror-init-";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstructionsCheckOutcome {
    pub checked_at_ms: u64,
    pub unchanged: bool,
    pub fetched: bool,
    pub candidate: Option<InstructionCandidateRecord>,
    pub safe_error_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstructionsServiceError {
    pub reason: &'static str,
    pub safe_message: String,
    pub retryable: bool,
    pub actual_revision: Option<ConfigurationRevision>,
}

impl std::fmt::Display for InstructionsServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.safe_message)
    }
}

impl std::error::Error for InstructionsServiceError {}

impl From<StoreError> for InstructionsServiceError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::RevisionDrift { actual, .. } => Self {
                reason: "revision_drift",
                safe_message: "The instructions source changed before the check completed"
                    .to_string(),
                retryable: true,
                actual_revision: actual,
            },
            _ => Self {
                reason: "instructions_state_failed",
                safe_message: "The instructions state could not be read or persisted".to_string(),
                retryable: true,
                actual_revision: None,
            },
        }
    }
}

impl From<ManagedGitError> for InstructionsServiceError {
    fn from(error: ManagedGitError) -> Self {
        Self {
            reason: error.reason,
            safe_message: error.safe_message,
            retryable: error.retryable,
            actual_revision: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateAcquisitionIssue {
    pub code: String,
    pub safe_reason: String,
    pub origin: ValueOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredCandidateInspection {
    pub report: ConfigurationCandidateInspection,
    #[serde(default)]
    pub acquisition_issues: Vec<CandidateAcquisitionIssue>,
    pub signature_required: bool,
    pub signature_verified: bool,
}

#[derive(Clone)]
pub struct InstructionsService {
    paths: DataPaths,
    store: Store,
    git: ManagedGitRunner,
}

impl InstructionsService {
    pub fn new(paths: DataPaths, store: Store) -> Self {
        Self {
            git: ManagedGitRunner::new(paths.clone()),
            paths,
            store,
        }
    }

    pub fn shutdown(&self) {
        self.git.shutdown();
    }

    pub fn check_all_sources(&self) -> Vec<InstructionsCheckOutcome> {
        let sources = match self.store.list_instruction_sources() {
            Ok(sources) => sources,
            Err(_) => return Vec::new(),
        };
        sources
            .into_iter()
            .filter_map(|source| {
                self.check_source(&source.id, source.revision, now_ms())
                    .ok()
            })
            .collect()
    }

    pub fn check_source(
        &self,
        source_id: &InstructionSourceId,
        expected_revision: ConfigurationRevision,
        checked_at_ms: u64,
    ) -> Result<InstructionsCheckOutcome, InstructionsServiceError> {
        let source = self
            .store
            .get_instruction_source(source_id)?
            .ok_or_else(source_not_found)?;
        if source.revision != expected_revision {
            return Err(InstructionsServiceError {
                reason: "revision_drift",
                safe_message: "The instructions source revision is stale".to_string(),
                retryable: true,
                actual_revision: Some(source.revision),
            });
        }
        validate_source(&source)?;
        let mirror = self.mirror_path(&source.canonical_remote_identity);
        let owner = repository_owner(&mirror);
        let _owner = owner.lock().expect("instructions mirror owner lock");
        self.ensure_mirror(&mirror)?;

        let resolved = match self.resolve_remote(&source, &mirror) {
            Ok(resolved) => resolved,
            Err(error) => {
                return self.finish_failed_check(&source, checked_at_ms, error.reason);
            }
        };
        let matching = self
            .store
            .get_instruction_candidate_for_commit(&source.id, &resolved.commit)?;
        if candidate_matches(matching.as_ref(), &resolved)
            || source.active_source_commit_sha.as_ref() == Some(&resolved.commit)
                && resolved.tag_object.is_none()
        {
            self.store.record_instruction_source_check_at_commit(
                &source.id,
                source.revision,
                checked_at_ms,
                None,
                Some(&resolved.commit),
            )?;
            return Ok(InstructionsCheckOutcome {
                checked_at_ms,
                unchanged: true,
                fetched: false,
                candidate: matching.map(|candidate| candidate.record),
                safe_error_reason: None,
            });
        }
        let previous = self.store.latest_instruction_candidate(&source.id)?;

        let candidate_refs = candidate_refs(&source.id, &resolved.commit);
        let mut fetched = false;
        let commit_in_mirror = self.object_is_commit(&mirror, &resolved.commit);
        if !commit_in_mirror || !matches!(source.tracking, TrackingRef::Commit(_)) {
            if let Err(error) = self.git.run(ManagedGitCommand::Fetch {
                git_dir: mirror.clone(),
                remote: source.canonical_remote_identity.clone(),
                source: resolved.fetch_source.clone(),
                target: candidate_refs.source.clone(),
            }) {
                return self.finish_failed_check(&source, checked_at_ms, error.reason);
            }
            fetched = true;
        } else {
            self.pin_candidate_commit(&mirror, &candidate_refs.source, &resolved.commit)?;
        }

        let observed_commit = match self.resolve_fetched_commit(&mirror, &candidate_refs.source) {
            Ok(commit) => commit,
            Err(error) => {
                return self.finish_failed_check(&source, checked_at_ms, error.reason);
            }
        };
        if observed_commit != resolved.commit {
            return self.finish_failed_check(
                &source,
                checked_at_ms,
                "instructions_remote_identity_changed",
            );
        }
        self.pin_candidate_commit(&mirror, &candidate_refs.commit, &observed_commit)?;

        let history_rewritten = if matches!(source.tracking, TrackingRef::Branch(_)) {
            let comparison = source.active_source_commit_sha.as_ref().or_else(|| {
                previous
                    .as_ref()
                    .map(|candidate| &candidate.record.observed_source_commit_sha)
            });
            match comparison {
                Some(previous_commit) if previous_commit != &observed_commit => {
                    !self.is_ancestor(&mirror, previous_commit, &observed_commit)?
                }
                _ => false,
            }
        } else {
            false
        };

        let signature_verified = if source.require_signed_commit {
            self.git
                .run(ManagedGitCommand::VerifyCommit {
                    git_dir: mirror.clone(),
                    commit: observed_commit.as_str().to_string(),
                })
                .is_ok()
        } else {
            true
        };

        let (candidate_documents, mut acquisition_issues) =
            match self.read_source_snapshot(&mirror, &observed_commit) {
                Ok(snapshot) => snapshot,
                Err(error) if error.reason == "git_output_too_large" => (
                    Vec::new(),
                    vec![acquisition_issue(
                        "git_tree_too_large",
                        "The candidate tree exceeds the inspection output limit",
                        "<candidate>",
                    )],
                ),
                Err(error) => return Err(error),
            };
        let baseline_documents = match source.active_source_commit_sha.as_ref() {
            Some(active) => match self.read_source_snapshot(&mirror, active) {
                Ok((documents, issues)) if issues.is_empty() => documents,
                Ok(_) | Err(_) => {
                    acquisition_issues.push(acquisition_issue(
                        "baseline_source_unavailable",
                        "The active baseline could not be inspected safely",
                        "<baseline>",
                    ));
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        let settings = self.store.get_or_initialize_settings(checked_at_ms)?;
        let scheduled_agent_ids = self
            .store
            .list_schedules()?
            .into_iter()
            .filter(|schedule| schedule.enabled)
            .filter_map(|schedule| match schedule.target {
                ScheduleTarget::Agent { agent, .. } => Some(agent),
                ScheduleTarget::Orchestrator { .. } | ScheduleTarget::Session { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        let trust_digest = ContentSha256::parse(canonical_json_sha256(&serde_json::json!({
            "phase": "5b",
            "projectTrust": [],
        })))
        .expect("canonical trust digest is SHA-256");
        let baseline_dependencies = ConfigurationDependencySnapshot {
            instructions_source_commit_sha: source.active_source_commit_sha.clone(),
            canonical_remote_identity: source.canonical_remote_identity.clone(),
            tracking: source.tracking.clone(),
            project_content_sha256: None,
            personal_source_commit_sha: None,
            settings_revision: settings.revision,
            trust_digest: trust_digest.clone(),
        };
        let candidate_dependencies = ConfigurationDependencySnapshot {
            instructions_source_commit_sha: Some(observed_commit.clone()),
            canonical_remote_identity: source.canonical_remote_identity.clone(),
            tracking: source.tracking.clone(),
            project_content_sha256: None,
            personal_source_commit_sha: None,
            settings_revision: settings.revision,
            trust_digest,
        };
        let report = inspect_configuration_candidate(&ConfigurationCandidateInspectionInput {
            layer: ValueLayer::Instructions,
            baseline_documents,
            candidate_documents,
            baseline_dependencies,
            candidate_dependencies,
            scheduled_agent_ids,
            history_rewritten,
        });
        let inspection = StoredCandidateInspection {
            report,
            acquisition_issues,
            signature_required: source.require_signed_commit,
            signature_verified,
        };
        let activatable = inspection.report.activatable
            && inspection.acquisition_issues.is_empty()
            && inspection.signature_verified;
        let state = if !activatable {
            InstructionCandidateState::Blocked
        } else if source.policy == InstructionsUpdatePolicy::ApproveEach
            || inspection.report.local_approval_required
        {
            InstructionCandidateState::AwaitingApproval
        } else {
            InstructionCandidateState::Discovered
        };
        let diff_digest = ContentSha256::parse(canonical_json_sha256(&inspection))
            .expect("canonical stored inspection digest is SHA-256");
        let candidate = InstructionCandidateRecord {
            id: InstructionCandidateId::generate(),
            source_id: source.id.clone(),
            observed_source_commit_sha: observed_commit,
            observed_tag_object_sha: resolved.tag_object,
            state,
            safety: inspection.report.safety,
            diff_digest,
            activatable,
            created_at_ms: checked_at_ms,
            updated_at_ms: checked_at_ms,
        };
        let inspection_json = serde_json::to_value(&inspection).map_err(|_| state_error())?;
        let candidate = self.store.persist_instruction_candidate_check(
            &source.id,
            source.revision,
            checked_at_ms,
            &candidate,
            &inspection_json,
        )?;
        Ok(InstructionsCheckOutcome {
            checked_at_ms,
            unchanged: false,
            fetched,
            candidate: Some(candidate),
            safe_error_reason: None,
        })
    }

    pub fn load_candidate_inspection(
        &self,
        candidate_id: &InstructionCandidateId,
    ) -> Result<
        Option<(InstructionCandidateRecord, StoredCandidateInspection)>,
        InstructionsServiceError,
    > {
        self.store
            .get_instruction_candidate(candidate_id)?
            .map(|stored| {
                serde_json::from_value::<StoredCandidateInspection>(stored.inspection)
                    .map(|inspection| (stored.record, inspection))
                    .map_err(|_| state_error())
            })
            .transpose()
    }

    pub fn active_snapshot(
        &self,
        source_id: &InstructionSourceId,
    ) -> Result<Option<(InstructionsSourceRecord, Vec<SourceDocument>)>, InstructionsServiceError>
    {
        let Some(source) = self.store.get_instruction_source(source_id)? else {
            return Ok(None);
        };
        let Some(active_commit) = source.active_source_commit_sha.as_ref() else {
            return Ok(None);
        };
        let mirror = self.mirror_path(&source.canonical_remote_identity);
        let owner = repository_owner(&mirror);
        let _owner = owner.lock().expect("instructions mirror owner lock");
        let (documents, issues) = self.read_source_snapshot(&mirror, active_commit)?;
        if !issues.is_empty() {
            return Err(InstructionsServiceError {
                reason: "active_instructions_invalid",
                safe_message: "The active instructions snapshot failed integrity inspection"
                    .to_string(),
                retryable: false,
                actual_revision: None,
            });
        }
        Ok(Some((source, documents)))
    }

    pub fn candidate_snapshot(
        &self,
        candidate_id: &InstructionCandidateId,
    ) -> Result<
        Option<(
            InstructionsSourceRecord,
            InstructionCandidateRecord,
            Vec<SourceDocument>,
        )>,
        InstructionsServiceError,
    > {
        let Some(stored) = self.store.get_instruction_candidate(candidate_id)? else {
            return Ok(None);
        };
        if !stored.record.activatable
            || !matches!(
                stored.record.state,
                InstructionCandidateState::Discovered
                    | InstructionCandidateState::AwaitingApproval
                    | InstructionCandidateState::Approved
            )
        {
            return Err(InstructionsServiceError {
                reason: "instructions_candidate_not_activatable",
                safe_message: "The instructions candidate is not eligible for activation"
                    .to_string(),
                retryable: false,
                actual_revision: None,
            });
        }
        let source = self
            .store
            .get_instruction_source(&stored.record.source_id)?
            .ok_or_else(source_not_found)?;
        let mirror = self.mirror_path(&source.canonical_remote_identity);
        let owner = repository_owner(&mirror);
        let _owner = owner.lock().expect("instructions mirror owner lock");
        let (documents, issues) =
            self.read_source_snapshot(&mirror, &stored.record.observed_source_commit_sha)?;
        if !issues.is_empty() {
            return Err(InstructionsServiceError {
                reason: "instructions_candidate_changed",
                safe_message: "The instructions candidate no longer passes integrity inspection"
                    .to_string(),
                retryable: false,
                actual_revision: Some(source.revision),
            });
        }
        Ok(Some((source, stored.record, documents)))
    }

    pub fn export_candidate_git_bundle(
        &self,
        candidate_id: &InstructionCandidateId,
        expected_commit: &SourceCommitSha,
    ) -> Result<(String, Vec<u8>), InstructionsServiceError> {
        let (source, candidate, _) =
            self.candidate_snapshot(candidate_id)?
                .ok_or_else(|| InstructionsServiceError {
                    reason: "instructions_candidate_not_found",
                    safe_message: "The instructions candidate was not found".to_string(),
                    retryable: false,
                    actual_revision: None,
                })?;
        if &candidate.observed_source_commit_sha != expected_commit {
            return Err(InstructionsServiceError {
                reason: "instructions_candidate_changed",
                safe_message: "The instructions candidate changed before materialization"
                    .to_string(),
                retryable: true,
                actual_revision: Some(source.revision),
            });
        }
        let mirror = self.mirror_path(&source.canonical_remote_identity);
        let owner = repository_owner(&mirror);
        let _owner = owner.lock().expect("instructions mirror owner lock");
        let export_ref = candidate_refs(&source.id, expected_commit).commit;
        self.pin_candidate_commit(&mirror, &export_ref, expected_commit)?;
        let bytes = self.git.create_bundle_bytes(&mirror, &export_ref)?;
        Ok((export_ref, bytes))
    }

    pub fn export_active_git_bundle(
        &self,
        source_id: &InstructionSourceId,
        expected_commit: &SourceCommitSha,
    ) -> Result<(String, Vec<u8>), InstructionsServiceError> {
        let source = self
            .store
            .get_instruction_source(source_id)?
            .ok_or_else(source_not_found)?;
        if source.active_source_commit_sha.as_ref() != Some(expected_commit) {
            return Err(InstructionsServiceError {
                reason: "active_instructions_changed",
                safe_message: "The active instructions changed before bundle materialization"
                    .to_string(),
                retryable: true,
                actual_revision: Some(source.revision),
            });
        }
        let mirror = self.mirror_path(&source.canonical_remote_identity);
        let owner = repository_owner(&mirror);
        let _owner = owner.lock().expect("instructions mirror owner lock");
        let export_ref = format!(
            "refs/pandamux/exports/{}/{}",
            sha256_hex(source_id.as_str().as_bytes()),
            expected_commit.as_str()
        );
        self.pin_candidate_commit(&mirror, &export_ref, expected_commit)?;
        let bytes = self.git.create_bundle_bytes(&mirror, &export_ref)?;
        Ok((export_ref, bytes))
    }

    fn finish_failed_check(
        &self,
        source: &InstructionsSourceRecord,
        checked_at_ms: u64,
        reason: &'static str,
    ) -> Result<InstructionsCheckOutcome, InstructionsServiceError> {
        self.store.record_instruction_source_check(
            &source.id,
            source.revision,
            checked_at_ms,
            Some(reason),
        )?;
        Ok(InstructionsCheckOutcome {
            checked_at_ms,
            unchanged: false,
            fetched: false,
            candidate: None,
            safe_error_reason: Some(reason.to_string()),
        })
    }

    fn mirror_path(&self, remote: &str) -> PathBuf {
        self.paths
            .instructions()
            .join(format!("{}.git", sha256_hex(remote.as_bytes())))
    }

    fn ensure_mirror(&self, mirror: &Path) -> Result<(), InstructionsServiceError> {
        self.paths
            .ensure_private_roots()
            .map_err(|_| managed_path_error())?;
        self.recover_mirror_candidates(mirror)?;
        if mirror.exists() {
            let output = self.git.run(ManagedGitCommand::IsBareRepository {
                git_dir: mirror.to_path_buf(),
            })?;
            if parse_trimmed(&output.stdout)? == "true" {
                return Ok(());
            }
            return Err(managed_path_error());
        }
        let name = mirror
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(managed_path_error)?;
        let candidate = self.paths.instructions().join(format!(
            "{MIRROR_INIT_PREFIX}{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        create_private_directory(&candidate)?;
        self.git.run(ManagedGitCommand::InitBare {
            git_dir: candidate.clone(),
        })?;
        let output = self.git.run(ManagedGitCommand::IsBareRepository {
            git_dir: candidate.clone(),
        })?;
        if parse_trimmed(&output.stdout)? != "true" {
            return Err(managed_path_error());
        }
        fs::rename(&candidate, mirror).map_err(|_| managed_path_error())?;
        Ok(())
    }

    fn recover_mirror_candidates(&self, mirror: &Path) -> Result<(), InstructionsServiceError> {
        let target = mirror
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(managed_path_error)?;
        let prefix = format!("{MIRROR_INIT_PREFIX}{target}-");
        for entry in fs::read_dir(self.paths.instructions()).map_err(|_| managed_path_error())? {
            let entry = entry.map_err(|_| managed_path_error())?;
            let name = entry.file_name();
            if !name.to_string_lossy().starts_with(&prefix) {
                continue;
            }
            let candidate = entry.path();
            let valid = self
                .git
                .run(ManagedGitCommand::IsBareRepository {
                    git_dir: candidate.clone(),
                })
                .ok()
                .and_then(|output| parse_trimmed(&output.stdout).ok())
                .as_deref()
                == Some("true");
            if !mirror.exists() && valid {
                fs::rename(candidate, mirror).map_err(|_| managed_path_error())?;
            } else {
                move_owned_mirror_candidate(&candidate, self.paths.recovery())?;
            }
        }
        Ok(())
    }

    fn resolve_remote(
        &self,
        source: &InstructionsSourceRecord,
        mirror: &Path,
    ) -> Result<ResolvedRemote, ManagedGitError> {
        match &source.tracking {
            TrackingRef::Branch(reference) => {
                let output = self.git.run(ManagedGitCommand::LsRemote {
                    git_dir: mirror.to_path_buf(),
                    remote: source.canonical_remote_identity.clone(),
                    references: vec![reference.clone()],
                })?;
                let values = parse_ls_remote(&output.stdout)?;
                let commit = exact_remote_ref(&values, reference)?;
                Ok(ResolvedRemote {
                    commit,
                    tag_object: None,
                    fetch_source: reference.clone(),
                })
            }
            TrackingRef::Tag(reference) => {
                let peeled = format!("{reference}^{{}}");
                let output = self.git.run(ManagedGitCommand::LsRemote {
                    git_dir: mirror.to_path_buf(),
                    remote: source.canonical_remote_identity.clone(),
                    references: vec![reference.clone(), peeled.clone()],
                })?;
                let values = parse_ls_remote(&output.stdout)?;
                let direct = exact_remote_ref(&values, reference)?;
                let commit = values
                    .get(&peeled)
                    .cloned()
                    .unwrap_or_else(|| direct.clone());
                Ok(ResolvedRemote {
                    tag_object: (direct != commit).then_some(direct),
                    commit,
                    fetch_source: reference.clone(),
                })
            }
            TrackingRef::Commit(commit) => {
                let output = self.git.run(ManagedGitCommand::LsRemote {
                    git_dir: mirror.to_path_buf(),
                    remote: source.canonical_remote_identity.clone(),
                    references: Vec::new(),
                })?;
                let values = parse_ls_remote(&output.stdout)?;
                let advertised = values.values().any(|value| value == commit);
                let local = self.object_is_commit(mirror, commit);
                if !advertised && !local {
                    // A closed fetch of the exact object is the explicit-fetchability
                    // check. Failure is classified without exposing remote details.
                }
                Ok(ResolvedRemote {
                    commit: commit.clone(),
                    tag_object: None,
                    fetch_source: commit.as_str().to_string(),
                })
            }
        }
    }

    fn resolve_fetched_commit(
        &self,
        mirror: &Path,
        source_ref: &str,
    ) -> Result<SourceCommitSha, ManagedGitError> {
        let output = self.git.run(ManagedGitCommand::RevParse {
            git_dir: mirror.to_path_buf(),
            revision: format!("{source_ref}^{{commit}}"),
        })?;
        let commit = SourceCommitSha::parse(parse_trimmed_git(&output.stdout)?)
            .map_err(|_| invalid_git_state())?;
        let kind = self.git.run(ManagedGitCommand::ObjectType {
            git_dir: mirror.to_path_buf(),
            object: commit.as_str().to_string(),
        })?;
        if parse_trimmed_git(&kind.stdout)? != "commit" {
            return Err(invalid_git_state());
        }
        Ok(commit)
    }

    fn pin_candidate_commit(
        &self,
        mirror: &Path,
        reference: &str,
        commit: &SourceCommitSha,
    ) -> Result<(), InstructionsServiceError> {
        if let Ok(output) = self.git.run(ManagedGitCommand::RevParse {
            git_dir: mirror.to_path_buf(),
            revision: reference.to_string(),
        }) {
            let existing = parse_trimmed(&output.stdout)?;
            if existing == commit.as_str() {
                return Ok(());
            }
            return Err(InstructionsServiceError {
                reason: "candidate_ref_conflict",
                safe_message: "A managed candidate reference has conflicting content".to_string(),
                retryable: false,
                actual_revision: None,
            });
        }
        self.git.run(ManagedGitCommand::UpdateRefs {
            git_dir: mirror.to_path_buf(),
            transaction: format!("start\ncreate {reference} {commit}\nprepare\ncommit\n"),
        })?;
        Ok(())
    }

    fn object_is_commit(&self, mirror: &Path, commit: &SourceCommitSha) -> bool {
        self.git
            .run(ManagedGitCommand::ObjectType {
                git_dir: mirror.to_path_buf(),
                object: commit.as_str().to_string(),
            })
            .ok()
            .and_then(|output| parse_trimmed(&output.stdout).ok())
            .as_deref()
            == Some("commit")
    }

    fn is_ancestor(
        &self,
        mirror: &Path,
        ancestor: &SourceCommitSha,
        descendant: &SourceCommitSha,
    ) -> Result<bool, InstructionsServiceError> {
        let output = self.git.run(ManagedGitCommand::IsAncestor {
            git_dir: mirror.to_path_buf(),
            ancestor: ancestor.as_str().to_string(),
            descendant: descendant.as_str().to_string(),
        })?;
        Ok(output.exit_code == Some(0))
    }

    fn read_source_snapshot(
        &self,
        mirror: &Path,
        commit: &SourceCommitSha,
    ) -> Result<(Vec<SourceDocument>, Vec<CandidateAcquisitionIssue>), InstructionsServiceError>
    {
        let output = self.git.run(ManagedGitCommand::ListTree {
            git_dir: mirror.to_path_buf(),
            treeish: commit.as_str().to_string(),
        })?;
        let mut issues = Vec::new();
        let mut entries = Vec::new();
        let mut casefolded = BTreeSet::new();
        for record in output.stdout.split(|byte| *byte == 0) {
            if record.is_empty() {
                continue;
            }
            let Ok(record) = std::str::from_utf8(record) else {
                issues.push(acquisition_issue(
                    "configuration_path_non_utf8",
                    "A repository path is not valid UTF-8",
                    "<invalid>",
                ));
                continue;
            };
            let Some((metadata, path)) = record.split_once('\t') else {
                issues.push(acquisition_issue(
                    "git_tree_invalid",
                    "The candidate tree contains an invalid entry",
                    "<invalid>",
                ));
                continue;
            };
            let mut metadata = metadata.split_whitespace();
            let mode = metadata.next().unwrap_or_default();
            let kind = metadata.next().unwrap_or_default();
            let object = metadata.next().unwrap_or_default();
            if metadata.next().is_some() || mode != "100644" || kind != "blob" {
                issues.push(acquisition_issue(
                    "configuration_special_mode_rejected",
                    "Links, executable files, submodules, and special modes are not allowed",
                    path,
                ));
                continue;
            }
            if SourceCommitSha::parse(object.to_string()).is_err()
                || !valid_instructions_path(path)
                || !portable_path(path)
            {
                issues.push(acquisition_issue(
                    "configuration_path_rejected",
                    "The candidate contains an unsupported or unsafe path",
                    path,
                ));
                continue;
            }
            if !casefolded.insert(path.to_lowercase()) {
                issues.push(acquisition_issue(
                    "configuration_path_case_collision",
                    "Candidate paths collide when compared without case",
                    path,
                ));
                continue;
            }
            entries.push((path.to_string(), object.to_string()));
        }
        if entries.len() > pandamux_orchestrator::schema::MAX_SOURCE_FILES {
            issues.push(acquisition_issue(
                "config_file_count_limit",
                "The candidate contains more configuration files than allowed",
                "<candidate>",
            ));
            entries.truncate(pandamux_orchestrator::schema::MAX_SOURCE_FILES);
        }

        let mut sized = Vec::new();
        let mut aggregate = 0usize;
        for (path, object) in entries {
            let output = self.git.run(ManagedGitCommand::CatFileSize {
                git_dir: mirror.to_path_buf(),
                object: object.clone(),
            })?;
            let size = parse_trimmed(&output.stdout)?
                .parse::<usize>()
                .map_err(|_| managed_path_error())?;
            let limit = if path == "orchestrator.yaml" {
                pandamux_orchestrator::schema::MAX_YAML_FILE_BYTES
            } else {
                pandamux_orchestrator::schema::MAX_MARKDOWN_FILE_BYTES
            };
            if size > limit {
                issues.push(acquisition_issue(
                    "config_file_too_large",
                    "A candidate configuration file exceeds its size limit",
                    &path,
                ));
                continue;
            }
            aggregate = aggregate.saturating_add(size);
            sized.push((path, object, size));
        }
        if aggregate > pandamux_orchestrator::schema::MAX_SOURCE_BYTES {
            issues.push(acquisition_issue(
                "config_bundle_size_limit",
                "The candidate configuration set exceeds its aggregate size limit",
                "<candidate>",
            ));
            return Ok((Vec::new(), issues));
        }
        let mut documents = Vec::new();
        for (path, object, size) in sized {
            let output = self.git.run(ManagedGitCommand::CatFile {
                git_dir: mirror.to_path_buf(),
                object,
                max_bytes: size,
            })?;
            match SourceDocument::new(path.clone(), output.stdout) {
                Ok(document) => documents.push(document),
                Err(_) => issues.push(acquisition_issue(
                    "configuration_path_rejected",
                    "The candidate contains an unsupported or unsafe path",
                    &path,
                )),
            }
        }
        Ok((documents, issues))
    }
}

pub struct InstructionsPollingService {
    state: Arc<(Mutex<bool>, Condvar)>,
    worker: Option<thread::JoinHandle<()>>,
}

impl InstructionsPollingService {
    pub fn start(service: InstructionsService) -> io::Result<Self> {
        Self::start_with_interval(service, INSTRUCTIONS_POLL_INTERVAL)
    }

    fn start_with_interval(service: InstructionsService, interval: Duration) -> io::Result<Self> {
        let state = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_state = state.clone();
        let worker = thread::Builder::new()
            .name("pandamux-instructions-poller".to_string())
            .spawn(move || {
                loop {
                    let _ = service.check_all_sources();
                    let (lock, wake) = &*worker_state;
                    let shutdown = lock.lock().expect("instructions poller state lock");
                    if *shutdown {
                        break;
                    }
                    let (shutdown, _) = wake
                        .wait_timeout(shutdown, interval)
                        .expect("instructions poller wait");
                    if *shutdown {
                        break;
                    }
                }
            })?;
        Ok(Self {
            state,
            worker: Some(worker),
        })
    }

    pub fn shutdown(mut self) -> io::Result<()> {
        self.stop_and_join()
    }

    fn stop_and_join(&mut self) -> io::Result<()> {
        let (lock, wake) = &*self.state;
        *lock.lock().expect("instructions poller state lock") = true;
        wake.notify_all();
        match self.worker.take() {
            Some(worker) => worker
                .join()
                .map_err(|_| io::Error::other("instructions poller did not join cleanly")),
            None => Ok(()),
        }
    }
}

impl Drop for InstructionsPollingService {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}

#[derive(Clone)]
struct ResolvedRemote {
    commit: SourceCommitSha,
    tag_object: Option<SourceCommitSha>,
    fetch_source: String,
}

struct CandidateRefs {
    source: String,
    commit: String,
}

fn candidate_refs(source_id: &InstructionSourceId, commit: &SourceCommitSha) -> CandidateRefs {
    let source = sha256_hex(source_id.as_str().as_bytes());
    let base = format!("refs/pandamux/candidates/{source}/{}", commit.as_str());
    CandidateRefs {
        source: format!("{base}/source"),
        commit: format!("{base}/commit"),
    }
}

fn candidate_matches(
    candidate: Option<&crate::StoredInstructionCandidate>,
    resolved: &ResolvedRemote,
) -> bool {
    candidate.is_some_and(|candidate| {
        candidate.record.observed_source_commit_sha == resolved.commit
            && candidate.record.observed_tag_object_sha == resolved.tag_object
    })
}

fn parse_ls_remote(bytes: &[u8]) -> Result<BTreeMap<String, SourceCommitSha>, ManagedGitError> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid_git_state())?;
    let mut values = BTreeMap::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let (object, reference) = line.split_once('\t').ok_or_else(invalid_git_state)?;
        let object = SourceCommitSha::parse(object.to_string()).map_err(|_| invalid_git_state())?;
        if values.insert(reference.to_string(), object).is_some() {
            return Err(invalid_git_state());
        }
    }
    Ok(values)
}

fn exact_remote_ref(
    values: &BTreeMap<String, SourceCommitSha>,
    reference: &str,
) -> Result<SourceCommitSha, ManagedGitError> {
    values
        .get(reference)
        .cloned()
        .ok_or_else(|| ManagedGitError {
            reason: "tracked_ref_not_found",
            safe_message: "The tracked instructions reference was not found".to_string(),
            retryable: false,
        })
}

fn validate_source(source: &InstructionsSourceRecord) -> Result<(), InstructionsServiceError> {
    if source.canonical_remote_identity.trim() != source.canonical_remote_identity
        || source.canonical_remote_identity.is_empty()
        || source.canonical_remote_identity.len() > 4_096
        || source
            .canonical_remote_identity
            .bytes()
            .any(|byte| byte.is_ascii_control())
    {
        return Err(InstructionsServiceError {
            reason: "instructions_remote_invalid",
            safe_message: "The instructions remote identity is invalid".to_string(),
            retryable: false,
            actual_revision: None,
        });
    }
    let valid_tracking = match &source.tracking {
        TrackingRef::Branch(reference) => valid_full_ref(reference, "refs/heads/"),
        TrackingRef::Tag(reference) => valid_full_ref(reference, "refs/tags/"),
        TrackingRef::Commit(_) => true,
    };
    if !valid_tracking
        || source.policy == InstructionsUpdatePolicy::TrustBranch
            && !matches!(source.tracking, TrackingRef::Branch(_))
    {
        return Err(InstructionsServiceError {
            reason: "instructions_tracking_invalid",
            safe_message: "Instructions tracking must use a valid full branch, tag, or commit"
                .to_string(),
            retryable: false,
            actual_revision: None,
        });
    }
    Ok(())
}

fn valid_full_ref(reference: &str, prefix: &str) -> bool {
    reference.starts_with(prefix)
        && reference.len() > prefix.len()
        && reference.len() <= 512
        && !["..", "@{", "//"]
            .iter()
            .any(|pattern| reference.contains(pattern))
        && !reference.ends_with(['.', '/'])
        && !reference.bytes().any(|byte| {
            byte.is_ascii_control()
                || matches!(byte, b' ' | b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\')
        })
}

fn valid_instructions_path(path: &str) -> bool {
    path == "orchestrator.yaml"
        || nested_markdown(path, "agents/")
        || nested_markdown(path, "knowledge/")
        || nested_markdown(path, "prompts/")
}

fn nested_markdown(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix).is_some_and(|remainder| {
        !remainder.is_empty() && !remainder.contains('/') && remainder.ends_with(".md")
    })
}

fn portable_path(path: &str) -> bool {
    !path.split('/').any(|segment| {
        if segment.is_empty()
            || segment.ends_with(' ')
            || segment.ends_with('.')
            || segment.chars().any(|character| {
                character.is_control()
                    || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
            })
        {
            return true;
        }
        let device = segment
            .split_once('.')
            .map_or(segment, |(prefix, _)| prefix)
            .to_ascii_uppercase();
        matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || device
                .strip_prefix("COM")
                .or_else(|| device.strip_prefix("LPT"))
                .is_some_and(|suffix| matches!(suffix.as_bytes(), [b'1'..=b'9']))
    })
}

fn acquisition_issue(code: &str, reason: &str, path: &str) -> CandidateAcquisitionIssue {
    CandidateAcquisitionIssue {
        code: code.to_string(),
        safe_reason: reason.to_string(),
        origin: ValueOrigin {
            layer: ValueLayer::Instructions,
            logical_path: safe_diagnostic_path(path),
            field: "/".to_string(),
        },
    }
}

fn safe_diagnostic_path(path: &str) -> String {
    let mut safe = String::new();
    for character in path.chars() {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        if safe.len() + character.len_utf8() > 240 {
            break;
        }
        safe.push(character);
    }
    if safe.is_empty() {
        "<invalid>".to_string()
    } else {
        safe
    }
}

fn repository_owner(path: &Path) -> Arc<Mutex<()>> {
    static OWNERS: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let owners = OWNERS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut owners = owners.lock().expect("instructions mirror owner registry");
    owners.retain(|_, owner| owner.strong_count() > 0);
    if let Some(owner) = owners.get(path).and_then(Weak::upgrade) {
        return owner;
    }
    let owner = Arc::new(Mutex::new(()));
    owners.insert(path.to_path_buf(), Arc::downgrade(&owner));
    owner
}

fn create_private_directory(path: &Path) -> Result<(), InstructionsServiceError> {
    fs::create_dir_all(path).map_err(|_| managed_path_error())?;
    let metadata = fs::symlink_metadata(path).map_err(|_| managed_path_error())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(managed_path_error());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| managed_path_error())?;
    }
    Ok(())
}

fn move_owned_mirror_candidate(
    candidate: &Path,
    recovery: &Path,
) -> Result<(), InstructionsServiceError> {
    create_private_directory(recovery)?;
    let name = candidate
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(managed_path_error)?;
    let destination = recovery.join(format!("{name}-{}", uuid::Uuid::new_v4().simple()));
    fs::rename(candidate, destination).map_err(|_| managed_path_error())
}

fn parse_trimmed(bytes: &[u8]) -> Result<String, InstructionsServiceError> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| managed_path_error())?
        .trim();
    if value.is_empty() {
        return Err(managed_path_error());
    }
    Ok(value.to_string())
}

fn parse_trimmed_git(bytes: &[u8]) -> Result<String, ManagedGitError> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| invalid_git_state())?
        .trim();
    if value.is_empty() {
        return Err(invalid_git_state());
    }
    Ok(value.to_string())
}

fn source_not_found() -> InstructionsServiceError {
    InstructionsServiceError {
        reason: "instructions_source_not_found",
        safe_message: "The instructions source was not found".to_string(),
        retryable: false,
        actual_revision: None,
    }
}

fn state_error() -> InstructionsServiceError {
    InstructionsServiceError {
        reason: "instructions_state_failed",
        safe_message: "The instructions state could not be read or persisted".to_string(),
        retryable: true,
        actual_revision: None,
    }
}

fn managed_path_error() -> InstructionsServiceError {
    InstructionsServiceError {
        reason: "managed_path_unsafe",
        safe_message: "The managed instructions path is unsafe".to_string(),
        retryable: false,
        actual_revision: None,
    }
}

fn invalid_git_state() -> ManagedGitError {
    ManagedGitError {
        reason: "instructions_git_state_invalid",
        safe_message: "The managed instructions repository has invalid Git state".to_string(),
        retryable: false,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    value
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{ConfigurationUpdateSafety, InstructionCandidateState};
    use std::process::Command;

    struct RemoteFixture {
        root: tempfile::TempDir,
        remote: PathBuf,
        work: PathBuf,
    }

    impl RemoteFixture {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("temporary remote fixture");
            let remote = root.path().join("remote.git");
            let work = root.path().join("work");
            git(root.path(), &["init", "--bare", path(&remote)]);
            git(root.path(), &["init", "-b", "main", path(&work)]);
            git(&work, &["config", "user.name", "PandaMUX Test"]);
            git(&work, &["config", "user.email", "pandamux@example.invalid"]);
            write_valid_tree(&work, "read_only", "Initial");
            git(&work, &["add", "--all"]);
            git(&work, &["commit", "-m", "initial"]);
            git(&work, &["remote", "add", "origin", path(&remote)]);
            git(&work, &["push", "-u", "origin", "main"]);
            Self { root, remote, work }
        }

        fn remote(&self) -> String {
            self.remote.to_string_lossy().into_owned()
        }

        fn head(&self) -> SourceCommitSha {
            SourceCommitSha::parse(git_output(&self.work, &["rev-parse", "HEAD"]))
                .expect("fixture commit")
        }

        fn advance(&self, label: &str) -> SourceCommitSha {
            write_valid_tree(&self.work, "read_only", label);
            git(&self.work, &["add", "--all"]);
            git(&self.work, &["commit", "-m", label]);
            git(&self.work, &["push", "origin", "main"]);
            self.head()
        }

        fn annotated_tag(&self, tag: &str) {
            git(&self.work, &["tag", "-a", tag, "-m", tag]);
            git(&self.work, &["push", "origin", &format!("refs/tags/{tag}")]);
        }

        fn lightweight_tag(&self, tag: &str) {
            git(&self.work, &["tag", tag]);
            git(&self.work, &["push", "origin", &format!("refs/tags/{tag}")]);
        }

        fn force_unrelated(&self) -> SourceCommitSha {
            let rewrite = self.root.path().join("rewrite");
            git(self.root.path(), &["init", "-b", "main", path(&rewrite)]);
            git(&rewrite, &["config", "user.name", "PandaMUX Test"]);
            git(
                &rewrite,
                &["config", "user.email", "pandamux@example.invalid"],
            );
            write_valid_tree(&rewrite, "read_only", "Rewritten");
            git(&rewrite, &["add", "--all"]);
            git(&rewrite, &["commit", "-m", "rewrite"]);
            git(&rewrite, &["remote", "add", "origin", path(&self.remote)]);
            git(
                &rewrite,
                &["push", "--force", "origin", "HEAD:refs/heads/main"],
            );
            SourceCommitSha::parse(git_output(&rewrite, &["rev-parse", "HEAD"]))
                .expect("rewritten commit")
        }

        fn make_invalid(&self) -> SourceCommitSha {
            fs::write(self.work.join("agents/builder.md"), b"not frontmatter\n")
                .expect("write invalid agent");
            fs::write(self.work.join("prompts/intake.md"), b"also invalid\n")
                .expect("write invalid prompt");
            git(&self.work, &["add", "--all"]);
            git(&self.work, &["commit", "-m", "invalid"]);
            git(&self.work, &["push", "origin", "main"]);
            self.head()
        }

        fn make_executable(&self) -> SourceCommitSha {
            git(
                &self.work,
                &["update-index", "--chmod=+x", "agents/builder.md"],
            );
            git(&self.work, &["commit", "-m", "executable mode"]);
            git(&self.work, &["push", "origin", "main"]);
            self.head()
        }
    }

    struct ServiceFixture {
        _root: tempfile::TempDir,
        paths: DataPaths,
        store: Store,
        service: InstructionsService,
    }

    impl ServiceFixture {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("temporary app-data fixture");
            let paths = DataPaths::from_root(root.path().join("app")).expect("paths");
            paths.ensure_private_roots().expect("private roots");
            let store = Store::in_memory().expect("store");
            let service = InstructionsService::new(paths.clone(), store.clone());
            Self {
                _root: root,
                paths,
                store,
                service,
            }
        }

        fn source(
            &self,
            remote: &RemoteFixture,
            tracking: TrackingRef,
            signed: bool,
        ) -> InstructionsSourceRecord {
            let source = InstructionsSourceRecord {
                id: InstructionSourceId::generate(),
                canonical_remote_identity: remote.remote(),
                tracking,
                policy: InstructionsUpdatePolicy::ApproveEach,
                require_signed_commit: signed,
                revision: ConfigurationRevision::INITIAL,
                last_checked_at_ms: None,
                last_error_reason: None,
                active_source_commit_sha: None,
                active_bundle_sha: None,
            };
            self.store
                .save_instruction_source(&source, 1, 1)
                .expect("save source");
            source
        }

        fn mark_active(
            &self,
            source: &InstructionsSourceRecord,
            commit: SourceCommitSha,
            now: u64,
        ) -> InstructionsSourceRecord {
            let mut source = self
                .store
                .get_instruction_source(&source.id)
                .expect("load source")
                .expect("source");
            source.revision = source.revision.checked_next().expect("next revision");
            source.active_source_commit_sha = Some(commit);
            self.store
                .save_instruction_source(&source, 1, now)
                .expect("save active source");
            source
        }
    }

    #[test]
    fn branch_no_change_skips_fetch_and_branch_advance_creates_candidate() {
        let remote = RemoteFixture::new();
        let fixture = ServiceFixture::new();
        let source = fixture.source(
            &remote,
            TrackingRef::Branch("refs/heads/main".to_string()),
            false,
        );
        let first = fixture
            .service
            .check_source(&source.id, source.revision, 10)
            .expect("first check");
        assert!(!first.unchanged);
        assert!(first.fetched, "first check outcome: {first:?}");
        let first_candidate = first.candidate.expect("candidate");
        assert_eq!(first_candidate.observed_source_commit_sha, remote.head());
        assert!(first_candidate.activatable);

        let second = fixture
            .service
            .check_source(&source.id, source.revision, 11)
            .expect("unchanged check");
        assert!(second.unchanged);
        assert!(!second.fetched);
        assert_eq!(second.candidate, Some(first_candidate));

        let advanced = remote.advance("Advance");
        let third = fixture
            .service
            .check_source(&source.id, source.revision, 12)
            .expect("advanced check");
        assert!(!third.unchanged);
        assert!(third.fetched);
        assert_eq!(
            third
                .candidate
                .expect("advanced candidate")
                .observed_source_commit_sha,
            advanced
        );
        let mirror = fs::read_dir(fixture.paths.instructions())
            .expect("mirror directory")
            .filter_map(Result::ok)
            .find(|entry| entry.path().extension().is_some_and(|value| value == "git"))
            .expect("bare mirror")
            .path();
        assert!(!mirror.join("orchestrator.yaml").exists());
        assert_eq!(
            git_output(
                fixture.paths.instructions(),
                &[
                    "--git-dir",
                    path(&mirror),
                    "rev-parse",
                    "--is-bare-repository"
                ]
            ),
            "true"
        );
    }

    #[test]
    fn annotated_lightweight_and_exact_commit_tracking_retain_distinct_identity() {
        let annotated_remote = RemoteFixture::new();
        annotated_remote.annotated_tag("v1");
        let fixture = ServiceFixture::new();
        let tag_source = fixture.source(
            &annotated_remote,
            TrackingRef::Tag("refs/tags/v1".to_string()),
            false,
        );
        let tagged = fixture
            .service
            .check_source(&tag_source.id, tag_source.revision, 20)
            .expect("tag check")
            .candidate
            .expect("tag candidate");
        assert_eq!(tagged.observed_source_commit_sha, annotated_remote.head());
        assert!(tagged.observed_tag_object_sha.is_some());

        let lightweight_remote = RemoteFixture::new();
        lightweight_remote.lightweight_tag("v1");
        let lightweight_fixture = ServiceFixture::new();
        let lightweight_source = lightweight_fixture.source(
            &lightweight_remote,
            TrackingRef::Tag("refs/tags/v1".to_string()),
            false,
        );
        let lightweight = lightweight_fixture
            .service
            .check_source(&lightweight_source.id, lightweight_source.revision, 21)
            .expect("lightweight tag check")
            .candidate
            .expect("lightweight candidate");
        assert_eq!(lightweight.observed_tag_object_sha, None);

        let commit_remote = RemoteFixture::new();
        let pinned = commit_remote.head();
        let commit_fixture = ServiceFixture::new();
        let commit_source =
            commit_fixture.source(&commit_remote, TrackingRef::Commit(pinned.clone()), false);
        let first = commit_fixture
            .service
            .check_source(&commit_source.id, commit_source.revision, 22)
            .expect("commit check");
        assert_eq!(
            first
                .candidate
                .as_ref()
                .expect("commit candidate")
                .observed_source_commit_sha,
            pinned
        );
        commit_remote.advance("Branch moved");
        let unchanged = commit_fixture
            .service
            .check_source(&commit_source.id, commit_source.revision, 23)
            .expect("pinned recheck");
        assert!(unchanged.unchanged);
        assert!(!unchanged.fetched);
    }

    #[test]
    fn force_push_requires_local_approval_and_retains_last_good() {
        let remote = RemoteFixture::new();
        let fixture = ServiceFixture::new();
        let source = fixture.source(
            &remote,
            TrackingRef::Branch("refs/heads/main".to_string()),
            false,
        );
        let first = fixture
            .service
            .check_source(&source.id, source.revision, 30)
            .expect("first check")
            .candidate
            .expect("first candidate");
        let active = fixture.mark_active(&source, first.observed_source_commit_sha.clone(), 31);
        let rewritten = remote.force_unrelated();
        let outcome = fixture
            .service
            .check_source(&active.id, active.revision, 32)
            .expect("rewrite check");
        let candidate = outcome.candidate.expect("rewrite candidate");
        assert_eq!(candidate.observed_source_commit_sha, rewritten);
        assert_eq!(candidate.safety, ConfigurationUpdateSafety::HistoryRewrite);
        assert_eq!(candidate.state, InstructionCandidateState::AwaitingApproval);
        assert_eq!(
            fixture
                .store
                .get_instruction_source(&active.id)
                .expect("load source")
                .expect("source")
                .active_source_commit_sha,
            active.active_source_commit_sha
        );
    }

    #[test]
    fn invalid_special_mode_and_unsigned_candidates_are_blocked_with_complete_issues() {
        let invalid_remote = RemoteFixture::new();
        invalid_remote.make_invalid();
        let fixture = ServiceFixture::new();
        let source = fixture.source(
            &invalid_remote,
            TrackingRef::Branch("refs/heads/main".to_string()),
            false,
        );
        let candidate = fixture
            .service
            .check_source(&source.id, source.revision, 40)
            .expect("invalid check")
            .candidate
            .expect("invalid candidate");
        assert_eq!(candidate.state, InstructionCandidateState::Blocked);
        assert!(!candidate.activatable);
        let (_, inspection) = fixture
            .service
            .load_candidate_inspection(&candidate.id)
            .expect("load inspection")
            .expect("inspection");
        assert!(inspection.report.issues.len() >= 2);
        let issue_files = inspection
            .report
            .issues
            .iter()
            .map(|issue| issue.issue.file.as_str())
            .collect::<Vec<_>>();
        assert!(issue_files.contains(&"agents/builder.md"));
        assert!(issue_files.contains(&"prompts/intake.md"));

        let mode_remote = RemoteFixture::new();
        mode_remote.make_executable();
        let mode_fixture = ServiceFixture::new();
        let mode_source = mode_fixture.source(
            &mode_remote,
            TrackingRef::Branch("refs/heads/main".to_string()),
            false,
        );
        let mode_candidate = mode_fixture
            .service
            .check_source(&mode_source.id, mode_source.revision, 41)
            .expect("mode check")
            .candidate
            .expect("mode candidate");
        assert!(!mode_candidate.activatable);
        let (_, mode_inspection) = mode_fixture
            .service
            .load_candidate_inspection(&mode_candidate.id)
            .expect("load mode inspection")
            .expect("mode inspection");
        assert!(mode_inspection.acquisition_issues.iter().any(|issue| {
            issue.code == "configuration_special_mode_rejected"
                && issue.origin.logical_path == "agents/builder.md"
        }));

        let unsigned_remote = RemoteFixture::new();
        let unsigned_fixture = ServiceFixture::new();
        let unsigned_source = unsigned_fixture.source(
            &unsigned_remote,
            TrackingRef::Branch("refs/heads/main".to_string()),
            true,
        );
        let unsigned_candidate = unsigned_fixture
            .service
            .check_source(&unsigned_source.id, unsigned_source.revision, 42)
            .expect("unsigned check")
            .candidate
            .expect("unsigned candidate");
        assert_eq!(unsigned_candidate.state, InstructionCandidateState::Blocked);
        let (_, unsigned_inspection) = unsigned_fixture
            .service
            .load_candidate_inspection(&unsigned_candidate.id)
            .expect("load unsigned inspection")
            .expect("unsigned inspection");
        assert!(unsigned_inspection.signature_required);
        assert!(!unsigned_inspection.signature_verified);
    }

    #[test]
    fn offline_and_missing_refs_return_safe_reasons_without_moving_active_state() {
        let remote = RemoteFixture::new();
        let fixture = ServiceFixture::new();
        let source = fixture.source(
            &remote,
            TrackingRef::Branch("refs/heads/main".to_string()),
            false,
        );
        let initial = fixture
            .service
            .check_source(&source.id, source.revision, 50)
            .expect("initial check")
            .candidate
            .expect("initial candidate");
        let active = fixture.mark_active(&source, initial.observed_source_commit_sha, 51);
        let unavailable = remote.root.path().join("remote-unavailable.git");
        fs::rename(&remote.remote, &unavailable).expect("make remote unavailable");
        let outcome = fixture
            .service
            .check_source(&active.id, active.revision, 52)
            .expect("offline is a durable check result");
        assert_eq!(
            outcome.safe_error_reason.as_deref(),
            Some("instructions_remote_unavailable")
        );
        assert_eq!(
            fixture
                .store
                .get_instruction_source(&active.id)
                .expect("load source")
                .expect("source")
                .active_source_commit_sha,
            active.active_source_commit_sha
        );
        assert!(
            !outcome
                .safe_error_reason
                .as_deref()
                .unwrap_or_default()
                .contains(&remote.remote())
        );

        let missing_remote = RemoteFixture::new();
        let missing_fixture = ServiceFixture::new();
        let missing = missing_fixture.source(
            &missing_remote,
            TrackingRef::Branch("refs/heads/missing".to_string()),
            false,
        );
        let missing_outcome = missing_fixture
            .service
            .check_source(&missing.id, missing.revision, 53)
            .expect("missing ref is durable");
        assert_eq!(
            missing_outcome.safe_error_reason.as_deref(),
            Some("tracked_ref_not_found")
        );
    }

    #[test]
    fn poller_checks_on_launch_and_joins_on_shutdown() {
        let remote = RemoteFixture::new();
        let fixture = ServiceFixture::new();
        let source = fixture.source(
            &remote,
            TrackingRef::Branch("refs/heads/main".to_string()),
            false,
        );
        let poller = InstructionsPollingService::start_with_interval(
            fixture.service.clone(),
            Duration::from_millis(20),
        )
        .expect("start poller");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while fixture
            .store
            .latest_instruction_candidate(&source.id)
            .expect("candidate query")
            .is_none()
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            fixture
                .store
                .latest_instruction_candidate(&source.id)
                .expect("candidate query")
                .is_some()
        );
        fixture.service.shutdown();
        poller.shutdown().expect("join poller");
    }

    #[test]
    fn acquisition_issue_paths_are_bounded_and_control_safe() {
        let hostile = format!("agents/builder.md\n{}\0", "猫".repeat(200));
        let issue = acquisition_issue("test", "A safe reason", &hostile);
        assert!(issue.origin.logical_path.len() <= 240);
        assert!(!issue.origin.logical_path.chars().any(char::is_control));
        assert!(issue.origin.logical_path.starts_with("agents/builder.md "));
    }

    fn write_valid_tree(root: &Path, access: &str, label: &str) {
        for directory in ["agents", "knowledge", "prompts"] {
            fs::create_dir_all(root.join(directory)).expect("create fixture directory");
        }
        fs::write(
            root.join("orchestrator.yaml"),
            "schemaVersion: 1\nkind: pandamux.orchestrator\nminPandamuxVersion: \"0.60.79\"\ndefaults:\n  mode: approve_first\n  defaultAgent: builder\n  maxParallelTasks: 4\n  maxTasksPerRun: 6\n  maxAttemptsPerTask: 2\nprompts:\n  intake: prompts/intake.md\n  planner: prompts/planner.md\n  handoff: prompts/handoff.md\nclasses: []\nproviderPolicy:\n  minRemainingMillionths: 100000\n  concurrency: {}\nfallbacks:\n  onRateLimit: next_provider\n  onNoProvider: queue\n  onFailure: ask\nverification:\n  useProjectCommands: false\n",
        )
        .expect("write orchestrator");
        fs::write(
            root.join("agents/builder.md"),
            format!(
                "---\nschemaVersion: 1\nkind: pandamux.agent\nid: builder\nname: Builder\ndescription: {label}\nspecialties: [implementation]\ntier: reasoning\neffort: high\naccessMode: {access}\nrequiredCapabilities: [longContext]\nproviders: {{ prefer: [codex], avoid: [] }}\nenvironments: [local]\nknowledge: [knowledge/guide.md]\ntools: {{ allow: [read], deny: [shell] }}\noutput: {{ kind: change_set }}\nverification: [cargo test]\nconcurrency: 1\nbudget: {{ maxTokens: 1000, maxCostMicroUsd: 1000, maxRuntimeMs: 1000 }}\n---\nBuild safely.\n"
            ),
        )
        .expect("write agent");
        fs::write(
            root.join("knowledge/guide.md"),
            "---\nschemaVersion: 1\nkind: pandamux.knowledge\nid: guide\ntitle: Guide\ntags: []\n---\nGuide.\n",
        )
        .expect("write knowledge");
        for (id, purpose) in [
            ("intake", "intake"),
            ("planner", "planner"),
            ("handoff", "handoff"),
        ] {
            fs::write(
                root.join(format!("prompts/{id}.md")),
                format!(
                    "---\nschemaVersion: 1\nkind: pandamux.prompt\nid: {id}\npurpose: {purpose}\n---\nPrompt.\n"
                ),
            )
            .expect("write prompt");
        }
    }

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-c")
            .arg("core.autocrlf=false")
            .arg("-c")
            .arg("core.safecrlf=false")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LC_ALL", "C")
            .output()
            .expect("run fixture git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-c")
            .arg("core.autocrlf=false")
            .arg("-c")
            .arg("core.safecrlf=false")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LC_ALL", "C")
            .output()
            .expect("run fixture git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("git output is UTF-8")
            .trim()
            .to_string()
    }

    fn path(path: &Path) -> &str {
        path.to_str().expect("fixture path is UTF-8")
    }
}
