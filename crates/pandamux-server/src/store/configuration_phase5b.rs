use super::{Store, StoreError, checked_i64, checked_u64, configuration_revision};
use pandamux_core::{
    ArchiveSha256, BundleActivationKind, BundleActivationRecord, BundleDistributionReceipt,
    BundleDistributionState, BundleSha, BundleSourceManifest, BundleTransferId,
    ConfigurationAuditRecord, ContentSha256, EnvironmentId, InstructionCandidateId,
    InstructionCandidateRecord, InstructionSourceId, InstructionsSourceRecord, OrganizationId,
    ProjectId, ProjectLayerTrustRecord, ProjectRepositoryRecord, SourceCommitSha, TrackingRef,
};
use rusqlite::{OptionalExtension, params};

#[derive(Clone, Debug, PartialEq)]
pub struct StoredInstructionCandidate {
    pub record: InstructionCandidateRecord,
    pub inspection: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PersonalRepoStateRecord {
    pub head_source_commit_sha: SourceCommitSha,
    pub revision: pandamux_core::ConfigurationRevision,
    pub last_request_identity: Option<ContentSha256>,
    pub last_request_digest: Option<ContentSha256>,
    pub last_result: Option<serde_json::Value>,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleObjectRecord {
    pub bundle_sha: BundleSha,
    pub source_manifest: BundleSourceManifest,
    pub payload_sha256: ContentSha256,
    pub archive_sha256: ArchiveSha256,
    pub archive_bytes: u64,
    pub compiler_version: String,
    pub object_path: String,
    pub verified_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleDistributionRecord {
    pub transfer_id: BundleTransferId,
    pub environment_id: EnvironmentId,
    pub bundle_sha: BundleSha,
    pub archive_sha256: ArchiveSha256,
    pub expected_prior_bundle_sha: Option<BundleSha>,
    pub state: BundleDistributionState,
    pub receipt: Option<BundleDistributionReceipt>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundleActivationFailpoint {
    BeforeTransaction,
    AfterTransaction,
}

impl Store {
    pub fn get_personal_repo_state(&self) -> Result<Option<PersonalRepoStateRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT head_source_commit_sha, revision, last_request_identity,
                    last_request_digest, last_result_json, updated_at_ms
             FROM personal_repo_state WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()?
        .map(
            |(head, revision, identity, digest, result, updated_at_ms)| {
                Ok(PersonalRepoStateRecord {
                    head_source_commit_sha: SourceCommitSha::parse(head)
                        .map_err(|error| StoreError::Data(error.to_string()))?,
                    revision: configuration_revision(revision, "personal_repo_state.revision")?,
                    last_request_identity: identity
                        .map(ContentSha256::parse)
                        .transpose()
                        .map_err(|error| StoreError::Data(error.to_string()))?,
                    last_request_digest: digest
                        .map(ContentSha256::parse)
                        .transpose()
                        .map_err(|error| StoreError::Data(error.to_string()))?,
                    last_result: result
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                    updated_at_ms: checked_u64(updated_at_ms, "personal_repo_state.updated_at_ms")?,
                })
            },
        )
        .transpose()
    }

    pub fn reconcile_personal_repo_state(
        &self,
        state: &PersonalRepoStateRecord,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO personal_repo_state (
                singleton, head_source_commit_sha, revision, last_request_identity,
                last_request_digest, last_result_json, updated_at_ms
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(singleton) DO UPDATE SET
                head_source_commit_sha = excluded.head_source_commit_sha,
                revision = excluded.revision,
                last_request_identity = excluded.last_request_identity,
                last_request_digest = excluded.last_request_digest,
                last_result_json = excluded.last_result_json,
                updated_at_ms = excluded.updated_at_ms",
            params![
                state.head_source_commit_sha.as_str(),
                checked_i64(state.revision.get(), "personal repository revision")?,
                state
                    .last_request_identity
                    .as_ref()
                    .map(ContentSha256::as_str),
                state
                    .last_request_digest
                    .as_ref()
                    .map(ContentSha256::as_str),
                state
                    .last_result
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                checked_i64(state.updated_at_ms, "personal repository updated_at_ms")?,
            ],
        )?;
        Ok(())
    }

    pub fn save_instruction_source(
        &self,
        source: &InstructionsSourceRecord,
        created_at_ms: u64,
        updated_at_ms: u64,
    ) -> Result<(), StoreError> {
        if let Some(existing) = self.get_instruction_source(&source.id)? {
            if source.revision < existing.revision {
                return Err(StoreError::Data(
                    "instruction source revision is older than stored state".to_string(),
                ));
            }
            if source.revision == existing.revision {
                if source == &existing {
                    return Ok(());
                }
                return Err(StoreError::Data(
                    "instruction source changed without a revision increment".to_string(),
                ));
            }
        }
        let (tracking_kind, tracking_value) = tracking_parts(&source.tracking);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO instruction_sources (
                id, canonical_remote_identity, tracking_kind, tracking_value,
                update_policy, require_signed_commit, revision, last_checked_at_ms,
                last_error_reason, active_source_commit_sha, active_bundle_sha,
                created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET
                canonical_remote_identity = excluded.canonical_remote_identity,
                tracking_kind = excluded.tracking_kind,
                tracking_value = excluded.tracking_value,
                update_policy = excluded.update_policy,
                require_signed_commit = excluded.require_signed_commit,
                revision = excluded.revision,
                last_checked_at_ms = excluded.last_checked_at_ms,
                last_error_reason = excluded.last_error_reason,
                active_source_commit_sha = excluded.active_source_commit_sha,
                active_bundle_sha = excluded.active_bundle_sha,
                updated_at_ms = excluded.updated_at_ms
             WHERE instruction_sources.revision < excluded.revision",
            params![
                source.id.as_str(),
                source.canonical_remote_identity,
                tracking_kind,
                tracking_value,
                enum_text(&source.policy)?,
                source.require_signed_commit,
                checked_i64(source.revision.get(), "instruction source revision")?,
                source
                    .last_checked_at_ms
                    .map(|value| checked_i64(value, "instruction source last checked"))
                    .transpose()?,
                source.last_error_reason,
                source
                    .active_source_commit_sha
                    .as_ref()
                    .map(SourceCommitSha::as_str),
                source.active_bundle_sha.as_ref().map(BundleSha::as_str),
                checked_i64(created_at_ms, "instruction source created_at_ms")?,
                checked_i64(updated_at_ms, "instruction source updated_at_ms")?,
            ],
        )?;
        Ok(())
    }

    pub fn get_instruction_source(
        &self,
        id: &InstructionSourceId,
    ) -> Result<Option<InstructionsSourceRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT canonical_remote_identity, tracking_kind, tracking_value,
                    update_policy, require_signed_commit, revision, last_checked_at_ms,
                    last_error_reason, active_source_commit_sha, active_bundle_sha
             FROM instruction_sources WHERE id = ?1",
            [id.as_str()],
            |row| {
                let tracking_kind = row.get::<_, String>(1)?;
                let tracking_value = row.get::<_, String>(2)?;
                let policy = row.get::<_, String>(3)?;
                let revision = row.get::<_, i64>(5)?;
                let last_checked_at_ms = row.get::<_, Option<i64>>(6)?;
                let active_source_commit_sha = row.get::<_, Option<String>>(8)?;
                let active_bundle_sha = row.get::<_, Option<String>>(9)?;
                Ok((
                    row.get::<_, String>(0)?,
                    tracking_kind,
                    tracking_value,
                    policy,
                    row.get::<_, bool>(4)?,
                    revision,
                    last_checked_at_ms,
                    row.get::<_, Option<String>>(7)?,
                    active_source_commit_sha,
                    active_bundle_sha,
                ))
            },
        )
        .optional()?
        .map(
            |(
                remote,
                tracking_kind,
                tracking_value,
                policy,
                require_signed_commit,
                revision,
                last_checked_at_ms,
                last_error_reason,
                active_source_commit_sha,
                active_bundle_sha,
            )| {
                Ok(InstructionsSourceRecord {
                    id: id.clone(),
                    canonical_remote_identity: remote,
                    tracking: parse_tracking(&tracking_kind, tracking_value)?,
                    policy: parse_enum(&policy)?,
                    require_signed_commit,
                    revision: configuration_revision(revision, "instruction_sources.revision")?,
                    last_checked_at_ms: last_checked_at_ms
                        .map(|value| checked_u64(value, "instruction_sources.last_checked_at_ms"))
                        .transpose()?,
                    last_error_reason,
                    active_source_commit_sha: active_source_commit_sha
                        .map(SourceCommitSha::parse)
                        .transpose()
                        .map_err(|error| StoreError::Data(error.to_string()))?,
                    active_bundle_sha: active_bundle_sha
                        .map(BundleSha::parse)
                        .transpose()
                        .map_err(|error| StoreError::Data(error.to_string()))?,
                })
            },
        )
        .transpose()
    }

    pub fn list_instruction_sources(&self) -> Result<Vec<InstructionsSourceRecord>, StoreError> {
        let ids = {
            let conn = self.conn.lock().unwrap();
            let mut statement = conn.prepare("SELECT id FROM instruction_sources ORDER BY id")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        ids.into_iter()
            .map(|id| {
                self.get_instruction_source(&InstructionSourceId::from(id))?
                    .ok_or_else(|| {
                        StoreError::Data(
                            "instruction source disappeared while it was being listed".to_string(),
                        )
                    })
            })
            .collect()
    }

    pub fn record_instruction_source_check(
        &self,
        id: &InstructionSourceId,
        expected_revision: pandamux_core::ConfigurationRevision,
        checked_at_ms: u64,
        safe_error_reason: Option<&str>,
    ) -> Result<(), StoreError> {
        self.record_instruction_source_check_at_commit(
            id,
            expected_revision,
            checked_at_ms,
            safe_error_reason,
            None,
        )
    }

    pub fn record_instruction_source_check_at_commit(
        &self,
        id: &InstructionSourceId,
        expected_revision: pandamux_core::ConfigurationRevision,
        checked_at_ms: u64,
        safe_error_reason: Option<&str>,
        observed_commit: Option<&SourceCommitSha>,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE instruction_sources
             SET last_checked_at_ms = ?1, last_error_reason = ?2, updated_at_ms = ?1
             WHERE id = ?3 AND revision = ?4",
            params![
                checked_i64(checked_at_ms, "instruction source checked_at_ms")?,
                safe_error_reason,
                id.as_str(),
                checked_i64(
                    expected_revision.get(),
                    "instruction source expected revision"
                )?,
            ],
        )?;
        match changed {
            1 => {
                if let Some(observed_commit) = observed_commit {
                    tx.execute(
                        "UPDATE instruction_candidates
                         SET state = 'superseded', updated_at_ms = ?1
                         WHERE source_id = ?2 AND observed_source_commit_sha <> ?3
                           AND state IN (
                               'discovered', 'inspecting', 'blocked',
                               'awaiting_approval', 'approved'
                           )",
                        params![
                            checked_i64(checked_at_ms, "candidate superseded_at_ms")?,
                            id.as_str(),
                            observed_commit.as_str(),
                        ],
                    )?;
                }
                tx.commit()?;
                Ok(())
            }
            0 => {
                let actual = tx
                    .query_row(
                        "SELECT revision FROM instruction_sources WHERE id = ?1",
                        [id.as_str()],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?
                    .map(|value| configuration_revision(value, "instruction_sources.revision"))
                    .transpose()?;
                Err(StoreError::RevisionDrift {
                    expected: expected_revision,
                    actual,
                })
            }
            _ => Err(StoreError::Data(
                "instruction source check updated more than one row".to_string(),
            )),
        }
    }

    pub fn insert_instruction_candidate(
        &self,
        candidate: &InstructionCandidateRecord,
        inspection: &serde_json::Value,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO instruction_candidates (
                id, source_id, observed_source_commit_sha, observed_tag_object_sha,
                state, safety, diff_digest, activatable, inspection_json,
                created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                candidate.id.as_str(),
                candidate.source_id.as_str(),
                candidate.observed_source_commit_sha.as_str(),
                candidate
                    .observed_tag_object_sha
                    .as_ref()
                    .map(SourceCommitSha::as_str),
                enum_text(&candidate.state)?,
                enum_text(&candidate.safety)?,
                candidate.diff_digest.as_str(),
                candidate.activatable,
                serde_json::to_string(inspection)?,
                checked_i64(candidate.created_at_ms, "candidate created_at_ms")?,
                checked_i64(candidate.updated_at_ms, "candidate updated_at_ms")?,
            ],
        )?;
        Ok(())
    }

    pub fn save_instruction_candidate(
        &self,
        candidate: &InstructionCandidateRecord,
        inspection: &serde_json::Value,
    ) -> Result<InstructionCandidateRecord, StoreError> {
        if let Some(existing) = self.get_instruction_candidate_for_commit(
            &candidate.source_id,
            &candidate.observed_source_commit_sha,
        )? {
            let mut stored = candidate.clone();
            stored.id = existing.record.id;
            stored.created_at_ms = existing.record.created_at_ms;
            let conn = self.conn.lock().unwrap();
            let changed = conn.execute(
                "UPDATE instruction_candidates
                 SET observed_tag_object_sha = ?1, state = ?2, safety = ?3,
                     diff_digest = ?4, activatable = ?5, inspection_json = ?6,
                     updated_at_ms = ?7
                 WHERE id = ?8",
                params![
                    stored
                        .observed_tag_object_sha
                        .as_ref()
                        .map(SourceCommitSha::as_str),
                    enum_text(&stored.state)?,
                    enum_text(&stored.safety)?,
                    stored.diff_digest.as_str(),
                    stored.activatable,
                    serde_json::to_string(inspection)?,
                    checked_i64(stored.updated_at_ms, "candidate updated_at_ms")?,
                    stored.id.as_str(),
                ],
            )?;
            if changed != 1 {
                return Err(StoreError::Data(
                    "instruction candidate update did not affect one row".to_string(),
                ));
            }
            return Ok(stored);
        }
        self.insert_instruction_candidate(candidate, inspection)?;
        Ok(candidate.clone())
    }

    pub fn persist_instruction_candidate_check(
        &self,
        source_id: &InstructionSourceId,
        expected_revision: pandamux_core::ConfigurationRevision,
        checked_at_ms: u64,
        candidate: &InstructionCandidateRecord,
        inspection: &serde_json::Value,
    ) -> Result<InstructionCandidateRecord, StoreError> {
        if &candidate.source_id != source_id {
            return Err(StoreError::Data(
                "instruction candidate source does not match checked source".to_string(),
            ));
        }
        let state = enum_text(&candidate.state)?;
        let safety = enum_text(&candidate.safety)?;
        let inspection = serde_json::to_string(inspection)?;
        let checked_at_ms_i64 = checked_i64(checked_at_ms, "instruction source checked_at_ms")?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE instruction_sources
             SET last_checked_at_ms = ?1, last_error_reason = NULL, updated_at_ms = ?1
             WHERE id = ?2 AND revision = ?3",
            params![
                checked_at_ms_i64,
                source_id.as_str(),
                checked_i64(
                    expected_revision.get(),
                    "instruction source expected revision"
                )?,
            ],
        )?;
        if changed != 1 {
            let actual = tx
                .query_row(
                    "SELECT revision FROM instruction_sources WHERE id = ?1",
                    [source_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .map(|value| configuration_revision(value, "instruction_sources.revision"))
                .transpose()?;
            return Err(StoreError::RevisionDrift {
                expected: expected_revision,
                actual,
            });
        }

        let existing = tx
            .query_row(
                "SELECT id, created_at_ms FROM instruction_candidates
                 WHERE source_id = ?1 AND observed_source_commit_sha = ?2",
                params![
                    source_id.as_str(),
                    candidate.observed_source_commit_sha.as_str(),
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let mut stored = candidate.clone();
        if let Some((id, created_at_ms)) = existing {
            stored.id = InstructionCandidateId::from(id);
            stored.created_at_ms =
                checked_u64(created_at_ms, "instruction_candidates.created_at_ms")?;
            tx.execute(
                "UPDATE instruction_candidates
                 SET observed_tag_object_sha = ?1, state = ?2, safety = ?3,
                     diff_digest = ?4, activatable = ?5, inspection_json = ?6,
                     updated_at_ms = ?7
                 WHERE id = ?8",
                params![
                    stored
                        .observed_tag_object_sha
                        .as_ref()
                        .map(SourceCommitSha::as_str),
                    state,
                    safety,
                    stored.diff_digest.as_str(),
                    stored.activatable,
                    inspection,
                    checked_i64(stored.updated_at_ms, "candidate updated_at_ms")?,
                    stored.id.as_str(),
                ],
            )?;
        } else {
            tx.execute(
                "INSERT INTO instruction_candidates (
                    id, source_id, observed_source_commit_sha, observed_tag_object_sha,
                    state, safety, diff_digest, activatable, inspection_json,
                    created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    stored.id.as_str(),
                    stored.source_id.as_str(),
                    stored.observed_source_commit_sha.as_str(),
                    stored
                        .observed_tag_object_sha
                        .as_ref()
                        .map(SourceCommitSha::as_str),
                    state,
                    safety,
                    stored.diff_digest.as_str(),
                    stored.activatable,
                    inspection,
                    checked_i64(stored.created_at_ms, "candidate created_at_ms")?,
                    checked_i64(stored.updated_at_ms, "candidate updated_at_ms")?,
                ],
            )?;
        }
        tx.execute(
            "UPDATE instruction_candidates
             SET state = 'superseded', updated_at_ms = ?1
             WHERE source_id = ?2 AND observed_source_commit_sha <> ?3
               AND state IN (
                   'discovered', 'inspecting', 'blocked',
                   'awaiting_approval', 'approved'
               )",
            params![
                checked_at_ms_i64,
                source_id.as_str(),
                stored.observed_source_commit_sha.as_str(),
            ],
        )?;
        tx.commit()?;
        Ok(stored)
    }

    pub fn get_instruction_candidate(
        &self,
        id: &InstructionCandidateId,
    ) -> Result<Option<StoredInstructionCandidate>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT source_id, observed_source_commit_sha, observed_tag_object_sha,
                        state, safety, diff_digest, activatable, inspection_json,
                        created_at_ms, updated_at_ms
                 FROM instruction_candidates WHERE id = ?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, bool>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                    ))
                },
            )
            .optional()?;
        row.map(
            |(
                source_id,
                source_commit,
                tag_object,
                state,
                safety,
                diff_digest,
                activatable,
                inspection,
                created_at_ms,
                updated_at_ms,
            )| {
                Ok(StoredInstructionCandidate {
                    record: InstructionCandidateRecord {
                        id: id.clone(),
                        source_id: InstructionSourceId::from(source_id),
                        observed_source_commit_sha: SourceCommitSha::parse(source_commit)
                            .map_err(|error| StoreError::Data(error.to_string()))?,
                        observed_tag_object_sha: tag_object
                            .map(SourceCommitSha::parse)
                            .transpose()
                            .map_err(|error| StoreError::Data(error.to_string()))?,
                        state: parse_enum(&state)?,
                        safety: parse_enum(&safety)?,
                        diff_digest: ContentSha256::parse(diff_digest)
                            .map_err(|error| StoreError::Data(error.to_string()))?,
                        activatable,
                        created_at_ms: checked_u64(
                            created_at_ms,
                            "instruction_candidates.created_at_ms",
                        )?,
                        updated_at_ms: checked_u64(
                            updated_at_ms,
                            "instruction_candidates.updated_at_ms",
                        )?,
                    },
                    inspection: serde_json::from_str(&inspection)?,
                })
            },
        )
        .transpose()
    }

    pub fn get_instruction_candidate_for_commit(
        &self,
        source_id: &InstructionSourceId,
        source_commit_sha: &SourceCommitSha,
    ) -> Result<Option<StoredInstructionCandidate>, StoreError> {
        let id = {
            let conn = self.conn.lock().unwrap();
            conn.query_row(
                "SELECT id FROM instruction_candidates
                 WHERE source_id = ?1 AND observed_source_commit_sha = ?2",
                params![source_id.as_str(), source_commit_sha.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        };
        id.map(|id| {
            self.get_instruction_candidate(&InstructionCandidateId::from(id))?
                .ok_or_else(|| {
                    StoreError::Data(
                        "instruction candidate disappeared while it was being loaded".to_string(),
                    )
                })
        })
        .transpose()
    }

    pub fn latest_instruction_candidate(
        &self,
        source_id: &InstructionSourceId,
    ) -> Result<Option<StoredInstructionCandidate>, StoreError> {
        let id = {
            let conn = self.conn.lock().unwrap();
            conn.query_row(
                "SELECT id FROM instruction_candidates
                 WHERE source_id = ?1 AND state IN (
                     'discovered', 'inspecting', 'blocked',
                     'awaiting_approval', 'approved'
                 )
                 ORDER BY updated_at_ms DESC, id DESC LIMIT 1",
                [source_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        };
        id.map(|id| {
            self.get_instruction_candidate(&InstructionCandidateId::from(id))?
                .ok_or_else(|| {
                    StoreError::Data(
                        "instruction candidate disappeared while it was being loaded".to_string(),
                    )
                })
        })
        .transpose()
    }

    pub fn save_project_repository(
        &self,
        repository: &ProjectRepositoryRecord,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let existing = tx
            .query_row(
                "SELECT project_id, organization_id, canonical_remote_identity,
                        canonical_root_identity, display_path, revision,
                        created_at_ms, updated_at_ms
                 FROM project_repositories WHERE id = ?1",
                [repository.id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .optional()?
            .map(
                |(
                    project_id,
                    organization_id,
                    remote,
                    root,
                    display_path,
                    revision,
                    created_at_ms,
                    updated_at_ms,
                )|
                 -> Result<ProjectRepositoryRecord, StoreError> {
                    Ok(ProjectRepositoryRecord {
                        id: repository.id.clone(),
                        project_id: ProjectId::from(project_id),
                        organization_id: OrganizationId::from(organization_id),
                        canonical_remote_identity: remote,
                        canonical_repository_root_identity: root,
                        display_path,
                        revision: configuration_revision(
                            revision,
                            "project_repositories.revision",
                        )?,
                        created_at_ms: checked_u64(
                            created_at_ms,
                            "project_repositories.created_at_ms",
                        )?,
                        updated_at_ms: checked_u64(
                            updated_at_ms,
                            "project_repositories.updated_at_ms",
                        )?,
                    })
                },
            )
            .transpose()?;
        let identity_changed = existing.as_ref().is_some_and(|existing| {
            existing.project_id != repository.project_id
                || existing.organization_id != repository.organization_id
                || existing.canonical_remote_identity != repository.canonical_remote_identity
                || existing.canonical_repository_root_identity
                    != repository.canonical_repository_root_identity
        });
        if let Some(existing) = existing {
            if repository.revision < existing.revision {
                return Err(StoreError::Data(
                    "project repository revision is older than stored state".to_string(),
                ));
            }
            if repository.revision == existing.revision {
                if repository == &existing {
                    return Ok(());
                }
                return Err(StoreError::Data(
                    "project repository changed without a revision increment".to_string(),
                ));
            }
        }
        tx.execute(
            "INSERT INTO project_repositories (
                id, project_id, organization_id, canonical_remote_identity,
                canonical_root_identity, display_path, revision, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                project_id = excluded.project_id,
                organization_id = excluded.organization_id,
                canonical_remote_identity = excluded.canonical_remote_identity,
                canonical_root_identity = excluded.canonical_root_identity,
                display_path = excluded.display_path,
                revision = excluded.revision,
                updated_at_ms = excluded.updated_at_ms
             WHERE project_repositories.revision < excluded.revision",
            params![
                repository.id.as_str(),
                repository.project_id.as_str(),
                repository.organization_id.as_str(),
                repository.canonical_remote_identity,
                repository.canonical_repository_root_identity,
                repository.display_path,
                checked_i64(repository.revision.get(), "project repository revision")?,
                checked_i64(repository.created_at_ms, "project repository created_at_ms")?,
                checked_i64(repository.updated_at_ms, "project repository updated_at_ms")?,
            ],
        )?;
        if identity_changed {
            tx.execute(
                "UPDATE project_layer_trust
                 SET revoked_at_ms = ?1
                 WHERE repository_id = ?2 AND revoked_at_ms IS NULL",
                params![
                    checked_i64(repository.updated_at_ms, "project trust revoked_at_ms")?,
                    repository.id.as_str(),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_project_repository(
        &self,
        project_id: &ProjectId,
        organization_id: &OrganizationId,
    ) -> Result<Option<ProjectRepositoryRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, canonical_remote_identity, canonical_root_identity,
                    display_path, revision, created_at_ms, updated_at_ms
             FROM project_repositories
             WHERE project_id = ?1 AND organization_id = ?2",
            params![project_id.as_str(), organization_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()?
        .map(
            |(id, remote, root, display_path, revision, created_at_ms, updated_at_ms)| {
                Ok(ProjectRepositoryRecord {
                    id: pandamux_core::ProjectRepositoryId::from(id),
                    project_id: project_id.clone(),
                    organization_id: organization_id.clone(),
                    canonical_remote_identity: remote,
                    canonical_repository_root_identity: root,
                    display_path,
                    revision: configuration_revision(revision, "project_repositories.revision")?,
                    created_at_ms: checked_u64(
                        created_at_ms,
                        "project_repositories.created_at_ms",
                    )?,
                    updated_at_ms: checked_u64(
                        updated_at_ms,
                        "project_repositories.updated_at_ms",
                    )?,
                })
            },
        )
        .transpose()
    }

    pub fn insert_project_layer_trust(
        &self,
        trust: &ProjectLayerTrustRecord,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let repository_scope = conn
            .query_row(
                "SELECT organization_id, project_id FROM project_repositories WHERE id = ?1",
                [trust.repository_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if repository_scope.as_ref()
            != Some(&(
                trust.organization_id.as_str().to_string(),
                trust.project_id.as_str().to_string(),
            ))
        {
            return Err(StoreError::Data(
                "project trust scope does not match its repository".to_string(),
            ));
        }
        conn.execute(
            "INSERT INTO project_layer_trust (
                id, organization_id, project_id, repository_id, project_content_sha256,
                approved_by, approved_at_ms, revoked_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                trust.id.as_str(),
                trust.organization_id.as_str(),
                trust.project_id.as_str(),
                trust.repository_id.as_str(),
                trust.project_content_sha256.as_str(),
                trust.approved_by,
                checked_i64(trust.approved_at_ms, "project trust approved_at_ms")?,
                trust
                    .revoked_at_ms
                    .map(|value| checked_i64(value, "project trust revoked_at_ms"))
                    .transpose()?,
            ],
        )?;
        Ok(())
    }

    pub fn get_active_project_layer_trust(
        &self,
        organization_id: &OrganizationId,
        project_id: &ProjectId,
        repository_id: &pandamux_core::ProjectRepositoryId,
        content_sha256: &ContentSha256,
    ) -> Result<Option<ProjectLayerTrustRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, approved_by, approved_at_ms
             FROM project_layer_trust
             WHERE organization_id = ?1 AND project_id = ?2
               AND repository_id = ?3 AND project_content_sha256 = ?4
               AND revoked_at_ms IS NULL",
            params![
                organization_id.as_str(),
                project_id.as_str(),
                repository_id.as_str(),
                content_sha256.as_str(),
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?
        .map(|(id, approved_by, approved_at_ms)| {
            Ok(ProjectLayerTrustRecord {
                id: pandamux_core::ProjectLayerTrustId::from(id),
                organization_id: organization_id.clone(),
                project_id: project_id.clone(),
                repository_id: repository_id.clone(),
                project_content_sha256: content_sha256.clone(),
                approved_by,
                approved_at_ms: checked_u64(approved_at_ms, "project_layer_trust.approved_at_ms")?,
                revoked_at_ms: None,
            })
        })
        .transpose()
    }

    pub fn revoke_project_layer_trust(
        &self,
        repository_id: &pandamux_core::ProjectRepositoryId,
        revoked_at_ms: u64,
    ) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE project_layer_trust
             SET revoked_at_ms = ?1
             WHERE repository_id = ?2 AND revoked_at_ms IS NULL",
            params![
                checked_i64(revoked_at_ms, "project trust revoked_at_ms")?,
                repository_id.as_str(),
            ],
        )
        .map_err(StoreError::from)
    }

    pub fn save_bundle_object(&self, object: &BundleObjectRecord) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let existing = load_bundle_object(&conn, &object.bundle_sha)?;
        if let Some(existing) = existing {
            if existing == *object {
                return Ok(());
            }
            return Err(StoreError::Data(
                "bundle object identity conflicts with stored metadata".to_string(),
            ));
        }
        conn.execute(
            "INSERT INTO bundle_objects (
                bundle_sha, source_manifest_json, payload_sha256, archive_sha256,
                archive_bytes, compiler_version, object_path, verified_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                object.bundle_sha.as_str(),
                serde_json::to_string(&object.source_manifest)?,
                object.payload_sha256.as_str(),
                object.archive_sha256.as_str(),
                checked_i64(object.archive_bytes, "bundle archive bytes")?,
                object.compiler_version,
                object.object_path,
                checked_i64(object.verified_at_ms, "bundle verified_at_ms")?,
            ],
        )?;
        Ok(())
    }

    pub fn get_bundle_object(
        &self,
        bundle_sha: &BundleSha,
    ) -> Result<Option<BundleObjectRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_bundle_object(&conn, bundle_sha)
    }

    pub fn current_bundle_activation(&self) -> Result<Option<BundleActivationRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_current_bundle_activation(&conn)
    }

    pub fn activate_bundle_with_outbox(
        &self,
        activation: &BundleActivationRecord,
        candidate_id: Option<&InstructionCandidateId>,
        expected_source_revision: Option<pandamux_core::ConfigurationRevision>,
        audit: &ConfigurationAuditRecord,
        distributions: &[BundleDistributionRecord],
        failpoint: Option<BundleActivationFailpoint>,
    ) -> Result<(), StoreError> {
        if failpoint == Some(BundleActivationFailpoint::BeforeTransaction) {
            return Err(StoreError::Data(
                "injected failure before bundle activation".to_string(),
            ));
        }
        if audit.bundle_sha.as_ref() != Some(&activation.bundle_sha) {
            return Err(StoreError::Data(
                "bundle activation audit identity does not match".to_string(),
            ));
        }
        if distributions.iter().any(|distribution| {
            distribution.bundle_sha != activation.bundle_sha
                || distribution.expected_prior_bundle_sha != activation.prior_bundle_sha
                || distribution.state != BundleDistributionState::Pending
                || distribution.receipt.is_some()
        }) {
            return Err(StoreError::Data(
                "bundle distribution does not match its activation".to_string(),
            ));
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let duplicate = tx
            .query_row(
                "SELECT bundle_sha, source_manifest_json, kind, actor, prior_bundle_sha,
                        activated_at_ms
                 FROM bundle_activations WHERE id = ?1",
                [activation.id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?;
        if let Some((bundle, manifest, kind, actor, prior, activated_at_ms)) = duplicate {
            let stored = BundleActivationRecord {
                id: activation.id.clone(),
                bundle_sha: BundleSha::parse(bundle)
                    .map_err(|_| StoreError::Data("invalid stored bundle identity".to_string()))?,
                source_manifest: serde_json::from_str(&manifest)?,
                activated_at_ms: checked_u64(
                    activated_at_ms,
                    "bundle_activations.activated_at_ms",
                )?,
                kind: parse_enum(&kind)?,
                actor,
                prior_bundle_sha: prior.map(BundleSha::parse).transpose().map_err(|_| {
                    StoreError::Data("invalid stored prior bundle identity".to_string())
                })?,
            };
            if stored == *activation {
                return Ok(());
            }
            return Err(StoreError::Data(
                "bundle activation identity was reused with different content".to_string(),
            ));
        }

        let current = load_current_bundle_activation(&tx)?;
        if current.as_ref().map(|record| &record.bundle_sha) != activation.prior_bundle_sha.as_ref()
        {
            return Err(StoreError::Data(
                "active bundle changed before activation".to_string(),
            ));
        }
        if load_bundle_object(&tx, &activation.bundle_sha)?.is_none() {
            return Err(StoreError::Data(
                "bundle activation references a missing object".to_string(),
            ));
        }
        tx.execute(
            "UPDATE bundle_activations SET deactivated_at_ms = ?1
             WHERE deactivated_at_ms IS NULL",
            [checked_i64(
                activation.activated_at_ms,
                "bundle deactivated_at_ms",
            )?],
        )?;
        insert_bundle_activation(&tx, activation, candidate_id)?;
        if let Some(candidate_id) = candidate_id {
            let expected_source_revision = expected_source_revision.ok_or_else(|| {
                StoreError::Data(
                    "candidate activation requires an expected source revision".to_string(),
                )
            })?;
            let changed = tx.execute(
                "UPDATE instruction_candidates
                 SET state = 'activated', updated_at_ms = ?1
                 WHERE id = ?2 AND activatable = 1
                   AND state IN ('approved', 'awaiting_approval')",
                params![
                    checked_i64(activation.activated_at_ms, "candidate updated_at_ms")?,
                    candidate_id.as_str(),
                ],
            )?;
            if changed != 1 {
                return Err(StoreError::Data(
                    "instruction candidate is not activatable".to_string(),
                ));
            }
            let source_changed = tx.execute(
                "UPDATE instruction_sources
                 SET active_source_commit_sha = (
                         SELECT observed_source_commit_sha
                         FROM instruction_candidates WHERE id = ?1
                     ),
                     active_bundle_sha = ?2,
                     revision = revision + 1,
                     updated_at_ms = ?3
                 WHERE id = (
                     SELECT source_id FROM instruction_candidates WHERE id = ?1
                 ) AND revision = ?4",
                params![
                    candidate_id.as_str(),
                    activation.bundle_sha.as_str(),
                    checked_i64(activation.activated_at_ms, "source activation time")?,
                    checked_i64(expected_source_revision.get(), "source activation revision")?,
                ],
            )?;
            if source_changed != 1 {
                return Err(StoreError::RevisionDrift {
                    expected: expected_source_revision,
                    actual: None,
                });
            }
        } else if expected_source_revision.is_some() {
            return Err(StoreError::Data(
                "source revision was supplied without a candidate".to_string(),
            ));
        }
        insert_configuration_audit(&tx, audit)?;
        for distribution in distributions {
            tx.execute(
                "INSERT INTO bundle_distribution (
                    transfer_id, environment_id, bundle_sha, archive_sha256,
                    expected_prior_bundle_sha, state, receipt_json,
                    created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', NULL, ?6, ?7)",
                params![
                    distribution.transfer_id.as_str(),
                    distribution.environment_id.as_str(),
                    distribution.bundle_sha.as_str(),
                    distribution.archive_sha256.as_str(),
                    distribution
                        .expected_prior_bundle_sha
                        .as_ref()
                        .map(BundleSha::as_str),
                    checked_i64(distribution.created_at_ms, "distribution created_at_ms")?,
                    checked_i64(distribution.updated_at_ms, "distribution updated_at_ms")?,
                ],
            )?;
        }
        tx.commit()?;
        if failpoint == Some(BundleActivationFailpoint::AfterTransaction) {
            return Err(StoreError::Data(
                "injected failure after bundle activation".to_string(),
            ));
        }
        Ok(())
    }

    pub fn list_incomplete_bundle_distributions(
        &self,
        environment_id: Option<&EnvironmentId>,
    ) -> Result<Vec<BundleDistributionRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(
            "SELECT transfer_id, environment_id, bundle_sha, archive_sha256,
                    expected_prior_bundle_sha, state, receipt_json,
                    created_at_ms, updated_at_ms
             FROM bundle_distribution
             WHERE state IN ('pending', 'uploading')
               AND (?1 IS NULL OR environment_id = ?1)
             ORDER BY created_at_ms, transfer_id",
        )?;
        let rows = statement.query_map(
            [environment_id.map(EnvironmentId::as_str)],
            distribution_from_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(bundle_distribution_from_sql)
            .collect()
    }

    pub fn mark_bundle_distribution_uploading(
        &self,
        transfer_id: &BundleTransferId,
        updated_at_ms: u64,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute(
            "UPDATE bundle_distribution SET state = 'uploading', updated_at_ms = ?1
             WHERE transfer_id = ?2 AND state IN ('pending', 'uploading')",
            params![
                checked_i64(updated_at_ms, "distribution updated_at_ms")?,
                transfer_id.as_str(),
            ],
        )? == 1)
    }

    pub fn reset_bundle_distribution_pending(
        &self,
        transfer_id: &BundleTransferId,
        updated_at_ms: u64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE bundle_distribution SET state = 'pending', updated_at_ms = ?1
             WHERE transfer_id = ?2 AND state = 'uploading'",
            params![
                checked_i64(updated_at_ms, "distribution updated_at_ms")?,
                transfer_id.as_str(),
            ],
        )?;
        Ok(())
    }

    pub fn complete_bundle_distribution(
        &self,
        receipt: &BundleDistributionReceipt,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let existing = load_bundle_distribution(&conn, &receipt.transfer_id)?.ok_or_else(|| {
            StoreError::Data("bundle distribution receipt has no outbox row".to_string())
        })?;
        if existing.environment_id != receipt.environment_id
            || existing.bundle_sha != receipt.bundle_sha
            || existing.archive_sha256 != receipt.archive_sha256
            || existing.expected_prior_bundle_sha != receipt.prior_bundle_sha
            || receipt.stored_bundle_sha != receipt.bundle_sha
            || receipt.active_bundle_sha != receipt.bundle_sha
            || receipt.state != BundleDistributionState::Active
        {
            return Err(StoreError::Data(
                "bundle distribution receipt does not match its outbox row".to_string(),
            ));
        }
        if let Some(stored) = existing.receipt {
            if stored == *receipt {
                return Ok(());
            }
            return Err(StoreError::Data(
                "bundle distribution receipt identity conflicts".to_string(),
            ));
        }
        let changed = conn.execute(
            "UPDATE bundle_distribution
             SET state = 'active', receipt_json = ?1, updated_at_ms = ?2
             WHERE transfer_id = ?3 AND state IN ('pending', 'uploading')",
            params![
                serde_json::to_string(receipt)?,
                checked_i64(receipt.received_at_ms, "distribution received_at_ms")?,
                receipt.transfer_id.as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::Data(
                "bundle distribution could not be completed".to_string(),
            ));
        }
        Ok(())
    }

    pub fn accept_received_bundle(
        &self,
        object: &BundleObjectRecord,
        activation: &BundleActivationRecord,
        receipt: &BundleDistributionReceipt,
    ) -> Result<BundleDistributionReceipt, StoreError> {
        if activation.bundle_sha != object.bundle_sha
            || receipt.bundle_sha != object.bundle_sha
            || receipt.archive_sha256 != object.archive_sha256
            || receipt.stored_bundle_sha != object.bundle_sha
            || receipt.active_bundle_sha != object.bundle_sha
            || receipt.prior_bundle_sha != activation.prior_bundle_sha
            || receipt.state != BundleDistributionState::Active
        {
            return Err(StoreError::Data(
                "received bundle identities do not agree".to_string(),
            ));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if let Some(existing) = load_bundle_distribution(&tx, &receipt.transfer_id)? {
            if existing.environment_id == receipt.environment_id
                && existing.bundle_sha == receipt.bundle_sha
                && existing.archive_sha256 == receipt.archive_sha256
                && existing.expected_prior_bundle_sha == receipt.prior_bundle_sha
                && let Some(stored_receipt) = existing.receipt
            {
                return Ok(stored_receipt);
            }
            return Err(StoreError::Data(
                "bundle transfer identity was reused with different content".to_string(),
            ));
        }
        if let Some(existing) = load_bundle_object(&tx, &object.bundle_sha)? {
            if existing != *object {
                return Err(StoreError::Data(
                    "received bundle object conflicts with stored metadata".to_string(),
                ));
            }
        } else {
            insert_bundle_object(&tx, object)?;
        }
        let current = load_current_bundle_activation(&tx)?;
        if current.as_ref().map(|record| &record.bundle_sha) != activation.prior_bundle_sha.as_ref()
        {
            return Err(StoreError::Data(
                "node active bundle does not match the expected prior bundle".to_string(),
            ));
        }
        tx.execute(
            "UPDATE bundle_activations SET deactivated_at_ms = ?1
             WHERE deactivated_at_ms IS NULL",
            [checked_i64(
                activation.activated_at_ms,
                "bundle deactivated_at_ms",
            )?],
        )?;
        insert_bundle_activation(&tx, activation, None)?;
        tx.execute(
            "INSERT INTO bundle_distribution (
                transfer_id, environment_id, bundle_sha, archive_sha256,
                expected_prior_bundle_sha, state, receipt_json,
                created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?7)",
            params![
                receipt.transfer_id.as_str(),
                receipt.environment_id.as_str(),
                receipt.bundle_sha.as_str(),
                receipt.archive_sha256.as_str(),
                receipt.prior_bundle_sha.as_ref().map(BundleSha::as_str),
                serde_json::to_string(receipt)?,
                checked_i64(receipt.received_at_ms, "distribution received_at_ms")?,
            ],
        )?;
        tx.commit()?;
        Ok(receipt.clone())
    }

    pub fn append_configuration_audit(
        &self,
        audit: &ConfigurationAuditRecord,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO configuration_audit (
                id, kind, actor, source_id, candidate_id, bundle_sha,
                details_json, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                audit.id.as_str(),
                audit.kind,
                audit.actor,
                audit.source_id.as_ref().map(InstructionSourceId::as_str),
                audit
                    .candidate_id
                    .as_ref()
                    .map(InstructionCandidateId::as_str),
                audit.bundle_sha.as_ref().map(BundleSha::as_str),
                serde_json::to_string(&audit.details)?,
                checked_i64(audit.created_at_ms, "configuration audit created_at_ms")?,
            ],
        )?;
        Ok(())
    }

    pub fn list_configuration_audit(
        &self,
        before_at_ms: Option<u64>,
        limit: u32,
    ) -> Result<(Vec<ConfigurationAuditRecord>, bool), StoreError> {
        let limit = limit.clamp(1, 100);
        let fetch = limit.saturating_add(1);
        let before = before_at_ms
            .map(|value| checked_i64(value, "configuration audit cursor"))
            .transpose()?;
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(
            "SELECT id, kind, actor, source_id, candidate_id, bundle_sha,
                    details_json, created_at_ms
             FROM configuration_audit
             WHERE (?1 IS NULL OR created_at_ms < ?1)
             ORDER BY created_at_ms DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![before, i64::from(fetch)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?;
        let mut entries = rows
            .map(|row| {
                let (id, kind, actor, source, candidate, bundle, details, created) = row?;
                Ok(ConfigurationAuditRecord {
                    id: pandamux_core::ConfigurationAuditId::from(id),
                    kind,
                    actor,
                    source_id: source.map(InstructionSourceId::from),
                    candidate_id: candidate.map(InstructionCandidateId::from),
                    bundle_sha: bundle
                        .map(BundleSha::parse)
                        .transpose()
                        .map_err(|error| StoreError::Data(error.to_string()))?,
                    details: serde_json::from_str(&details)?,
                    created_at_ms: checked_u64(created, "configuration audit created_at_ms")?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let has_more = entries.len() > limit as usize;
        entries.truncate(limit as usize);
        Ok((entries, has_more))
    }
}

fn insert_bundle_object(
    conn: &rusqlite::Connection,
    object: &BundleObjectRecord,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO bundle_objects (
            bundle_sha, source_manifest_json, payload_sha256, archive_sha256,
            archive_bytes, compiler_version, object_path, verified_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            object.bundle_sha.as_str(),
            serde_json::to_string(&object.source_manifest)?,
            object.payload_sha256.as_str(),
            object.archive_sha256.as_str(),
            checked_i64(object.archive_bytes, "bundle archive bytes")?,
            object.compiler_version,
            object.object_path,
            checked_i64(object.verified_at_ms, "bundle verified_at_ms")?,
        ],
    )?;
    Ok(())
}

fn load_bundle_object(
    conn: &rusqlite::Connection,
    bundle_sha: &BundleSha,
) -> Result<Option<BundleObjectRecord>, StoreError> {
    conn.query_row(
        "SELECT source_manifest_json, payload_sha256, archive_sha256,
                archive_bytes, compiler_version, object_path, verified_at_ms
         FROM bundle_objects WHERE bundle_sha = ?1",
        [bundle_sha.as_str()],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
            ))
        },
    )
    .optional()?
    .map(
        |(manifest, payload, archive, archive_bytes, compiler, path, verified_at_ms)| {
            let archive = archive.ok_or_else(|| {
                StoreError::Data("bundle object is missing its archive identity".to_string())
            })?;
            let archive_bytes = archive_bytes.ok_or_else(|| {
                StoreError::Data("bundle object is missing its archive length".to_string())
            })?;
            Ok(BundleObjectRecord {
                bundle_sha: bundle_sha.clone(),
                source_manifest: serde_json::from_str(&manifest)?,
                payload_sha256: ContentSha256::parse(payload).map_err(|_| {
                    StoreError::Data("invalid stored bundle payload identity".to_string())
                })?,
                archive_sha256: ArchiveSha256::parse(archive).map_err(|_| {
                    StoreError::Data("invalid stored bundle archive identity".to_string())
                })?,
                archive_bytes: checked_u64(archive_bytes, "bundle_objects.archive_bytes")?,
                compiler_version: compiler,
                object_path: path,
                verified_at_ms: checked_u64(verified_at_ms, "bundle_objects.verified_at_ms")?,
            })
        },
    )
    .transpose()
}

fn insert_bundle_activation(
    conn: &rusqlite::Connection,
    activation: &BundleActivationRecord,
    candidate_id: Option<&InstructionCandidateId>,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO bundle_activations (
            id, bundle_sha, candidate_id, source_manifest_json, kind, actor,
            prior_bundle_sha, activated_at_ms, deactivated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
        params![
            activation.id.as_str(),
            activation.bundle_sha.as_str(),
            candidate_id.map(InstructionCandidateId::as_str),
            serde_json::to_string(&activation.source_manifest)?,
            enum_text(&activation.kind)?,
            activation.actor,
            activation.prior_bundle_sha.as_ref().map(BundleSha::as_str),
            checked_i64(activation.activated_at_ms, "bundle activated_at_ms")?,
        ],
    )?;
    Ok(())
}

fn load_current_bundle_activation(
    conn: &rusqlite::Connection,
) -> Result<Option<BundleActivationRecord>, StoreError> {
    conn.query_row(
        "SELECT id, bundle_sha, source_manifest_json, kind, actor,
                prior_bundle_sha, activated_at_ms
         FROM bundle_activations WHERE deactivated_at_ms IS NULL",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)?,
            ))
        },
    )
    .optional()?
    .map(
        |(id, bundle, manifest, kind, actor, prior, activated_at_ms)| {
            Ok(BundleActivationRecord {
                id: pandamux_core::BundleActivationId::from(id),
                bundle_sha: BundleSha::parse(bundle)
                    .map_err(|_| StoreError::Data("invalid stored bundle identity".to_string()))?,
                source_manifest: serde_json::from_str(&manifest)?,
                activated_at_ms: checked_u64(
                    activated_at_ms,
                    "bundle_activations.activated_at_ms",
                )?,
                kind: parse_enum::<BundleActivationKind>(&kind)?,
                actor,
                prior_bundle_sha: prior.map(BundleSha::parse).transpose().map_err(|_| {
                    StoreError::Data("invalid stored prior bundle identity".to_string())
                })?,
            })
        },
    )
    .transpose()
}

type BundleDistributionSql = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    i64,
    i64,
);

fn distribution_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BundleDistributionSql> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn bundle_distribution_from_sql(
    (transfer, environment, bundle, archive, prior, state, receipt, created, updated): BundleDistributionSql,
) -> Result<BundleDistributionRecord, StoreError> {
    Ok(BundleDistributionRecord {
        transfer_id: BundleTransferId::from(transfer),
        environment_id: EnvironmentId::from(environment),
        bundle_sha: BundleSha::parse(bundle)
            .map_err(|_| StoreError::Data("invalid distribution bundle identity".to_string()))?,
        archive_sha256: ArchiveSha256::parse(archive)
            .map_err(|_| StoreError::Data("invalid distribution archive identity".to_string()))?,
        expected_prior_bundle_sha: prior.map(BundleSha::parse).transpose().map_err(|_| {
            StoreError::Data("invalid distribution prior bundle identity".to_string())
        })?,
        state: parse_enum::<BundleDistributionState>(&state)?,
        receipt: receipt
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
        created_at_ms: checked_u64(created, "bundle_distribution.created_at_ms")?,
        updated_at_ms: checked_u64(updated, "bundle_distribution.updated_at_ms")?,
    })
}

fn load_bundle_distribution(
    conn: &rusqlite::Connection,
    transfer_id: &BundleTransferId,
) -> Result<Option<BundleDistributionRecord>, StoreError> {
    conn.query_row(
        "SELECT transfer_id, environment_id, bundle_sha, archive_sha256,
                expected_prior_bundle_sha, state, receipt_json,
                created_at_ms, updated_at_ms
         FROM bundle_distribution WHERE transfer_id = ?1",
        [transfer_id.as_str()],
        distribution_from_row,
    )
    .optional()?
    .map(bundle_distribution_from_sql)
    .transpose()
}

fn insert_configuration_audit(
    conn: &rusqlite::Connection,
    audit: &ConfigurationAuditRecord,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO configuration_audit (
            id, kind, actor, source_id, candidate_id, bundle_sha,
            details_json, created_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            audit.id.as_str(),
            audit.kind,
            audit.actor,
            audit.source_id.as_ref().map(InstructionSourceId::as_str),
            audit
                .candidate_id
                .as_ref()
                .map(InstructionCandidateId::as_str),
            audit.bundle_sha.as_ref().map(BundleSha::as_str),
            serde_json::to_string(&audit.details)?,
            checked_i64(audit.created_at_ms, "configuration audit created_at_ms")?,
        ],
    )?;
    Ok(())
}

fn enum_text(value: &impl serde::Serialize) -> Result<String, StoreError> {
    let value = serde_json::to_value(value)?;
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| StoreError::Data("serialized enum was not a string".to_string()))
}

