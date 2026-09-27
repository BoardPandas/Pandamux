pub mod migrations;

use pandamux_core::{
    AttachmentRecord, ScheduleRecord, Thread, ThreadEvent, ThreadEventKind, ThreadId, Turn, TurnId,
    TurnStatus, UserSettings,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(err) => write!(f, "Store SQL error: {err}"),
            Self::Json(err) => write!(f, "Store JSON error: {err}"),
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
                id, thread_id, seq, input_json, status,
                started_at_ms, ended_at_ms, usage_json,
                checkpoint_before, checkpoint_after
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                ended_at_ms = excluded.ended_at_ms,
                usage_json = excluded.usage_json,
                checkpoint_before = excluded.checkpoint_before,
                checkpoint_after = excluded.checkpoint_after;",
            params![
                turn.id.as_str(),
                turn.thread_id.as_str(),
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
            "SELECT id, thread_id, seq, input_json, status,
                    started_at_ms, ended_at_ms, usage_json,
                    checkpoint_before, checkpoint_after
             FROM turns WHERE id = ?1",
        )?;

        let row = stmt
            .query_row(params![id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            })
            .optional()?;

        let Some((
            id_str,
            thread_id_str,
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
            "SELECT id, thread_id, seq, input_json, status,
                    started_at_ms, ended_at_ms, usage_json,
                    checkpoint_before, checkpoint_after
             FROM turns WHERE thread_id = ?1 ORDER BY seq ASC",
        )?;

        let rows = stmt.query_map(params![thread_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?;

        let mut result = Vec::new();
        for r in rows {
            let (
                id_str,
                thread_id_str,
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
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT value_json FROM settings WHERE key = 'user_settings'")?;
        let json: Option<String> = stmt.query_row([], |row| row.get(0)).optional()?;

        match json {
            Some(j) => Ok(Some(serde_json::from_str(&j)?)),
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
}
