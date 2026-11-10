use super::{Store, StoreError, checked_i64, checked_u64, configuration_revision};
use pandamux_core::{
    BundleActivationId, BundleSha, BundleTransferId, ConfigurationAuditId, ConfigurationRevision,
    ContentSha256, EnvironmentId, InstructionSourceId, InstructionsSourceRecord,
    InstructionsUpdatePolicy, MutationAuditId, MutationAuditRecord, MutationCaller, MutationImpact,
    MutationProposal, MutationProposalId, MutationResolution, MutationTarget, ProjectId,
    ProjectLayerTrustId, ProjectRepositoryId, SourceCommitSha, TrackingRef,
    TrustedSettingsMutationState, UserSettings,
};
use pandamux_orchestrator::{canonical_json_bytes, canonical_json_sha256};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_MUTATION_LIST: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationApplyFailpoint {
    BeforeCommit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredMutationProposal {
    pub proposal: MutationProposal,
    pub caller_identity: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MutationProposalSave {
    pub proposal: MutationProposal,
    pub created: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectTrustMutationState {
    pub project_id: ProjectId,
    pub repository_id: ProjectRepositoryId,
    pub project_content_sha256: ContentSha256,
    pub trusted: bool,
}

impl Store {
    pub fn save_mutation_proposal(
        &self,
        proposal: &MutationProposal,
        caller_identity: &str,
    ) -> Result<MutationProposalSave, StoreError> {
        validate_proposal(proposal, caller_identity)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "INSERT INTO mutation_proposals (
                id, request_id, caller_identity, caller_json, tool_name,
                environment_id, target_json, expected_revision, old_json,
                new_json, old_digest, new_digest, impact_json, resolution,
                audit_id, created_at_ms, expires_at_ms, resolved_at_ms
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                ?13, 'pending', NULL, ?14, ?15, NULL
             )
             ON CONFLICT(caller_identity, tool_name, request_id) DO NOTHING",
            params![
                proposal.id.as_str(),
                proposal.request_id,
                caller_identity,
                serde_json::to_string(&proposal.caller)?,
                proposal.tool_name,
                proposal.environment_id.as_str(),
                serde_json::to_string(&proposal.target)?,
                checked_i64(
                    proposal.expected_revision.get(),
                    "mutation expected revision"
                )?,
                serde_json::to_string(&proposal.old_json)?,
                serde_json::to_string(&proposal.new_json)?,
                proposal.old_digest.as_str(),
                proposal.new_digest.as_str(),
                serde_json::to_string(&proposal.impact)?,
                checked_i64(proposal.created_at_ms, "mutation created_at_ms")?,
                checked_i64(proposal.expires_at_ms, "mutation expires_at_ms")?,
            ],
        )?;
        let stored = load_mutation_by_identity(
            &tx,
            caller_identity,
            &proposal.tool_name,
            &proposal.request_id,
        )?
        .ok_or_else(|| {
            StoreError::Data("mutation proposal was not available after insert".to_string())
        })?;
        if !same_proposal_request(&stored.proposal, proposal) {
            return Err(StoreError::Data(
                "mutation request identity conflicts with stored content".to_string(),
            ));
        }
        tx.commit()?;
        Ok(MutationProposalSave {
            proposal: stored.proposal,
            created: changed == 1,
        })
    }

    pub fn get_mutation_proposal(
        &self,
        proposal_id: &MutationProposalId,
    ) -> Result<Option<MutationProposal>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(load_mutation_by_id(&conn, proposal_id)?.map(|stored| stored.proposal))
    }

    pub fn get_stored_mutation_proposal(
        &self,
        proposal_id: &MutationProposalId,
    ) -> Result<Option<StoredMutationProposal>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_mutation_by_id(&conn, proposal_id)
    }

    pub fn get_mutation_audit_for_proposal(
        &self,
        proposal_id: &MutationProposalId,
    ) -> Result<Option<MutationAuditRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, target_json, old_digest, new_digest, resolver_json,
                    applied_revision, applied_at_ms
             FROM mutation_audit WHERE proposal_id = ?1",
            [proposal_id.as_str()],
            |row| {
                let id = row.get::<_, String>(0)?;
                let target = target_from_value(json_column(row, 1)?)?;
                let old_digest = content_digest_from_sql(row.get::<_, String>(2)?)?;
                let new_digest = content_digest_from_sql(row.get::<_, String>(3)?)?;
                let resolver = caller_from_value(json_column(row, 4)?)?;
                let applied_revision = revision_from_sql(row.get::<_, i64>(5)?)?;
                let applied_at_ms = unsigned_from_sql(row.get::<_, i64>(6)?)?;
                Ok(MutationAuditRecord {
                    id: MutationAuditId::from(id),
                    proposal_id: proposal_id.clone(),
                    target,
                    old_digest,
                    new_digest,
                    resolver,
                    applied_revision,
                    applied_at_ms,
                })
            },
        )
        .optional()
        .map_err(StoreError::from)
    }

    pub fn list_mutation_proposals(
        &self,
        include_terminal: bool,
        before_created_at_ms: Option<u64>,
        limit: u32,
    ) -> Result<(Vec<MutationProposal>, bool), StoreError> {
        let limit = limit.clamp(1, MAX_MUTATION_LIST);
        let fetch = limit.saturating_add(1);
        let before = before_created_at_ms
            .map(|value| checked_i64(value, "mutation list cursor"))
            .transpose()?;
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(
            "SELECT id, request_id, caller_identity, caller_json, tool_name,
                    environment_id, target_json, expected_revision, old_json,
                    new_json, old_digest, new_digest, impact_json, resolution,
                    audit_id, created_at_ms, expires_at_ms
             FROM mutation_proposals
             WHERE (?1 = 1 OR resolution = 'pending')
               AND (?2 IS NULL OR created_at_ms < ?2)
             ORDER BY created_at_ms DESC, id DESC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![include_terminal, before, i64::from(fetch)],
            stored_mutation_from_row,
        )?;
        let mut stored = rows.collect::<Result<Vec<_>, _>>()?;
        let has_more = stored.len() > limit as usize;
        stored.truncate(limit as usize);
        Ok((
            stored.into_iter().map(|stored| stored.proposal).collect(),
            has_more,
        ))
    }

    pub fn expire_mutation_proposals(&self, now_ms: u64) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute(
            "UPDATE mutation_proposals
             SET resolution = 'expired', resolved_at_ms = ?1
             WHERE resolution = 'pending' AND expires_at_ms <= ?1",
            [checked_i64(now_ms, "mutation expiry time")?],
        )?)
    }

    pub fn cancel_mutation_proposals_for_caller(
        &self,
        caller_identity: &str,
        now_ms: u64,
    ) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute(
            "UPDATE mutation_proposals
             SET resolution = 'cancelled_disconnected', resolved_at_ms = ?1
             WHERE caller_identity = ?2 AND resolution = 'pending'",
            params![
                checked_i64(now_ms, "mutation disconnect time")?,
                caller_identity,
            ],
        )?)
    }

    pub fn reject_mutation_proposal(
        &self,
        proposal_id: &MutationProposalId,
        now_ms: u64,
    ) -> Result<MutationProposal, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut stored = load_mutation_by_id(&tx, proposal_id)?
            .ok_or_else(|| StoreError::Data("mutation proposal was not found".to_string()))?;
        if stored.proposal.resolution == MutationResolution::Rejected {
            return Ok(stored.proposal);
        }
        if stored.proposal.resolution != MutationResolution::Pending {
            return Err(StoreError::Data(
                "mutation proposal already has a terminal resolution".to_string(),
            ));
        }
        let resolution = if now_ms >= stored.proposal.expires_at_ms {
            MutationResolution::Expired
        } else {
            MutationResolution::Rejected
        };
        settle_without_audit(&tx, proposal_id, resolution, now_ms)?;
        stored.proposal.resolution = resolution;
        tx.commit()?;
        Ok(stored.proposal)
    }

    pub fn apply_db_mutation_proposal(
        &self,
        proposal_id: &MutationProposalId,
        resolver: &MutationCaller,
        applied_at_ms: u64,
        failpoint: Option<MutationApplyFailpoint>,
    ) -> Result<MutationProposal, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut stored = load_mutation_by_id(&tx, proposal_id)?
            .ok_or_else(|| StoreError::Data("mutation proposal was not found".to_string()))?;
        if stored.proposal.resolution == MutationResolution::ApprovedApplied {
            return Ok(stored.proposal);
        }
        if stored.proposal.resolution != MutationResolution::Pending {
            return Err(StoreError::Data(
                "mutation proposal already has a terminal resolution".to_string(),
            ));
        }
        if applied_at_ms >= stored.proposal.expires_at_ms {
            settle_without_audit(&tx, proposal_id, MutationResolution::Expired, applied_at_ms)?;
            stored.proposal.resolution = MutationResolution::Expired;
            tx.commit()?;
            return Ok(stored.proposal);
        }

        let applied_revision = match &stored.proposal.target {
            MutationTarget::Settings => {
                apply_settings_mutation(&tx, &stored.proposal, applied_at_ms)?
            }
            MutationTarget::InstructionsSource { source_id } => apply_instruction_mutation(
                &tx,
                &stored.proposal,
                &InstructionSourceId::from(source_id.clone()),
                resolver,
                applied_at_ms,
            )?,
            MutationTarget::ProjectLayerTrust { project_id } => apply_project_trust_mutation(
                &tx,
                &stored.proposal,
                project_id,
                resolver,
                applied_at_ms,
            )?,
            MutationTarget::PersonalConfiguration => {
                return Err(StoreError::Data(
                    "personal configuration requires its Git ref transaction".to_string(),
                ));
            }
        };
        let Some(applied_revision) = applied_revision else {
            settle_without_audit(
                &tx,
                proposal_id,
                MutationResolution::SupersededRevision,
                applied_at_ms,
            )?;
            stored.proposal.resolution = MutationResolution::SupersededRevision;
            tx.commit()?;
            return Ok(stored.proposal);
        };
        let audit = mutation_audit(&stored.proposal, resolver, applied_revision, applied_at_ms);
        insert_mutation_audit(&tx, &audit)?;
        resolve_applied(&tx, proposal_id, &audit.id, applied_at_ms)?;
        if failpoint == Some(MutationApplyFailpoint::BeforeCommit) {
            return Err(StoreError::Data(
                "injected failure before mutation commit".to_string(),
            ));
        }
        tx.commit()?;
        stored.proposal.resolution = MutationResolution::ApprovedApplied;
        stored.proposal.audit_id = Some(audit.id);
        Ok(stored.proposal)
    }

    pub fn finalize_personal_mutation_proposal(
        &self,
        proposal_id: &MutationProposalId,
        resolver: &MutationCaller,
        applied_revision: ConfigurationRevision,
        applied_at_ms: u64,
        failpoint: Option<MutationApplyFailpoint>,
    ) -> Result<MutationProposal, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut stored = load_mutation_by_id(&tx, proposal_id)?
            .ok_or_else(|| StoreError::Data("mutation proposal was not found".to_string()))?;
        if stored.proposal.resolution == MutationResolution::ApprovedApplied {
            return Ok(stored.proposal);
        }
        if stored.proposal.resolution != MutationResolution::Pending
            || stored.proposal.target != MutationTarget::PersonalConfiguration
        {
            return Err(StoreError::Data(
                "personal mutation proposal is not pending".to_string(),
            ));
        }
        if applied_revision
            != stored
                .proposal
                .expected_revision
                .checked_next()
                .map_err(|error| StoreError::Data(error.to_string()))?
        {
            return Err(StoreError::Data(
                "personal mutation result has an unexpected revision".to_string(),
            ));
        }
        let audit = mutation_audit(&stored.proposal, resolver, applied_revision, applied_at_ms);
        insert_mutation_audit(&tx, &audit)?;
        resolve_applied(&tx, proposal_id, &audit.id, applied_at_ms)?;
        if failpoint == Some(MutationApplyFailpoint::BeforeCommit) {
            return Err(StoreError::Data(
                "injected failure before personal mutation audit commit".to_string(),
            ));
        }
        tx.commit()?;
        stored.proposal.resolution = MutationResolution::ApprovedApplied;
        stored.proposal.audit_id = Some(audit.id);
        Ok(stored.proposal)
    }
}

