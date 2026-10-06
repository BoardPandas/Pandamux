mod capacity_phase4d;
mod failover_phase4f;
pub mod migrations;
mod usage_phase4c;
mod usage_query_phase4c;

pub use usage_phase4c::{
    DEFAULT_USAGE_OUTBOX_MAX_AGE_MS, DEFAULT_USAGE_OUTBOX_MAX_BYTES, DEFAULT_USAGE_OUTBOX_MAX_ROWS,
    UsageHubIngestOutcome, UsageHubSyncState, UsageOutboxCompaction, UsageOutboxRetention,
};

use pandamux_core::{
    AgentId, AttachmentRecord, AttemptExecutionEvidence, CapacityConfidence, CapacityEvidenceScope,
    CapacitySnapshot, ContinuationState, EnvironmentId, FailoverPool, NodeStoreId,
    OrganizationPolicy, ProjectId, ProviderCapabilities, ProviderInstanceId, ProviderKind,
    ScheduleId, ScheduleRecord, ScheduleTarget, SubscriptionKey, Thread, ThreadEvent,
    ThreadEventKind, ThreadId, ThreadOrigin, TokenCounts, Turn, TurnAttemptStatus, TurnId,
    TurnStatus, TurnUsage, TurnUsageComponent, UsageFactKind, UsageObservation, UsageObservationId,
    UsageProvider, UsageSourceKind, UsageSubscription, UsageUnknownField, UserSettings,
    project_capacity_revisions,
};
use pandamux_protocol::{
    CapacityQueryCursor, CapacityQueryParams, CapacityQueryResult, EventEnvelope, UsageExportItem,
    UsageExportParams, UsageExportResult, UsageProviderSubscriptionMapping, UsageQueryCursor,
    UsageQueryItem, UsageQueryParams, UsageQueryResult, UsageScannerDiagnostic,
    UsageSourceCoverageState,
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

const MAX_RETAINED_SCANNER_QUARANTINES_PER_SOURCE: i64 = 4_096;

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Json(serde_json::Error),
    Data(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(err) => write!(f, "Store SQL error: {err}"),
            Self::Json(err) => write!(f, "Store JSON error: {err}"),
            Self::Data(err) => write!(f, "Store data error: {err}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sql(err)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

/// Durable evidence of the exact provider configuration selected for a turn.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderExecutionAudit {
    pub turn_id: TurnId,
    pub thread_id: ThreadId,
    pub environment_id: EnvironmentId,
    pub provider_instance_id: ProviderInstanceId,
    pub provider_kind: ProviderKind,
    pub profile_dir: String,
    pub binary_path: Option<String>,
    pub capabilities: ProviderCapabilities,
    pub model: String,
    pub created_at_ms: u64,
}

/// Durable scanner position committed in the same transaction as accepted facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageScannerCursor {
    pub environment_id: EnvironmentId,
    pub provider_instance_id: ProviderInstanceId,
    pub source_kind: UsageSourceKind,
    pub node_store_id: NodeStoreId,
    pub file_identity: String,
    pub canonical_path: String,
    pub committed_offset: u64,
    pub committed_prefix_sha256: String,
    pub parser_version: u32,
    pub parser_state_json: String,
    pub observed_size: u64,
    pub observed_modified_at_ms: Option<u64>,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageBatchResult {
    pub inserted: u64,
    pub duplicates: u64,
    pub reconciled: u64,
    pub reconciliation_mismatches: u64,
    pub quarantined: u64,
    pub last_outbox_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageQuarantineRecord {
    pub environment_id: EnvironmentId,
    pub provider_instance_id: ProviderInstanceId,
    pub source_kind: UsageSourceKind,
    pub file_identity_sha256: String,
    pub record_offset: u64,
    pub parser_version: u32,
    pub reason_code: String,
    pub reason_detail: String,
    pub safe_excerpt_sha256: String,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManagedUsageBackfillResult {
    pub batch: UsageBatchResult,
    pub unmapped: u64,
    pub invalid: u64,
}

struct ManagedUsageCandidate {
    turn_id: TurnId,
    thread_id: ThreadId,
    environment_id: EnvironmentId,
    provider_instance_id: ProviderInstanceId,
    ended_at_ms: Option<u64>,
    usage_json: String,
    usage: TurnUsage,
    provider_kind: ProviderKind,
    model: String,
    agent_id: Option<AgentId>,
    schedule_id: Option<ScheduleId>,
}

#[derive(Clone, Debug)]
struct PendingUsageReconciliationMismatch {
    managed_observation_id: UsageObservationId,
    transcript_observation_id: UsageObservationId,
    provider_instance_id: ProviderInstanceId,
    source_kind: UsageSourceKind,
}

/// A thread-safe, single-writer SQLite store wrapper.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    /// Open or create a persistent SQLite store at the given path.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let mut conn = Connection::open(path)?;
        Self::init_connection(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory SQLite store for unit testing.
    pub fn in_memory() -> Result<Self, StoreError> {
        let mut conn = Connection::open_in_memory()?;
        Self::init_connection(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn init_connection(conn: &mut Connection) -> Result<(), StoreError> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )?;
        migrations::apply_migrations(conn)?;
        Ok(())
    }

    /// Flush and truncate the write-ahead log upon graceful shutdown.
    pub fn checkpoint_truncate(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    // ── Threads ──────────────────────────────────────────────────────────

    pub fn save_thread(&self, thread: &Thread) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let project_id = thread.project_id.as_ref().map(|id| id.as_str().to_string());
        let parent_id = thread
            .parent_thread_id
            .as_ref()
            .map(|id| id.as_str().to_string());
        let workspace_json = serde_json::to_string(&thread.workspace)?;
        let status_str = serde_json::to_string(&thread.status)?;
        let access_str = serde_json::to_string(&thread.access_mode)?;
        let agent_json = thread
            .agent
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let origin_json = serde_json::to_string(&thread.origin)?;

        conn.execute(
            "INSERT INTO threads (
                id, project_id, environment_id, parent_thread_id, title,
                provider_instance_id, model, effort, access_mode,
                workspace_json, status, agent_json, origin_json,
                created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                status = excluded.status,
                access_mode = excluded.access_mode,
                workspace_json = excluded.workspace_json,
                agent_json = excluded.agent_json,
                updated_at_ms = excluded.updated_at_ms;",
            params![
                thread.id.as_str(),
                project_id,
                thread.environment_id.as_str(),
                parent_id,
                thread.title,
                thread.provider_instance_id.as_str(),
                thread.model,
                thread.effort,
                access_str,
                workspace_json,
                status_str,
                agent_json,
                origin_json,
                thread.created_at_ms as i64,
                thread.updated_at_ms as i64,
            ],
        )?;

        Ok(())
    }

    pub fn get_thread(&self, id: &ThreadId) -> Result<Option<Thread>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, environment_id, parent_thread_id, title,
                    provider_instance_id, model, effort, access_mode,
                    workspace_json, status, agent_json, origin_json,
                    created_at_ms, updated_at_ms
             FROM threads WHERE id = ?1",
        )?;

        let row = stmt
            .query_row(params![id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, i64>(14)?,
                ))
            })
            .optional()?;

        let Some((
            id_str,
            proj_str,
            env_str,
            parent_str,
            title,
            prov_str,
            model,
            effort,
            access_str,
            ws_json,
            status_str,
            agent_json,
            origin_json,
            created,
            updated,
        )) = row
        else {
            return Ok(None);
        };

        Ok(Some(Thread {
            id: ThreadId::from(id_str),
            project_id: proj_str.map(Into::into),
            environment_id: env_str.into(),
            parent_thread_id: parent_str.map(Into::into),
            title,
            provider_instance_id: prov_str.into(),
            model,
            effort,
            access_mode: serde_json::from_str(&access_str)?,
            workspace: serde_json::from_str(&ws_json)?,
            status: serde_json::from_str(&status_str)?,
            agent: agent_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?,
            origin: serde_json::from_str(&origin_json)?,
            created_at_ms: created as u64,
            updated_at_ms: updated as u64,
        }))
    }

    pub fn list_threads(&self) -> Result<Vec<Thread>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, environment_id, parent_thread_id, title,
                    provider_instance_id, model, effort, access_mode,
                    workspace_json, status, agent_json, origin_json,
                    created_at_ms, updated_at_ms
             FROM threads ORDER BY updated_at_ms DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, i64>(13)?,
                row.get::<_, i64>(14)?,
            ))
        })?;

        let mut result = Vec::new();
        for r in rows {
            let (
                id_str,
                proj_str,
                env_str,
                parent_str,
                title,
                prov_str,
                model,
                effort,
                access_str,
                ws_json,
                status_str,
                agent_json,
                origin_json,
                created,
                updated,
            ) = r?;
            result.push(Thread {
                id: ThreadId::from(id_str),
                project_id: proj_str.map(Into::into),
                environment_id: env_str.into(),
                parent_thread_id: parent_str.map(Into::into),
                title,
                provider_instance_id: prov_str.into(),
                model,
                effort,
                access_mode: serde_json::from_str(&access_str)?,
                workspace: serde_json::from_str(&ws_json)?,
                status: serde_json::from_str(&status_str)?,
                agent: agent_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()?,
                origin: serde_json::from_str(&origin_json)?,
                created_at_ms: created as u64,
                updated_at_ms: updated as u64,
            });
        }

        Ok(result)
    }

    // ── Turns ─────────────────────────────────────────────────────────────

    pub fn save_turn(&self, turn: &Turn) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let input_json = serde_json::to_string(&turn.input)?;
        let status_str = serde_json::to_string(&turn.status)?;
        let usage_json = turn.usage.as_ref().map(serde_json::to_string).transpose()?;

        conn.execute(
            "INSERT INTO turns (
                id, thread_id, environment_id, provider_instance_id,
                seq, input_json, status,
                started_at_ms, ended_at_ms, usage_json,
                checkpoint_before, checkpoint_after
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                ended_at_ms = excluded.ended_at_ms,
                usage_json = excluded.usage_json,
                checkpoint_before = excluded.checkpoint_before,
                checkpoint_after = excluded.checkpoint_after;",
            params![
                turn.id.as_str(),
                turn.thread_id.as_str(),
                turn.environment_id.as_str(),
                turn.provider_instance_id.as_str(),
                turn.seq as i64,
                input_json,
                status_str,
                turn.started_at_ms as i64,
                turn.ended_at_ms.map(|t| t as i64),
                usage_json,
                turn.checkpoint_before,
                turn.checkpoint_after,
            ],
        )?;

        Ok(())
    }

    pub fn get_turn(&self, id: &TurnId) -> Result<Option<Turn>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, thread_id, environment_id, provider_instance_id,
                    seq, input_json, status,
                    started_at_ms, ended_at_ms, usage_json,
                    checkpoint_before, checkpoint_after
             FROM turns WHERE id = ?1",
        )?;

        let row = stmt
            .query_row(params![id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                ))
            })
            .optional()?;

        let Some((
            id_str,
            thread_id_str,
            environment_id_str,
            provider_instance_id_str,
            seq,
            input_json,
            status_str,
            started_ms,
            ended_ms,
            usage_json,
            cp_before,
            cp_after,
        )) = row
        else {
            return Ok(None);
        };

        Ok(Some(Turn {
            id: TurnId::from(id_str),
            thread_id: ThreadId::from(thread_id_str),
            environment_id: EnvironmentId::from(environment_id_str),
            provider_instance_id: ProviderInstanceId::from(provider_instance_id_str),
            seq: seq as u64,
            input: serde_json::from_str(&input_json)?,
            status: serde_json::from_str(&status_str)?,
            started_at_ms: started_ms as u64,
            ended_at_ms: ended_ms.map(|t| t as u64),
            usage: usage_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?,
            checkpoint_before: cp_before,
            checkpoint_after: cp_after,
        }))
    }

    pub fn list_turns(&self, thread_id: &ThreadId) -> Result<Vec<Turn>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, thread_id, environment_id, provider_instance_id,
                    seq, input_json, status,
                    started_at_ms, ended_at_ms, usage_json,
                    checkpoint_before, checkpoint_after
             FROM turns WHERE thread_id = ?1 ORDER BY seq ASC",
        )?;

        let rows = stmt.query_map(params![thread_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
            ))
        })?;

        let mut result = Vec::new();
        for r in rows {
            let (
                id_str,
                thread_id_str,
                environment_id_str,
                provider_instance_id_str,
                seq,
                input_json,
                status_str,
                started_ms,
                ended_ms,
                usage_json,
                cp_before,
                cp_after,
            ) = r?;
            result.push(Turn {
                id: TurnId::from(id_str),
                thread_id: ThreadId::from(thread_id_str),
                environment_id: EnvironmentId::from(environment_id_str),
                provider_instance_id: ProviderInstanceId::from(provider_instance_id_str),
                seq: seq as u64,
                input: serde_json::from_str(&input_json)?,
                status: serde_json::from_str(&status_str)?,
                started_at_ms: started_ms as u64,
                ended_at_ms: ended_ms.map(|t| t as u64),
                usage: usage_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()?,
                checkpoint_before: cp_before,
                checkpoint_after: cp_after,
            });
        }

        Ok(result)
    }

    pub fn record_provider_execution(
        &self,
        audit: &ProviderExecutionAudit,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let provider_kind = serde_json::to_string(&audit.provider_kind)?;
        let capabilities_json = serde_json::to_string(&audit.capabilities)?;
        conn.execute(
            "INSERT INTO provider_execution_audit (
                turn_id, thread_id, environment_id, provider_instance_id,
                provider_kind, profile_dir, binary_path, capabilities_json,
                model, created_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                audit.turn_id.as_str(),
                audit.thread_id.as_str(),
                audit.environment_id.as_str(),
                audit.provider_instance_id.as_str(),
                provider_kind,
                audit.profile_dir.as_str(),
                audit.binary_path.as_deref(),
                capabilities_json,
                audit.model.as_str(),
                audit.created_at_ms as i64,
            ],
        )?;
        Ok(())
    }

    pub fn get_provider_execution_audit(
        &self,
        turn_id: &TurnId,
    ) -> Result<Option<ProviderExecutionAudit>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT turn_id, thread_id, environment_id, provider_instance_id,
                    provider_kind, profile_dir, binary_path, capabilities_json,
                    model, created_at_ms
             FROM provider_execution_audit WHERE turn_id = ?1",
        )?;
        let row = stmt
            .query_row(params![turn_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            })
            .optional()?;
        let Some((
            turn_id,
            thread_id,
            environment_id,
            provider_instance_id,
            provider_kind,
            profile_dir,
            binary_path,
            capabilities_json,
            model,
            created_at_ms,
        )) = row
        else {
            return Ok(None);
        };

        Ok(Some(ProviderExecutionAudit {
            turn_id: TurnId::from(turn_id),
            thread_id: ThreadId::from(thread_id),
            environment_id: EnvironmentId::from(environment_id),
            provider_instance_id: ProviderInstanceId::from(provider_instance_id),
            provider_kind: serde_json::from_str(&provider_kind)?,
            profile_dir,
            binary_path,
            capabilities: serde_json::from_str(&capabilities_json)?,
            model,
            created_at_ms: checked_u64(created_at_ms, "provider_execution_audit.created_at_ms")?,
        }))
    }

    // Canonical organization and failover policy

    pub fn save_failover_pool(
        &self,
        pool: &FailoverPool,
        updated_at_ms: u64,
    ) -> Result<(), StoreError> {
        pool.validate()
            .map_err(|error| StoreError::Data(error.to_string()))?;
        let revision = checked_i64(pool.revision, "failover pool revision")?;
        let updated_at_ms = checked_i64(updated_at_ms, "failover pool updated_at_ms")?;
        let definition_json = serde_json::to_string(pool)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let existing = tx
            .query_row(
                "SELECT revision, definition_json FROM failover_pools WHERE id = ?1",
                [pool.id.as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((existing_revision, existing_json)) = existing {
            if revision < existing_revision {
                return Err(StoreError::Data(format!(
                    "failover pool {} revision {} is older than stored revision {}",
                    pool.id, pool.revision, existing_revision
                )));
            }
            if revision == existing_revision && definition_json != existing_json {
                return Err(StoreError::Data(format!(
                    "failover pool {} changed without a revision increment",
                    pool.id
                )));
            }
        }
        tx.execute(
            "INSERT INTO failover_pools (
                id, display_name, revision, definition_json, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                display_name = excluded.display_name,
                revision = excluded.revision,
                definition_json = excluded.definition_json,
                updated_at_ms = excluded.updated_at_ms",
            params![
                pool.id.as_str(),
                pool.display_name,
                revision,
                definition_json,
                updated_at_ms,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_failover_pool(
        &self,
        id: &pandamux_core::FailoverPoolId,
    ) -> Result<Option<FailoverPool>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let json = conn
            .query_row(
                "SELECT definition_json FROM failover_pools WHERE id = ?1",
                [id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        json.map(|json| {
            let pool: FailoverPool = serde_json::from_str(&json)?;
            pool.validate()
                .map_err(|error| StoreError::Data(error.to_string()))?;
            Ok(pool)
        })
        .transpose()
    }

    pub fn list_failover_pools(&self) -> Result<Vec<FailoverPool>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT definition_json FROM failover_pools ORDER BY display_name ASC, id ASC",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut pools = Vec::new();
        for row in rows {
            let pool: FailoverPool = serde_json::from_str(&row?)?;
            pool.validate()
                .map_err(|error| StoreError::Data(error.to_string()))?;
            pools.push(pool);
        }
        Ok(pools)
    }

    pub fn save_organization_policy(
        &self,
        policy: &OrganizationPolicy,
        updated_at_ms: u64,
    ) -> Result<(), StoreError> {
        policy
            .validate()
            .map_err(|error| StoreError::Data(error.to_string()))?;
        let revision = checked_i64(policy.revision, "organization policy revision")?;
        let updated_at_ms = checked_i64(updated_at_ms, "organization policy updated_at_ms")?;
        let policy_json = serde_json::to_string(policy)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let existing = tx
            .query_row(
                "SELECT revision, policy_json FROM organization_policies WHERE id = ?1",
                [policy.id.as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((existing_revision, existing_json)) = existing {
            if revision < existing_revision {
                return Err(StoreError::Data(format!(
                    "organization {} revision {} is older than stored revision {}",
                    policy.id, policy.revision, existing_revision
                )));
            }
            if revision == existing_revision && policy_json != existing_json {
                return Err(StoreError::Data(format!(
                    "organization {} changed without a revision increment",
                    policy.id
                )));
            }
        }

        for project_id in &policy.project_ids {
            let assigned = tx
                .query_row(
                    "SELECT organization_id FROM organization_project_assignments
                     WHERE project_id = ?1",
                    [project_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if assigned
                .as_deref()
                .is_some_and(|id| id != policy.id.as_str())
            {
                return Err(StoreError::Data(format!(
                    "project {project_id} is already assigned to organization {}",
                    assigned.unwrap_or_default()
                )));
            }
        }

        tx.execute(
            "INSERT INTO organization_policies (
                id, display_name, revision, policy_json, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                display_name = excluded.display_name,
                revision = excluded.revision,
                policy_json = excluded.policy_json,
                updated_at_ms = excluded.updated_at_ms",
            params![
                policy.id.as_str(),
                policy.display_name,
                revision,
                policy_json,
                updated_at_ms,
            ],
        )?;
        tx.execute(
            "DELETE FROM organization_project_assignments WHERE organization_id = ?1",
            [policy.id.as_str()],
        )?;
        for project_id in &policy.project_ids {
            tx.execute(
                "INSERT INTO organization_project_assignments (project_id, organization_id)
                 VALUES (?1, ?2)",
                params![project_id.as_str(), policy.id.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn organization_policy_for_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<OrganizationPolicy, StoreError> {
        if project_id.as_str().trim().is_empty() {
            return Err(StoreError::Data("project id must not be empty".to_string()));
        }
        let conn = self.conn.lock().unwrap();
        let json = conn
            .query_row(
                "SELECT p.policy_json
                 FROM organization_project_assignments a
                 JOIN organization_policies p ON p.id = a.organization_id
                 WHERE a.project_id = ?1",
                [project_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| {
                StoreError::Data(format!(
                    "project {project_id} has no organization assignment"
                ))
            })?;
        let policy: OrganizationPolicy = serde_json::from_str(&json)?;
        policy
            .validate()
            .map_err(|error| StoreError::Data(error.to_string()))?;
        if !policy.project_ids.iter().any(|id| id == project_id) {
            return Err(StoreError::Data(format!(
                "project {project_id} assignment disagrees with canonical policy"
            )));
        }
        Ok(policy)
    }

    pub fn authorize_provider_for_project(
        &self,
        project_id: &ProjectId,
        environment_id: &EnvironmentId,
        provider_instance_id: &ProviderInstanceId,
    ) -> Result<(OrganizationPolicy, SubscriptionKey), StoreError> {
        let policy = self.organization_policy_for_project(project_id)?;
        policy
            .authorize_provider_instance(provider_instance_id)
            .map_err(|error| StoreError::Data(error.to_string()))?;
        let (subscription_key, _) = self
            .usage_subscription_for_provider(environment_id, provider_instance_id)?
            .ok_or_else(|| {
                StoreError::Data("provider instance has no named subscription mapping".to_string())
            })?;
        policy
            .authorize_route(provider_instance_id, &subscription_key)
            .map_err(|error| StoreError::Data(error.to_string()))?;
        Ok((policy, subscription_key))
    }

    /// Persist user settings and the canonical organization tables in one
    /// transaction. Policy changes require a revision bump and provider removal
    /// is blocked while an enabled schedule still names that instance.
    pub fn save_settings_with_organization_policies(
        &self,
        settings: &UserSettings,
        updated_at_ms: u64,
    ) -> Result<(), StoreError> {
        let updated_at_ms = checked_i64(updated_at_ms, "organization policy updated_at_ms")?;
        let settings_json = serde_json::to_string(settings)?;
        let mut organization_ids = BTreeSet::new();
        let mut assigned_projects = BTreeMap::<String, String>::new();
        let mut failover_pool_ids = BTreeSet::new();
        for pool in &settings.failover_pools {
            pool.validate()
                .map_err(|error| StoreError::Data(error.to_string()))?;
            if !failover_pool_ids.insert(pool.id.as_str().to_string()) {
                return Err(StoreError::Data(format!(
                    "duplicate failover pool id: {}",
                    pool.id
                )));
            }
        }
        for policy in &settings.organizations {
            policy
                .validate()
                .map_err(|error| StoreError::Data(error.to_string()))?;
            if !organization_ids.insert(policy.id.as_str().to_string()) {
                return Err(StoreError::Data(format!(
                    "duplicate organization id: {}",
                    policy.id
                )));
            }
            for project_id in &policy.project_ids {
                if let Some(existing) = assigned_projects.insert(
                    project_id.as_str().to_string(),
                    policy.id.as_str().to_string(),
                ) {
                    return Err(StoreError::Data(format!(
                        "project {project_id} is assigned to both {existing} and {}",
                        policy.id
                    )));
                }
            }
            for pool_id in &policy.failover_pool_ids {
                if !failover_pool_ids.contains(pool_id.as_str()) {
                    return Err(StoreError::Data(format!(
                        "organization {} references missing failover pool {pool_id}",
                        policy.id
                    )));
                }
            }
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let provider_subscription_mappings = {
            let mut stmt = tx.prepare(
                "SELECT provider_instance_id, subscription_key
                 FROM provider_subscription_mappings",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<Result<BTreeSet<_>, _>>()?
        };
        let enabled_provider_ids = settings
            .providers
            .iter()
            .filter(|provider| provider.enabled)
            .map(|provider| provider.id.as_str())
            .collect::<BTreeSet<_>>();
        for pool in &settings.failover_pools {
            for member in &pool.members {
                if !enabled_provider_ids.contains(member.provider_instance_id.as_str()) {
                    return Err(StoreError::Data(format!(
                        "failover pool '{}' references a disabled or missing provider instance",
                        pool.display_name
                    )));
                }
                if !provider_subscription_mappings.contains(&(
                    member.provider_instance_id.as_str().to_string(),
                    member.subscription_key.as_str().to_string(),
                )) {
                    return Err(StoreError::Data(format!(
                        "failover pool '{}' member has no exact named subscription mapping",
                        pool.display_name
                    )));
                }
            }
        }
        for policy in &settings.organizations {
            let has_valid_provider = policy.allowed_provider_instances.iter().any(|provider| {
                enabled_provider_ids.contains(provider.as_str())
                    && provider_subscription_mappings.iter().any(
                        |(mapped_provider, subscription_key)| {
                            mapped_provider == provider.as_str()
                                && policy
                                    .allowed_subscriptions
                                    .iter()
                                    .any(|allowed| allowed.as_str() == subscription_key)
                        },
                    )
            });
            if !has_valid_provider {
                return Err(StoreError::Data(format!(
                    "impact review blocked organization '{}' from losing its last valid named provider",
                    policy.display_name
                )));
            }
            for pool_id in &policy.failover_pool_ids {
                let pool = settings
                    .failover_pools
                    .iter()
                    .find(|pool| &pool.id == pool_id)
                    .expect("referenced failover pool was validated");
                for member in &pool.members {
                    policy
                        .authorize_route(&member.provider_instance_id, &member.subscription_key)
                        .map_err(|error| StoreError::Data(error.to_string()))?;
                }
            }
        }
        let existing = {
            let mut stmt = tx.prepare(
                "SELECT id, revision, policy_json FROM organization_policies ORDER BY id ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.map(|row| row.map(|(id, revision, json)| (id, (revision, json))))
                .collect::<Result<BTreeMap<_, _>, _>>()?
        };
        let existing_pools = {
            let mut stmt = tx.prepare(
                "SELECT id, revision, definition_json FROM failover_pools ORDER BY id ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.map(|row| row.map(|(id, revision, json)| (id, (revision, json))))
                .collect::<Result<BTreeMap<_, _>, _>>()?
        };
        let enabled_schedule_routes = {
            let mut stmt = tx.prepare(
                "SELECT definition_json FROM schedules WHERE enabled = 1 ORDER BY id ASC",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut routes = Vec::new();
            for row in rows {
                let schedule: ScheduleRecord = serde_json::from_str(&row?)?;
                if let ScheduleTarget::Session {
                    provider: Some(provider),
                    ..
                } = &schedule.target
                {
                    routes.push((
                        provider.clone(),
                        schedule.project_id.clone(),
                        schedule.name.clone(),
                    ));
                }
            }
            routes
        };

        for policy in &settings.organizations {
            let policy_json = serde_json::to_string(policy)?;
            if let Some((existing_revision, existing_json)) = existing.get(policy.id.as_str()) {
                let revision = checked_i64(policy.revision, "organization policy revision")?;
                if revision < *existing_revision {
                    return Err(StoreError::Data(format!(
                        "organization {} revision {} is older than stored revision {}",
                        policy.id, policy.revision, existing_revision
                    )));
                }
                if revision == *existing_revision && policy_json != *existing_json {
                    return Err(StoreError::Data(format!(
                        "organization {} changed without a revision increment",
                        policy.id
                    )));
                }

                let previous: OrganizationPolicy = serde_json::from_str(existing_json)?;
                let removed = previous
                    .allowed_provider_instances
                    .iter()
                    .filter(|provider| !policy.allowed_provider_instances.contains(provider));
                for provider in removed {
                    if let Some((_, _, schedule_name)) = enabled_schedule_routes.iter().find(
                        |(scheduled_provider, project_id, _)| {
                            scheduled_provider == provider.as_str()
                                && project_id.as_ref().is_none_or(|project_id| {
                                    previous.project_ids.contains(project_id)
                                })
                        },
                    ) {
                        let provider_name = settings
                            .providers
                            .iter()
                            .find(|candidate| &candidate.id == provider)
                            .map(|candidate| candidate.display_name.as_str())
                            .unwrap_or("Unnamed provider instance");
                        return Err(StoreError::Data(format!(
                            "impact review blocked removal of '{provider_name}' because enabled schedule '{schedule_name}' uses it"
                        )));
                    }
                }
            }
        }

        for pool in &settings.failover_pools {
            let pool_json = serde_json::to_string(pool)?;
            if let Some((existing_revision, existing_json)) = existing_pools.get(pool.id.as_str()) {
                let revision = checked_i64(pool.revision, "failover pool revision")?;
                if revision < *existing_revision {
                    return Err(StoreError::Data(format!(
                        "failover pool {} revision {} is older than stored revision {}",
                        pool.id, pool.revision, existing_revision
                    )));
                }
                if revision == *existing_revision && pool_json != *existing_json {
                    return Err(StoreError::Data(format!(
                        "failover pool {} changed without a revision increment",
                        pool.id
                    )));
                }
            }
        }

        for (id, (_, existing_json)) in &existing {
            if !organization_ids.contains(id) {
                let previous: OrganizationPolicy = serde_json::from_str(existing_json)?;
                for provider in previous.allowed_provider_instances {
                    if let Some((_, _, schedule_name)) = enabled_schedule_routes.iter().find(
                        |(scheduled_provider, project_id, _)| {
                            scheduled_provider == provider.as_str()
                                && project_id.as_ref().is_none_or(|project_id| {
                                    previous.project_ids.contains(project_id)
                                })
                        },
                    ) {
                        let provider_name = settings
                            .providers
                            .iter()
                            .find(|candidate| candidate.id == provider)
                            .map(|candidate| candidate.display_name.as_str())
                            .unwrap_or("Unnamed provider instance");
                        return Err(StoreError::Data(format!(
                            "impact review blocked removal of organization '{}' because enabled schedule '{schedule_name}' uses '{provider_name}'",
                            previous.display_name
                        )));
                    }
                }
            }
        }

        tx.execute(
            "INSERT INTO settings (key, value_json) VALUES ('user_settings', ?1)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
            [settings_json],
        )?;
        for pool in &settings.failover_pools {
            let revision = checked_i64(pool.revision, "failover pool revision")?;
            let pool_json = serde_json::to_string(pool)?;
            tx.execute(
                "INSERT INTO failover_pools (
                    id, display_name, revision, definition_json, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                    display_name = excluded.display_name,
                    revision = excluded.revision,
                    definition_json = excluded.definition_json,
                    updated_at_ms = excluded.updated_at_ms",
                params![
                    pool.id.as_str(),
                    pool.display_name,
                    revision,
                    pool_json,
                    updated_at_ms,
                ],
            )?;
        }
        for id in existing_pools.keys() {
            if !failover_pool_ids.contains(id) {
                tx.execute("DELETE FROM failover_pools WHERE id = ?1", [id])?;
            }
        }
        tx.execute("DELETE FROM organization_project_assignments", [])?;
        for policy in &settings.organizations {
            let revision = checked_i64(policy.revision, "organization policy revision")?;
            let policy_json = serde_json::to_string(policy)?;
            tx.execute(
                "INSERT INTO organization_policies (
                    id, display_name, revision, policy_json, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                    display_name = excluded.display_name,
                    revision = excluded.revision,
                    policy_json = excluded.policy_json,
                    updated_at_ms = excluded.updated_at_ms",
                params![
                    policy.id.as_str(),
                    policy.display_name,
                    revision,
                    policy_json,
                    updated_at_ms,
                ],
            )?;
            for project_id in &policy.project_ids {
                tx.execute(
                    "INSERT INTO organization_project_assignments (project_id, organization_id)
                     VALUES (?1, ?2)",
                    params![project_id.as_str(), policy.id.as_str()],
                )?;
            }
        }
        for id in existing.keys() {
            if !organization_ids.contains(id) {
                tx.execute("DELETE FROM organization_policies WHERE id = ?1", [id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    // Normalized usage

    pub fn save_usage_subscription(
        &self,
        subscription: &UsageSubscription,
        updated_at_ms: u64,
    ) -> Result<(), StoreError> {
        if subscription.key.as_str().trim().is_empty()
            || subscription.display_name.trim().is_empty()
            || subscription.provider.as_str().trim().is_empty()
        {
            return Err(StoreError::Data(
                "usage subscription key, provider, and display name must not be empty".to_string(),
            ));
        }
        if subscription
            .masked_account_label
            .as_deref()
            .and_then(|label| label.split_once('@'))
            .is_some_and(|(local, _)| !local.contains('*'))
        {
            return Err(StoreError::Data(
                "usage subscription email labels must be masked".to_string(),
            ));
        }
        let updated_at_ms = checked_i64(updated_at_ms, "usage subscription updated_at_ms")?;
        let conn = self.conn.lock().unwrap();
        let existing_provider = conn
            .query_row(
                "SELECT provider FROM usage_subscriptions WHERE subscription_key = ?1",
                [subscription.key.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if existing_provider
            .as_deref()
            .is_some_and(|provider| provider != subscription.provider.as_str())
        {
            return Err(StoreError::Data(
                "an existing named subscription cannot be reassigned to another provider"
                    .to_string(),
            ));
        }
        conn.execute(
            "INSERT INTO usage_subscriptions (
                subscription_key, provider, display_name, masked_account_label,
                created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(subscription_key) DO UPDATE SET
                display_name = excluded.display_name,
                masked_account_label = excluded.masked_account_label,
                updated_at_ms = excluded.updated_at_ms",
            params![
                subscription.key.as_str(),
                subscription.provider.as_str(),
                subscription.display_name,
                subscription.masked_account_label,
                updated_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn list_usage_subscriptions(&self) -> Result<Vec<UsageSubscription>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT subscription_key, provider, display_name, masked_account_label
             FROM usage_subscriptions
             ORDER BY provider ASC, display_name ASC, subscription_key ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(UsageSubscription {
                key: SubscriptionKey::from(row.get::<_, String>(0)?),
                provider: UsageProvider::from(row.get::<_, String>(1)?),
                display_name: row.get(2)?,
                masked_account_label: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn list_provider_subscription_mappings(
        &self,
    ) -> Result<Vec<UsageProviderSubscriptionMapping>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT environment_id, provider_instance_id, subscription_key
             FROM provider_subscription_mappings
             ORDER BY environment_id ASC, provider_instance_id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(UsageProviderSubscriptionMapping {
                environment_id: EnvironmentId::from(row.get::<_, String>(0)?),
                provider_instance_id: ProviderInstanceId::from(row.get::<_, String>(1)?),
                subscription_key: SubscriptionKey::from(row.get::<_, String>(2)?),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Atomically update the user-facing subscription metadata and the provider
    /// instance that consumes that quota boundary.
    pub fn save_usage_subscription_mapping(
        &self,
        subscription: &UsageSubscription,
        environment_id: &EnvironmentId,
        provider_instance_id: &ProviderInstanceId,
        updated_at_ms: u64,
    ) -> Result<(), StoreError> {
        if subscription.key.as_str().trim().is_empty()
            || subscription.display_name.trim().is_empty()
            || subscription.provider.as_str().trim().is_empty()
        {
            return Err(StoreError::Data(
                "usage subscription key, provider, and display name must not be empty".to_string(),
            ));
        }
        if subscription
            .masked_account_label
            .as_deref()
            .and_then(|label| label.split_once('@'))
            .is_some_and(|(local, _)| !local.contains('*'))
        {
            return Err(StoreError::Data(
                "usage subscription email labels must be masked".to_string(),
            ));
        }
        for (field, value) in [
            ("environment id", environment_id.as_str()),
            ("provider instance id", provider_instance_id.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(StoreError::Data(format!("{field} must not be empty")));
            }
        }
        let updated_at_ms = checked_i64(updated_at_ms, "usage subscription updated_at_ms")?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let existing_provider = tx
            .query_row(
                "SELECT provider FROM usage_subscriptions WHERE subscription_key = ?1",
                [subscription.key.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if existing_provider
            .as_deref()
            .is_some_and(|provider| provider != subscription.provider.as_str())
        {
            return Err(StoreError::Data(
                "an existing named subscription cannot be reassigned to another provider"
                    .to_string(),
            ));
        }
        tx.execute(
            "INSERT INTO usage_subscriptions (
                subscription_key, provider, display_name, masked_account_label,
                created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(subscription_key) DO UPDATE SET
                display_name = excluded.display_name,
                masked_account_label = excluded.masked_account_label,
                updated_at_ms = excluded.updated_at_ms",
            params![
                subscription.key.as_str(),
                subscription.provider.as_str(),
                subscription.display_name,
                subscription.masked_account_label,
                updated_at_ms,
            ],
        )?;
        tx.execute(
            "INSERT INTO provider_subscription_mappings (
                environment_id, provider_instance_id, subscription_key, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(environment_id, provider_instance_id) DO UPDATE SET
                subscription_key = excluded.subscription_key,
                updated_at_ms = excluded.updated_at_ms",
            params![
                environment_id.as_str(),
                provider_instance_id.as_str(),
                subscription.key.as_str(),
                updated_at_ms,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn map_provider_subscription(
        &self,
        environment_id: &EnvironmentId,
        provider_instance_id: &ProviderInstanceId,
        subscription_key: &SubscriptionKey,
        updated_at_ms: u64,
    ) -> Result<(), StoreError> {
        for (field, value) in [
            ("environment id", environment_id.as_str()),
            ("provider instance id", provider_instance_id.as_str()),
            ("subscription key", subscription_key.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(StoreError::Data(format!("{field} must not be empty")));
            }
        }
        let updated_at_ms = checked_i64(updated_at_ms, "provider mapping updated_at_ms")?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO provider_subscription_mappings (
                environment_id, provider_instance_id, subscription_key, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(environment_id, provider_instance_id) DO UPDATE SET
                subscription_key = excluded.subscription_key,
                updated_at_ms = excluded.updated_at_ms",
            params![
                environment_id.as_str(),
                provider_instance_id.as_str(),
                subscription_key.as_str(),
                updated_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn usage_node_store_id(&self) -> Result<NodeStoreId, StoreError> {
        let conn = self.conn.lock().unwrap();
        let value = conn.query_row(
            "SELECT node_store_id FROM usage_store_meta WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )?;
        Ok(NodeStoreId::from(value))
    }

    pub fn usage_subscription_for_provider(
        &self,
        environment_id: &EnvironmentId,
        provider_instance_id: &ProviderInstanceId,
    ) -> Result<Option<(SubscriptionKey, UsageProvider)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT m.subscription_key, s.provider
             FROM provider_subscription_mappings m
             JOIN usage_subscriptions s
               ON s.subscription_key = m.subscription_key
             WHERE m.environment_id = ?1 AND m.provider_instance_id = ?2",
            params![environment_id.as_str(), provider_instance_id.as_str()],
            |row| {
                Ok((
                    SubscriptionKey::from(row.get::<_, String>(0)?),
                    UsageProvider::from(row.get::<_, String>(1)?),
                ))
            },
        )
        .optional()
        .map_err(StoreError::from)
    }

    pub fn usage_subscription_by_key(
        &self,
        subscription_key: &SubscriptionKey,
    ) -> Result<Option<UsageSubscription>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT provider, display_name, masked_account_label
             FROM usage_subscriptions
             WHERE subscription_key = ?1",
            [subscription_key.as_str()],
            |row| {
                Ok(UsageSubscription {
                    key: subscription_key.clone(),
                    provider: UsageProvider::from(row.get::<_, String>(0)?),
                    display_name: row.get(1)?,
                    masked_account_label: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(StoreError::from)
    }

    pub fn get_usage_scanner_cursor(
        &self,
        environment_id: &EnvironmentId,
        provider_instance_id: &ProviderInstanceId,
        source_kind: &UsageSourceKind,
        file_identity: &str,
    ) -> Result<Option<UsageScannerCursor>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT environment_id, provider_instance_id, source_kind, file_identity,
                        node_store_id, canonical_path, committed_offset,
                        committed_prefix_sha256, parser_version, parser_state_json,
                        observed_size, observed_modified_at_ms, updated_at_ms
                 FROM usage_scanner_file_cursors
                 WHERE environment_id = ?1 AND provider_instance_id = ?2
                   AND source_kind = ?3 AND file_identity = ?4",
                params![
                    environment_id.as_str(),
                    provider_instance_id.as_str(),
                    source_kind.as_str(),
                    file_identity
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, i64>(10)?,
                        row.get::<_, Option<i64>>(11)?,
                        row.get::<_, i64>(12)?,
                    ))
                },
            )
            .optional()?;
        row.map(|row| {
            Ok(UsageScannerCursor {
                environment_id: EnvironmentId::from(row.0),
                provider_instance_id: ProviderInstanceId::from(row.1),
                source_kind: UsageSourceKind::from(row.2),
                file_identity: row.3,
                node_store_id: NodeStoreId::from(row.4),
                canonical_path: row.5,
                committed_offset: checked_u64(
                    row.6,
                    "usage_scanner_file_cursors.committed_offset",
                )?,
                committed_prefix_sha256: row.7,
                parser_version: checked_u32(row.8, "usage_scanner_file_cursors.parser_version")?,
                parser_state_json: row.9,
                observed_size: checked_u64(row.10, "usage_scanner_file_cursors.observed_size")?,
                observed_modified_at_ms: row
                    .11
                    .map(|value| {
                        checked_u64(value, "usage_scanner_file_cursors.observed_modified_at_ms")
                    })
                    .transpose()?,
                updated_at_ms: checked_u64(row.12, "usage_scanner_file_cursors.updated_at_ms")?,
            })
        })
        .transpose()
    }

    pub fn record_usage_batch(
        &self,
        environment_id: &EnvironmentId,
        observations: &[UsageObservation],
        cursor: Option<&UsageScannerCursor>,
    ) -> Result<UsageBatchResult, StoreError> {
        self.record_usage_transaction(environment_id, observations, cursor, &[], None, None)
    }

    pub fn record_usage_scan_batch(
        &self,
        environment_id: &EnvironmentId,
        observations: &[UsageObservation],
        cursor: Option<&UsageScannerCursor>,
        quarantines: &[UsageQuarantineRecord],
        diagnostic: Option<&UsageScannerDiagnostic>,
    ) -> Result<UsageBatchResult, StoreError> {
        self.record_usage_transaction(
            environment_id,
            observations,
            cursor,
            quarantines,
            diagnostic,
            None,
        )
    }

    fn record_usage_transaction(
        &self,
        environment_id: &EnvironmentId,
        observations: &[UsageObservation],
        cursor: Option<&UsageScannerCursor>,
        quarantines: &[UsageQuarantineRecord],
        diagnostic: Option<&UsageScannerDiagnostic>,
        fail_after_observations: Option<usize>,
    ) -> Result<UsageBatchResult, StoreError> {
        if environment_id.as_str().trim().is_empty() {
            return Err(StoreError::Data(
                "authenticated environment id must not be empty".to_string(),
            ));
        }
        if let Some(cursor) = cursor {
            validate_usage_cursor(cursor)?;
            if &cursor.environment_id != environment_id {
                return Err(StoreError::Data(
                    "usage cursor environment does not match authenticated environment".to_string(),
                ));
            }
        }
        for observation in observations {
            observation
                .validate()
                .map_err(|error| StoreError::Data(error.to_string()))?;
            if cursor.is_some_and(|cursor| {
                cursor.provider_instance_id != observation.provider_instance_id
                    || cursor.source_kind != observation.source_kind
            }) {
                return Err(StoreError::Data(
                    "usage observation does not match its scanner cursor scope".to_string(),
                ));
            }
        }
        for quarantine in quarantines {
            validate_usage_quarantine(quarantine)?;
            if &quarantine.environment_id != environment_id {
                return Err(StoreError::Data(
                    "usage quarantine environment does not match authenticated environment"
                        .to_string(),
                ));
            }
            if cursor.is_some_and(|cursor| {
                cursor.provider_instance_id != quarantine.provider_instance_id
                    || cursor.source_kind != quarantine.source_kind
            }) {
                return Err(StoreError::Data(
                    "usage quarantine does not match its scanner cursor scope".to_string(),
                ));
            }
        }
        if let Some(diagnostic) = diagnostic {
            validate_usage_diagnostic(diagnostic)?;
            if &diagnostic.environment_id != environment_id {
                return Err(StoreError::Data(
                    "usage diagnostic environment does not match authenticated environment"
                        .to_string(),
                ));
            }
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if let Some(cursor) = cursor {
            let node_store_id = tx.query_row(
                "SELECT node_store_id FROM usage_store_meta WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0),
            )?;
            if cursor.node_store_id.as_str() != node_store_id {
                return Err(StoreError::Data(
                    "usage cursor node store identity changed".to_string(),
                ));
            }
        }
        let mut result = UsageBatchResult::default();
        let created_at_ms = checked_i64(store_now_ms(), "usage_observations.created_at_ms")?;

        for (index, observation) in observations.iter().enumerate() {
            let mut pending_reconciliation_mismatches = Vec::new();
            let mapped_subscription = tx
                .query_row(
                    "SELECT m.subscription_key, s.provider
                     FROM provider_subscription_mappings m
                     JOIN usage_subscriptions s
                       ON s.subscription_key = m.subscription_key
                     WHERE m.environment_id = ?1 AND m.provider_instance_id = ?2",
                    params![
                        environment_id.as_str(),
                        observation.provider_instance_id.as_str()
                    ],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            if mapped_subscription.as_ref().map(|value| value.0.as_str())
                != Some(observation.subscription_key.as_str())
                || mapped_subscription.as_ref().map(|value| value.1.as_str())
                    != Some(observation.provider.as_str())
            {
                return Err(StoreError::Data(format!(
                    "provider instance {} is not mapped to provider {} and subscription {} in environment {}",
                    observation.provider_instance_id,
                    observation.provider.as_str(),
                    observation.subscription_key,
                    environment_id
                )));
            }

            let reconciliation_key = usage_reconciliation_key(observation)?;
            if let Some(reconciliation_key) = reconciliation_key.as_deref() {
                let existing =
                    load_usage_observations_by_exact_identity(&tx, environment_id, observation)?;
                if let Some(same_id) = existing
                    .iter()
                    .find(|existing| existing.id == observation.id)
                {
                    if !usage_measurement_compatible(same_id, observation) {
                        return Err(StoreError::Data(format!(
                            "usage observation id collision for {}",
                            observation.id
                        )));
                    }
                    if let Some(outbox_seq) = enrich_usage_observation_cost_if_safe(
                        &tx,
                        environment_id,
                        same_id,
                        observation,
                        created_at_ms,
                    )? {
                        result.reconciled = checked_increment(
                            result.reconciled,
                            "usage reconciliation count overflow",
                        )?;
                        result.last_outbox_seq = result.last_outbox_seq.max(outbox_seq);
                    } else {
                        result.duplicates =
                            checked_increment(result.duplicates, "usage duplicate count overflow")?;
                    }
                    continue;
                }

                if let Some(compatible) = existing
                    .iter()
                    .find(|existing| usage_measurement_compatible(existing, observation))
                {
                    if compatible.source_kind == UsageSourceKind::ManagedTurn
                        && observation.source_kind != UsageSourceKind::ManagedTurn
                    {
                        if let Some(outbox_seq) = enrich_usage_observation_cost_if_safe(
                            &tx,
                            environment_id,
                            compatible,
                            observation,
                            created_at_ms,
                        )? {
                            result.last_outbox_seq = result.last_outbox_seq.max(outbox_seq);
                        }
                        result.reconciled = checked_increment(
                            result.reconciled,
                            "usage reconciliation count overflow",
                        )?;
                        continue;
                    }
                    if observation.source_kind == UsageSourceKind::ManagedTurn
                        && compatible.source_kind != UsageSourceKind::ManagedTurn
                    {
                        let mut promoted = observation.clone();
                        if promoted.provider_reported_micro_usd.is_none()
                            && compatible.provider_reported_micro_usd.is_some()
                            && !has_separate_managed_cost(&tx, environment_id, &promoted)?
                        {
                            promoted.provider_reported_micro_usd =
                                compatible.provider_reported_micro_usd;
                        }
                        let outbox_seq = promote_usage_observation_to_managed(
                            &tx,
                            environment_id,
                            &compatible.id,
                            &promoted,
                            reconciliation_key,
                            created_at_ms,
                        )?;
                        result.reconciled = checked_increment(
                            result.reconciled,
                            "usage reconciliation count overflow",
                        )?;
                        result.last_outbox_seq = result.last_outbox_seq.max(outbox_seq);
                        continue;
                    }
                    if let Some(outbox_seq) = enrich_usage_observation_cost_if_safe(
                        &tx,
                        environment_id,
                        compatible,
                        observation,
                        created_at_ms,
                    )? {
                        result.last_outbox_seq = result.last_outbox_seq.max(outbox_seq);
                    }
                    result.reconciled = checked_increment(
                        result.reconciled,
                        "usage reconciliation count overflow",
                    )?;
                    continue;
                }

                if !existing.is_empty() {
                    pending_reconciliation_mismatches.extend(
                        existing.iter().filter_map(|existing| {
                            managed_transcript_mismatch(existing, observation)
                        }),
                    );
                    result.reconciliation_mismatches = checked_increment(
                        result.reconciliation_mismatches,
                        "usage reconciliation mismatch count overflow",
                    )?;
                }
            }

            let other_tokens_json = serde_json::to_string(&observation.tokens.other)?;
            let unknown_fields_json = serde_json::to_string(&observation.unknown_fields)?;
            let inserted = tx.execute(
                "INSERT INTO usage_observations (
                    environment_id, observation_id, source_kind, parser_version,
                    provider_native_session_id, provider_native_turn_id, occurred_at_ms,
                    provider, provider_instance_id, subscription_key, model,
                    input_tokens, cached_input_tokens, cache_write_tokens, output_tokens,
                    reasoning_tokens, other_tokens_json, unknown_fields_json,
                    provider_reported_micro_usd, source_record_count, thread_id,
                    turn_id, agent_id, schedule_id, fact_kind, created_at_ms,
                    reconciliation_key
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23,
                    ?24, ?25, ?26, ?27
                 ) ON CONFLICT(environment_id, observation_id) DO NOTHING",
                params![
                    environment_id.as_str(),
                    observation.id.as_str(),
                    observation.source_kind.as_str(),
                    i64::from(observation.parser_version),
                    observation.provider_native_session_id,
                    observation.provider_native_turn_id,
                    optional_checked_i64(observation.occurred_at_ms, "occurred_at_ms")?,
                    observation.provider.as_str(),
                    observation.provider_instance_id.as_str(),
                    observation.subscription_key.as_str(),
                    observation.model,
                    checked_i64(observation.tokens.input, "input_tokens")?,
                    checked_i64(observation.tokens.cached_input, "cached_input_tokens")?,
                    checked_i64(observation.tokens.cache_write, "cache_write_tokens")?,
                    checked_i64(observation.tokens.output, "output_tokens")?,
                    checked_i64(observation.tokens.reasoning, "reasoning_tokens")?,
                    other_tokens_json,
                    unknown_fields_json,
                    optional_checked_i64(
                        observation.provider_reported_micro_usd,
                        "provider_reported_micro_usd"
                    )?,
                    checked_i64(observation.source_record_count, "source_record_count")?,
                    observation.thread_id.as_ref().map(ThreadId::as_str),
                    observation.turn_id.as_ref().map(TurnId::as_str),
                    observation.agent_id.as_ref().map(|id| id.as_str()),
                    observation.schedule_id.as_ref().map(ScheduleId::as_str),
                    observation.fact_kind.as_str(),
                    created_at_ms,
                    reconciliation_key,
                ],
            )?;
            if inserted == 0 {
                let existing = load_usage_observation(&tx, environment_id, &observation.id)?
                    .ok_or_else(|| {
                        StoreError::Data(
                            "usage observation conflict did not resolve to a stored fact"
                                .to_string(),
                        )
                    })?;
                if !usage_measurement_compatible(&existing, observation) {
                    return Err(StoreError::Data(format!(
                        "usage observation id collision for {}",
                        observation.id
                    )));
                }
                if let Some(outbox_seq) = enrich_usage_observation_cost_if_safe(
                    &tx,
                    environment_id,
                    &existing,
                    observation,
                    created_at_ms,
                )? {
                    result.reconciled = checked_increment(
                        result.reconciled,
                        "usage reconciliation count overflow",
                    )?;
                    result.last_outbox_seq = result.last_outbox_seq.max(outbox_seq);
                } else {
                    result.duplicates =
                        checked_increment(result.duplicates, "usage duplicate count overflow")?;
                }
                continue;
            }

            for mismatch in &pending_reconciliation_mismatches {
                tx.execute(
                    "INSERT INTO usage_reconciliation_mismatches (
                        environment_id, managed_observation_id,
                        transcript_observation_id, provider_instance_id,
                        source_kind, detected_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(
                        environment_id,
                        managed_observation_id,
                        transcript_observation_id
                     ) DO NOTHING",
                    params![
                        environment_id.as_str(),
                        mismatch.managed_observation_id.as_str(),
                        mismatch.transcript_observation_id.as_str(),
                        mismatch.provider_instance_id.as_str(),
                        mismatch.source_kind.as_str(),
                        created_at_ms,
                    ],
                )?;
            }

            tx.execute(
                "INSERT INTO usage_outbox (
                    environment_id, observation_id, enqueued_at_ms, payload_bytes
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    environment_id.as_str(),
                    observation.id.as_str(),
                    created_at_ms,
                    usage_observation_payload_bytes(observation)?,
                ],
            )?;
            result.inserted = checked_increment(result.inserted, "usage insert count overflow")?;
            result.last_outbox_seq = checked_u64(tx.last_insert_rowid(), "usage_outbox.seq")?;
            tx.execute(
                "UPDATE usage_observations
                 SET source_node_store_id = (
                         SELECT node_store_id FROM usage_store_meta WHERE singleton = 1
                     ),
                     source_outbox_seq = ?1
                 WHERE environment_id = ?2 AND observation_id = ?3",
                params![
                    checked_i64(result.last_outbox_seq, "usage_outbox.seq")?,
                    environment_id.as_str(),
                    observation.id.as_str(),
                ],
            )?;

            if fail_after_observations == Some(index + 1) {
                return Err(StoreError::Data(
                    "injected usage transaction failure".to_string(),
                ));
            }
        }

        for quarantine in quarantines {
            let inserted = tx.execute(
                "INSERT INTO usage_quarantine (
                    legacy_bucket_id, source_kind, reason_code, reason_detail,
                    safe_excerpt_sha256, created_at_ms, environment_id,
                    provider_instance_id, file_identity_sha256, record_offset,
                    parser_version
                 ) VALUES (
                    NULL, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10
                 ) ON CONFLICT DO NOTHING",
                params![
                    quarantine.source_kind.as_str(),
                    quarantine.reason_code,
                    quarantine.reason_detail,
                    quarantine.safe_excerpt_sha256,
                    checked_i64(quarantine.created_at_ms, "quarantine created_at_ms")?,
                    quarantine.environment_id.as_str(),
                    quarantine.provider_instance_id.as_str(),
                    quarantine.file_identity_sha256,
                    checked_i64(quarantine.record_offset, "quarantine record_offset")?,
                    i64::from(quarantine.parser_version),
                ],
            )?;
            if inserted > 0 {
                result.quarantined =
                    checked_increment(result.quarantined, "usage quarantine count overflow")?;
            }
        }

        let quarantine_scopes = quarantines
            .iter()
            .map(|quarantine| {
                (
                    quarantine.environment_id.clone(),
                    quarantine.provider_instance_id.clone(),
                    quarantine.source_kind.clone(),
                )
            })
            .collect::<BTreeSet<_>>();
        for (environment_id, provider_instance_id, source_kind) in quarantine_scopes {
            tx.execute(
                "DELETE FROM usage_quarantine
                 WHERE id IN (
                    SELECT id FROM usage_quarantine
                    WHERE legacy_bucket_id IS NULL
                      AND environment_id = ?1
                      AND provider_instance_id = ?2
                      AND source_kind = ?3
                    ORDER BY created_at_ms DESC, id DESC
                    LIMIT -1 OFFSET ?4
                 )",
                params![
                    environment_id.as_str(),
                    provider_instance_id.as_str(),
                    source_kind.as_str(),
                    MAX_RETAINED_SCANNER_QUARANTINES_PER_SOURCE,
                ],
            )?;
        }

        if let Some(cursor) = cursor {
            tx.execute(
                "INSERT INTO usage_scanner_file_cursors (
                    environment_id, provider_instance_id, source_kind, node_store_id,
                    file_identity, canonical_path, committed_offset,
                    committed_prefix_sha256, parser_version, parser_state_json,
                    observed_size, observed_modified_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(
                    environment_id, provider_instance_id, source_kind, file_identity
                 ) DO UPDATE SET
                    node_store_id = excluded.node_store_id,
                    canonical_path = excluded.canonical_path,
                    committed_offset = excluded.committed_offset,
                    committed_prefix_sha256 = excluded.committed_prefix_sha256,
                    parser_version = excluded.parser_version,
                    parser_state_json = excluded.parser_state_json,
                    observed_size = excluded.observed_size,
                    observed_modified_at_ms = excluded.observed_modified_at_ms,
                    updated_at_ms = excluded.updated_at_ms",
                params![
                    cursor.environment_id.as_str(),
                    cursor.provider_instance_id.as_str(),
                    cursor.source_kind.as_str(),
                    cursor.node_store_id.as_str(),
                    cursor.file_identity,
                    cursor.canonical_path,
                    checked_i64(cursor.committed_offset, "committed_offset")?,
                    cursor.committed_prefix_sha256,
                    i64::from(cursor.parser_version),
                    cursor.parser_state_json,
                    checked_i64(cursor.observed_size, "observed_size")?,
                    optional_checked_i64(
                        cursor.observed_modified_at_ms,
                        "observed_modified_at_ms"
                    )?,
                    checked_i64(cursor.updated_at_ms, "cursor updated_at_ms")?,
                ],
            )?;
        }

        if let Some(diagnostic) = diagnostic {
            let quarantined_count = checked_u64(
                tx.query_row(
                    "SELECT COUNT(*) FROM usage_quarantine
                     WHERE environment_id = ?1 AND provider_instance_id = ?2
                       AND source_kind = ?3",
                    params![
                        diagnostic.environment_id.as_str(),
                        diagnostic.provider_instance_id.as_str(),
                        diagnostic.source_kind.as_str(),
                    ],
                    |row| row.get(0),
                )?,
                "scanner quarantine count",
            )?;
            let mut diagnostic = diagnostic.clone();
            diagnostic.quarantined_count = quarantined_count;
            if result.reconciliation_mismatches > 0 {
                diagnostic.coverage_state = UsageSourceCoverageState::Partial;
                diagnostic.last_safe_error_code = Some("managed_transcript_mismatch".to_string());
            }
            upsert_usage_diagnostic(&tx, &diagnostic)?;
        }
        tx.commit()?;
        Ok(result)
    }

    pub fn backfill_managed_turn_usage(
        &self,
        environment_id: &EnvironmentId,
        limit: u32,
    ) -> Result<ManagedUsageBackfillResult, StoreError> {
        if !(1..=10_000).contains(&limit) {
            return Err(StoreError::Data(
                "managed usage backfill limit must be between 1 and 10000".to_string(),
            ));
        }
        let candidates = {
            let conn = self.conn.lock().unwrap();
            let completed_attempt = serde_json::to_string(&TurnAttemptStatus::Completed)?;
            let mut stmt = conn.prepare(
                "SELECT t.id, t.thread_id, t.environment_id,
                        COALESCE(completed.provider_instance_id, t.provider_instance_id),
                        t.ended_at_ms, t.usage_json, completed.execution_json,
                        a.provider_kind, COALESCE(completed.model, a.model),
                        th.agent_json, th.origin_json
                 FROM turns t
                 JOIN provider_execution_audit a ON a.turn_id = t.id
                 JOIN threads th ON th.id = t.thread_id
                 LEFT JOIN turn_attempts completed ON completed.id = (
                    SELECT candidate.id FROM turn_attempts candidate
                    WHERE candidate.turn_id = t.id AND candidate.status = ?2
                    ORDER BY candidate.ordinal DESC LIMIT 1
                 )
                 LEFT JOIN managed_usage_backfill_state b
                   ON b.environment_id = t.environment_id
                  AND b.turn_id = t.id
                  AND b.usage_json = t.usage_json
                 WHERE t.environment_id = ?1
                   AND t.usage_json IS NOT NULL
                   AND (
                        b.turn_id IS NULL OR
                        (
                            SELECT COUNT(*)
                              FROM managed_usage_backfill_facts expected
                             WHERE expected.environment_id = b.environment_id
                               AND expected.turn_id = b.turn_id
                        ) <> b.expected_fact_count OR
                        EXISTS (
                            SELECT 1
                              FROM managed_usage_backfill_facts expected
                             WHERE expected.environment_id = b.environment_id
                               AND expected.turn_id = b.turn_id
                               AND NOT EXISTS (
                                   SELECT 1
                                     FROM usage_observations managed
                                    WHERE managed.environment_id = expected.environment_id
                                      AND managed.turn_id = expected.turn_id
                                      AND managed.source_kind = 'managed_turn'
                                      AND managed.fact_kind = expected.fact_kind
                                      AND COALESCE(managed.provider_native_turn_id, '') =
                                          expected.provider_native_turn_id
                               )
                        )
                   )
                 ORDER BY t.started_at_ms, t.id
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(
                params![environment_id.as_str(), completed_attempt, i64::from(limit)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, String>(10)?,
                    ))
                },
            )?;
            let mut candidates = Vec::new();
            for row in rows {
                let row = row?;
                let attempt_execution = row
                    .6
                    .as_deref()
                    .map(serde_json::from_str::<AttemptExecutionEvidence>)
                    .transpose()?;
                let provider_kind = attempt_execution
                    .map(|execution| execution.provider_kind)
                    .unwrap_or(serde_json::from_str(&row.7)?);
                let agent_id = row
                    .9
                    .as_deref()
                    .map(serde_json::from_str::<pandamux_core::AgentInstanceRef>)
                    .transpose()?
                    .map(|agent| agent.agent_id);
                let origin: ThreadOrigin = serde_json::from_str(&row.10)?;
                let schedule_id = match origin {
                    ThreadOrigin::Schedule { schedule_id, .. } => Some(schedule_id),
                    _ => None,
                };
                let usage_json = row.5;
                candidates.push(ManagedUsageCandidate {
                    turn_id: TurnId::from(row.0),
                    thread_id: ThreadId::from(row.1),
                    environment_id: EnvironmentId::from(row.2),
                    provider_instance_id: ProviderInstanceId::from(row.3),
                    ended_at_ms: row
                        .4
                        .map(|value| checked_u64(value, "managed usage ended_at_ms"))
                        .transpose()?,
                    usage: serde_json::from_str(&usage_json)?,
                    usage_json,
                    provider_kind,
                    model: row.8,
                    agent_id,
                    schedule_id,
                });
            }
            candidates
        };

        let mut result = ManagedUsageBackfillResult::default();
        for candidate in candidates {
            let provider = UsageProvider::from(candidate.provider_kind);
            let Some((subscription_key, mapped_provider)) = self.usage_subscription_for_provider(
                &candidate.environment_id,
                &candidate.provider_instance_id,
            )?
            else {
                result.unmapped =
                    checked_increment(result.unmapped, "managed usage unmapped count overflow")?;
                continue;
            };
            if mapped_provider != provider {
                result.unmapped =
                    checked_increment(result.unmapped, "managed usage unmapped count overflow")?;
                continue;
            }
            let Some(observations) =
                managed_usage_observations(&candidate, provider, subscription_key)
            else {
                result.invalid =
                    checked_increment(result.invalid, "managed usage invalid count overflow")?;
                continue;
            };
            let expected_fact_count = u64::try_from(observations.len()).map_err(|_| {
                StoreError::Data("managed usage expected fact count overflow".to_string())
            })?;
            let batch = self.record_usage_batch(&candidate.environment_id, &observations, None)?;
            merge_usage_batch(&mut result.batch, &batch)?;
            let mut conn = self.conn.lock().unwrap();
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO managed_usage_backfill_state (
                    environment_id, turn_id, usage_json, expected_fact_count,
                    completed_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(environment_id, turn_id) DO UPDATE SET
                    usage_json = excluded.usage_json,
                    expected_fact_count = excluded.expected_fact_count,
                    completed_at_ms = excluded.completed_at_ms",
                params![
                    candidate.environment_id.as_str(),
                    candidate.turn_id.as_str(),
                    &candidate.usage_json,
                    checked_i64(expected_fact_count, "managed usage expected fact count")?,
                    checked_i64(store_now_ms(), "managed usage backfill completion time")?,
                ],
            )?;
            tx.execute(
                "DELETE FROM managed_usage_backfill_facts
                  WHERE environment_id = ?1 AND turn_id = ?2",
                params![
                    candidate.environment_id.as_str(),
                    candidate.turn_id.as_str(),
                ],
            )?;
            for observation in &observations {
                tx.execute(
                    "INSERT INTO managed_usage_backfill_facts (
                        environment_id, turn_id, fact_kind,
                        provider_native_turn_id
                     ) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        candidate.environment_id.as_str(),
                        candidate.turn_id.as_str(),
                        observation.fact_kind.as_str(),
                        observation.provider_native_turn_id.as_deref().unwrap_or(""),
                    ],
                )?;
            }
            tx.commit()?;
        }
        Ok(result)
    }

    pub fn export_usage(
        &self,
        params: &UsageExportParams,
    ) -> Result<UsageExportResult, StoreError> {
        params.validate().map_err(StoreError::Data)?;
        let after_seq = checked_i64(params.after_seq, "usage export after_seq")?;
        let query_limit = i64::from(params.limit) + 1;
        let (retained_from_seq, latest_seq) = self.usage_outbox_window()?;
        let node_coverage = self.usage_node_coverage_snapshot()?;
        let conn = self.conn.lock().unwrap();
        let node_store_id = conn.query_row(
            "SELECT node_store_id FROM usage_store_meta WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )?;
        let coverage_gap = if params.after_seq < latest_seq
            && params.after_seq.saturating_add(1) < retained_from_seq
        {
            Some(pandamux_protocol::UsageCoverageGap {
                requested_after_seq: params.after_seq,
                retained_from_seq,
                latest_seq,
            })
        } else {
            None
        };
        if coverage_gap.is_some() {
            return Ok(UsageExportResult {
                node_store_id: NodeStoreId::from(node_store_id),
                items: Vec::new(),
                next_after_seq: params.after_seq,
                has_more: false,
                retained_from_seq,
                latest_seq,
                coverage_gap,
                node_coverage: Some(node_coverage),
            });
        }
        let mut stmt = conn.prepare(
            "SELECT x.seq,
                    o.observation_id, o.source_kind, o.parser_version,
                    o.provider_native_session_id, o.provider_native_turn_id,
                    o.occurred_at_ms, o.provider, o.provider_instance_id,
                    o.subscription_key, o.model, o.input_tokens,
                    o.cached_input_tokens, o.cache_write_tokens, o.output_tokens,
                    o.reasoning_tokens, o.other_tokens_json, o.unknown_fields_json,
                    o.provider_reported_micro_usd, o.source_record_count,
                    o.thread_id, o.turn_id, o.agent_id, o.schedule_id, o.fact_kind
             FROM usage_outbox x
             JOIN usage_observations o
               ON o.environment_id = x.environment_id
              AND o.observation_id = x.observation_id
             WHERE x.seq > ?1
             ORDER BY x.seq
             LIMIT ?2",
        )?;
        let mut rows = stmt.query(params![after_seq, query_limit])?;
        let mut items = Vec::new();
        while let Some(row) = rows.next()? {
            items.push(UsageExportItem {
                seq: checked_u64(row.get(0)?, "usage_outbox.seq")?,
                observation: decode_usage_observation(row, 1)?,
            });
        }
        let has_more = items.len() > params.limit as usize;
        if has_more {
            items.pop();
        }
        let next_after_seq = items
            .last()
            .map(|item| item.seq)
            .unwrap_or(params.after_seq);
        Ok(UsageExportResult {
            node_store_id: NodeStoreId::from(node_store_id),
            items,
            next_after_seq,
            has_more,
            retained_from_seq,
            latest_seq,
            coverage_gap: None,
            node_coverage: Some(node_coverage),
        })
    }

    pub fn acknowledge_usage(&self, through_seq: u64) -> Result<u64, StoreError> {
        Ok(self
            .acknowledge_and_compact_usage(through_seq, UsageOutboxRetention::default())?
            .acknowledged)
    }

    pub fn query_usage(&self, params: &UsageQueryParams) -> Result<UsageQueryResult, StoreError> {
        params.validate().map_err(StoreError::Data)?;
        let mut sql = String::from(
            "SELECT environment_id,
                    observation_id, source_kind, parser_version,
                    provider_native_session_id, provider_native_turn_id,
                    occurred_at_ms, provider, provider_instance_id, subscription_key,
                    model, input_tokens, cached_input_tokens, cache_write_tokens,
                    output_tokens, reasoning_tokens, other_tokens_json,
                    unknown_fields_json, provider_reported_micro_usd, source_record_count,
                    thread_id, turn_id, agent_id, schedule_id, fact_kind
             FROM usage_observations WHERE 1 = 1",
        );
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(value) = params.from_ms {
            sql.push_str(" AND occurred_at_ms >= ?");
            values.push(Box::new(checked_i64(value, "usage query from_ms")?));
        }
        if let Some(value) = params.to_ms {
            sql.push_str(" AND occurred_at_ms <= ?");
            values.push(Box::new(checked_i64(value, "usage query to_ms")?));
        }
        push_usage_filter(
            &mut sql,
            &mut values,
            "environment_id",
            params.environment_id.as_ref().map(EnvironmentId::as_str),
        );
        push_usage_filter(
            &mut sql,
            &mut values,
            "provider",
            params.provider.as_ref().map(UsageProvider::as_str),
        );
        push_usage_filter(
            &mut sql,
            &mut values,
            "provider_instance_id",
            params
                .provider_instance_id
                .as_ref()
                .map(ProviderInstanceId::as_str),
        );
        push_usage_filter(
            &mut sql,
            &mut values,
            "subscription_key",
            params
                .subscription_key
                .as_ref()
                .map(SubscriptionKey::as_str),
        );
        push_usage_filter(
            &mut sql,
            &mut values,
            "agent_id",
            params.agent_id.as_ref().map(|id| id.as_str()),
        );
        push_usage_filter(
            &mut sql,
            &mut values,
            "schedule_id",
            params.schedule_id.as_ref().map(ScheduleId::as_str),
        );
        push_usage_filter(&mut sql, &mut values, "model", params.model.as_deref());
        push_usage_filter(
            &mut sql,
            &mut values,
            "source_kind",
            params.source_kind.as_ref().map(UsageSourceKind::as_str),
        );
        if let Some(cursor) = &params.after {
            let occurred_at_ms = cursor
                .occurred_at_ms
                .map(|value| checked_i64(value, "usage query cursor occurred_at_ms"))
                .transpose()?
                .unwrap_or(-1);
            sql.push_str(
                " AND (
                    COALESCE(occurred_at_ms, -1) > ? OR
                    (COALESCE(occurred_at_ms, -1) = ? AND environment_id > ?) OR
                    (COALESCE(occurred_at_ms, -1) = ? AND environment_id = ?
                        AND observation_id > ?)
                  )",
            );
            values.push(Box::new(occurred_at_ms));
            values.push(Box::new(occurred_at_ms));
            values.push(Box::new(cursor.environment_id.as_str().to_string()));
            values.push(Box::new(occurred_at_ms));
            values.push(Box::new(cursor.environment_id.as_str().to_string()));
            values.push(Box::new(cursor.observation_id.as_str().to_string()));
        }
        sql.push_str(
            " ORDER BY COALESCE(occurred_at_ms, -1), environment_id, observation_id LIMIT ?",
        );
        values.push(Box::new(i64::from(params.limit) + 1));

        let conn = self.conn.lock().unwrap();
        let references: Vec<&dyn rusqlite::ToSql> =
            values.iter().map(|value| value.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(references.as_slice())?;
        let mut items = Vec::new();
        while let Some(row) = rows.next()? {
            items.push(UsageQueryItem {
                environment_id: EnvironmentId::from(row.get::<_, String>(0)?),
                observation: decode_usage_observation(row, 1)?,
            });
        }
        let has_more = items.len() > params.limit as usize;
        if has_more {
            items.pop();
        }
        let next = items.last().map(|item| UsageQueryCursor {
            occurred_at_ms: item.observation.occurred_at_ms,
            environment_id: item.environment_id.clone(),
            observation_id: item.observation.id.clone(),
        });
        let aggregate = usage_query_phase4c::load_usage_aggregate(&conn, params)?;
        let scanner_diagnostics = load_usage_scanner_diagnostics(&conn, params)?;
        let latest_source_at_ms = aggregate.latest_source_at_ms.max(
            scanner_diagnostics
                .iter()
                .filter_map(|diagnostic| diagnostic.latest_source_at_ms)
                .max(),
        );
        Ok(UsageQueryResult {
            items,
            next,
            has_more,
            aggregate: aggregate.aggregate,
            coverage: pandamux_protocol::UsageCoverage {
                quarantined_count: aggregate.quarantined_count,
                included_environments: aggregate.included_environments,
                observation_sources: aggregate.observation_sources,
                latest_source_at_ms,
                unknown_price_tokens: aggregate.unknown_price_tokens,
                priceable_tokens: aggregate.priceable_tokens,
                unknown_price_token_millionths: aggregate.unknown_price_token_millionths,
                environment_sync: aggregate.environment_sync,
                scanner_diagnostics,
            },
        })
    }

    pub fn save_capacity_snapshot(&self, snapshot: &CapacitySnapshot) -> Result<(), StoreError> {
        let mut stored = snapshot.clone();
        if stored.environment_id.is_none() {
            stored.environment_id = Some(EnvironmentId::new("env-legacy"));
        }
        stored.validate().map_err(StoreError::Data)?;
        let confidence = match stored.confidence {
            CapacityConfidence::Reported => "reported",
            CapacityConfidence::Estimated => "estimated",
            CapacityConfidence::Unknown => "unknown",
        };
        let evidence_scope = match stored.evidence_scope {
            CapacityEvidenceScope::Local => "local",
            CapacityEvidenceScope::Hub => "hub",
        };
        let environment_id = stored
            .environment_id
            .as_ref()
            .expect("capacity environment was normalized");
        let conn = self.conn.lock().unwrap();
        let inserted = conn.execute(
            "INSERT INTO capacity_snapshots (
                subscription_key, window_start_ms, window_end_ms, used_millionths,
                remaining_millionths, reset_at_ms, source, confidence,
                provider_version, as_of_ms, detail_json, schema_version,
                environment_id, provider_instance_id, window_name, evidence_scope,
                hard_limit, max_age_ms, received_at_ms
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, '{}', ?11,
                ?12, ?13, ?14, ?15, ?16, ?17, ?18
             )
             ON CONFLICT DO NOTHING",
            params![
                stored.subscription_key.as_str(),
                optional_checked_i64(stored.window_start_ms, "capacity window_start_ms")?,
                optional_checked_i64(stored.window_end_ms, "capacity window_end_ms")?,
                stored.used_millionths.map(i64::from),
                stored.remaining_millionths.map(i64::from),
                optional_checked_i64(stored.reset_at_ms, "capacity reset_at_ms")?,
                stored.source,
                confidence,
                stored.provider_version,
                checked_i64(stored.as_of_ms, "capacity as_of_ms")?,
                i64::from(stored.schema_version),
                environment_id.as_str(),
                stored
                    .provider_instance_id
                    .as_ref()
                    .map(ProviderInstanceId::as_str),
                stored.window_name,
                evidence_scope,
                stored.hard_limit,
                checked_i64(stored.max_age_ms, "capacity max_age_ms")?,
                checked_i64(stored.as_of_ms, "capacity received_at_ms")?,
            ],
        )?;
        if inserted == 0 {
            let mut stmt = conn.prepare(
                "SELECT subscription_key, window_start_ms, window_end_ms,
                        used_millionths, remaining_millionths, reset_at_ms, source,
                        confidence, provider_version, as_of_ms, schema_version,
                        environment_id, provider_instance_id, window_name, evidence_scope,
                        hard_limit, max_age_ms
                 FROM capacity_snapshots
                 WHERE subscription_key = ?1
                   AND environment_id = ?2
                   AND COALESCE(provider_instance_id, '') = COALESCE(?3, '')
                   AND COALESCE(window_name, '') = COALESCE(?4, '')
                   AND (window_name IS NOT NULL OR
                        COALESCE(window_start_ms, -1) = COALESCE(?5, -1))
                   AND (window_name IS NOT NULL OR
                        COALESCE(window_end_ms, -1) = COALESCE(?6, -1))
                   AND evidence_scope = ?7
                   AND source = ?8
                   AND as_of_ms = ?9",
            )?;
            let mut rows = stmt.query(params![
                stored.subscription_key.as_str(),
                environment_id.as_str(),
                stored
                    .provider_instance_id
                    .as_ref()
                    .map(ProviderInstanceId::as_str),
                stored.window_name,
                optional_checked_i64(stored.window_start_ms, "capacity window_start_ms")?,
                optional_checked_i64(stored.window_end_ms, "capacity window_end_ms")?,
                evidence_scope,
                stored.source,
                checked_i64(stored.as_of_ms, "capacity as_of_ms")?,
            ])?;
            let existing = rows
                .next()?
                .ok_or_else(|| {
                    StoreError::Data(
                        "capacity conflict did not resolve to a stored snapshot".to_string(),
                    )
                })
                .and_then(|row| decode_capacity_snapshot(row, 0))?;
            if existing != stored {
                return Err(StoreError::Data(format!(
                    "capacity snapshot identity collision for subscription {} at {}",
                    stored.subscription_key, stored.as_of_ms
                )));
            }
        }
        Ok(())
    }

    pub fn query_capacity(
        &self,
        params: &CapacityQueryParams,
    ) -> Result<CapacityQueryResult, StoreError> {
        params.validate().map_err(StoreError::Data)?;
        let mut sql = String::from(
            "SELECT id, subscription_key, window_start_ms, window_end_ms,
                    used_millionths, remaining_millionths, reset_at_ms, source,
                    confidence, provider_version, as_of_ms, schema_version,
                    environment_id, provider_instance_id, window_name, evidence_scope,
                    hard_limit, max_age_ms
             FROM capacity_snapshots WHERE 1 = 1",
        );
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(subscription_key) = &params.subscription_key {
            sql.push_str(" AND subscription_key = ?");
            values.push(Box::new(subscription_key.as_str().to_string()));
        }
        if let Some(environment_id) = &params.environment_id {
            sql.push_str(" AND environment_id = ?");
            values.push(Box::new(environment_id.as_str().to_string()));
        }
        if let Some(cursor) = &params.after {
            let as_of_ms = checked_i64(cursor.as_of_ms, "capacity cursor as_of_ms")?;
            let sequence = checked_i64(cursor.sequence, "capacity cursor sequence")?;
            sql.push_str(
                " AND (
                    as_of_ms > ? OR
                    (as_of_ms = ? AND subscription_key > ?) OR
                    (as_of_ms = ? AND subscription_key = ? AND id > ?)
                  )",
            );
            values.push(Box::new(as_of_ms));
            values.push(Box::new(as_of_ms));
            values.push(Box::new(cursor.subscription_key.as_str().to_string()));
            values.push(Box::new(as_of_ms));
            values.push(Box::new(cursor.subscription_key.as_str().to_string()));
            values.push(Box::new(sequence));
        }
        sql.push_str(" ORDER BY as_of_ms, subscription_key, id LIMIT ?");
        values.push(Box::new(i64::from(params.limit) + 1));

        let conn = self.conn.lock().unwrap();
        let references: Vec<&dyn rusqlite::ToSql> =
            values.iter().map(|value| value.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(references.as_slice())?;
        let mut values = Vec::new();
        while let Some(row) = rows.next()? {
            let sequence = checked_u64(row.get(0)?, "capacity_snapshots.id")?;
            let snapshot = decode_capacity_snapshot(row, 1)?;
            values.push((sequence, snapshot));
        }
        let has_more = values.len() > params.limit as usize;
        if has_more {
            values.pop();
        }
        let next = values
            .last()
            .map(|(sequence, snapshot)| CapacityQueryCursor {
                as_of_ms: snapshot.as_of_ms,
                subscription_key: snapshot.subscription_key.clone(),
                sequence: *sequence,
            });
        let snapshots = values
            .into_iter()
            .map(|(_, snapshot)| snapshot)
            .collect::<Vec<_>>();
        drop(rows);
        drop(stmt);

        let mut projection_sql = String::from(
            "SELECT subscription_key, window_start_ms, window_end_ms,
                    used_millionths, remaining_millionths, reset_at_ms, source,
                    confidence, provider_version, as_of_ms, schema_version,
                    environment_id, provider_instance_id, window_name, evidence_scope,
                    hard_limit, max_age_ms
             FROM capacity_snapshots WHERE 1 = 1",
        );
        let mut projection_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(subscription_key) = &params.subscription_key {
            projection_sql.push_str(" AND subscription_key = ?");
            projection_values.push(Box::new(subscription_key.as_str().to_string()));
        }
        if let Some(environment_id) = &params.environment_id {
            projection_sql.push_str(" AND environment_id = ?");
            projection_values.push(Box::new(environment_id.as_str().to_string()));
        }
        projection_sql.push_str(" ORDER BY as_of_ms, subscription_key, id");
        let projection_references = projection_values
            .iter()
            .map(|value| value.as_ref())
            .collect::<Vec<&dyn rusqlite::ToSql>>();
        let mut projection_stmt = conn.prepare(&projection_sql)?;
        let mut projection_rows = projection_stmt.query(projection_references.as_slice())?;
        let mut projection_snapshots = Vec::new();
        while let Some(row) = projection_rows.next()? {
            projection_snapshots.push(decode_capacity_snapshot(row, 0)?);
        }
        let at_ms = params.at_ms.unwrap_or_else(current_time_ms);
        let effective = project_capacity_revisions(&projection_snapshots, at_ms, 5 * 60 * 1000);
        let offline_environments = load_offline_capacity_environments(&conn, params)?;

        Ok(CapacityQueryResult {
            snapshots,
            next,
            has_more,
            effective,
            offline_environments,
        })
    }

    pub fn get_latest_seq(&self, thread_id: &ThreadId) -> Result<u64, StoreError> {
        let conn = self.conn.lock().unwrap();
        let seq: Option<i64> = conn
            .query_row(
                "SELECT MAX(seq) FROM events WHERE thread_id = ?1",
                params![thread_id.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        Ok(seq.unwrap_or(0) as u64)
    }

    // ── Resumes ───────────────────────────────────────────────────────────

    pub fn save_resume_token(&self, thread_id: &ThreadId, token: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        conn.execute(
            "INSERT INTO thread_resumes (thread_id, resume_token, updated_at_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(thread_id) DO UPDATE SET
                resume_token = excluded.resume_token,
                updated_at_ms = excluded.updated_at_ms;",
            params![thread_id.as_str(), token, now],
        )?;

        Ok(())
    }

    pub fn get_resume_token(&self, thread_id: &ThreadId) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT resume_token FROM thread_resumes WHERE thread_id = ?1")?;
        let token: Option<String> = stmt
            .query_row(params![thread_id.as_str()], |row| row.get(0))
            .optional()?;

        Ok(token)
    }

    pub fn delete_resume_token(&self, thread_id: &ThreadId) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM thread_resumes WHERE thread_id = ?1",
            params![thread_id.as_str()],
        )?;
        Ok(())
    }

    pub fn reconcile_active_turns(&self) -> Result<usize, StoreError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let interrupted_status = serde_json::to_string(&TurnStatus::Interrupted)?;
        let idle_status = serde_json::to_string(&pandamux_core::ThreadStatus::Idle)?;
        let failed_attempt = serde_json::to_string(&TurnAttemptStatus::Failed)?;
        let running_attempt = serde_json::to_string(&TurnAttemptStatus::Running)?;
        let retrying_attempt = serde_json::to_string(&TurnAttemptStatus::Retrying)?;
        let failed_continuation = serde_json::to_string(&ContinuationState::Failed)?;
        let running_continuation = serde_json::to_string(&ContinuationState::Running)?;

        let count = tx.execute(
            "UPDATE turns SET status = ?1 WHERE status IN ('\"running\"', '\"awaiting_approval\"', '\"requested\"')",
            params![interrupted_status],
        )?;

        tx.execute(
            "UPDATE turn_attempts
             SET status = ?1,
                 ended_at_ms = CASE WHEN started_at_ms > ?4 THEN started_at_ms ELSE ?4 END
             WHERE status IN (?2, ?3)",
            params![failed_attempt, running_attempt, retrying_attempt, now],
        )?;
        tx.execute(
            "UPDATE continuations
             SET state = ?1,
                 updated_at_ms = CASE WHEN updated_at_ms > ?3 THEN updated_at_ms ELSE ?3 END
             WHERE state = ?2
               AND target_turn_id IN (SELECT id FROM turns WHERE status = ?4)",
            params![
                failed_continuation,
                running_continuation,
                now,
                interrupted_status,
            ],
        )?;
        tx.execute(
            "UPDATE threads SET status = ?1 WHERE status IN ('\"working\"', '\"awaiting_approval\"')",
            params![idle_status],
        )?;
        tx.commit()?;

        Ok(count)
    }

    // ── Events ────────────────────────────────────────────────────────────

    pub fn append_event(&self, event: &ThreadEvent) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let payload_json = serde_json::to_string(&event.kind)?;

        conn.execute(
            "INSERT INTO events (thread_id, seq, at_ms, kind, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                event.thread_id.as_str(),
                event.seq as i64,
                event.at_ms as i64,
                "thread_event",
                payload_json,
            ],
        )?;

        Ok(())
    }

    pub fn get_events(
        &self,
        thread_id: &ThreadId,
        since_seq: Option<u64>,
    ) -> Result<Vec<ThreadEvent>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let since = since_seq.unwrap_or(0) as i64;

        let mut stmt = conn.prepare(
            "SELECT seq, at_ms, payload_json FROM events
             WHERE thread_id = ?1 AND seq > ?2
             ORDER BY seq ASC",
        )?;

        let rows = stmt.query_map(params![thread_id.as_str(), since], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

        let mut events = Vec::new();
        for r in rows {
            let (seq, at_ms, payload_json) = r?;
            let kind: ThreadEventKind = serde_json::from_str(&payload_json)?;
            events.push(ThreadEvent {
                thread_id: thread_id.clone(),
                seq: seq as u64,
                at_ms: at_ms as u64,
                kind,
            });
        }

        Ok(events)
    }

    /// Highest durable event row sequence used by the node proxy replay stream.
    pub fn latest_event_row_id(&self) -> Result<u64, StoreError> {
        let conn = self.conn.lock().unwrap();
        let id: Option<i64> = conn.query_row("SELECT MAX(id) FROM events", [], |row| row.get(0))?;
        checked_u64(id.unwrap_or(0), "events.id")
    }

    /// Durable row sequence for a thread event after it has been committed.
    pub fn event_row_id(
        &self,
        thread_id: &ThreadId,
        event_seq: u64,
    ) -> Result<Option<u64>, StoreError> {
        let event_seq = i64::try_from(event_seq)
            .map_err(|_| StoreError::Data("event sequence exceeds SQLite i64".to_string()))?;
        let conn = self.conn.lock().unwrap();
        let id: Option<i64> = conn
            .query_row(
                "SELECT id FROM events WHERE thread_id = ?1 AND seq = ?2",
                params![thread_id.as_str(), event_seq],
                |row| row.get(0),
            )
            .optional()?;
        id.map(|value| checked_u64(value, "events.id")).transpose()
    }

    /// Read a bounded durable proxy event page in insertion order.
    pub fn get_event_envelopes_after(
        &self,
        environment_id: &EnvironmentId,
        after_row_id: u64,
        through_row_id: u64,
        limit: usize,
    ) -> Result<Vec<(u64, EventEnvelope)>, StoreError> {
        let after = i64::try_from(after_row_id)
            .map_err(|_| StoreError::Data("proxy cursor exceeds SQLite i64".to_string()))?;
        let through = i64::try_from(through_row_id)
            .map_err(|_| StoreError::Data("proxy cursor exceeds SQLite i64".to_string()))?;
        let limit = i64::try_from(limit)
            .map_err(|_| StoreError::Data("proxy page limit exceeds SQLite i64".to_string()))?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, thread_id, seq, at_ms, payload_json FROM events
             WHERE id > ?1 AND id <= ?2 ORDER BY id ASC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![after, through, limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        let mut events = Vec::new();
        for row in rows {
            let (row_id, thread_id, event_seq, at_ms, payload_json) = row?;
            events.push((
                checked_u64(row_id, "events.id")?,
                EventEnvelope {
                    subscription_id: "node-proxy".to_string(),
                    environment_id: environment_id.clone(),
                    thread_id: Some(ThreadId::from(thread_id)),
                    run_id: None,
                    seq: checked_u64(event_seq, "events.seq")?,
                    at_ms: checked_u64(at_ms, "events.at_ms")?,
                    kind: serde_json::from_str(&payload_json)?,
                },
            ));
        }
        Ok(events)
    }

    // ── Schedules ─────────────────────────────────────────────────────────

    pub fn save_schedule(&self, schedule: &ScheduleRecord) -> Result<(), StoreError> {
        schedule.validate().map_err(StoreError::Data)?;
        let conn = self.conn.lock().unwrap();
        let def_json = serde_json::to_string(schedule)?;
        let updated_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        conn.execute(
            "INSERT INTO schedules (id, environment_id, definition_json, enabled, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                definition_json = excluded.definition_json,
                enabled = excluded.enabled,
                updated_at_ms = excluded.updated_at_ms;",
            params![
                schedule.id.as_str(),
                schedule.environment.id.as_str(),
                def_json,
                if schedule.enabled { 1 } else { 0 },
                updated_ms,
            ],
        )?;

        Ok(())
    }

    pub fn list_schedules(&self) -> Result<Vec<ScheduleRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT definition_json FROM schedules ORDER BY id ASC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;

        let mut result = Vec::new();
        for r in rows {
            let json = r?;
            result.push(serde_json::from_str(&json)?);
        }

        Ok(result)
    }

    pub fn get_schedule(&self, id: &ScheduleId) -> Result<Option<ScheduleRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let json = conn
            .query_row(
                "SELECT definition_json FROM schedules WHERE id = ?1",
                [id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(StoreError::from))
            .transpose()
    }

    // ── Settings ──────────────────────────────────────────────────────────

    pub fn get_settings(&self) -> Result<Option<UserSettings>, StoreError> {
        let json: Option<String> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt =
                conn.prepare("SELECT value_json FROM settings WHERE key = 'user_settings'")?;
            stmt.query_row([], |row| row.get(0)).optional()?
        };

        match json {
            Some(j) => {
                let mut settings: UserSettings = serde_json::from_str(&j)?;
                let original = settings.clone();
                settings.normalize();
                if settings != original {
                    self.save_settings(&settings)?;
                }
                Ok(Some(settings))
            }
            None => Ok(None),
        }
    }

    pub fn save_settings(&self, settings: &UserSettings) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let json = serde_json::to_string(settings)?;

        conn.execute(
            "INSERT INTO settings (key, value_json) VALUES ('user_settings', ?1)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json;",
            params![json],
        )?;

        Ok(())
    }

    // ── Attachments ───────────────────────────────────────────────────────

    pub fn save_attachment(&self, record: &AttachmentRecord) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO attachments (id, thread_id, file_name, mime_type, size_bytes, file_path, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                file_name = excluded.file_name,
                mime_type = excluded.mime_type,
                size_bytes = excluded.size_bytes,
                file_path = excluded.file_path,
                created_at_ms = excluded.created_at_ms;",
            params![
                record.id,
                record.thread_id.as_str(),
                record.file_name,
                record.mime_type,
                record.size_bytes as i64,
                record.file_path,
                record.created_at_ms as i64,
            ],
        )?;
        Ok(())
    }

    pub fn get_attachment(
        &self,
        thread_id: &ThreadId,
        id: &str,
    ) -> Result<Option<AttachmentRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, thread_id, file_name, mime_type, size_bytes, file_path, created_at_ms
             FROM attachments WHERE thread_id = ?1 AND id = ?2",
        )?;
        let row = stmt
            .query_row(params![thread_id.as_str(), id], |row| {
                Ok(AttachmentRecord {
                    id: row.get(0)?,
                    thread_id: ThreadId::from(row.get::<_, String>(1)?),
                    file_name: row.get(2)?,
                    mime_type: row.get(3)?,
                    size_bytes: row.get::<_, i64>(4)? as u64,
                    file_path: row.get(5)?,
                    created_at_ms: row.get::<_, i64>(6)? as u64,
                })
            })
            .optional()?;
        Ok(row)
    }

    pub fn list_attachments(
        &self,
        thread_id: &ThreadId,
    ) -> Result<Vec<AttachmentRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, thread_id, file_name, mime_type, size_bytes, file_path, created_at_ms
             FROM attachments WHERE thread_id = ?1 ORDER BY created_at_ms ASC",
        )?;
        let rows = stmt.query_map(params![thread_id.as_str()], |row| {
            Ok(AttachmentRecord {
                id: row.get(0)?,
                thread_id: ThreadId::from(row.get::<_, String>(1)?),
                file_name: row.get(2)?,
                mime_type: row.get(3)?,
                size_bytes: row.get::<_, i64>(4)? as u64,
                file_path: row.get(5)?,
                created_at_ms: row.get::<_, i64>(6)? as u64,
            })
        })?;

        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    pub fn get_attachments_by_ids(
        &self,
        thread_id: &ThreadId,
        ids: &[String],
    ) -> Result<Vec<AttachmentRecord>, StoreError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap();
        let mut result = Vec::new();
        for id in ids {
            let mut stmt = conn.prepare(
                "SELECT id, thread_id, file_name, mime_type, size_bytes, file_path, created_at_ms
                 FROM attachments WHERE thread_id = ?1 AND id = ?2",
            )?;
            let row = stmt
                .query_row(params![thread_id.as_str(), id.as_str()], |row| {
                    Ok(AttachmentRecord {
                        id: row.get(0)?,
                        thread_id: ThreadId::from(row.get::<_, String>(1)?),
                        file_name: row.get(2)?,
                        mime_type: row.get(3)?,
                        size_bytes: row.get::<_, i64>(4)? as u64,
                        file_path: row.get(5)?,
                        created_at_ms: row.get::<_, i64>(6)? as u64,
                    })
                })
                .optional()?;
            if let Some(record) = row {
                result.push(record);
            }
        }
        Ok(result)
    }
}

fn validate_usage_cursor(cursor: &UsageScannerCursor) -> Result<(), StoreError> {
    if cursor.environment_id.as_str().trim().is_empty()
        || cursor.provider_instance_id.as_str().trim().is_empty()
        || cursor.source_kind.as_str().trim().is_empty()
        || cursor.node_store_id.as_str().trim().is_empty()
        || cursor.file_identity.trim().is_empty()
        || cursor.canonical_path.trim().is_empty()
    {
        return Err(StoreError::Data(
            "usage cursor scope, file identity, and diagnostic path must not be empty".to_string(),
        ));
    }
    if cursor.parser_version == 0 {
        return Err(StoreError::Data(
            "usage cursor parser version must be greater than zero".to_string(),
        ));
    }
    if cursor.committed_offset > cursor.observed_size {
        return Err(StoreError::Data(
            "usage cursor offset must not exceed observed file size".to_string(),
        ));
    }
    if !is_sha256(&cursor.committed_prefix_sha256) {
        return Err(StoreError::Data(
            "usage cursor prefix hash must be lowercase SHA-256".to_string(),
        ));
    }
    if cursor.parser_state_json.len() > 65_536
        || serde_json::from_str::<serde_json::Value>(&cursor.parser_state_json).is_err()
    {
        return Err(StoreError::Data(
            "usage cursor parser state must be bounded valid JSON".to_string(),
        ));
    }
    Ok(())
}

fn validate_usage_quarantine(quarantine: &UsageQuarantineRecord) -> Result<(), StoreError> {
    if quarantine.environment_id.as_str().trim().is_empty()
        || quarantine.provider_instance_id.as_str().trim().is_empty()
        || quarantine.source_kind.as_str().trim().is_empty()
        || !is_safe_diagnostic_code(&quarantine.reason_code)
        || quarantine.reason_detail.trim().is_empty()
        || quarantine.reason_detail.len() > 256
    {
        return Err(StoreError::Data(
            "usage quarantine scope and bounded reason must not be empty".to_string(),
        ));
    }
    if quarantine.parser_version == 0 {
        return Err(StoreError::Data(
            "usage quarantine parser version must be greater than zero".to_string(),
        ));
    }
    if !is_sha256(&quarantine.file_identity_sha256) || !is_sha256(&quarantine.safe_excerpt_sha256) {
        return Err(StoreError::Data(
            "usage quarantine hashes must be lowercase SHA-256".to_string(),
        ));
    }
    Ok(())
}

fn validate_usage_diagnostic(diagnostic: &UsageScannerDiagnostic) -> Result<(), StoreError> {
    if diagnostic.environment_id.as_str().trim().is_empty()
        || diagnostic.provider_instance_id.as_str().trim().is_empty()
        || diagnostic.source_kind.as_str().trim().is_empty()
        || diagnostic.parser_version == 0
    {
        return Err(StoreError::Data(
            "usage scanner diagnostic scope and parser version must be valid".to_string(),
        ));
    }
    if diagnostic
        .last_safe_error_code
        .as_ref()
        .is_some_and(|code| !is_safe_diagnostic_code(code))
    {
        return Err(StoreError::Data(
            "usage scanner safe error code must be absent or bounded".to_string(),
        ));
    }
    Ok(())
}

fn is_safe_diagnostic_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn checked_increment(value: u64, message: &str) -> Result<u64, StoreError> {
    value
        .checked_add(1)
        .ok_or_else(|| StoreError::Data(message.to_string()))
}

fn merge_usage_batch(
    target: &mut UsageBatchResult,
    source: &UsageBatchResult,
) -> Result<(), StoreError> {
    target.inserted = target
        .inserted
        .checked_add(source.inserted)
        .ok_or_else(|| StoreError::Data("usage insert count overflow".to_string()))?;
    target.duplicates = target
        .duplicates
        .checked_add(source.duplicates)
        .ok_or_else(|| StoreError::Data("usage duplicate count overflow".to_string()))?;
    target.reconciled = target
        .reconciled
        .checked_add(source.reconciled)
        .ok_or_else(|| StoreError::Data("usage reconciliation count overflow".to_string()))?;
    target.reconciliation_mismatches = target
        .reconciliation_mismatches
        .checked_add(source.reconciliation_mismatches)
        .ok_or_else(|| {
            StoreError::Data("usage reconciliation mismatch count overflow".to_string())
        })?;
    target.quarantined = target
        .quarantined
        .checked_add(source.quarantined)
        .ok_or_else(|| StoreError::Data("usage quarantine count overflow".to_string()))?;
    target.last_outbox_seq = target.last_outbox_seq.max(source.last_outbox_seq);
    Ok(())
}

fn cost_usd_to_micro(value: f64) -> Option<u64> {
    if !value.is_finite() || value < 0.0 || value > (u64::MAX as f64) / 1_000_000.0 {
        return None;
    }
    Some((value * 1_000_000.0).round() as u64)
}

fn managed_usage_observations(
    candidate: &ManagedUsageCandidate,
    provider: UsageProvider,
    subscription_key: SubscriptionKey,
) -> Option<Vec<UsageObservation>> {
    let provenance = candidate.usage.provenance.as_ref();
    let provider_native_session_id = provenance
        .and_then(|value| value.provider_native_session_id.clone())
        .filter(|value| !value.trim().is_empty());
    let provider_native_result_id = provenance
        .and_then(|value| value.provider_native_turn_id.clone())
        .filter(|value| !value.trim().is_empty());
    let provider_reported_micro_usd = candidate
        .usage
        .provider_reported_micro_usd
        .or_else(|| candidate.usage.cost_usd.and_then(cost_usd_to_micro));
    if candidate.usage.cost_usd.is_some()
        && candidate.usage.provider_reported_micro_usd.is_none()
        && provider_reported_micro_usd.is_none()
    {
        return None;
    }

    if candidate.usage.components.is_empty() {
        let model = provenance
            .and_then(|value| value.model.as_deref())
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| (!candidate.model.trim().is_empty()).then_some(candidate.model.clone()));
        let mut unknown_fields = candidate.usage.unknown_fields.clone();
        if provider_native_result_id.is_none() {
            unknown_fields.insert(UsageUnknownField::Identity);
        }
        let observation = UsageObservation {
            id: UsageObservationId::new(format!(
                "usage-managed-{}",
                stable_usage_hash(&[
                    candidate.environment_id.as_str(),
                    candidate.turn_id.as_str(),
                ])
            )),
            source_kind: UsageSourceKind::ManagedTurn,
            parser_version: 1,
            provider_native_session_id,
            provider_native_turn_id: provider_native_result_id,
            occurred_at_ms: provenance
                .and_then(|value| value.occurred_at_ms)
                .or(candidate.ended_at_ms),
            provider,
            provider_instance_id: candidate.provider_instance_id.clone(),
            subscription_key,
            model,
            tokens: TokenCounts {
                input: candidate.usage.input_tokens,
                cached_input: candidate.usage.cache_read_tokens.unwrap_or(0),
                cache_write: candidate.usage.cache_write_tokens.unwrap_or(0),
                output: candidate.usage.output_tokens,
                ..TokenCounts::default()
            },
            unknown_fields,
            provider_reported_micro_usd,
            source_record_count: 1,
            thread_id: Some(candidate.thread_id.clone()),
            turn_id: Some(candidate.turn_id.clone()),
            agent_id: candidate.agent_id.clone(),
            schedule_id: candidate.schedule_id.clone(),
            fact_kind: UsageFactKind::Aggregate,
        };
        return observation.validate().is_ok().then_some(vec![observation]);
    }

    let mut observations = Vec::new();
    if let Some(cost) = provider_reported_micro_usd {
        let mut unknown_fields = BTreeSet::new();
        if provider_native_result_id.is_none() {
            unknown_fields.insert(UsageUnknownField::Identity);
        }
        let result_identity = provider_native_result_id.as_deref().unwrap_or("");
        observations.push(UsageObservation {
            id: UsageObservationId::new(format!(
                "usage-managed-cost-{}",
                stable_usage_hash(&[
                    candidate.environment_id.as_str(),
                    candidate.turn_id.as_str(),
                    result_identity,
                ])
            )),
            source_kind: UsageSourceKind::ManagedTurn,
            parser_version: 2,
            provider_native_session_id: provider_native_session_id.clone(),
            provider_native_turn_id: provider_native_result_id,
            occurred_at_ms: provenance
                .and_then(|value| value.occurred_at_ms)
                .or(candidate.ended_at_ms),
            provider: provider.clone(),
            provider_instance_id: candidate.provider_instance_id.clone(),
            subscription_key: subscription_key.clone(),
            model: None,
            tokens: TokenCounts::default(),
            unknown_fields,
            provider_reported_micro_usd: Some(cost),
            source_record_count: 1,
            thread_id: Some(candidate.thread_id.clone()),
            turn_id: Some(candidate.turn_id.clone()),
            agent_id: candidate.agent_id.clone(),
            schedule_id: candidate.schedule_id.clone(),
            fact_kind: UsageFactKind::TurnCost,
        });
    }

    let mut components: BTreeMap<String, &TurnUsageComponent> = BTreeMap::new();
    for component in &candidate.usage.components {
        let message_id = component.provider_native_message_id.trim();
        if message_id.is_empty()
            || component
                .model
                .as_ref()
                .is_some_and(|model| model.trim().is_empty())
        {
            return None;
        }
        if let Some(existing) = components.get(message_id) {
            if existing.model != component.model
                || existing.input_tokens != component.input_tokens
                || existing.output_tokens != component.output_tokens
                || existing.cache_read_tokens != component.cache_read_tokens
                || existing.cache_write_tokens != component.cache_write_tokens
            {
                return None;
            }
            continue;
        }
        components.insert(message_id.to_string(), component);
    }

    for (message_id, component) in components {
        let mut unknown_fields = BTreeSet::new();
        if component.model.is_none() {
            unknown_fields.insert(UsageUnknownField::Model);
        }
        if component.cache_read_tokens.is_none() {
            unknown_fields.insert(UsageUnknownField::CacheReadTokens);
        }
        if component.cache_write_tokens.is_none() {
            unknown_fields.insert(UsageUnknownField::CacheWriteTokens);
        }
        let observation = UsageObservation {
            id: UsageObservationId::new(format!(
                "usage-managed-token-{}",
                stable_usage_hash(&[
                    candidate.environment_id.as_str(),
                    candidate.turn_id.as_str(),
                    &message_id,
                ])
            )),
            source_kind: UsageSourceKind::ManagedTurn,
            parser_version: 2,
            provider_native_session_id: provider_native_session_id.clone(),
            provider_native_turn_id: Some(message_id),
            occurred_at_ms: component.occurred_at_ms,
            provider: provider.clone(),
            provider_instance_id: candidate.provider_instance_id.clone(),
            subscription_key: subscription_key.clone(),
            model: component.model.clone(),
            tokens: TokenCounts {
                input: component.input_tokens,
                cached_input: component.cache_read_tokens.unwrap_or(0),
                cache_write: component.cache_write_tokens.unwrap_or(0),
                output: component.output_tokens,
                ..TokenCounts::default()
            },
            unknown_fields,
            provider_reported_micro_usd: None,
            source_record_count: 1,
            thread_id: Some(candidate.thread_id.clone()),
            turn_id: Some(candidate.turn_id.clone()),
            agent_id: candidate.agent_id.clone(),
            schedule_id: candidate.schedule_id.clone(),
            fact_kind: UsageFactKind::TokenComponent,
        };
        if observation.validate().is_err() {
            return None;
        }
        observations.push(observation);
    }

    if observations.is_empty()
        || observations
            .iter()
            .any(|observation| observation.validate().is_err())
    {
        return None;
    }
    Some(observations)
}

fn usage_reconciliation_key(observation: &UsageObservation) -> Result<Option<String>, StoreError> {
    if let Some(turn_id) = observation
        .provider_native_turn_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(Some(format!(
            "native:v2:{}",
            stable_usage_hash(&[
                observation
                    .provider_native_session_id
                    .as_deref()
                    .unwrap_or(""),
                turn_id,
                observation.fact_kind.as_str(),
            ])
        )));
    }
    Ok(None)
}

fn stable_usage_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn usage_measurement_compatible(existing: &UsageObservation, incoming: &UsageObservation) -> bool {
    let ignore_legacy_cache_presence = existing.parser_version < 2 || incoming.parser_version < 2;
    existing.provider_native_session_id == incoming.provider_native_session_id
        && existing.provider_native_turn_id == incoming.provider_native_turn_id
        && existing.provider == incoming.provider
        && existing.provider_instance_id == incoming.provider_instance_id
        && existing.subscription_key == incoming.subscription_key
        && existing.model == incoming.model
        && existing.tokens == incoming.tokens
        && usage_unknown_fields_for_reconciliation(
            &existing.unknown_fields,
            ignore_legacy_cache_presence,
        ) == usage_unknown_fields_for_reconciliation(
            &incoming.unknown_fields,
            ignore_legacy_cache_presence,
        )
        && existing.fact_kind == incoming.fact_kind
        && (existing.provider_reported_micro_usd == incoming.provider_reported_micro_usd
            || existing.provider_reported_micro_usd.is_none()
            || incoming.provider_reported_micro_usd.is_none())
}

fn managed_transcript_mismatch(
    existing: &UsageObservation,
    incoming: &UsageObservation,
) -> Option<PendingUsageReconciliationMismatch> {
    let (managed, transcript) = match (&existing.source_kind, &incoming.source_kind) {
        (
            UsageSourceKind::ManagedTurn,
            UsageSourceKind::ClaudeTranscript | UsageSourceKind::CodexTranscript,
        ) => (existing, incoming),
        (
            UsageSourceKind::ClaudeTranscript | UsageSourceKind::CodexTranscript,
            UsageSourceKind::ManagedTurn,
        ) => (incoming, existing),
        _ => return None,
    };
    Some(PendingUsageReconciliationMismatch {
        managed_observation_id: managed.id.clone(),
        transcript_observation_id: transcript.id.clone(),
        provider_instance_id: transcript.provider_instance_id.clone(),
        source_kind: transcript.source_kind.clone(),
    })
}

fn usage_unknown_fields_for_reconciliation(
    fields: &BTreeSet<UsageUnknownField>,
    ignore_cache_presence: bool,
) -> BTreeSet<UsageUnknownField> {
    fields
        .iter()
        .copied()
        .filter(|field| {
            *field != UsageUnknownField::OccurredAt
                && !(ignore_cache_presence
                    && matches!(
                        field,
                        UsageUnknownField::CacheReadTokens | UsageUnknownField::CacheWriteTokens
                    ))
        })
        .collect()
}

fn has_separate_managed_cost(
    tx: &rusqlite::Transaction<'_>,
    environment_id: &EnvironmentId,
    observation: &UsageObservation,
) -> Result<bool, StoreError> {
    let Some(turn_id) = observation.turn_id.as_ref() else {
        return Ok(false);
    };
    Ok(tx.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM usage_observations
             WHERE environment_id = ?1 AND turn_id = ?2
               AND source_kind = 'managed_turn' AND fact_kind = 'turn_cost'
         )",
        params![environment_id.as_str(), turn_id.as_str()],
        |row| row.get(0),
    )?)
}

fn enrich_usage_observation_cost_if_safe(
    tx: &rusqlite::Transaction<'_>,
    environment_id: &EnvironmentId,
    existing: &UsageObservation,
    incoming: &UsageObservation,
    enqueued_at_ms: i64,
) -> Result<Option<u64>, StoreError> {
    let Some(cost) = incoming.provider_reported_micro_usd else {
        return Ok(None);
    };
    if existing.provider_reported_micro_usd.is_some()
        || has_separate_managed_cost(tx, environment_id, existing)?
    {
        return Ok(None);
    }
    let mut enriched = existing.clone();
    enriched.provider_reported_micro_usd = Some(cost);
    tx.execute(
        "UPDATE usage_observations
            SET provider_reported_micro_usd = ?1
          WHERE environment_id = ?2 AND observation_id = ?3",
        params![
            checked_i64(cost, "provider_reported_micro_usd")?,
            environment_id.as_str(),
            existing.id.as_str(),
        ],
    )?;
    let payload_bytes = usage_observation_payload_bytes(&enriched)?;
    tx.execute(
        "UPDATE usage_outbox SET payload_bytes = ?1
         WHERE environment_id = ?2 AND observation_id = ?3",
        params![payload_bytes, environment_id.as_str(), existing.id.as_str()],
    )?;
    tx.execute(
        "INSERT INTO usage_outbox (
            environment_id, observation_id, enqueued_at_ms, payload_bytes
         ) VALUES (?1, ?2, ?3, ?4)",
        params![
            environment_id.as_str(),
            existing.id.as_str(),
            enqueued_at_ms,
            payload_bytes,
        ],
    )?;
    let outbox_seq = checked_u64(tx.last_insert_rowid(), "usage_outbox.seq")?;
    tx.execute(
        "UPDATE usage_observations
         SET source_node_store_id = (
                 SELECT node_store_id FROM usage_store_meta WHERE singleton = 1
             ),
             source_outbox_seq = ?1
         WHERE environment_id = ?2 AND observation_id = ?3",
        params![
            checked_i64(outbox_seq, "usage_outbox.seq")?,
            environment_id.as_str(),
            existing.id.as_str(),
        ],
    )?;
    Ok(Some(outbox_seq))
}

fn promote_usage_observation_to_managed(
    tx: &rusqlite::Transaction<'_>,
    environment_id: &EnvironmentId,
    existing_id: &UsageObservationId,
    managed: &UsageObservation,
    reconciliation_key: &str,
    enqueued_at_ms: i64,
) -> Result<u64, StoreError> {
    let other_tokens_json = serde_json::to_string(&managed.tokens.other)?;
    let unknown_fields_json = serde_json::to_string(&managed.unknown_fields)?;
    let updated = tx.execute(
        "UPDATE usage_observations SET
            source_kind = ?1,
            parser_version = ?2,
            provider_native_session_id = ?3,
            provider_native_turn_id = ?4,
            occurred_at_ms = ?5,
            provider = ?6,
            provider_instance_id = ?7,
            subscription_key = ?8,
            model = ?9,
            input_tokens = ?10,
            cached_input_tokens = ?11,
            cache_write_tokens = ?12,
            output_tokens = ?13,
            reasoning_tokens = ?14,
            other_tokens_json = ?15,
            unknown_fields_json = ?16,
            provider_reported_micro_usd = ?17,
            source_record_count = ?18,
            thread_id = ?19,
            turn_id = ?20,
            agent_id = ?21,
            schedule_id = ?22,
            fact_kind = ?23,
            reconciliation_key = ?24
         WHERE environment_id = ?25 AND observation_id = ?26",
        params![
            managed.source_kind.as_str(),
            i64::from(managed.parser_version),
            managed.provider_native_session_id,
            managed.provider_native_turn_id,
            optional_checked_i64(managed.occurred_at_ms, "occurred_at_ms")?,
            managed.provider.as_str(),
            managed.provider_instance_id.as_str(),
            managed.subscription_key.as_str(),
            managed.model,
            checked_i64(managed.tokens.input, "input_tokens")?,
            checked_i64(managed.tokens.cached_input, "cached_input_tokens")?,
            checked_i64(managed.tokens.cache_write, "cache_write_tokens")?,
            checked_i64(managed.tokens.output, "output_tokens")?,
            checked_i64(managed.tokens.reasoning, "reasoning_tokens")?,
            other_tokens_json,
            unknown_fields_json,
            optional_checked_i64(
                managed.provider_reported_micro_usd,
                "provider_reported_micro_usd"
            )?,
            checked_i64(managed.source_record_count, "source_record_count")?,
            managed.thread_id.as_ref().map(ThreadId::as_str),
            managed.turn_id.as_ref().map(TurnId::as_str),
            managed.agent_id.as_ref().map(|id| id.as_str()),
            managed.schedule_id.as_ref().map(ScheduleId::as_str),
            managed.fact_kind.as_str(),
            reconciliation_key,
            environment_id.as_str(),
            existing_id.as_str(),
        ],
    )?;
    if updated != 1 {
        return Err(StoreError::Data(
            "managed usage reconciliation target disappeared".to_string(),
        ));
    }
    let mut promoted = managed.clone();
    promoted.id = existing_id.clone();
    let payload_bytes = usage_observation_payload_bytes(&promoted)?;
    tx.execute(
        "UPDATE usage_outbox SET payload_bytes = ?1
         WHERE environment_id = ?2 AND observation_id = ?3",
        params![payload_bytes, environment_id.as_str(), existing_id.as_str()],
    )?;
    tx.execute(
        "INSERT INTO usage_outbox (
            environment_id, observation_id, enqueued_at_ms, payload_bytes
         ) VALUES (?1, ?2, ?3, ?4)",
        params![
            environment_id.as_str(),
            existing_id.as_str(),
            enqueued_at_ms,
            payload_bytes,
        ],
    )?;
    let outbox_seq = checked_u64(tx.last_insert_rowid(), "usage_outbox.seq")?;
    tx.execute(
        "UPDATE usage_observations
         SET source_node_store_id = (
                 SELECT node_store_id FROM usage_store_meta WHERE singleton = 1
             ),
             source_outbox_seq = ?1
         WHERE environment_id = ?2 AND observation_id = ?3",
        params![
            checked_i64(outbox_seq, "usage_outbox.seq")?,
            environment_id.as_str(),
            existing_id.as_str(),
        ],
    )?;
    Ok(outbox_seq)
}

fn usage_observation_payload_bytes(observation: &UsageObservation) -> Result<i64, StoreError> {
    i64::try_from(serde_json::to_vec(observation)?.len())
        .map_err(|_| StoreError::Data("usage observation payload size overflow".to_string()))
}

fn coverage_state_value(state: UsageSourceCoverageState) -> &'static str {
    match state {
        UsageSourceCoverageState::Complete => "complete",
        UsageSourceCoverageState::Partial => "partial",
        UsageSourceCoverageState::Unavailable => "unavailable",
    }
}

fn upsert_usage_diagnostic(
    tx: &rusqlite::Transaction<'_>,
    diagnostic: &UsageScannerDiagnostic,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT INTO usage_scanner_diagnostics (
            environment_id, provider_instance_id, source_kind, coverage_state,
            parser_version, last_scan_started_at_ms, last_success_at_ms,
            latest_source_at_ms, source_lag_ms, files_scanned, bytes_scanned,
            quarantined_count, last_safe_error_code, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(environment_id, provider_instance_id, source_kind) DO UPDATE SET
            coverage_state = excluded.coverage_state,
            parser_version = excluded.parser_version,
            last_scan_started_at_ms = excluded.last_scan_started_at_ms,
            last_success_at_ms = COALESCE(
                excluded.last_success_at_ms,
                usage_scanner_diagnostics.last_success_at_ms
            ),
            latest_source_at_ms = excluded.latest_source_at_ms,
            source_lag_ms = excluded.source_lag_ms,
            files_scanned = excluded.files_scanned,
            bytes_scanned = excluded.bytes_scanned,
            quarantined_count = excluded.quarantined_count,
            last_safe_error_code = excluded.last_safe_error_code,
            updated_at_ms = excluded.updated_at_ms",
        params![
            diagnostic.environment_id.as_str(),
            diagnostic.provider_instance_id.as_str(),
            diagnostic.source_kind.as_str(),
            coverage_state_value(diagnostic.coverage_state),
            i64::from(diagnostic.parser_version),
            checked_i64(
                diagnostic.last_scan_started_at_ms,
                "diagnostic last_scan_started_at_ms"
            )?,
            optional_checked_i64(
                diagnostic.last_success_at_ms,
                "diagnostic last_success_at_ms"
            )?,
            optional_checked_i64(
                diagnostic.latest_source_at_ms,
                "diagnostic latest_source_at_ms"
            )?,
            optional_checked_i64(diagnostic.source_lag_ms, "diagnostic source_lag_ms")?,
            checked_i64(diagnostic.files_scanned, "diagnostic files_scanned")?,
            checked_i64(diagnostic.bytes_scanned, "diagnostic bytes_scanned")?,
            checked_i64(diagnostic.quarantined_count, "diagnostic quarantined_count")?,
            diagnostic.last_safe_error_code,
            checked_i64(store_now_ms(), "diagnostic updated_at_ms")?,
        ],
    )?;
    Ok(())
}

fn store_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn load_usage_observation(
    conn: &Connection,
    environment_id: &EnvironmentId,
    observation_id: &UsageObservationId,
) -> Result<Option<UsageObservation>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT observation_id, source_kind, parser_version,
                provider_native_session_id, provider_native_turn_id,
                occurred_at_ms, provider, provider_instance_id, subscription_key,
                model, input_tokens, cached_input_tokens, cache_write_tokens,
                output_tokens, reasoning_tokens, other_tokens_json,
                unknown_fields_json, provider_reported_micro_usd, source_record_count,
                thread_id, turn_id, agent_id, schedule_id, fact_kind
         FROM usage_observations
         WHERE environment_id = ?1 AND observation_id = ?2",
    )?;
    let mut rows = stmt.query(params![environment_id.as_str(), observation_id.as_str()])?;
    rows.next()?
        .map(|row| decode_usage_observation(row, 0))
        .transpose()
}

fn load_usage_observations_by_exact_identity(
    conn: &Connection,
    environment_id: &EnvironmentId,
    observation: &UsageObservation,
) -> Result<Vec<UsageObservation>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT observation_id, source_kind, parser_version,
                provider_native_session_id, provider_native_turn_id,
                occurred_at_ms, provider, provider_instance_id, subscription_key,
                model, input_tokens, cached_input_tokens, cache_write_tokens,
                output_tokens, reasoning_tokens, other_tokens_json,
                unknown_fields_json, provider_reported_micro_usd, source_record_count,
                thread_id, turn_id, agent_id, schedule_id, fact_kind
         FROM usage_observations
         WHERE environment_id = ?1
           AND COALESCE(provider_native_session_id, '') = ?2
           AND provider_native_turn_id = ?3
           AND fact_kind = ?4
         ORDER BY observation_id",
    )?;
    let mut rows = stmt.query(params![
        environment_id.as_str(),
        observation
            .provider_native_session_id
            .as_deref()
            .unwrap_or(""),
        observation.provider_native_turn_id,
        observation.fact_kind.as_str(),
    ])?;
    let mut observations = Vec::new();
    while let Some(row) = rows.next()? {
        observations.push(decode_usage_observation(row, 0)?);
    }
    Ok(observations)
}

fn decode_usage_observation(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> Result<UsageObservation, StoreError> {
    let parser_version = checked_u32(row.get(offset + 2)?, "usage_observations.parser_version")?;
    let other_tokens_json: String = row.get(offset + 15)?;
    let unknown_fields_json: String = row.get(offset + 16)?;
    let observation = UsageObservation {
        id: UsageObservationId::from(row.get::<_, String>(offset)?),
        source_kind: UsageSourceKind::from(row.get::<_, String>(offset + 1)?),
        parser_version,
        provider_native_session_id: row.get(offset + 3)?,
        provider_native_turn_id: row.get(offset + 4)?,
        occurred_at_ms: row
            .get::<_, Option<i64>>(offset + 5)?
            .map(|value| checked_u64(value, "usage_observations.occurred_at_ms"))
            .transpose()?,
        provider: UsageProvider::from(row.get::<_, String>(offset + 6)?),
        provider_instance_id: ProviderInstanceId::from(row.get::<_, String>(offset + 7)?),
        subscription_key: SubscriptionKey::from(row.get::<_, String>(offset + 8)?),
        model: row.get(offset + 9)?,
        tokens: TokenCounts {
            input: checked_u64(row.get(offset + 10)?, "usage_observations.input_tokens")?,
            cached_input: checked_u64(
                row.get(offset + 11)?,
                "usage_observations.cached_input_tokens",
            )?,
            cache_write: checked_u64(
                row.get(offset + 12)?,
                "usage_observations.cache_write_tokens",
            )?,
            output: checked_u64(row.get(offset + 13)?, "usage_observations.output_tokens")?,
            reasoning: checked_u64(row.get(offset + 14)?, "usage_observations.reasoning_tokens")?,
            other: serde_json::from_str::<BTreeMap<String, u64>>(&other_tokens_json)?,
        },
        unknown_fields: serde_json::from_str::<BTreeSet<UsageUnknownField>>(&unknown_fields_json)?,
        provider_reported_micro_usd: row
            .get::<_, Option<i64>>(offset + 17)?
            .map(|value| checked_u64(value, "usage_observations.provider_reported_micro_usd"))
            .transpose()?,
        source_record_count: checked_u64(
            row.get(offset + 18)?,
            "usage_observations.source_record_count",
        )?,
        thread_id: row
            .get::<_, Option<String>>(offset + 19)?
            .map(ThreadId::from),
        turn_id: row.get::<_, Option<String>>(offset + 20)?.map(TurnId::from),
        agent_id: row
            .get::<_, Option<String>>(offset + 21)?
            .map(AgentId::from),
        schedule_id: row
            .get::<_, Option<String>>(offset + 22)?
            .map(ScheduleId::from),
        fact_kind: UsageFactKind::from(row.get::<_, String>(offset + 23)?),
    };
    observation
        .validate()
        .map_err(|error| StoreError::Data(error.to_string()))?;
    Ok(observation)
}

fn load_usage_scanner_diagnostics(
    conn: &Connection,
    params: &UsageQueryParams,
) -> Result<Vec<UsageScannerDiagnostic>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT environment_id, provider_instance_id, source_kind, coverage_state,
                parser_version, last_scan_started_at_ms, last_success_at_ms,
                latest_source_at_ms, source_lag_ms, files_scanned, bytes_scanned,
                quarantined_count, last_safe_error_code
         FROM usage_scanner_diagnostics
         ORDER BY environment_id, provider_instance_id, source_kind",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, Option<i64>>(6)?,
            row.get::<_, Option<i64>>(7)?,
            row.get::<_, Option<i64>>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, i64>(10)?,
            row.get::<_, i64>(11)?,
            row.get::<_, Option<String>>(12)?,
        ))
    })?;
    let mut diagnostics = Vec::new();
    for row in rows {
        let row = row?;
        let environment_id = EnvironmentId::from(row.0);
        let provider_instance_id = ProviderInstanceId::from(row.1);
        let source_kind = UsageSourceKind::from(row.2);
        if params
            .environment_id
            .as_ref()
            .is_some_and(|filter| filter != &environment_id)
            || params
                .provider_instance_id
                .as_ref()
                .is_some_and(|filter| filter != &provider_instance_id)
            || params
                .source_kind
                .as_ref()
                .is_some_and(|filter| filter != &source_kind)
        {
            continue;
        }
        let coverage_state = match row.3.as_str() {
            "complete" => UsageSourceCoverageState::Complete,
            "partial" => UsageSourceCoverageState::Partial,
            "unavailable" => UsageSourceCoverageState::Unavailable,
            other => {
                return Err(StoreError::Data(format!(
                    "unknown usage scanner coverage state {other}"
                )));
            }
        };
        diagnostics.push(UsageScannerDiagnostic {
            environment_id,
            provider_instance_id,
            source_kind,
            coverage_state,
            parser_version: checked_u32(row.4, "diagnostic parser_version")?,
            last_scan_started_at_ms: checked_u64(row.5, "diagnostic last_scan_started_at_ms")?,
            last_success_at_ms: row
                .6
                .map(|value| checked_u64(value, "diagnostic last_success_at_ms"))
                .transpose()?,
            latest_source_at_ms: row
                .7
                .map(|value| checked_u64(value, "diagnostic latest_source_at_ms"))
                .transpose()?,
            source_lag_ms: row
                .8
                .map(|value| checked_u64(value, "diagnostic source_lag_ms"))
                .transpose()?,
            files_scanned: checked_u64(row.9, "diagnostic files_scanned")?,
            bytes_scanned: checked_u64(row.10, "diagnostic bytes_scanned")?,
            quarantined_count: checked_u64(row.11, "diagnostic quarantined_count")?,
            last_safe_error_code: row.12,
        });
    }
    let mut legacy_debt_stmt = conn.prepare(
        "SELECT environment_id, provider_instance_id, source_kind,
                parser_version, detected_at_ms, latest_source_at_ms
           FROM usage_legacy_reconciliation_debt
          ORDER BY environment_id, provider_instance_id, source_kind",
    )?;
    let legacy_debt_rows = legacy_debt_stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, Option<i64>>(5)?,
        ))
    })?;
    for row in legacy_debt_rows {
        let row = row?;
        apply_usage_coverage_debt(
            &mut diagnostics,
            params,
            row.0,
            row.1,
            row.2,
            row.3,
            row.4,
            row.5,
            "legacy_reconciliation_debt",
        )?;
    }

    let mut mismatch_stmt = conn.prepare(
        "SELECT m.environment_id, m.provider_instance_id, m.source_kind,
                MAX(o.parser_version), MIN(m.detected_at_ms),
                MAX(o.occurred_at_ms)
           FROM usage_reconciliation_mismatches m
           JOIN usage_observations o
             ON o.environment_id = m.environment_id
            AND o.observation_id = m.transcript_observation_id
          GROUP BY m.environment_id, m.provider_instance_id, m.source_kind
          ORDER BY m.environment_id, m.provider_instance_id, m.source_kind",
    )?;
    let mismatch_rows = mismatch_stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, Option<i64>>(5)?,
        ))
    })?;
    for row in mismatch_rows {
        let row = row?;
        apply_usage_coverage_debt(
            &mut diagnostics,
            params,
            row.0,
            row.1,
            row.2,
            row.3,
            row.4,
            row.5,
            "managed_transcript_mismatch",
        )?;
    }
    diagnostics.sort_by(|left, right| {
        (
            &left.environment_id,
            &left.provider_instance_id,
            &left.source_kind,
        )
            .cmp(&(
                &right.environment_id,
                &right.provider_instance_id,
                &right.source_kind,
            ))
    });
    Ok(diagnostics)
}

