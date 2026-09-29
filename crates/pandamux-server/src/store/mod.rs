pub mod migrations;

use pandamux_core::{
    AgentId, AttachmentRecord, CapacityConfidence, CapacitySnapshot, EnvironmentId, FailoverPool,
    NodeStoreId, OrganizationPolicy, ProjectId, ProviderCapabilities, ProviderInstanceId,
    ProviderKind, ScheduleId, ScheduleRecord, SubscriptionKey, Thread, ThreadEvent,
    ThreadEventKind, ThreadId, TokenCounts, Turn, TurnId, TurnStatus, UsageObservation,
    UsageObservationId, UsageProvider, UsageSourceKind, UsageSubscription, UserSettings,
};
use pandamux_protocol::{
    CapacityQueryCursor, CapacityQueryParams, CapacityQueryResult, EventEnvelope, UsageExportItem,
    UsageExportParams, UsageExportResult, UsageQueryCursor, UsageQueryItem, UsageQueryParams,
    UsageQueryResult,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

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
    pub source_kind: UsageSourceKind,
    pub file_identity: String,
    pub canonical_path: String,
    pub committed_offset: u64,
    pub parser_version: u32,
    pub observed_size: u64,
    pub observed_modified_at_ms: Option<u64>,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageBatchResult {
    pub inserted: u64,
    pub duplicates: u64,
    pub last_outbox_seq: u64,
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
        conn.execute(
            "INSERT INTO usage_subscriptions (
                subscription_key, provider, display_name, masked_account_label,
                created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(subscription_key) DO UPDATE SET
                provider = excluded.provider,
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

    pub fn get_usage_scanner_cursor(
        &self,
        source_kind: &UsageSourceKind,
        file_identity: &str,
    ) -> Result<Option<UsageScannerCursor>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT source_kind, file_identity, canonical_path, committed_offset,
                        parser_version, observed_size, observed_modified_at_ms, updated_at_ms
                 FROM usage_scanner_cursors
                 WHERE source_kind = ?1 AND file_identity = ?2",
                params![source_kind.as_str(), file_identity],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .optional()?;
        row.map(|row| {
            Ok(UsageScannerCursor {
                source_kind: UsageSourceKind::from(row.0),
                file_identity: row.1,
                canonical_path: row.2,
                committed_offset: checked_u64(row.3, "usage_scanner_cursors.committed_offset")?,
                parser_version: checked_u32(row.4, "usage_scanner_cursors.parser_version")?,
                observed_size: checked_u64(row.5, "usage_scanner_cursors.observed_size")?,
                observed_modified_at_ms: row
                    .6
                    .map(|value| {
                        checked_u64(value, "usage_scanner_cursors.observed_modified_at_ms")
                    })
                    .transpose()?,
                updated_at_ms: checked_u64(row.7, "usage_scanner_cursors.updated_at_ms")?,
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
        if environment_id.as_str().trim().is_empty() {
            return Err(StoreError::Data(
                "authenticated environment id must not be empty".to_string(),
            ));
        }
        if let Some(cursor) = cursor {
            validate_usage_cursor(cursor)?;
        }
        for observation in observations {
            observation
                .validate()
                .map_err(|error| StoreError::Data(error.to_string()))?;
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut result = UsageBatchResult::default();
        let created_at_ms = checked_i64(store_now_ms(), "usage_observations.created_at_ms")?;

        for observation in observations {
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

            let other_tokens_json = serde_json::to_string(&observation.tokens.other)?;
            let inserted = tx.execute(
                "INSERT INTO usage_observations (
                    environment_id, observation_id, source_kind, parser_version,
                    provider_native_session_id, provider_native_turn_id, occurred_at_ms,
                    provider, provider_instance_id, subscription_key, model,
                    input_tokens, cached_input_tokens, cache_write_tokens, output_tokens,
                    reasoning_tokens, other_tokens_json, provider_reported_micro_usd,
                    source_record_count, thread_id, agent_id, schedule_id, created_at_ms
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23
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
                    optional_checked_i64(
                        observation.provider_reported_micro_usd,
                        "provider_reported_micro_usd"
                    )?,
                    checked_i64(observation.source_record_count, "source_record_count")?,
                    observation.thread_id.as_ref().map(ThreadId::as_str),
                    observation.agent_id.as_ref().map(|id| id.as_str()),
                    observation.schedule_id.as_ref().map(ScheduleId::as_str),
                    created_at_ms,
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
                if existing != *observation {
                    return Err(StoreError::Data(format!(
                        "usage observation id collision for {}",
                        observation.id
                    )));
                }
                result.duplicates = result.duplicates.checked_add(1).ok_or_else(|| {
                    StoreError::Data("usage duplicate count overflow".to_string())
                })?;
                continue;
            }

            tx.execute(
                "INSERT INTO usage_outbox (environment_id, observation_id, enqueued_at_ms)
                 VALUES (?1, ?2, ?3)",
                params![
                    environment_id.as_str(),
                    observation.id.as_str(),
                    created_at_ms
                ],
            )?;
            result.inserted = result
                .inserted
                .checked_add(1)
                .ok_or_else(|| StoreError::Data("usage insert count overflow".to_string()))?;
            result.last_outbox_seq = checked_u64(tx.last_insert_rowid(), "usage_outbox.seq")?;
        }

        if let Some(cursor) = cursor {
            tx.execute(
                "INSERT INTO usage_scanner_cursors (
                    source_kind, file_identity, canonical_path, committed_offset,
                    parser_version, observed_size, observed_modified_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(source_kind, file_identity) DO UPDATE SET
                    canonical_path = excluded.canonical_path,
                    committed_offset = excluded.committed_offset,
                    parser_version = excluded.parser_version,
                    observed_size = excluded.observed_size,
                    observed_modified_at_ms = excluded.observed_modified_at_ms,
                    updated_at_ms = excluded.updated_at_ms",
                params![
                    cursor.source_kind.as_str(),
                    cursor.file_identity,
                    cursor.canonical_path,
                    checked_i64(cursor.committed_offset, "committed_offset")?,
                    i64::from(cursor.parser_version),
                    checked_i64(cursor.observed_size, "observed_size")?,
                    optional_checked_i64(
                        cursor.observed_modified_at_ms,
                        "observed_modified_at_ms"
                    )?,
                    checked_i64(cursor.updated_at_ms, "cursor updated_at_ms")?,
                ],
            )?;
        }
        tx.commit()?;
        Ok(result)
    }

    pub fn export_usage(
        &self,
        params: &UsageExportParams,
    ) -> Result<UsageExportResult, StoreError> {
        params.validate().map_err(StoreError::Data)?;
        let after_seq = checked_i64(params.after_seq, "usage export after_seq")?;
        let query_limit = i64::from(params.limit) + 1;
        let conn = self.conn.lock().unwrap();
        let node_store_id = conn.query_row(
            "SELECT node_store_id FROM usage_store_meta WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )?;
        let mut stmt = conn.prepare(
            "SELECT x.seq,
                    o.observation_id, o.source_kind, o.parser_version,
                    o.provider_native_session_id, o.provider_native_turn_id,
                    o.occurred_at_ms, o.provider, o.provider_instance_id,
                    o.subscription_key, o.model, o.input_tokens,
                    o.cached_input_tokens, o.cache_write_tokens, o.output_tokens,
                    o.reasoning_tokens, o.other_tokens_json,
                    o.provider_reported_micro_usd, o.source_record_count,
                    o.thread_id, o.agent_id, o.schedule_id
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
        })
    }

    pub fn acknowledge_usage(&self, through_seq: u64) -> Result<u64, StoreError> {
        let through_seq = checked_i64(through_seq, "usage acknowledgement sequence")?;
        let acknowledged_at_ms = checked_i64(store_now_ms(), "usage acknowledgement time")?;
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "UPDATE usage_outbox
             SET acknowledged_at_ms = ?1
             WHERE seq <= ?2 AND acknowledged_at_ms IS NULL",
            params![acknowledged_at_ms, through_seq],
        )?;
        u64::try_from(count)
            .map_err(|_| StoreError::Data("usage acknowledgement count overflow".to_string()))
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
                    provider_reported_micro_usd, source_record_count,
                    thread_id, agent_id, schedule_id
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
        let included_environments: BTreeSet<EnvironmentId> = items
            .iter()
            .map(|item| item.environment_id.clone())
            .collect();
        let quarantined_count = checked_u64(
            conn.query_row("SELECT COUNT(*) FROM usage_quarantine", [], |row| {
                row.get(0)
            })?,
            "usage_quarantine count",
        )?;
        Ok(UsageQueryResult {
            items,
            next,
            has_more,
            coverage: pandamux_protocol::UsageCoverage {
                quarantined_count,
                included_environments: included_environments.into_iter().collect(),
            },
        })
    }

    pub fn save_capacity_snapshot(&self, snapshot: &CapacitySnapshot) -> Result<(), StoreError> {
        snapshot.validate().map_err(StoreError::Data)?;
        let confidence = match snapshot.confidence {
            CapacityConfidence::Reported => "reported",
            CapacityConfidence::Estimated => "estimated",
            CapacityConfidence::Unknown => "unknown",
        };
        let conn = self.conn.lock().unwrap();
        let inserted = conn.execute(
            "INSERT INTO capacity_snapshots (
                subscription_key, window_start_ms, window_end_ms, used_millionths,
                remaining_millionths, reset_at_ms, source, confidence,
                provider_version, as_of_ms, detail_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, '{}')
             ON CONFLICT DO NOTHING",
            params![
                snapshot.subscription_key.as_str(),
                optional_checked_i64(snapshot.window_start_ms, "capacity window_start_ms")?,
                optional_checked_i64(snapshot.window_end_ms, "capacity window_end_ms")?,
                snapshot.used_millionths.map(i64::from),
                snapshot.remaining_millionths.map(i64::from),
                optional_checked_i64(snapshot.reset_at_ms, "capacity reset_at_ms")?,
                snapshot.source,
                confidence,
                snapshot.provider_version,
                checked_i64(snapshot.as_of_ms, "capacity as_of_ms")?,
            ],
        )?;
        if inserted == 0 {
            let mut stmt = conn.prepare(
                "SELECT subscription_key, window_start_ms, window_end_ms,
                        used_millionths, remaining_millionths, reset_at_ms, source,
                        confidence, provider_version, as_of_ms
                 FROM capacity_snapshots
                 WHERE subscription_key = ?1
                   AND window_start_ms IS ?2
                   AND window_end_ms IS ?3
                   AND source = ?4
                   AND as_of_ms = ?5",
            )?;
            let mut rows = stmt.query(params![
                snapshot.subscription_key.as_str(),
                optional_checked_i64(snapshot.window_start_ms, "capacity window_start_ms")?,
                optional_checked_i64(snapshot.window_end_ms, "capacity window_end_ms")?,
                snapshot.source,
                checked_i64(snapshot.as_of_ms, "capacity as_of_ms")?,
            ])?;
            let existing = rows
                .next()?
                .ok_or_else(|| {
                    StoreError::Data(
                        "capacity conflict did not resolve to a stored snapshot".to_string(),
                    )
                })
                .and_then(|row| decode_capacity_snapshot(row, 0))?;
            if existing != *snapshot {
                return Err(StoreError::Data(format!(
                    "capacity snapshot identity collision for subscription {} at {}",
                    snapshot.subscription_key, snapshot.as_of_ms
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
                    confidence, provider_version, as_of_ms
             FROM capacity_snapshots WHERE 1 = 1",
        );
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(subscription_key) = &params.subscription_key {
            sql.push_str(" AND subscription_key = ?");
            values.push(Box::new(subscription_key.as_str().to_string()));
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
        Ok(CapacityQueryResult {
            snapshots: values.into_iter().map(|(_, snapshot)| snapshot).collect(),
            next,
            has_more,
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

    pub fn reconcile_active_turns(&self) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        let interrupted_status = serde_json::to_string(&TurnStatus::Interrupted)?;
        let idle_status = serde_json::to_string(&pandamux_core::ThreadStatus::Idle)?;

        let count = conn.execute(
            "UPDATE turns SET status = ?1 WHERE status IN ('\"running\"', '\"awaiting_approval\"', '\"requested\"')",
            params![interrupted_status],
        )?;

        conn.execute(
            "UPDATE threads SET status = ?1 WHERE status IN ('\"working\"', '\"awaiting_approval\"')",
            params![idle_status],
        )?;

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
    if cursor.source_kind.as_str().trim().is_empty()
        || cursor.file_identity.trim().is_empty()
        || cursor.canonical_path.trim().is_empty()
    {
        return Err(StoreError::Data(
            "usage cursor source, file identity, and diagnostic path must not be empty".to_string(),
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
                provider_reported_micro_usd, source_record_count,
                thread_id, agent_id, schedule_id
         FROM usage_observations
         WHERE environment_id = ?1 AND observation_id = ?2",
    )?;
    let mut rows = stmt.query(params![environment_id.as_str(), observation_id.as_str()])?;
    rows.next()?
        .map(|row| decode_usage_observation(row, 0))
        .transpose()
}

fn decode_usage_observation(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> Result<UsageObservation, StoreError> {
    let parser_version = checked_u32(row.get(offset + 2)?, "usage_observations.parser_version")?;
    let other_tokens_json: String = row.get(offset + 15)?;
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
        provider_reported_micro_usd: row
            .get::<_, Option<i64>>(offset + 16)?
            .map(|value| checked_u64(value, "usage_observations.provider_reported_micro_usd"))
            .transpose()?,
        source_record_count: checked_u64(
            row.get(offset + 17)?,
            "usage_observations.source_record_count",
        )?,
        thread_id: row
            .get::<_, Option<String>>(offset + 18)?
            .map(ThreadId::from),
        agent_id: row
            .get::<_, Option<String>>(offset + 19)?
            .map(AgentId::from),
        schedule_id: row
            .get::<_, Option<String>>(offset + 20)?
            .map(ScheduleId::from),
    };
    observation
        .validate()
        .map_err(|error| StoreError::Data(error.to_string()))?;
    Ok(observation)
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
    let snapshot = CapacitySnapshot {
        subscription_key: SubscriptionKey::from(row.get::<_, String>(offset)?),
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
        provider_version: row.get(offset + 8)?,
        as_of_ms: checked_u64(row.get(offset + 9)?, "capacity_snapshots.as_of_ms")?,
    };
    snapshot.validate().map_err(StoreError::Data)?;
    Ok(snapshot)
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
        EnvironmentId, ProviderInstanceId, ThreadStatus, ThreadWorkspace, TurnId, TurnInput,
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
                provider_instance_id: ProviderInstanceId::new("claude-work"),
                subscription_key: SubscriptionKey::new("subkey-work"),
            }],
            revision: 1,
        }
    }

    fn organization_policy(id: &str, project_id: &str) -> OrganizationPolicy {
        OrganizationPolicy {
            id: pandamux_core::OrganizationId::new(id),
            display_name: "Work organization".to_string(),
            project_ids: vec![ProjectId::new(project_id)],
            allowed_provider_instances: vec![ProviderInstanceId::new("claude-work")],
            allowed_subscriptions: vec![SubscriptionKey::new("subkey-work")],
            failover_pool_ids: vec![pandamux_core::FailoverPoolId::new("pool-work")],
            default_tier_mapping: None,
            max_concurrency: Some(2),
            daily_budget_micro_usd: Some(10_000_000),
            broadside: pandamux_core::BroadsidePolicy::default(),
            revision: 1,
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
            provider_reported_micro_usd: None,
            source_record_count: 1,
            thread_id: None,
            agent_id: None,
            schedule_id: None,
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

    fn usage_cursor(offset: u64) -> UsageScannerCursor {
        UsageScannerCursor {
            source_kind: UsageSourceKind::ClaudeTranscript,
            file_identity: "unix:42:99".to_string(),
            canonical_path: "/private/transcript.jsonl".to_string(),
            committed_offset: offset,
            parser_version: 2,
            observed_size: offset,
            observed_modified_at_ms: Some(2000),
            updated_at_ms: 3000,
        }
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
                Some(&usage_cursor(100)),
            )
            .expect("record usage");
        assert_eq!(inserted.inserted, 1);
        assert_eq!(inserted.duplicates, 0);
        assert_eq!(inserted.last_outbox_seq, 1);
        assert_eq!(
            store
                .get_usage_scanner_cursor(&UsageSourceKind::ClaudeTranscript, "unix:42:99")
                .expect("load cursor")
                .expect("cursor")
                .committed_offset,
            100
        );

        let duplicate = store
            .record_usage_batch(
                &environment_id,
                std::slice::from_ref(&observation),
                Some(&usage_cursor(120)),
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
                .record_usage_batch(&environment_id, &[collision], Some(&usage_cursor(140)))
                .is_err()
        );
        assert_eq!(
            store
                .get_usage_scanner_cursor(&UsageSourceKind::ClaudeTranscript, "unix:42:99")
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
                .record_usage_batch(&environment_id, &[observation], Some(&usage_cursor(100)))
                .is_err()
        );
        assert!(
            store
                .get_usage_scanner_cursor(&UsageSourceKind::ClaudeTranscript, "unix:42:99")
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
                subscription_key: SubscriptionKey::new("subkey-work"),
                window_start_ms: None,
                window_end_ms: None,
                used_millionths: Some(1_000_000 - remaining),
                remaining_millionths: Some(remaining),
                reset_at_ms: None,
                source: "provider_report".to_string(),
                confidence: CapacityConfidence::Reported,
                provider_version: Some("fixture-1".to_string()),
                as_of_ms,
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
                    subscription_key: SubscriptionKey::new("subkey-work"),
                    window_start_ms: None,
                    window_end_ms: None,
                    used_millionths: Some(900_000),
                    remaining_millionths: Some(100_000),
                    reset_at_ms: None,
                    source: "provider_report".to_string(),
                    confidence: CapacityConfidence::Reported,
                    provider_version: Some("fixture-1".to_string()),
                    as_of_ms: 1000,
                })
                .is_err()
        );

        let first = store
            .query_capacity(&CapacityQueryParams {
                subscription_key: Some(SubscriptionKey::new("subkey-work")),
                after: None,
                limit: 1,
            })
            .expect("first capacity page");
        assert_eq!(first.snapshots.len(), 1);
        assert!(first.has_more);

        let second = store
            .query_capacity(&CapacityQueryParams {
                subscription_key: Some(SubscriptionKey::new("subkey-work")),
                after: first.next,
                limit: 1,
            })
            .expect("second capacity page");
        assert_eq!(second.snapshots.len(), 1);
        assert!(!second.has_more);
        assert_eq!(second.snapshots[0].as_of_ms, 2000);
    }
}