fn parse_enum<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, StoreError> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).map_err(StoreError::from)
}

fn tracking_parts(tracking: &TrackingRef) -> (&'static str, &str) {
    match tracking {
        TrackingRef::Branch(value) => ("branch", value),
        TrackingRef::Tag(value) => ("tag", value),
        TrackingRef::Commit(value) => ("commit", value.as_str()),
    }
}

fn parse_tracking(kind: &str, value: String) -> Result<TrackingRef, StoreError> {
    match kind {
        "branch" => Ok(TrackingRef::Branch(value)),
        "tag" => Ok(TrackingRef::Tag(value)),
        "commit" => SourceCommitSha::parse(value)
            .map(TrackingRef::Commit)
            .map_err(|error| StoreError::Data(error.to_string())),
        _ => Err(StoreError::Data(format!(
            "unknown instruction tracking kind: {kind}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{
        ConfigurationRevision, ConfigurationUpdateSafety, InstructionCandidateState,
        InstructionsUpdatePolicy,
    };

    fn test_source(id: &str) -> InstructionsSourceRecord {
        InstructionsSourceRecord {
            id: InstructionSourceId::new(id),
            canonical_remote_identity: "https://example.invalid/org/repo".to_string(),
            tracking: TrackingRef::Branch("refs/heads/main".to_string()),
            policy: InstructionsUpdatePolicy::ApproveEach,
            require_signed_commit: false,
            revision: ConfigurationRevision::new(1).expect("revision"),
            last_checked_at_ms: Some(10),
            last_error_reason: None,
            active_source_commit_sha: None,
            active_bundle_sha: None,
        }
    }

    fn test_candidate(
        id: &str,
        source_id: &InstructionSourceId,
        commit: char,
        updated_at_ms: u64,
    ) -> InstructionCandidateRecord {
        InstructionCandidateRecord {
            id: InstructionCandidateId::new(id),
            source_id: source_id.clone(),
            observed_source_commit_sha: SourceCommitSha::parse(commit.to_string().repeat(40))
                .expect("commit"),
            observed_tag_object_sha: None,
            state: InstructionCandidateState::AwaitingApproval,
            safety: ConfigurationUpdateSafety::Ordinary,
            diff_digest: ContentSha256::parse(commit.to_string().repeat(64)).expect("digest"),
            activatable: true,
            created_at_ms: updated_at_ms,
            updated_at_ms,
        }
    }

    fn test_project_repository() -> ProjectRepositoryRecord {
        ProjectRepositoryRecord {
            id: pandamux_core::ProjectRepositoryId::new("repository-trust-test"),
            project_id: ProjectId::new("project-trust-test"),
            organization_id: OrganizationId::new("organization-trust-test"),
            canonical_remote_identity: "example.invalid/team/repository".to_string(),
            canonical_repository_root_identity: "root-identity-one".to_string(),
            display_path: "repository".to_string(),
            revision: ConfigurationRevision::new(1).expect("revision"),
            created_at_ms: 10,
            updated_at_ms: 10,
        }
    }

    fn test_project_trust(
        repository: &ProjectRepositoryRecord,
        id: &str,
        content: char,
        approved_at_ms: u64,
    ) -> ProjectLayerTrustRecord {
        ProjectLayerTrustRecord {
            id: pandamux_core::ProjectLayerTrustId::new(id),
            organization_id: repository.organization_id.clone(),
            project_id: repository.project_id.clone(),
            repository_id: repository.id.clone(),
            project_content_sha256: ContentSha256::parse(content.to_string().repeat(64))
                .expect("content hash"),
            approved_by: "local-control".to_string(),
            approved_at_ms,
            revoked_at_ms: None,
        }
    }

    fn test_bundle_object(identity: char, verified_at_ms: u64) -> BundleObjectRecord {
        let digest = identity.to_string().repeat(64);
        BundleObjectRecord {
            bundle_sha: BundleSha::parse(digest.clone()).expect("bundle identity"),
            source_manifest: BundleSourceManifest {
                instructions_commit_sha: SourceCommitSha::parse(identity.to_string().repeat(40))
                    .expect("source identity"),
                project_layer_content_sha256: None,
                personal_commit_sha: None,
                settings_revision: ConfigurationRevision::new(1).expect("revision"),
            },
            payload_sha256: ContentSha256::parse(digest).expect("payload identity"),
            archive_sha256: ArchiveSha256::parse(
                (if identity == 'a' { 'c' } else { 'd' })
                    .to_string()
                    .repeat(64),
            )
            .expect("archive identity"),
            archive_bytes: 42,
            compiler_version: "test-compiler".to_string(),
            object_path: format!("bundles/{}", identity.to_string().repeat(64)),
            verified_at_ms,
        }
    }

    fn test_activation(
        object: &BundleObjectRecord,
        id: &str,
        prior: Option<BundleSha>,
        activated_at_ms: u64,
    ) -> BundleActivationRecord {
        BundleActivationRecord {
            id: pandamux_core::BundleActivationId::new(id),
            bundle_sha: object.bundle_sha.clone(),
            source_manifest: object.source_manifest.clone(),
            activated_at_ms,
            kind: BundleActivationKind::TrustedBranch,
            actor: "local-control".to_string(),
            prior_bundle_sha: prior,
        }
    }

    fn test_activation_audit(
        activation: &BundleActivationRecord,
        id: &str,
    ) -> ConfigurationAuditRecord {
        ConfigurationAuditRecord {
            id: pandamux_core::ConfigurationAuditId::new(id),
            kind: "bundle_activated".to_string(),
            actor: activation.actor.clone(),
            source_id: None,
            candidate_id: None,
            bundle_sha: Some(activation.bundle_sha.clone()),
            details: serde_json::json!({ "activationId": activation.id }),
            created_at_ms: activation.activated_at_ms,
        }
    }

    #[test]
    fn source_and_candidate_round_trip_through_migration_thirteen_tables() {
        let store = Store::in_memory().expect("store");
        let source = test_source("instruction-source-test");
        store
            .save_instruction_source(&source, 1, 10)
            .expect("save source");
        assert_eq!(
            store
                .get_instruction_source(&source.id)
                .expect("load source"),
            Some(source.clone())
        );
        let mut conflicting_source = source.clone();
        conflicting_source.tracking = TrackingRef::Tag("refs/tags/v1".to_string());
        assert!(
            store
                .save_instruction_source(&conflicting_source, 1, 11)
                .is_err()
        );

        let candidate = InstructionCandidateRecord {
            id: InstructionCandidateId::new("instruction-candidate-test"),
            source_id: source.id,
            observed_source_commit_sha: SourceCommitSha::parse("a".repeat(40)).expect("commit"),
            observed_tag_object_sha: None,
            state: InstructionCandidateState::AwaitingApproval,
            safety: ConfigurationUpdateSafety::AccessWidening,
            diff_digest: ContentSha256::parse("b".repeat(64)).expect("digest"),
            activatable: true,
            created_at_ms: 20,
            updated_at_ms: 20,
        };
        let inspection = serde_json::json!({"issues": []});
        store
            .insert_instruction_candidate(&candidate, &inspection)
            .expect("insert candidate");
        assert_eq!(
            store
                .get_instruction_candidate(&candidate.id)
                .expect("load candidate"),
            Some(StoredInstructionCandidate {
                record: candidate,
                inspection,
            })
        );
    }

    #[test]
    fn candidate_check_revision_drift_rolls_back_candidate_insert() {
        let store = Store::in_memory().expect("store");
        let source = test_source("instruction-source-drift");
        store
            .save_instruction_source(&source, 1, 10)
            .expect("save source");
        let mut current = source.clone();
        current.revision = ConfigurationRevision::new(2).expect("revision");
        store
            .save_instruction_source(&current, 1, 11)
            .expect("advance source revision");
        let candidate = test_candidate("candidate-drift", &source.id, 'a', 20);

        let error = store
            .persist_instruction_candidate_check(
                &source.id,
                source.revision,
                20,
                &candidate,
                &serde_json::json!({"issues": []}),
            )
            .expect_err("stale source revision must fail");
        assert!(matches!(
            error,
            StoreError::RevisionDrift {
                actual: Some(actual),
                ..
            } if actual == current.revision
        ));
        assert_eq!(
            store
                .get_instruction_candidate(&candidate.id)
                .expect("load candidate"),
            None
        );
        assert_eq!(
            store
                .get_instruction_source(&source.id)
                .expect("load source")
                .expect("source")
                .last_checked_at_ms,
            source.last_checked_at_ms
        );
    }

    #[test]
    fn successful_checks_supersede_noncurrent_candidates() {
        let store = Store::in_memory().expect("store");
        let source = test_source("instruction-source-supersede");
        store
            .save_instruction_source(&source, 1, 10)
            .expect("save source");
        let first = test_candidate("candidate-first", &source.id, 'a', 20);
        let second = test_candidate("candidate-second", &source.id, 'b', 30);
        let inspection = serde_json::json!({"issues": []});
        store
            .persist_instruction_candidate_check(
                &source.id,
                source.revision,
                20,
                &first,
                &inspection,
            )
            .expect("persist first candidate");
        store
            .persist_instruction_candidate_check(
                &source.id,
                source.revision,
                30,
                &second,
                &inspection,
            )
            .expect("persist second candidate");

        assert_eq!(
            store
                .get_instruction_candidate(&first.id)
                .expect("load first")
                .expect("first")
                .record
                .state,
            InstructionCandidateState::Superseded
        );
        assert_eq!(
            store
                .latest_instruction_candidate(&source.id)
                .expect("load latest")
                .expect("latest")
                .record
                .id,
            second.id
        );

        store
            .record_instruction_source_check_at_commit(
                &source.id,
                source.revision,
                40,
                None,
                Some(&first.observed_source_commit_sha),
            )
            .expect("record return to first commit");
        assert_eq!(
            store
                .get_instruction_candidate(&second.id)
                .expect("load second")
                .expect("second")
                .record
                .state,
            InstructionCandidateState::Superseded
        );
        assert_eq!(
            store
                .latest_instruction_candidate(&source.id)
                .expect("load latest"),
            None
        );
    }

    #[test]
    fn project_trust_requires_every_exact_key_and_identity_change_revokes() {
        let store = Store::in_memory().expect("store");
        let mut repository = test_project_repository();
        store
            .save_project_repository(&repository)
            .expect("save repository");
        let trust = test_project_trust(&repository, "trust-one", 'a', 11);
        store
            .insert_project_layer_trust(&trust)
            .expect("insert trust");

        let lookup = |organization: &OrganizationId,
                      project: &ProjectId,
                      repository_id: &pandamux_core::ProjectRepositoryId,
                      content: &ContentSha256| {
            store
                .get_active_project_layer_trust(organization, project, repository_id, content)
                .expect("load trust")
        };
        assert_eq!(
            lookup(
                &trust.organization_id,
                &trust.project_id,
                &trust.repository_id,
                &trust.project_content_sha256,
            ),
            Some(trust.clone())
        );
        assert!(
            lookup(
                &OrganizationId::new("organization-other"),
                &trust.project_id,
                &trust.repository_id,
                &trust.project_content_sha256,
            )
            .is_none()
        );
        assert!(
            lookup(
                &trust.organization_id,
                &ProjectId::new("project-other"),
                &trust.repository_id,
                &trust.project_content_sha256,
            )
            .is_none()
        );
        assert!(
            lookup(
                &trust.organization_id,
                &trust.project_id,
                &pandamux_core::ProjectRepositoryId::new("repository-other"),
                &trust.project_content_sha256,
            )
            .is_none()
        );
        assert!(
            lookup(
                &trust.organization_id,
                &trust.project_id,
                &trust.repository_id,
                &ContentSha256::parse("b".repeat(64)).expect("other content hash"),
            )
            .is_none()
        );

        repository.canonical_remote_identity = "example.invalid/team/other".to_string();
        repository.revision = ConfigurationRevision::new(2).expect("revision");
        repository.updated_at_ms = 20;
        store
            .save_project_repository(&repository)
            .expect("change remote identity");
        assert!(
            lookup(
                &trust.organization_id,
                &trust.project_id,
                &trust.repository_id,
                &trust.project_content_sha256,
            )
            .is_none()
        );

        let root_trust = test_project_trust(&repository, "trust-two", 'a', 21);
        store
            .insert_project_layer_trust(&root_trust)
            .expect("insert replacement trust");
        repository.canonical_repository_root_identity = "root-identity-two".to_string();
        repository.revision = ConfigurationRevision::new(3).expect("revision");
        repository.updated_at_ms = 30;
        store
            .save_project_repository(&repository)
            .expect("change root identity");
        assert!(
            lookup(
                &root_trust.organization_id,
                &root_trust.project_id,
                &root_trust.repository_id,
                &root_trust.project_content_sha256,
            )
            .is_none()
        );

        let mut wrong_scope = test_project_trust(&repository, "trust-wrong-scope", 'c', 31);
        wrong_scope.organization_id = OrganizationId::new("organization-other");
        assert!(store.insert_project_layer_trust(&wrong_scope).is_err());
    }

    #[test]
    fn activation_crash_boundaries_leave_old_or_new_complete_state() {
        let store = Store::in_memory().expect("store");
        let first_object = test_bundle_object('a', 10);
        store
            .save_bundle_object(&first_object)
            .expect("save first object");
        let first = test_activation(&first_object, "activation-first", None, 10);
        store
            .activate_bundle_with_outbox(
                &first,
                None,
                None,
                &test_activation_audit(&first, "audit-first"),
                &[],
                None,
            )
            .expect("activate first bundle");

        let second_object = test_bundle_object('b', 20);
        store
            .save_bundle_object(&second_object)
            .expect("save second object");
        let second = test_activation(
            &second_object,
            "activation-second",
            Some(first_object.bundle_sha.clone()),
            20,
        );
        let distribution = BundleDistributionRecord {
            transfer_id: BundleTransferId::new("transfer-second"),
            environment_id: EnvironmentId::new("environment-remote"),
            bundle_sha: second_object.bundle_sha.clone(),
            archive_sha256: second_object.archive_sha256.clone(),
            expected_prior_bundle_sha: Some(first_object.bundle_sha.clone()),
            state: BundleDistributionState::Pending,
            receipt: None,
            created_at_ms: 20,
            updated_at_ms: 20,
        };
        assert!(
            store
                .activate_bundle_with_outbox(
                    &second,
                    None,
                    None,
                    &test_activation_audit(&second, "audit-second-before"),
                    std::slice::from_ref(&distribution),
                    Some(BundleActivationFailpoint::BeforeTransaction),
                )
                .is_err()
        );
        assert_eq!(
            store
                .current_bundle_activation()
                .expect("current activation"),
            Some(first.clone())
        );
        assert!(
            store
                .list_incomplete_bundle_distributions(None)
                .expect("pending distributions")
                .is_empty()
        );

        assert!(
            store
                .activate_bundle_with_outbox(
                    &second,
                    None,
                    None,
                    &test_activation_audit(&second, "audit-second-after"),
                    std::slice::from_ref(&distribution),
                    Some(BundleActivationFailpoint::AfterTransaction),
                )
                .is_err()
        );
        assert_eq!(
            store
                .current_bundle_activation()
                .expect("current activation"),
            Some(second)
        );
        assert_eq!(
            store
                .list_incomplete_bundle_distributions(None)
                .expect("pending distributions"),
            vec![distribution]
        );
    }
}