fn validate_proposal(proposal: &MutationProposal, caller_identity: &str) -> Result<(), StoreError> {
    for value in [
        proposal.request_id.as_str(),
        proposal.tool_name.as_str(),
        caller_identity,
    ] {
        if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(StoreError::Data(
                "mutation proposal metadata is invalid".to_string(),
            ));
        }
    }
    if proposal.resolution != MutationResolution::Pending
        || proposal.audit_id.is_some()
        || proposal.expires_at_ms <= proposal.created_at_ms
        || canonical_digest(&proposal.old_json)? != proposal.old_digest
        || canonical_digest(&proposal.new_json)? != proposal.new_digest
    {
        return Err(StoreError::Data(
            "mutation proposal content is invalid".to_string(),
        ));
    }
    Ok(())
}

fn same_proposal_request(left: &MutationProposal, right: &MutationProposal) -> bool {
    left.request_id == right.request_id
        && left.caller == right.caller
        && left.tool_name == right.tool_name
        && left.environment_id == right.environment_id
        && left.target == right.target
        && left.expected_revision == right.expected_revision
        && left.old_json == right.old_json
        && left.new_json == right.new_json
        && left.old_digest == right.old_digest
        && left.new_digest == right.new_digest
        && left.impact == right.impact
}

fn load_mutation_by_identity(
    conn: &rusqlite::Connection,
    caller_identity: &str,
    tool_name: &str,
    request_id: &str,
) -> Result<Option<StoredMutationProposal>, StoreError> {
    conn.query_row(
        "SELECT id, request_id, caller_identity, caller_json, tool_name,
                environment_id, target_json, expected_revision, old_json,
                new_json, old_digest, new_digest, impact_json, resolution,
                audit_id, created_at_ms, expires_at_ms
         FROM mutation_proposals
         WHERE caller_identity = ?1 AND tool_name = ?2 AND request_id = ?3",
        params![caller_identity, tool_name, request_id],
        stored_mutation_from_row,
    )
    .optional()
    .map_err(StoreError::from)
}