#[allow(clippy::too_many_arguments)]
fn apply_usage_coverage_debt(
    diagnostics: &mut Vec<UsageScannerDiagnostic>,
    params: &UsageQueryParams,
    environment_id: String,
    provider_instance_id: String,
    source_kind: String,
    parser_version: i64,
    detected_at_ms: i64,
    latest_source_at_ms: Option<i64>,
    safe_error_code: &'static str,
) -> Result<(), StoreError> {
    let environment_id = EnvironmentId::from(environment_id);
    let provider_instance_id = ProviderInstanceId::from(provider_instance_id);
    let source_kind = UsageSourceKind::from(source_kind);
    if params
        .environment_id
        .as_ref()
        .is_some_and(|filter| filter != &environment_id)
        || params
            .provider_instance_id
            .as_ref()
            .is_some_and(|filter| filter != &provider_instance_id)
        || params
            .source_kind
            .as_ref()
            .is_some_and(|filter| filter != &source_kind)
    {
        return Ok(());
    }
    if let Some(diagnostic) = diagnostics.iter_mut().find(|diagnostic| {
        diagnostic.environment_id == environment_id
            && diagnostic.provider_instance_id == provider_instance_id
            && diagnostic.source_kind == source_kind
    }) {
        diagnostic.coverage_state = UsageSourceCoverageState::Partial;
        diagnostic.last_safe_error_code = Some(safe_error_code.to_string());
        return Ok(());
    }
    diagnostics.push(UsageScannerDiagnostic {
        environment_id,
        provider_instance_id,
        source_kind,
        coverage_state: UsageSourceCoverageState::Partial,
        parser_version: checked_u32(parser_version, "coverage debt parser_version")?,
        last_scan_started_at_ms: checked_u64(detected_at_ms, "coverage debt detected_at_ms")?,
        last_success_at_ms: None,
        latest_source_at_ms: latest_source_at_ms
            .map(|value| checked_u64(value, "coverage debt latest_source_at_ms"))
            .transpose()?,
        source_lag_ms: None,
        files_scanned: 0,
        bytes_scanned: 0,
        quarantined_count: 0,
        last_safe_error_code: Some(safe_error_code.to_string()),
    });
    Ok(())
}

