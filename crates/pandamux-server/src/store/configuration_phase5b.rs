use super::{Store, StoreError, checked_i64, checked_u64, configuration_revision};
use pandamux_core::{
    BundleSha, ConfigurationAuditRecord, ContentSha256, InstructionCandidateId,
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
        let conn = self.conn.lock().unwrap();
        let existing = conn
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
        conn.execute(
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
        Ok(())
    }

    pub fn insert_project_layer_trust(
        &self,
        trust: &ProjectLayerTrustRecord,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
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
}