fn load_mutation_by_id(
    conn: &rusqlite::Connection,
    proposal_id: &MutationProposalId,
) -> Result<Option<StoredMutationProposal>, StoreError> {
    conn.query_row(
        "SELECT id, request_id, caller_identity, caller_json, tool_name,
                environment_id, target_json, expected_revision, old_json,
                new_json, old_digest, new_digest, impact_json, resolution,
                audit_id, created_at_ms, expires_at_ms
         FROM mutation_proposals WHERE id = ?1",
        [proposal_id.as_str()],
        stored_mutation_from_row,
    )
    .optional()
    .map_err(StoreError::from)
}

fn stored_mutation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMutationProposal> {
    let id = row.get::<_, String>(0)?;
    let request_id = row.get::<_, String>(1)?;
    let caller_identity = row.get::<_, String>(2)?;
    let caller = json_column(row, 3)?;
    let tool_name = row.get::<_, String>(4)?;
    let environment_id = row.get::<_, String>(5)?;
    let target = json_column(row, 6)?;
    let expected_revision = row.get::<_, i64>(7)?;
    let old_json = json_column(row, 8)?;
    let new_json = json_column(row, 9)?;
    let old_digest = row.get::<_, String>(10)?;
    let new_digest = row.get::<_, String>(11)?;
    let impact = json_column(row, 12)?;
    let resolution = row.get::<_, String>(13)?;
    let audit_id = row.get::<_, Option<String>>(14)?;
    let created_at_ms = row.get::<_, i64>(15)?;
    let expires_at_ms = row.get::<_, i64>(16)?;
    let proposal = MutationProposal {
        id: MutationProposalId::from(id),
        request_id,
        caller: caller_from_value(caller)?,
        tool_name,
        environment_id: EnvironmentId::from(environment_id),
        target: target_from_value(target)?,
        expected_revision: revision_from_sql(expected_revision)?,
        old_json,
        new_json,
        old_digest: content_digest_from_sql(old_digest)?,
        new_digest: content_digest_from_sql(new_digest)?,
        impact: impact_from_value(impact)?,
        created_at_ms: unsigned_from_sql(created_at_ms)?,
        expires_at_ms: unsigned_from_sql(expires_at_ms)?,
        resolution: resolution_from_sql(resolution)?,
        audit_id: audit_id.map(MutationAuditId::from),
    };
    Ok(StoredMutationProposal {
        proposal,
        caller_identity,
    })
}