fn decode_capacity_snapshot(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> Result<CapacitySnapshot, StoreError> {
    let confidence_value: String = row.get(offset + 7)?;
    let confidence = match confidence_value.as_str() {
        "reported" => CapacityConfidence::Reported,
        "estimated" => CapacityConfidence::Estimated,
        "unknown" => CapacityConfidence::Unknown,
        _ => {
            return Err(StoreError::Data(format!(
                "capacity snapshot contains unknown confidence: {confidence_value}"
            )));
        }
    };
    let evidence_scope_value: String = row.get(offset + 14)?;
    let evidence_scope = match evidence_scope_value.as_str() {
        "local" => CapacityEvidenceScope::Local,
        "hub" => CapacityEvidenceScope::Hub,
        _ => {
            return Err(StoreError::Data(format!(
                "capacity snapshot contains unknown evidence scope: {evidence_scope_value}"
            )));
        }
    };
    let snapshot = CapacitySnapshot {
        schema_version: checked_u32(row.get(offset + 10)?, "capacity_snapshots.schema_version")?,
        subscription_key: SubscriptionKey::from(row.get::<_, String>(offset)?),
        environment_id: Some(EnvironmentId::from(row.get::<_, String>(offset + 11)?)),
        provider_instance_id: row
            .get::<_, Option<String>>(offset + 12)?
            .map(ProviderInstanceId::from),
        window_name: row.get(offset + 13)?,
        window_start_ms: optional_checked_u64(
            row.get(offset + 1)?,
            "capacity_snapshots.window_start_ms",
        )?,
        window_end_ms: optional_checked_u64(
            row.get(offset + 2)?,
            "capacity_snapshots.window_end_ms",
        )?,
        used_millionths: optional_checked_u32(
            row.get(offset + 3)?,
            "capacity_snapshots.used_millionths",
        )?,
        remaining_millionths: optional_checked_u32(
            row.get(offset + 4)?,
            "capacity_snapshots.remaining_millionths",
        )?,
        reset_at_ms: optional_checked_u64(row.get(offset + 5)?, "capacity_snapshots.reset_at_ms")?,
        source: row.get(offset + 6)?,
        confidence,
        evidence_scope,
        hard_limit: row.get(offset + 15)?,
        provider_version: row.get(offset + 8)?,
        as_of_ms: checked_u64(row.get(offset + 9)?, "capacity_snapshots.as_of_ms")?,
        max_age_ms: checked_u64(row.get(offset + 16)?, "capacity_snapshots.max_age_ms")?,
    };
    snapshot.validate().map_err(StoreError::Data)?;
    Ok(snapshot)
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn load_offline_capacity_environments(
    conn: &Connection,
    params: &CapacityQueryParams,
) -> Result<Vec<EnvironmentId>, StoreError> {
    let mut sql = String::from(
        "SELECT environment_id
         FROM usage_hub_environment_status
         WHERE sync_state IN ('stale', 'unreachable')",
    );
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(environment_id) = &params.environment_id {
        sql.push_str(" AND environment_id = ?");
        values.push(Box::new(environment_id.as_str().to_string()));
    }
    sql.push_str(" ORDER BY environment_id");
    let references = values
        .iter()
        .map(|value| value.as_ref())
        .collect::<Vec<&dyn rusqlite::ToSql>>();
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(references.as_slice())?;
    let mut environments = Vec::new();
    while let Some(row) = rows.next()? {
        environments.push(EnvironmentId::from(row.get::<_, String>(0)?));
    }
    Ok(environments)
}

fn push_usage_filter(
    sql: &mut String,
    values: &mut Vec<Box<dyn rusqlite::ToSql>>,
    column: &str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        sql.push_str(" AND ");
        sql.push_str(column);
        sql.push_str(" = ?");
        values.push(Box::new(value.to_string()));
    }
}

fn checked_i64(value: u64, field: &str) -> Result<i64, StoreError> {
    i64::try_from(value)
        .map_err(|_| StoreError::Data(format!("{field} exceeds SQLite integer range: {value}")))
}

fn optional_checked_i64(value: Option<u64>, field: &str) -> Result<Option<i64>, StoreError> {
    value.map(|value| checked_i64(value, field)).transpose()
}

fn checked_u64(value: i64, field: &str) -> Result<u64, StoreError> {
    u64::try_from(value)
        .map_err(|_| StoreError::Data(format!("{field} contains a negative value: {value}")))
}

fn checked_u32(value: i64, field: &str) -> Result<u32, StoreError> {
    u32::try_from(value)
        .map_err(|_| StoreError::Data(format!("{field} is outside the u32 range: {value}")))
}

fn optional_checked_u64(value: Option<i64>, field: &str) -> Result<Option<u64>, StoreError> {
    value.map(|value| checked_u64(value, field)).transpose()
}

fn optional_checked_u32(value: Option<i64>, field: &str) -> Result<Option<u32>, StoreError> {
    value.map(|value| checked_u32(value, field)).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{
        AttemptBoundary, EnvironmentId, ProviderInstanceId, RoutingDecision, ThreadStatus,
        ThreadWorkspace, TurnAttempt, TurnAttemptId, TurnId, TurnInput, TurnUsage,
        TurnUsageComponent, TurnUsageProvenance,
    };

    fn dummy_thread(id: &str) -> Thread {
        Thread {
            id: ThreadId::from(id),
            project_id: None,
            environment_id: EnvironmentId::from("env-local"),
            parent_thread_id: None,
            title: "Store Test".to_string(),
            provider_instance_id: ProviderInstanceId::from("prov-claude"),
            model: "claude-3-7-sonnet".to_string(),
            effort: None,
            access_mode: Default::default(),
            workspace: ThreadWorkspace {
                cwd: "/test/dir".to_string(),
                worktree: None,
            },
            status: ThreadStatus::Idle,
            agent: None,
            origin: Default::default(),
            created_at_ms: 1000,
            updated_at_ms: 1000,
        }
    }

    #[test]
    fn store_in_memory_crud_and_events() {
        let store = Store::in_memory().expect("open in_memory store");

        // Save & retrieve thread
        let thread = dummy_thread("thread-101");
        store.save_thread(&thread).expect("save thread");

        let retrieved = store
            .get_thread(&ThreadId::from("thread-101"))
            .expect("get thread");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().title, "Store Test");

        // Append events
        let event1 = ThreadEvent {
            thread_id: ThreadId::from("thread-101"),
            seq: 1,
            at_ms: 1010,
            kind: ThreadEventKind::TurnRequested {
                turn_id: TurnId::from("turn-1"),
                input: TurnInput {
                    text: "Hello SQLite".to_string(),
                    attachment_ids: vec![],
                    attachments: vec![],
                    model: None,
                    effort: None,
                },
            },
        };
        store.append_event(&event1).expect("append event 1");

        let event2 = ThreadEvent {
            thread_id: ThreadId::from("thread-101"),
            seq: 2,
            at_ms: 1020,
            kind: ThreadEventKind::TurnStarted {
                turn_id: TurnId::from("turn-1"),
            },
        };
        store.append_event(&event2).expect("append event 2");

        let events = store
            .get_events(&ThreadId::from("thread-101"), None)
            .expect("get events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[1].seq, 2);

        // Test since_seq
        let since_1 = store
            .get_events(&ThreadId::from("thread-101"), Some(1))
            .expect("events since 1");
        assert_eq!(since_1.len(), 1);
        assert_eq!(since_1[0].seq, 2);

        // Settings roundtrip
        let mut settings = UserSettings::default();
        settings.ui.theme = "light".to_string();
        store.save_settings(&settings).expect("save settings");

        let loaded_settings = store.get_settings().expect("load settings").unwrap();
        assert_eq!(loaded_settings.ui.theme, "light");
    }

    #[test]
    fn loading_version_two_settings_persists_disabled_best_effort_profiles() {
        let store = Store::in_memory().expect("open in-memory store");
        let mut settings = UserSettings {
            version: 2,
            ..UserSettings::default()
        };
        settings.providers.retain(|provider| {
            !matches!(
                provider.provider,
                pandamux_core::ProviderKind::Cursor
                    | pandamux_core::ProviderKind::Grok
                    | pandamux_core::ProviderKind::OpenCode
            )
        });
        store
            .save_settings(&settings)
            .expect("save version two settings");

        let migrated = store
            .get_settings()
            .expect("migrate settings")
            .expect("stored settings");
        assert_eq!(migrated.version, pandamux_core::SETTINGS_SCHEMA_VERSION);

        let stored_json: String = store
            .conn
            .lock()
            .expect("store lock")
            .query_row(
                "SELECT value_json FROM settings WHERE key = 'user_settings'",
                [],
                |row| row.get(0),
            )
            .expect("read persisted settings");
        let persisted: UserSettings =
            serde_json::from_str(&stored_json).expect("parse persisted settings");
        assert_eq!(persisted, migrated);
        assert_eq!(
            persisted
                .providers
                .iter()
                .filter(|provider| !provider.enabled && provider.disabled_reason.is_some())
                .count(),
            3
        );
    }

    #[test]
    fn store_turn_lifecycle_and_resumption() {
        let store = Store::in_memory().expect("open in_memory store");
        let thread_id = ThreadId::from("thread-turns-1");
        let mut thread = dummy_thread("thread-turns-1");
        thread.status = ThreadStatus::Working;
        store.save_thread(&thread).expect("save thread");

        // Save Turn
        let turn = Turn {
            id: TurnId::from("turn-101"),
            thread_id: thread_id.clone(),
            environment_id: thread.environment_id.clone(),
            provider_instance_id: thread.provider_instance_id.clone(),
            seq: 1,
            input: TurnInput {
                text: "Build feature".to_string(),
                attachment_ids: vec![],
                attachments: vec![],
                model: Some("fast".to_string()),
                effort: None,
            },
            status: TurnStatus::Running,
            started_at_ms: 1000,
            ended_at_ms: None,
            usage: None,
            checkpoint_before: Some("ref-before".to_string()),
            checkpoint_after: None,
        };
        store.save_turn(&turn).expect("save turn");

        let retrieved = store
            .get_turn(&TurnId::from("turn-101"))
            .expect("get turn")
            .unwrap();
        assert_eq!(retrieved.status, TurnStatus::Running);
        assert_eq!(retrieved.checkpoint_before.as_deref(), Some("ref-before"));
        assert_eq!(retrieved.environment_id, thread.environment_id);
        assert_eq!(retrieved.provider_instance_id, thread.provider_instance_id);

        let audit = ProviderExecutionAudit {
            turn_id: turn.id.clone(),
            thread_id: thread_id.clone(),
            environment_id: thread.environment_id.clone(),
            provider_instance_id: thread.provider_instance_id.clone(),
            provider_kind: ProviderKind::Claude,
            profile_dir: "/profiles/claude/work".to_string(),
            binary_path: Some("/bin/claude-work".to_string()),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_resume: true,
                ..ProviderCapabilities::default()
            },
            model: "claude-3-7-sonnet".to_string(),
            created_at_ms: 1000,
        };
        store
            .record_provider_execution(&audit)
            .expect("record provider audit");
        assert_eq!(
            store
                .get_provider_execution_audit(&turn.id)
                .expect("get provider audit"),
            Some(audit)
        );

        // Save resume token
        store
            .save_resume_token(&thread_id, "token-xyz-123")
            .expect("save resume token");
        let token = store
            .get_resume_token(&thread_id)
            .expect("get resume token");
        assert_eq!(token.as_deref(), Some("token-xyz-123"));
        store
            .delete_resume_token(&thread_id)
            .expect("delete resume token");
        assert_eq!(
            store
                .get_resume_token(&thread_id)
                .expect("get deleted resume token"),
            None
        );

        // Reconcile on simulated restart
        let reconciled_count = store.reconcile_active_turns().expect("reconcile turns");
        assert_eq!(reconciled_count, 1);

        let turn_after = store
            .get_turn(&TurnId::from("turn-101"))
            .expect("get turn")
            .unwrap();
        assert_eq!(turn_after.status, TurnStatus::Interrupted);

        let thread_after = store.get_thread(&thread_id).expect("get thread").unwrap();
        assert_eq!(thread_after.status, ThreadStatus::Idle);
    }

    #[test]
    fn attachment_crud_round_trip() {
        let store = Store::in_memory().expect("open in_memory store");
        let thread_id = ThreadId::from("thread-att-1");
        let thread = dummy_thread("thread-att-1");
        store.save_thread(&thread).expect("save thread");

        let att1 = AttachmentRecord {
            id: "att-1".to_string(),
            thread_id: thread_id.clone(),
            file_name: "screenshot.png".to_string(),
            mime_type: "image/png".to_string(),
            size_bytes: 102400,
            file_path: "/data/attachments/thread-att-1/att-1".to_string(),
            created_at_ms: 1000,
        };

        let att2 = AttachmentRecord {
            id: "att-2".to_string(),
            thread_id: thread_id.clone(),
            file_name: "log.txt".to_string(),
            mime_type: "text/plain".to_string(),
            size_bytes: 2048,
            file_path: "/data/attachments/thread-att-1/att-2".to_string(),
            created_at_ms: 1005,
        };

        store.save_attachment(&att1).expect("save att1");
        store.save_attachment(&att2).expect("save att2");

        let retrieved1 = store
            .get_attachment(&thread_id, "att-1")
            .expect("get att1")
            .expect("att1 exists");
        assert_eq!(retrieved1.file_name, "screenshot.png");
        assert_eq!(retrieved1.size_bytes, 102400);

        let list = store
            .list_attachments(&thread_id)
            .expect("list attachments");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "att-1");
        assert_eq!(list[1].id, "att-2");

        let by_ids = store
            .get_attachments_by_ids(&thread_id, &["att-2".to_string()])
            .expect("get by ids");
        assert_eq!(by_ids.len(), 1);
        assert_eq!(by_ids[0].id, "att-2");
    }

    fn failover_pool(id: &str) -> FailoverPool {
        FailoverPool {
            id: pandamux_core::FailoverPoolId::new(id),
            display_name: "Primary pool".to_string(),
            members: vec![pandamux_core::FailoverPoolMember {
                provider_instance_id: ProviderInstanceId::new("claude-default"),
                subscription_key: SubscriptionKey::new("subkey-work"),
                model: "claude-model".to_string(),
                declared_capabilities: ProviderCapabilities::default(),
                user_preference: 0,
            }],
            requirements: pandamux_core::RoutingCapabilityRequirements::default(),
            revision: 1,
        }
    }

    fn organization_policy(id: &str, project_id: &str) -> OrganizationPolicy {
        OrganizationPolicy {
            id: pandamux_core::OrganizationId::new(id),
            display_name: "Work organization".to_string(),
            project_ids: vec![ProjectId::new(project_id)],
            allowed_provider_instances: vec![ProviderInstanceId::new("claude-default")],
            allowed_subscriptions: vec![SubscriptionKey::new("subkey-work")],
            failover_pool_ids: vec![pandamux_core::FailoverPoolId::new("pool-work")],
            default_tier_mapping: None,
            max_concurrency: Some(2),
            daily_budget_micro_usd: Some(10_000_000),
            broadside: pandamux_core::BroadsidePolicy::default(),
            revision: 1,
        }
    }

    fn configure_policy_mappings(store: &Store) {
        for (key, provider, display_name, provider_instance) in [
            (
                "subkey-work",
                UsageProvider::Claude,
                "Work Claude",
                "claude-default",
            ),
            (
                "subkey-codex",
                UsageProvider::Codex,
                "Work Codex",
                "codex-default",
            ),
        ] {
            store
                .save_usage_subscription_mapping(
                    &UsageSubscription {
                        key: SubscriptionKey::new(key),
                        provider,
                        display_name: display_name.to_string(),
                        masked_account_label: None,
                    },
                    &EnvironmentId::new("env-local"),
                    &ProviderInstanceId::new(provider_instance),
                    999,
                )
                .expect("save policy mapping");
        }
    }

    #[test]
    fn canonical_policy_persistence_is_revisioned_and_fail_closed() {
        let store = Store::in_memory().expect("open store");
        let pool = failover_pool("pool-work");
        store
            .save_failover_pool(&pool, 1000)
            .expect("save failover pool");
        assert_eq!(
            store
                .get_failover_pool(&pool.id)
                .expect("load failover pool"),
            Some(pool.clone())
        );

        let policy = organization_policy("org-work", "proj-one");
        store
            .save_organization_policy(&policy, 1000)
            .expect("save organization");
        assert_eq!(
            store
                .organization_policy_for_project(&ProjectId::new("proj-one"))
                .expect("resolve organization"),
            policy
        );
        assert!(
            store
                .organization_policy_for_project(&ProjectId::new("proj-missing"))
                .is_err()
        );

        let conflicting = organization_policy("org-other", "proj-one");
        assert!(store.save_organization_policy(&conflicting, 1001).is_err());
        assert_eq!(
            store
                .organization_policy_for_project(&ProjectId::new("proj-one"))
                .expect("original assignment remains")
                .id
                .as_str(),
            "org-work"
        );
    }

    #[test]
    fn canonical_policy_updates_require_revision_increments() {
        let store = Store::in_memory().expect("open store");
        let mut pool = failover_pool("pool-work");
        store
            .save_failover_pool(&pool, 1000)
            .expect("save failover pool");
        pool.display_name = "Changed without revision".to_string();
        assert!(store.save_failover_pool(&pool, 1001).is_err());
        pool.revision = 2;
        store
            .save_failover_pool(&pool, 1002)
            .expect("save revised failover pool");

        let mut policy = organization_policy("org-work", "proj-one");
        store
            .save_organization_policy(&policy, 1000)
            .expect("save organization");
        policy.display_name = "Changed without revision".to_string();
        assert!(store.save_organization_policy(&policy, 1001).is_err());
        policy.revision = 2;
        policy.project_ids = vec![ProjectId::new("proj-two")];
        store
            .save_organization_policy(&policy, 1002)
            .expect("save revised organization");
        assert!(
            store
                .organization_policy_for_project(&ProjectId::new("proj-one"))
                .is_err()
        );
        assert_eq!(
            store
                .organization_policy_for_project(&ProjectId::new("proj-two"))
                .expect("new assignment")
                .revision,
            2
        );
    }

    #[test]
    fn settings_persist_canonical_policy_and_route_by_named_subscription() {
        let store = Store::in_memory().expect("open store");
        let subscription = UsageSubscription {
            key: SubscriptionKey::new("subkey-work"),
            provider: UsageProvider::Claude,
            display_name: "Work Claude".to_string(),
            masked_account_label: Some("te***@example.com".to_string()),
        };
        store
            .save_usage_subscription_mapping(
                &subscription,
                &EnvironmentId::new("env-local"),
                &ProviderInstanceId::new("claude-default"),
                1000,
            )
            .expect("save named subscription mapping");

        let settings = UserSettings {
            organizations: vec![organization_policy("org-work", "proj-one")],
            failover_pools: vec![failover_pool("pool-work")],
            ..UserSettings::default()
        };
        store
            .save_settings_with_organization_policies(&settings, 1001)
            .expect("save canonical settings");
        let (policy, key) = store
            .authorize_provider_for_project(
                &ProjectId::new("proj-one"),
                &EnvironmentId::new("env-local"),
                &ProviderInstanceId::new("claude-default"),
            )
            .expect("allowed route");
        assert_eq!(policy.display_name, "Work organization");
        assert_eq!(key, subscription.key);
        assert!(
            store
                .authorize_provider_for_project(
                    &ProjectId::new("proj-one"),
                    &EnvironmentId::new("env-local"),
                    &ProviderInstanceId::new("claude-personal"),
                )
                .is_err()
        );

        let catalog = store
            .list_usage_subscriptions()
            .expect("subscription catalog");
        assert_eq!(catalog[0].display_name, "Work Claude");
        assert_eq!(
            store
                .list_provider_subscription_mappings()
                .expect("mapping catalog")
                .len(),
            1
        );

        let mut wrong_provider = subscription.clone();
        wrong_provider.provider = UsageProvider::Codex;
        let error = store
            .save_usage_subscription_mapping(
                &wrong_provider,
                &EnvironmentId::new("env-local"),
                &ProviderInstanceId::new("codex-work"),
                1002,
            )
            .expect_err("subscription provider identity must be immutable");
        assert!(error.to_string().contains("cannot be reassigned"));
    }

    #[test]
    fn organization_provider_removal_requires_schedule_impact_review() {
        let store = Store::in_memory().expect("open store");
        configure_policy_mappings(&store);
        let mut policy = organization_policy("org-work", "proj-one");
        policy.failover_pool_ids.clear();
        policy
            .allowed_provider_instances
            .push(ProviderInstanceId::new("codex-default"));
        policy
            .allowed_subscriptions
            .push(SubscriptionKey::new("subkey-codex"));
        let settings = UserSettings {
            organizations: vec![policy.clone()],
            ..UserSettings::default()
        };
        store
            .save_settings_with_organization_policies(&settings, 1000)
            .expect("save initial policy");
        store
            .save_schedule(&ScheduleRecord {
                id: ScheduleId::new("schedule-policy-impact"),
                name: "Nightly review".to_string(),
                project_id: Some(ProjectId::new("proj-one")),
                environment: pandamux_core::ScheduleEnvironmentRef {
                    id: EnvironmentId::new("env-local"),
                    display_name: "Local".to_string(),
                },
                trigger: pandamux_core::ScheduleTrigger {
                    cron: "0 0 * * *".to_string(),
                    timezone: "UTC".to_string(),
                },
                target: ScheduleTarget::Session {
                    project_path: "/repo".to_string(),
                    prompt: "Review".to_string(),
                    provider: Some("claude-default".to_string()),
                    model: None,
                    thread_mode: Default::default(),
                },
                approval: Default::default(),
                overlap: Default::default(),
                catch_up: Default::default(),
                budget: Default::default(),
                notify: Vec::new(),
                full_access_opt_in: false,
                enabled: true,
            })
            .expect("save enabled schedule");

        policy.revision = 2;
        policy.allowed_provider_instances = vec![ProviderInstanceId::new("codex-default")];
        policy.allowed_subscriptions = vec![SubscriptionKey::new("subkey-codex")];
        let removed_settings = UserSettings {
            organizations: vec![policy],
            ..settings
        };
        let error = store
            .save_settings_with_organization_policies(&removed_settings, 1001)
            .expect_err("provider removal must be blocked");
        assert!(error.to_string().contains("impact review blocked"));
        assert!(error.to_string().contains("Nightly review"));
        assert!(!error.to_string().contains("claude-default"));
    }

    #[test]
    fn schedule_impact_review_is_scoped_to_the_assigned_organization() {
        let store = Store::in_memory().expect("open store");
        configure_policy_mappings(&store);
        let mut work = organization_policy("org-work", "proj-one");
        work.failover_pool_ids.clear();
        work.allowed_provider_instances
            .push(ProviderInstanceId::new("codex-default"));
        work.allowed_subscriptions
            .push(SubscriptionKey::new("subkey-codex"));
        let mut personal = organization_policy("org-personal", "proj-two");
        personal.failover_pool_ids.clear();
        personal
            .allowed_provider_instances
            .push(ProviderInstanceId::new("codex-default"));
        personal
            .allowed_subscriptions
            .push(SubscriptionKey::new("subkey-codex"));
        let mut settings = UserSettings {
            organizations: vec![work, personal.clone()],
            ..UserSettings::default()
        };
        store
            .save_settings_with_organization_policies(&settings, 1000)
            .expect("save initial policies");
        store
            .save_schedule(&ScheduleRecord {
                id: ScheduleId::new("schedule-work-only"),
                name: "Work review".to_string(),
                project_id: Some(ProjectId::new("proj-one")),
                environment: pandamux_core::ScheduleEnvironmentRef {
                    id: EnvironmentId::new("env-local"),
                    display_name: "Local".to_string(),
                },
                trigger: pandamux_core::ScheduleTrigger {
                    cron: "0 0 * * *".to_string(),
                    timezone: "UTC".to_string(),
                },
                target: ScheduleTarget::Session {
                    project_path: "/repo".to_string(),
                    prompt: "Review".to_string(),
                    provider: Some("claude-default".to_string()),
                    model: None,
                    thread_mode: Default::default(),
                },
                approval: Default::default(),
                overlap: Default::default(),
                catch_up: Default::default(),
                budget: Default::default(),
                notify: Vec::new(),
                full_access_opt_in: false,
                enabled: true,
            })
            .expect("save work schedule");

        personal.revision = 2;
        personal.allowed_provider_instances = vec![ProviderInstanceId::new("codex-default")];
        personal.allowed_subscriptions = vec![SubscriptionKey::new("subkey-codex")];
        settings.organizations[1] = personal;
        store
            .save_settings_with_organization_policies(&settings, 1001)
            .expect("unrelated work schedule must not block personal policy edit");
    }

    fn usage_observation(id: &str, input: u64) -> UsageObservation {
        UsageObservation {
            id: UsageObservationId::new(id),
            source_kind: UsageSourceKind::ClaudeTranscript,
            parser_version: 2,
            provider_native_session_id: Some("session-fixture".to_string()),
            provider_native_turn_id: Some(format!("native-{id}")),
            occurred_at_ms: Some(1_700_000_000_000 + input),
            provider: UsageProvider::Claude,
            provider_instance_id: ProviderInstanceId::new("claude-work"),
            subscription_key: SubscriptionKey::new("subkey-work"),
            model: Some("claude-model".to_string()),
            tokens: TokenCounts {
                input,
                output: 5,
                ..TokenCounts::default()
            },
            unknown_fields: BTreeSet::new(),
            provider_reported_micro_usd: None,
            source_record_count: 1,
            thread_id: None,
            turn_id: None,
            agent_id: None,
            schedule_id: None,
            fact_kind: UsageFactKind::Aggregate,
        }
    }

    fn configure_usage_mapping(store: &Store) {
        store
            .save_usage_subscription(
                &UsageSubscription {
                    key: SubscriptionKey::new("subkey-work"),
                    provider: UsageProvider::Claude,
                    display_name: "Work Claude".to_string(),
                    masked_account_label: Some("p***@example.com".to_string()),
                },
                1000,
            )
            .expect("save subscription");
        store
            .map_provider_subscription(
                &EnvironmentId::new("env-local"),
                &ProviderInstanceId::new("claude-work"),
                &SubscriptionKey::new("subkey-work"),
                1000,
            )
            .expect("map subscription");
        assert!(
            store
                .save_usage_subscription(
                    &UsageSubscription {
                        key: SubscriptionKey::new("subkey-unmasked"),
                        provider: UsageProvider::Claude,
                        display_name: "Unsafe label".to_string(),
                        masked_account_label: Some("person@example.com".to_string()),
                    },
                    1000,
                )
                .is_err()
        );
    }

    fn usage_cursor(store: &Store, offset: u64) -> UsageScannerCursor {
        UsageScannerCursor {
            environment_id: EnvironmentId::new("env-local"),
            provider_instance_id: ProviderInstanceId::new("claude-work"),
            source_kind: UsageSourceKind::ClaudeTranscript,
            node_store_id: store.usage_node_store_id().expect("node store identity"),
            file_identity: "unix:42:99".to_string(),
            canonical_path: "/private/transcript.jsonl".to_string(),
            committed_offset: offset,
            committed_prefix_sha256: "0".repeat(64),
            parser_version: 2,
            parser_state_json: "{}".to_string(),
            observed_size: offset,
            observed_modified_at_ms: Some(2000),
            updated_at_ms: 3000,
        }
    }

    fn save_managed_usage_fixture(store: &Store) -> Turn {
        let mut thread = dummy_thread("thread-managed");
        thread.environment_id = EnvironmentId::new("env-local");
        thread.provider_instance_id = ProviderInstanceId::new("claude-work");
        thread.model = "claude-model".to_string();
        store.save_thread(&thread).expect("save managed thread");
        let turn = Turn {
            id: TurnId::new("turn-managed"),
            thread_id: thread.id.clone(),
            environment_id: thread.environment_id.clone(),
            provider_instance_id: thread.provider_instance_id.clone(),
            seq: 1,
            input: TurnInput {
                text: "fixture".to_string(),
                ..TurnInput::default()
            },
            status: TurnStatus::Completed,
            started_at_ms: 900,
            ended_at_ms: Some(1_000),
            usage: Some(TurnUsage {
                input_tokens: 10,
                output_tokens: 5,
                provenance: Some(TurnUsageProvenance {
                    provider_native_session_id: Some("session-fixture".to_string()),
                    provider_native_turn_id: Some("native-managed".to_string()),
                    occurred_at_ms: Some(1_000),
                    model: Some("claude-model".to_string()),
                }),
                ..TurnUsage::default()
            }),
            checkpoint_before: None,
            checkpoint_after: None,
        };
        store.save_turn(&turn).expect("save managed turn");
        store
            .record_provider_execution(&ProviderExecutionAudit {
                turn_id: turn.id.clone(),
                thread_id: thread.id,
                environment_id: thread.environment_id,
                provider_instance_id: thread.provider_instance_id,
                provider_kind: ProviderKind::Claude,
                profile_dir: "/profiles/claude/work".to_string(),
                binary_path: None,
                capabilities: ProviderCapabilities::default(),
                model: "claude-model".to_string(),
                created_at_ms: 900,
            })
            .expect("save provider execution");
        turn
    }

    fn usage_component(
        message_id: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        occurred_at_ms: u64,
    ) -> TurnUsageComponent {
        TurnUsageComponent {
            provider_native_message_id: message_id.to_string(),
            model: Some(model.to_string()),
            input_tokens,
            output_tokens,
            occurred_at_ms: Some(occurred_at_ms),
            ..TurnUsageComponent::default()
        }
    }

    #[test]
    fn component_backfill_adds_missing_messages_and_separate_cost_across_restart() {
        let file = tempfile::NamedTempFile::new().expect("temporary component database");
        let path = file.path().to_path_buf();
        {
            let store = Store::open(&path).expect("open component store");
            configure_usage_mapping(&store);
            let mut turn = save_managed_usage_fixture(&store);
            turn.usage = Some(TurnUsage {
                input_tokens: 10,
                output_tokens: 5,
                provenance: Some(TurnUsageProvenance {
                    provider_native_session_id: Some("session-components".to_string()),
                    provider_native_turn_id: Some("result-uuid".to_string()),
                    occurred_at_ms: Some(1_000),
                    model: Some("selected-routing-model".to_string()),
                }),
                components: vec![usage_component(
                    "message-one",
                    "claude-actual-one",
                    10,
                    5,
                    901,
                )],
                ..TurnUsage::default()
            });
            store.save_turn(&turn).expect("save first component");

            let first = store
                .backfill_managed_turn_usage(&EnvironmentId::new("env-local"), 100)
                .expect("backfill first component");
            assert_eq!(first.batch.inserted, 1);

            let usage = turn.usage.as_mut().expect("component usage");
            usage.input_tokens = 30;
            usage.output_tokens = 12;
            usage.cost_usd = Some(0.001234);
            usage.provider_reported_micro_usd = Some(1_234);
            let mut second = usage_component("message-two", "claude-actual-two", 20, 7, 902);
            second.cache_read_tokens = Some(3);
            second.cache_write_tokens = Some(2);
            usage.components.push(second);
            store
                .save_turn(&turn)
                .expect("save completed component set");

            let completed = store
                .backfill_managed_turn_usage(&EnvironmentId::new("env-local"), 100)
                .expect("backfill completed component set");
            assert_eq!(completed.batch.inserted, 2);
            assert_eq!(completed.batch.duplicates, 1);
        }

        let store = Store::open(&path).expect("reopen component store");
        let query = store
            .query_usage(&UsageQueryParams::default())
            .expect("query component usage after restart");
        assert_eq!(query.items.len(), 3);
        assert_eq!(query.aggregate.tokens.input, 30);
        assert_eq!(query.aggregate.tokens.output, 12);
        assert_eq!(query.aggregate.tokens.cached_input, 3);
        assert_eq!(query.aggregate.tokens.cache_write, 2);
        assert_eq!(query.aggregate.provider_reported_micro_usd, Some(1_234));
        assert_eq!(
            query
                .items
                .iter()
                .filter(|item| item.observation.fact_kind == UsageFactKind::TokenComponent)
                .count(),
            2
        );
        assert_eq!(
            query
                .items
                .iter()
                .filter(|item| item.observation.fact_kind == UsageFactKind::TurnCost)
                .count(),
            1
        );
        assert!(
            query.items.iter().all(|item| {
                item.observation.model.as_deref() != Some("selected-routing-model")
            })
        );
        assert_eq!(
            store
                .export_usage(&UsageExportParams::default())
                .expect("export component facts")
                .items
                .len(),
            3
        );
        let mut decoy_aggregate = query
            .items
            .iter()
            .find(|item| item.observation.fact_kind == UsageFactKind::TokenComponent)
            .expect("token component for decoy aggregate")
            .observation
            .clone();
        decoy_aggregate.id = UsageObservationId::new("decoy-legacy-aggregate");
        decoy_aggregate.fact_kind = UsageFactKind::Aggregate;
        decoy_aggregate.provider_native_session_id = None;
        decoy_aggregate.provider_native_turn_id = None;
        store
            .record_usage_batch(&EnvironmentId::new("env-local"), &[decoy_aggregate], None)
            .expect("record decoy legacy aggregate");
        store
            .conn
            .lock()
            .expect("component store lock")
            .execute(
                "DELETE FROM usage_observations
                  WHERE provider_native_turn_id = 'message-two'",
                [],
            )
            .expect("remove one component fixture");
        let repaired = store
            .backfill_managed_turn_usage(&EnvironmentId::new("env-local"), 100)
            .expect("repair missing completed component");
        assert_eq!(repaired.batch.inserted, 1);
        assert_eq!(repaired.batch.duplicates, 2);
        let replay = store
            .backfill_managed_turn_usage(&EnvironmentId::new("env-local"), 100)
            .expect("repeat completed component backfill");
        assert_eq!(replay.batch, UsageBatchResult::default());
    }

    #[test]
    fn usage_batch_is_atomic_idempotent_and_collision_safe() {
        let store = Store::in_memory().expect("open store");
        configure_usage_mapping(&store);
        let environment_id = EnvironmentId::new("env-local");
        let observation = usage_observation("usage-one", 10);

        let inserted = store
            .record_usage_batch(
                &environment_id,
                std::slice::from_ref(&observation),
                Some(&usage_cursor(&store, 100)),
            )
            .expect("record usage");
        assert_eq!(inserted.inserted, 1);
        assert_eq!(inserted.duplicates, 0);
        assert_eq!(inserted.last_outbox_seq, 1);
        assert_eq!(
            store
                .get_usage_scanner_cursor(
                    &environment_id,
                    &ProviderInstanceId::new("claude-work"),
                    &UsageSourceKind::ClaudeTranscript,
                    "unix:42:99",
                )
                .expect("load cursor")
                .expect("cursor")
                .committed_offset,
            100
        );

        let duplicate = store
            .record_usage_batch(
                &environment_id,
                std::slice::from_ref(&observation),
                Some(&usage_cursor(&store, 120)),
            )
            .expect("replay usage");
        assert_eq!(duplicate.inserted, 0);
        assert_eq!(duplicate.duplicates, 1);
        assert_eq!(
            store
                .export_usage(&UsageExportParams::default())
                .expect("export usage")
                .items
                .len(),
            1
        );

        let mut collision = observation;
        collision.tokens.input = 999;
        assert!(
            store
                .record_usage_batch(
                    &environment_id,
                    &[collision],
                    Some(&usage_cursor(&store, 140)),
                )
                .is_err()
        );
        assert_eq!(
            store
                .get_usage_scanner_cursor(
                    &environment_id,
                    &ProviderInstanceId::new("claude-work"),
                    &UsageSourceKind::ClaudeTranscript,
                    "unix:42:99",
                )
                .expect("load cursor")
                .expect("cursor")
                .committed_offset,
            120
        );

        let mut wrong_provider = usage_observation("usage-wrong-provider", 10);
        wrong_provider.provider = UsageProvider::Codex;
        assert!(
            store
                .record_usage_batch(&environment_id, &[wrong_provider], None)
                .is_err()
        );
    }

    #[test]
    fn failed_usage_insert_does_not_advance_cursor() {
        let store = Store::in_memory().expect("open store");
        let environment_id = EnvironmentId::new("env-local");
        let observation = usage_observation("usage-unmapped", 10);
        assert!(
            store
                .record_usage_batch(
                    &environment_id,
                    &[observation],
                    Some(&usage_cursor(&store, 100)),
                )
                .is_err()
        );
        assert!(
            store
                .get_usage_scanner_cursor(
                    &environment_id,
                    &ProviderInstanceId::new("claude-work"),
                    &UsageSourceKind::ClaudeTranscript,
                    "unix:42:99",
                )
                .expect("load cursor")
                .is_none()
        );
        assert!(
            store
                .export_usage(&UsageExportParams::default())
                .expect("export usage")
                .items
                .is_empty()
        );
    }

    #[test]
    fn injected_scan_transaction_failure_restarts_without_loss_or_duplicates() {
        let file = tempfile::NamedTempFile::new().expect("temporary scanner database");
        let path = file.path().to_path_buf();
        let environment_id = EnvironmentId::new("env-local");
        let observation = usage_observation("usage-restart", 10);
        let quarantine = UsageQuarantineRecord {
            environment_id: environment_id.clone(),
            provider_instance_id: ProviderInstanceId::new("claude-work"),
            source_kind: UsageSourceKind::ClaudeTranscript,
            file_identity_sha256: "1".repeat(64),
            record_offset: 50,
            parser_version: 2,
            reason_code: "invalid_json".to_string(),
            reason_detail: "complete JSONL record failed provider parsing".to_string(),
            safe_excerpt_sha256: "2".repeat(64),
            created_at_ms: 3_000,
        };
        let diagnostic = UsageScannerDiagnostic {
            environment_id: environment_id.clone(),
            provider_instance_id: ProviderInstanceId::new("claude-work"),
            source_kind: UsageSourceKind::ClaudeTranscript,
            coverage_state: UsageSourceCoverageState::Partial,
            parser_version: 2,
            last_scan_started_at_ms: 2_000,
            last_success_at_ms: Some(3_000),
            latest_source_at_ms: Some(1_000),
            source_lag_ms: Some(2_000),
            files_scanned: 1,
            bytes_scanned: 100,
            quarantined_count: 0,
            last_safe_error_code: Some("invalid_json".to_string()),
        };
        {
            let store = Store::open(&path).expect("open scanner store");
            configure_usage_mapping(&store);
            let cursor = usage_cursor(&store, 100);
            assert!(
                store
                    .record_usage_transaction(
                        &environment_id,
                        std::slice::from_ref(&observation),
                        Some(&cursor),
                        std::slice::from_ref(&quarantine),
                        Some(&diagnostic),
                        Some(1),
                    )
                    .is_err()
            );
        }

        let store = Store::open(&path).expect("reopen scanner store");
        let cursor = usage_cursor(&store, 100);
        assert!(
            store
                .query_usage(&UsageQueryParams::default())
                .expect("query rolled back usage")
                .items
                .is_empty()
        );
        assert!(
            store
                .get_usage_scanner_cursor(
                    &environment_id,
                    &ProviderInstanceId::new("claude-work"),
                    &UsageSourceKind::ClaudeTranscript,
                    "unix:42:99",
                )
                .expect("query rolled back cursor")
                .is_none()
        );

        let retry = store
            .record_usage_scan_batch(
                &environment_id,
                std::slice::from_ref(&observation),
                Some(&cursor),
                std::slice::from_ref(&quarantine),
                Some(&diagnostic),
            )
            .expect("retry scan transaction");
        assert_eq!(retry.inserted, 1);
        assert_eq!(retry.quarantined, 1);
        let replay = store
            .record_usage_scan_batch(
                &environment_id,
                std::slice::from_ref(&observation),
                Some(&cursor),
                std::slice::from_ref(&quarantine),
                Some(&diagnostic),
            )
            .expect("replay scan transaction");
        assert_eq!(replay.inserted, 0);
        assert_eq!(replay.duplicates, 1);
        assert_eq!(replay.quarantined, 0);
        let query = store
            .query_usage(&UsageQueryParams::default())
            .expect("query retried usage");
        assert_eq!(query.items.len(), 1);
        assert_eq!(query.coverage.quarantined_count, 1);
        assert_eq!(query.coverage.scanner_diagnostics.len(), 1);
    }

    #[test]
    fn scanner_quarantine_retention_is_durably_bounded() {
        let store = Store::in_memory().expect("open store");
        let environment_id = EnvironmentId::new("env-local");
        let total = u64::try_from(MAX_RETAINED_SCANNER_QUARANTINES_PER_SOURCE)
            .expect("positive quarantine limit")
            + 3;
        let quarantines = (0..total)
            .map(|record_offset| UsageQuarantineRecord {
                environment_id: environment_id.clone(),
                provider_instance_id: ProviderInstanceId::new("claude-work"),
                source_kind: UsageSourceKind::ClaudeTranscript,
                file_identity_sha256: "1".repeat(64),
                record_offset,
                parser_version: 2,
                reason_code: "invalid_json".to_string(),
                reason_detail: "complete JSONL record failed provider parsing".to_string(),
                safe_excerpt_sha256: format!("{record_offset:064x}"),
                created_at_ms: record_offset,
            })
            .collect::<Vec<_>>();
        let result = store
            .record_usage_scan_batch(&environment_id, &[], None, &quarantines, None)
            .expect("record bounded scanner quarantines");
        assert_eq!(result.quarantined, total);

        let retained = store
            .conn
            .lock()
            .expect("store lock")
            .query_row(
                "SELECT COUNT(*) FROM usage_quarantine WHERE legacy_bucket_id IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("retained quarantine count");
        assert_eq!(retained, MAX_RETAINED_SCANNER_QUARANTINES_PER_SOURCE);
    }

    #[test]
    fn managed_turn_is_authoritative_when_transcript_repeats_it() {
        let store = Store::in_memory().expect("open store");
        configure_usage_mapping(&store);
        let turn = save_managed_usage_fixture(&store);

        let managed = store
            .backfill_managed_turn_usage(&EnvironmentId::new("env-local"), 100)
            .expect("backfill managed usage");
        assert_eq!(managed.batch.inserted, 1);

        let mut transcript = usage_observation("usage-transcript-copy", 10);
        transcript.provider_native_turn_id = Some("native-managed".to_string());
        transcript.occurred_at_ms = Some(1_000);
        let reconciled = store
            .record_usage_batch(&EnvironmentId::new("env-local"), &[transcript], None)
            .expect("reconcile transcript copy");
        assert_eq!(reconciled.inserted, 0);
        assert_eq!(reconciled.reconciled, 1);
        let query = store
            .query_usage(&UsageQueryParams::default())
            .expect("query reconciled usage");
        assert_eq!(query.items.len(), 1);
        assert_eq!(
            query.items[0].observation.source_kind,
            UsageSourceKind::ManagedTurn
        );
        assert_eq!(query.items[0].observation.turn_id.as_ref(), Some(&turn.id));
        assert_eq!(
            store
                .backfill_managed_turn_usage(&EnvironmentId::new("env-local"), 100)
                .expect("repeat managed backfill")
                .batch
                .inserted,
            0
        );
    }

    #[test]
    fn managed_usage_uses_the_completed_retry_attempt_identity() {
        let store = Store::in_memory().expect("open store");
        configure_usage_mapping(&store);
        store
            .save_usage_subscription_mapping(
                &UsageSubscription {
                    key: SubscriptionKey::new("subkey-fallback"),
                    provider: UsageProvider::Codex,
                    display_name: "Fallback Codex".to_string(),
                    masked_account_label: None,
                },
                &EnvironmentId::new("env-local"),
                &ProviderInstanceId::new("codex-fallback"),
                1_000,
            )
            .expect("save fallback mapping");
        let mut turn = save_managed_usage_fixture(&store);
        turn.usage
            .as_mut()
            .and_then(|usage| usage.provenance.as_mut())
            .expect("managed provenance")
            .model = None;
        store.save_turn(&turn).expect("clear reported model");
        store
            .save_turn_attempt(&TurnAttempt {
                id: TurnAttemptId::new("attempt-managed-retry"),
                turn_id: turn.id.clone(),
                thread_id: turn.thread_id.clone(),
                ordinal: 2,
                environment_id: turn.environment_id.clone(),
                provider_instance_id: ProviderInstanceId::new("codex-fallback"),
                subscription_key: Some(SubscriptionKey::new("subkey-fallback")),
                model: "codex-retry-model".to_string(),
                boundary: AttemptBoundary::VisibleOutputBegun,
                status: TurnAttemptStatus::Completed,
                routing: RoutingDecision::default(),
                execution: Some(AttemptExecutionEvidence {
                    provider_kind: ProviderKind::Codex,
                    profile_dir: "/profiles/codex/fallback".to_string(),
                    binary_path: Some("/bin/codex".to_string()),
                    capabilities: ProviderCapabilities::default(),
                    provider_version: Some("fixture-2".to_string()),
                }),
                checkpoint: None,
                partial_output: String::new(),
                capacity_evidence: None,
                started_at_ms: 950,
                ended_at_ms: Some(1_000),
            })
            .expect("save completed retry attempt");

        let result = store
            .backfill_managed_turn_usage(&EnvironmentId::new("env-local"), 100)
            .expect("backfill retried managed usage");
        assert_eq!(result.batch.inserted, 1);
        let observation = &store
            .query_usage(&UsageQueryParams::default())
            .expect("query managed usage")
            .items[0]
            .observation;
        assert_eq!(observation.provider, UsageProvider::Codex);
        assert_eq!(observation.provider_instance_id.as_str(), "codex-fallback");
        assert_eq!(observation.subscription_key.as_str(), "subkey-fallback");
        assert_eq!(observation.model.as_deref(), Some("codex-retry-model"));
    }

    #[test]
    fn managed_turn_promotes_an_earlier_transcript_copy() {
        let store = Store::in_memory().expect("open store");
        configure_usage_mapping(&store);
        let turn = save_managed_usage_fixture(&store);

        let mut transcript = usage_observation("usage-transcript-first", 10);
        transcript.provider_native_turn_id = Some("native-managed".to_string());
        transcript.occurred_at_ms = Some(1_000);
        store
            .record_usage_batch(&EnvironmentId::new("env-local"), &[transcript], None)
            .expect("record transcript before managed fact");

        let managed = store
            .backfill_managed_turn_usage(&EnvironmentId::new("env-local"), 100)
            .expect("promote transcript to managed fact");
        assert_eq!(managed.batch.inserted, 0);
        assert_eq!(managed.batch.reconciled, 1);

        let query = store
            .query_usage(&UsageQueryParams::default())
            .expect("query promoted usage");
        assert_eq!(query.items.len(), 1);
        assert_eq!(
            query.items[0].observation.source_kind,
            UsageSourceKind::ManagedTurn
        );
        assert_eq!(query.items[0].observation.turn_id.as_ref(), Some(&turn.id));

        let exported = store
            .export_usage(&UsageExportParams::default())
            .expect("export transcript and managed revisions");
        assert_eq!(exported.items.len(), 2);
        assert_eq!(exported.items[0].observation, exported.items[1].observation);
        assert_eq!(
            exported.items[1].observation.source_kind,
            UsageSourceKind::ManagedTurn
        );
    }

    fn exact_component_observation(
        id: &str,
        source_kind: UsageSourceKind,
        message_id: &str,
        occurred_at_ms: u64,
    ) -> UsageObservation {
        let mut observation = usage_observation(id, 10);
        observation.source_kind = source_kind;
        observation.provider_native_turn_id = Some(message_id.to_string());
        observation.occurred_at_ms = Some(occurred_at_ms);
        observation.fact_kind = UsageFactKind::TokenComponent;
        observation
    }

    #[test]
    fn exact_component_reconciliation_is_order_independent_and_keeps_conflicts() {
        for transcript_first in [false, true] {
            let store = Store::in_memory().expect("open reconciliation store");
            configure_usage_mapping(&store);
            let environment_id = EnvironmentId::new("env-local");
            let managed = exact_component_observation(
                "managed-component",
                UsageSourceKind::ManagedTurn,
                "message-shared",
                1_000,
            );
            let mut transcript = exact_component_observation(
                "transcript-component",
                UsageSourceKind::ClaudeTranscript,
                "message-shared",
                1_125,
            );
            transcript.provider_reported_micro_usd = Some(77);

            let ordered = if transcript_first {
                vec![transcript, managed]
            } else {
                vec![managed, transcript]
            };
            store
                .record_usage_batch(&environment_id, &ordered[..1], None)
                .expect("record first component source");
            let reconciled = store
                .record_usage_batch(&environment_id, &ordered[1..], None)
                .expect("reconcile second component source");
            assert_eq!(reconciled.reconciled, 1);
            assert_eq!(reconciled.reconciliation_mismatches, 0);

            let query = store
                .query_usage(&UsageQueryParams::default())
                .expect("query reconciled component");
            assert_eq!(query.items.len(), 1);
            assert_eq!(
                query.items[0].observation.source_kind,
                UsageSourceKind::ManagedTurn
            );
            assert_eq!(
                query.items[0].observation.provider_reported_micro_usd,
                Some(77)
            );
        }

        for transcript_first in [false, true] {
            let store = Store::in_memory().expect("open mismatch store");
            configure_usage_mapping(&store);
            let environment_id = EnvironmentId::new("env-local");
            for (suffix, mutate) in ["cost", "model", "tokens"].into_iter().enumerate() {
                let message_id = format!("message-conflict-{mutate}");
                let mut managed = exact_component_observation(
                    &format!("managed-{suffix}"),
                    UsageSourceKind::ManagedTurn,
                    &message_id,
                    2_000 + suffix as u64,
                );
                let mut transcript = exact_component_observation(
                    &format!("transcript-{suffix}"),
                    UsageSourceKind::ClaudeTranscript,
                    &message_id,
                    2_100 + suffix as u64,
                );
                match mutate {
                    "cost" => {
                        managed.provider_reported_micro_usd = Some(10);
                        transcript.provider_reported_micro_usd = Some(11);
                    }
                    "model" => transcript.model = Some("claude-conflicting".to_string()),
                    "tokens" => transcript.tokens.output += 1,
                    _ => unreachable!(),
                }

                let ordered = if transcript_first {
                    vec![transcript, managed]
                } else {
                    vec![managed, transcript]
                };
                store
                    .record_usage_batch(&environment_id, &ordered[..1], None)
                    .expect("record mismatch baseline");
                let mismatch = store
                    .record_usage_batch(&environment_id, &ordered[1..], None)
                    .expect("retain mismatched component");
                assert_eq!(mismatch.inserted, 1);
                assert_eq!(mismatch.reconciliation_mismatches, 1);
            }

            let clean_diagnostic = UsageScannerDiagnostic {
                environment_id: environment_id.clone(),
                provider_instance_id: ProviderInstanceId::new("claude-work"),
                source_kind: UsageSourceKind::ClaudeTranscript,
                coverage_state: UsageSourceCoverageState::Complete,
                parser_version: 2,
                last_scan_started_at_ms: 3_000,
                last_success_at_ms: Some(3_001),
                latest_source_at_ms: Some(2_102),
                source_lag_ms: Some(899),
                files_scanned: 1,
                bytes_scanned: 100,
                quarantined_count: 0,
                last_safe_error_code: None,
            };
            store
                .record_usage_scan_batch(&environment_id, &[], None, &[], Some(&clean_diagnostic))
                .expect("record later clean scanner pass");
            let query = store
                .query_usage(&UsageQueryParams::default())
                .expect("query visible mismatches");
            assert_eq!(query.items.len(), 6);
            assert_eq!(query.coverage.scanner_diagnostics.len(), 1);
            assert_eq!(
                query.coverage.scanner_diagnostics[0].coverage_state,
                UsageSourceCoverageState::Partial
            );
            assert_eq!(
                query.coverage.scanner_diagnostics[0]
                    .last_safe_error_code
                    .as_deref(),
                Some("managed_transcript_mismatch")
            );
        }

        let store = Store::in_memory().expect("open distinct component store");
        configure_usage_mapping(&store);
        let environment_id = EnvironmentId::new("env-local");
        let first = exact_component_observation(
            "distinct-one",
            UsageSourceKind::ClaudeTranscript,
            "message-distinct-one",
            3_000,
        );
        let second = exact_component_observation(
            "distinct-two",
            UsageSourceKind::ClaudeTranscript,
            "message-distinct-two",
            3_001,
        );
        let distinct = store
            .record_usage_batch(&environment_id, &[first, second], None)
            .expect("record distinct equal measurements");
        assert_eq!(distinct.inserted, 2);
        assert_eq!(distinct.reconciled, 0);
        assert_eq!(distinct.reconciliation_mismatches, 0);

        let mut legacy = exact_component_observation(
            "legacy-parser-component",
            UsageSourceKind::ClaudeTranscript,
            "message-legacy-parser",
            4_000,
        );
        legacy.parser_version = 1;
        legacy.unknown_fields.clear();
        store
            .record_usage_batch(&environment_id, std::slice::from_ref(&legacy), None)
            .expect("record parser-one component");
        let mut rescanned = legacy;
        rescanned.parser_version = 2;
        rescanned
            .unknown_fields
            .insert(UsageUnknownField::CacheReadTokens);
        rescanned
            .unknown_fields
            .insert(UsageUnknownField::CacheWriteTokens);
        let replay = store
            .record_usage_batch(&environment_id, &[rescanned], None)
            .expect("accept parser-two cache-presence enrichment");
        assert_eq!(replay.duplicates, 1);
    }

    #[test]
    fn usage_export_ack_and_filtered_query_are_bounded() {
        let store = Store::in_memory().expect("open store");
        configure_usage_mapping(&store);
        let environment_id = EnvironmentId::new("env-local");
        store
            .record_usage_batch(
                &environment_id,
                &[
                    usage_observation("usage-one", 10),
                    usage_observation("usage-two", 20),
                ],
                None,
            )
            .expect("record usage");

        let first = store
            .export_usage(&UsageExportParams {
                after_seq: 0,
                limit: 1,
            })
            .expect("first page");
        assert_eq!(first.items.len(), 1);
        assert!(first.has_more);
        assert_eq!(first.next_after_seq, 1);
        assert!(first.node_store_id.as_str().starts_with("store-"));

        let second = store
            .export_usage(&UsageExportParams {
                after_seq: first.next_after_seq,
                limit: 1,
            })
            .expect("second page");
        assert_eq!(second.items.len(), 1);
        assert!(!second.has_more);
        assert_eq!(store.acknowledge_usage(1).expect("ack usage"), 1);
        assert_eq!(store.acknowledge_usage(1).expect("repeat ack"), 0);

        let query = store
            .query_usage(&UsageQueryParams {
                environment_id: Some(environment_id),
                subscription_key: Some(SubscriptionKey::new("subkey-work")),
                limit: 1,
                ..UsageQueryParams::default()
            })
            .expect("query usage");
        assert_eq!(query.items.len(), 1);
        assert!(query.has_more);
        assert!(query.next.is_some());
        assert_eq!(query.coverage.included_environments.len(), 1);
    }

    #[test]
    fn capacity_query_uses_stable_bounded_cursor() {
        let store = Store::in_memory().expect("open store");
        configure_usage_mapping(&store);
        for (as_of_ms, remaining) in [(1000, 750_000), (2000, 500_000)] {
            let snapshot = CapacitySnapshot {
                schema_version: pandamux_core::CAPACITY_SNAPSHOT_SCHEMA_VERSION,
                subscription_key: SubscriptionKey::new("subkey-work"),
                environment_id: Some(EnvironmentId::new("env-local")),
                provider_instance_id: Some(ProviderInstanceId::new("claude-default")),
                window_name: Some("primary".to_string()),
                window_start_ms: None,
                window_end_ms: None,
                used_millionths: Some(1_000_000 - remaining),
                remaining_millionths: Some(remaining),
                reset_at_ms: None,
                source: "provider_report".to_string(),
                confidence: CapacityConfidence::Reported,
                evidence_scope: CapacityEvidenceScope::Local,
                hard_limit: false,
                provider_version: Some("fixture-1".to_string()),
                as_of_ms,
                max_age_ms: pandamux_core::DEFAULT_CAPACITY_MAX_AGE_MS,
            };
            store
                .save_capacity_snapshot(&snapshot)
                .expect("save capacity snapshot");
            store
                .save_capacity_snapshot(&snapshot)
                .expect("replay capacity snapshot");
        }

        assert!(
            store
                .save_capacity_snapshot(&CapacitySnapshot {
                    schema_version: pandamux_core::CAPACITY_SNAPSHOT_SCHEMA_VERSION,
                    subscription_key: SubscriptionKey::new("subkey-work"),
                    environment_id: Some(EnvironmentId::new("env-local")),
                    provider_instance_id: Some(ProviderInstanceId::new("claude-default")),
                    window_name: Some("primary".to_string()),
                    window_start_ms: Some(900),
                    window_end_ms: Some(1_100),
                    used_millionths: Some(900_000),
                    remaining_millionths: Some(100_000),
                    reset_at_ms: None,
                    source: "provider_report".to_string(),
                    confidence: CapacityConfidence::Reported,
                    evidence_scope: CapacityEvidenceScope::Local,
                    hard_limit: false,
                    provider_version: Some("fixture-1".to_string()),
                    as_of_ms: 1000,
                    max_age_ms: pandamux_core::DEFAULT_CAPACITY_MAX_AGE_MS,
                })
                .is_err()
        );

        let first = store
            .query_capacity(&CapacityQueryParams {
                subscription_key: Some(SubscriptionKey::new("subkey-work")),
                environment_id: None,
                at_ms: Some(2_000),
                after: None,
                limit: 1,
            })
            .expect("first capacity page");
        assert_eq!(first.snapshots.len(), 1);
        assert!(first.has_more);

        let second = store
            .query_capacity(&CapacityQueryParams {
                subscription_key: Some(SubscriptionKey::new("subkey-work")),
                environment_id: None,
                at_ms: Some(2_000),
                after: first.next,
                limit: 1,
            })
            .expect("second capacity page");
        assert_eq!(second.snapshots.len(), 1);
        assert!(!second.has_more);
        assert_eq!(second.snapshots[0].as_of_ms, 2000);
    }
}
