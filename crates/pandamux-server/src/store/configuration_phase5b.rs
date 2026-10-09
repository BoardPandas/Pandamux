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

    #[test]
    fn source_and_candidate_round_trip_through_migration_thirteen_tables() {
        let store = Store::in_memory().expect("store");
        let source = InstructionsSourceRecord {
            id: InstructionSourceId::new("instruction-source-test"),
            canonical_remote_identity: "https://example.invalid/org/repo".to_string(),
            tracking: TrackingRef::Branch("refs/heads/main".to_string()),
            policy: InstructionsUpdatePolicy::ApproveEach,
            require_signed_commit: false,
            revision: ConfigurationRevision::new(1).expect("revision"),
            last_checked_at_ms: Some(10),
            last_error_reason: None,
            active_source_commit_sha: None,
            active_bundle_sha: None,
        };
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
}