fn json_column(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Value> {
    let json = row.get::<_, String>(index)?;
    serde_json::from_str(&json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn caller_from_value(value: Value) -> rusqlite::Result<MutationCaller> {
    serde_json::from_value(value).map_err(json_from_sql)
}

fn target_from_value(value: Value) -> rusqlite::Result<MutationTarget> {
    serde_json::from_value(value).map_err(json_from_sql)
}

fn impact_from_value(value: Value) -> rusqlite::Result<MutationImpact> {
    serde_json::from_value(value).map_err(json_from_sql)
}

fn json_from_sql(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

fn revision_from_sql(value: i64) -> rusqlite::Result<ConfigurationRevision> {
    configuration_revision(value, "mutation expected revision").map_err(store_from_sql)
}

fn content_digest_from_sql(value: String) -> rusqlite::Result<ContentSha256> {
    ContentSha256::parse(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn resolution_from_sql(value: String) -> rusqlite::Result<MutationResolution> {
    serde_json::from_value(Value::String(value)).map_err(json_from_sql)
}

fn unsigned_from_sql(value: i64) -> rusqlite::Result<u64> {
    checked_u64(value, "mutation timestamp").map_err(store_from_sql)
}

fn store_from_sql(error: StoreError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Integer, Box::new(error))
}

fn settle_without_audit(
    conn: &rusqlite::Connection,
    proposal_id: &MutationProposalId,
    resolution: MutationResolution,
    resolved_at_ms: u64,
) -> Result<(), StoreError> {
    let resolution = serde_json::to_value(resolution)?
        .as_str()
        .ok_or_else(|| StoreError::Data("mutation resolution is invalid".to_string()))?
        .to_string();
    let changed = conn.execute(
        "UPDATE mutation_proposals
         SET resolution = ?1, resolved_at_ms = ?2
         WHERE id = ?3 AND resolution = 'pending'",
        params![
            resolution,
            checked_i64(resolved_at_ms, "mutation resolved_at_ms")?,
            proposal_id.as_str(),
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::Data(
            "mutation resolution race did not update one pending row".to_string(),
        ));
    }
    Ok(())
}

fn apply_settings_mutation(
    conn: &rusqlite::Connection,
    proposal: &MutationProposal,
    applied_at_ms: u64,
) -> Result<Option<ConfigurationRevision>, StoreError> {
    let current = conn
        .query_row(
            "SELECT value_json, revision FROM settings WHERE key = 'user_settings'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((current_json, current_revision)) = current else {
        return Ok(None);
    };
    let current_revision = configuration_revision(current_revision, "settings.revision")?;
    let mut current_settings: UserSettings = serde_json::from_str(&current_json)?;
    current_settings.normalize();
    let current_state = TrustedSettingsMutationState {
        trusted_configuration: current_settings.trusted_configuration.clone(),
    };
    let current_value = canonical_value(&current_state)?;
    if current_revision != proposal.expected_revision
        || canonical_digest(&current_value)? != proposal.old_digest
    {
        return Ok(None);
    }
    let next: TrustedSettingsMutationState = serde_json::from_value(proposal.new_json.clone())?;
    if canonical_digest(&canonical_value(&next)?)? != proposal.new_digest {
        return Err(StoreError::Data(
            "settings mutation new digest is invalid".to_string(),
        ));
    }
    current_settings.trusted_configuration = next.trusted_configuration;
    current_settings.normalize();
    let changed = conn.execute(
        "UPDATE settings
         SET value_json = ?1, revision = revision + 1, updated_at_ms = ?2
         WHERE key = 'user_settings' AND revision = ?3",
        params![
            serde_json::to_string(&current_settings)?,
            checked_i64(applied_at_ms, "settings mutation time")?,
            checked_i64(current_revision.get(), "settings mutation revision")?,
        ],
    )?;
    if changed != 1 {
        return Ok(None);
    }
    current_revision
        .checked_next()
        .map(Some)
        .map_err(|error| StoreError::Data(error.to_string()))
}

fn apply_instruction_mutation(
    conn: &rusqlite::Connection,
    proposal: &MutationProposal,
    source_id: &InstructionSourceId,
    resolver: &MutationCaller,
    applied_at_ms: u64,
) -> Result<Option<ConfigurationRevision>, StoreError> {
    let Some(current) = load_instruction_source(conn, source_id)? else {
        return Ok(None);
    };
    let current_value = canonical_value(&current)?;
    if current.revision != proposal.expected_revision
        || canonical_digest(&current_value)? != proposal.old_digest
    {
        return Ok(None);
    }
    let next: InstructionsSourceRecord = serde_json::from_value(proposal.new_json.clone())?;
    let expected_next = current
        .revision
        .checked_next()
        .map_err(|error| StoreError::Data(error.to_string()))?;
    if next.id != *source_id
        || next.revision != expected_next
        || next.canonical_remote_identity != current.canonical_remote_identity
        || canonical_digest(&canonical_value(&next)?)? != proposal.new_digest
    {
        return Err(StoreError::Data(
            "instructions mutation new state is invalid".to_string(),
        ));
    }
    if next.active_bundle_sha != current.active_bundle_sha {
        reactivate_retained_bundle(conn, proposal, &current, &next, resolver, applied_at_ms)?;
    }
    let (tracking_kind, tracking_value) = tracking_parts(&next.tracking);
    let changed = conn.execute(
        "UPDATE instruction_sources
         SET tracking_kind = ?1, tracking_value = ?2, update_policy = ?3,
             require_signed_commit = ?4, revision = ?5,
             last_checked_at_ms = ?6, last_error_reason = ?7,
             active_source_commit_sha = ?8, active_bundle_sha = ?9,
             updated_at_ms = ?10
         WHERE id = ?11 AND revision = ?12",
        params![
            tracking_kind,
            tracking_value,
            policy_text(next.policy),
            next.require_signed_commit,
            checked_i64(next.revision.get(), "instructions next revision")?,
            next.last_checked_at_ms
                .map(|value| checked_i64(value, "instructions last checked"))
                .transpose()?,
            next.last_error_reason,
            next.active_source_commit_sha
                .as_ref()
                .map(SourceCommitSha::as_str),
            next.active_bundle_sha.as_ref().map(BundleSha::as_str),
            checked_i64(applied_at_ms, "instructions mutation time")?,
            source_id.as_str(),
            checked_i64(current.revision.get(), "instructions expected revision")?,
        ],
    )?;
    Ok((changed == 1).then_some(next.revision))
}

fn reactivate_retained_bundle(
    conn: &rusqlite::Connection,
    proposal: &MutationProposal,
    current: &InstructionsSourceRecord,
    next: &InstructionsSourceRecord,
    resolver: &MutationCaller,
    applied_at_ms: u64,
) -> Result<(), StoreError> {
    let bundle_sha = next
        .active_bundle_sha
        .as_ref()
        .ok_or_else(|| StoreError::Data("bundle revert target is missing".to_string()))?;
    let (manifest_json, archive_sha256) = conn
        .query_row(
            "SELECT source_manifest_json, archive_sha256
             FROM bundle_objects WHERE bundle_sha = ?1",
            [bundle_sha.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::Data("retained bundle object is missing".to_string()))?;
    let source_manifest: pandamux_core::BundleSourceManifest =
        serde_json::from_str(&manifest_json)?;
    if next.active_source_commit_sha.as_ref() != Some(&source_manifest.instructions_commit_sha) {
        return Err(StoreError::Data(
            "bundle revert source commit does not match the retained object".to_string(),
        ));
    }
    let current_activation = conn
        .query_row(
            "SELECT bundle_sha FROM bundle_activations WHERE deactivated_at_ms IS NULL",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if current_activation.as_deref() != current.active_bundle_sha.as_ref().map(BundleSha::as_str) {
        return Err(StoreError::Data(
            "bundle activation changed before the revert".to_string(),
        ));
    }
    conn.execute(
        "UPDATE bundle_activations SET deactivated_at_ms = ?1
         WHERE deactivated_at_ms IS NULL",
        [checked_i64(
            applied_at_ms,
            "bundle revert deactivation time",
        )?],
    )?;
    let activation_id = BundleActivationId::generate();
    conn.execute(
        "INSERT INTO bundle_activations (
            id, bundle_sha, candidate_id, source_manifest_json, kind, actor,
            prior_bundle_sha, activated_at_ms, deactivated_at_ms
         ) VALUES (?1, ?2, NULL, ?3, 'revert', ?4, ?5, ?6, NULL)",
        params![
            activation_id.as_str(),
            bundle_sha.as_str(),
            manifest_json,
            resolver.display_name,
            current.active_bundle_sha.as_ref().map(BundleSha::as_str),
            checked_i64(applied_at_ms, "bundle revert activation time")?,
        ],
    )?;
    let archive_sha256 = pandamux_core::ArchiveSha256::parse(archive_sha256)
        .map_err(|error| StoreError::Data(error.to_string()))?;
    for environment_id in &proposal.impact.environment_ids {
        if environment_id == &proposal.environment_id {
            continue;
        }
        conn.execute(
            "DELETE FROM bundle_distribution
             WHERE environment_id = ?1 AND bundle_sha = ?2",
            params![environment_id.as_str(), bundle_sha.as_str()],
        )?;
        let transfer_id = BundleTransferId::generate();
        conn.execute(
            "INSERT INTO bundle_distribution (
                transfer_id, environment_id, bundle_sha, archive_sha256,
                expected_prior_bundle_sha, state, receipt_json,
                created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', NULL, ?6, ?6)",
            params![
                transfer_id.as_str(),
                environment_id.as_str(),
                bundle_sha.as_str(),
                archive_sha256.as_str(),
                current.active_bundle_sha.as_ref().map(BundleSha::as_str),
                checked_i64(applied_at_ms, "bundle revert distribution time")?,
            ],
        )?;
    }
    let audit_id = ConfigurationAuditId::generate();
    conn.execute(
        "INSERT INTO configuration_audit (
            id, kind, actor, source_id, candidate_id, bundle_sha,
            details_json, created_at_ms
         ) VALUES (?1, 'bundle_reactivated', ?2, ?3, NULL, ?4, ?5, ?6)",
        params![
            audit_id.as_str(),
            resolver.display_name,
            next.id.as_str(),
            bundle_sha.as_str(),
            serde_json::to_string(&serde_json::json!({
                "activationId": activation_id,
                "priorBundleSha": current.active_bundle_sha.clone(),
            }))?,
            checked_i64(applied_at_ms, "configuration audit time")?,
        ],
    )?;
    Ok(())
}

fn apply_project_trust_mutation(
    conn: &rusqlite::Connection,
    proposal: &MutationProposal,
    project_id: &ProjectId,
    resolver: &MutationCaller,
    applied_at_ms: u64,
) -> Result<Option<ConfigurationRevision>, StoreError> {
    let old: ProjectTrustMutationState = serde_json::from_value(proposal.old_json.clone())?;
    let new: ProjectTrustMutationState = serde_json::from_value(proposal.new_json.clone())?;
    if old.project_id != *project_id
        || new.project_id != *project_id
        || old.repository_id != new.repository_id
        || old.project_content_sha256 != new.project_content_sha256
        || old.trusted == new.trusted
    {
        return Err(StoreError::Data(
            "project trust mutation state is invalid".to_string(),
        ));
    }
    let repository_revision = conn
        .query_row(
            "SELECT revision FROM project_repositories
             WHERE id = ?1 AND project_id = ?2",
            params![old.repository_id.as_str(), project_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(repository_revision) = repository_revision else {
        return Ok(None);
    };
    let repository_revision =
        configuration_revision(repository_revision, "project repository revision")?;
    let currently_trusted = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM project_layer_trust
            WHERE repository_id = ?1 AND project_id = ?2
              AND project_content_sha256 = ?3 AND revoked_at_ms IS NULL
         )",
        params![
            old.repository_id.as_str(),
            project_id.as_str(),
            old.project_content_sha256.as_str(),
        ],
        |row| row.get::<_, bool>(0),
    )?;
    let current_state = ProjectTrustMutationState {
        trusted: currently_trusted,
        ..old.clone()
    };
    if repository_revision != proposal.expected_revision
        || canonical_digest(&canonical_value(&current_state)?)? != proposal.old_digest
    {
        return Ok(None);
    }
    if new.trusted {
        let organization_id = conn.query_row(
            "SELECT organization_id FROM project_repositories WHERE id = ?1",
            [new.repository_id.as_str()],
            |row| row.get::<_, String>(0),
        )?;
        let trust_id = ProjectLayerTrustId::generate();
        conn.execute(
            "INSERT INTO project_layer_trust (
                id, organization_id, project_id, repository_id,
                project_content_sha256, approved_by, approved_at_ms, revoked_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                trust_id.as_str(),
                organization_id,
                project_id.as_str(),
                new.repository_id.as_str(),
                new.project_content_sha256.as_str(),
                resolver.display_name,
                checked_i64(applied_at_ms, "project trust approval time")?,
            ],
        )?;
    } else {
        let changed = conn.execute(
            "UPDATE project_layer_trust SET revoked_at_ms = ?1
             WHERE repository_id = ?2 AND project_id = ?3
               AND project_content_sha256 = ?4 AND revoked_at_ms IS NULL",
            params![
                checked_i64(applied_at_ms, "project trust revocation time")?,
                new.repository_id.as_str(),
                project_id.as_str(),
                new.project_content_sha256.as_str(),
            ],
        )?;
        if changed != 1 {
            return Ok(None);
        }
    }
    let next_revision = repository_revision
        .checked_next()
        .map_err(|error| StoreError::Data(error.to_string()))?;
    let changed = conn.execute(
        "UPDATE project_repositories
         SET revision = ?1, updated_at_ms = ?2
         WHERE id = ?3 AND revision = ?4",
        params![
            checked_i64(next_revision.get(), "project trust next revision")?,
            checked_i64(applied_at_ms, "project trust mutation time")?,
            new.repository_id.as_str(),
            checked_i64(repository_revision.get(), "project trust expected revision")?,
        ],
    )?;
    Ok((changed == 1).then_some(next_revision))
}

fn load_instruction_source(
    conn: &rusqlite::Connection,
    source_id: &InstructionSourceId,
) -> Result<Option<InstructionsSourceRecord>, StoreError> {
    conn.query_row(
        "SELECT canonical_remote_identity, tracking_kind, tracking_value,
                update_policy, require_signed_commit, revision,
                last_checked_at_ms, last_error_reason,
                active_source_commit_sha, active_bundle_sha
         FROM instruction_sources WHERE id = ?1",
        [source_id.as_str()],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
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
            signed,
            revision,
            checked,
            error,
            active_commit,
            active_bundle,
        )| {
            Ok(InstructionsSourceRecord {
                id: source_id.clone(),
                canonical_remote_identity: remote,
                tracking: parse_tracking(&tracking_kind, tracking_value)?,
                policy: parse_policy(&policy)?,
                require_signed_commit: signed,
                revision: configuration_revision(revision, "instruction source revision")?,
                last_checked_at_ms: checked
                    .map(|value| checked_u64(value, "instruction source checked_at_ms"))
                    .transpose()?,
                last_error_reason: error,
                active_source_commit_sha: active_commit
                    .map(SourceCommitSha::parse)
                    .transpose()
                    .map_err(|error| StoreError::Data(error.to_string()))?,
                active_bundle_sha: active_bundle
                    .map(BundleSha::parse)
                    .transpose()
                    .map_err(|error| StoreError::Data(error.to_string()))?,
            })
        },
    )
    .transpose()
}

fn tracking_parts(tracking: &TrackingRef) -> (&'static str, String) {
    match tracking {
        TrackingRef::Branch(value) => ("branch", value.clone()),
        TrackingRef::Tag(value) => ("tag", value.clone()),
        TrackingRef::Commit(value) => ("commit", value.as_str().to_string()),
    }
}

fn parse_tracking(kind: &str, value: String) -> Result<TrackingRef, StoreError> {
    match kind {
        "branch" => Ok(TrackingRef::Branch(value)),
        "tag" => Ok(TrackingRef::Tag(value)),
        "commit" => SourceCommitSha::parse(value)
            .map(TrackingRef::Commit)
            .map_err(|error| StoreError::Data(error.to_string())),
        _ => Err(StoreError::Data(
            "stored instruction tracking kind is invalid".to_string(),
        )),
    }
}

fn policy_text(policy: InstructionsUpdatePolicy) -> &'static str {
    match policy {
        InstructionsUpdatePolicy::ApproveEach => "approve_each",
        InstructionsUpdatePolicy::TrustBranch => "trust_branch",
    }
}

fn parse_policy(value: &str) -> Result<InstructionsUpdatePolicy, StoreError> {
    match value {
        "approve_each" => Ok(InstructionsUpdatePolicy::ApproveEach),
        "trust_branch" => Ok(InstructionsUpdatePolicy::TrustBranch),
        _ => Err(StoreError::Data(
            "stored instruction update policy is invalid".to_string(),
        )),
    }
}

fn mutation_audit(
    proposal: &MutationProposal,
    resolver: &MutationCaller,
    applied_revision: ConfigurationRevision,
    applied_at_ms: u64,
) -> MutationAuditRecord {
    MutationAuditRecord {
        id: MutationAuditId::generate(),
        proposal_id: proposal.id.clone(),
        target: proposal.target.clone(),
        old_digest: proposal.old_digest.clone(),
        new_digest: proposal.new_digest.clone(),
        resolver: resolver.clone(),
        applied_revision,
        applied_at_ms,
    }
}

fn insert_mutation_audit(
    conn: &rusqlite::Connection,
    audit: &MutationAuditRecord,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO mutation_audit (
            id, proposal_id, target_json, old_digest, new_digest,
            resolver_json, applied_revision, applied_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            audit.id.as_str(),
            audit.proposal_id.as_str(),
            serde_json::to_string(&audit.target)?,
            audit.old_digest.as_str(),
            audit.new_digest.as_str(),
            serde_json::to_string(&audit.resolver)?,
            checked_i64(audit.applied_revision.get(), "mutation applied revision")?,
            checked_i64(audit.applied_at_ms, "mutation applied_at_ms")?,
        ],
    )?;
    Ok(())
}

fn resolve_applied(
    conn: &rusqlite::Connection,
    proposal_id: &MutationProposalId,
    audit_id: &MutationAuditId,
    applied_at_ms: u64,
) -> Result<(), StoreError> {
    let changed = conn.execute(
        "UPDATE mutation_proposals
         SET resolution = 'approved_applied', audit_id = ?1, resolved_at_ms = ?2
         WHERE id = ?3 AND resolution = 'pending'",
        params![
            audit_id.as_str(),
            checked_i64(applied_at_ms, "mutation resolved_at_ms")?,
            proposal_id.as_str(),
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::Data(
            "mutation apply race did not resolve one pending proposal".to_string(),
        ));
    }
    Ok(())
}

fn canonical_value<T: Serialize>(value: &T) -> Result<Value, StoreError> {
    serde_json::from_slice(&canonical_json_bytes(value)).map_err(StoreError::from)
}

fn canonical_digest(value: &Value) -> Result<ContentSha256, StoreError> {
    ContentSha256::parse(canonical_json_sha256(value))
        .map_err(|error| StoreError::Data(error.to_string()))
}
