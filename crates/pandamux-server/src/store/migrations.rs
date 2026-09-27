use rusqlite::{Connection, Result};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Applies all pending versioned migrations inside a transaction.
pub fn apply_migrations(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            version INTEGER PRIMARY KEY,
            applied_at_ms INTEGER NOT NULL
        );"
    )?;

    let current_version: Option<i64> = conn
        .query_row(
            "SELECT MAX(version) FROM _migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(None);

    let current = current_version.unwrap_or(0);

    if current < 1 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS threads (
                id TEXT PRIMARY KEY,
                project_id TEXT,
                environment_id TEXT NOT NULL,
                parent_thread_id TEXT,
                title TEXT NOT NULL,
                provider_instance_id TEXT NOT NULL,
                model TEXT NOT NULL,
                effort TEXT,
                access_mode TEXT NOT NULL,
                workspace_json TEXT NOT NULL,
                status TEXT NOT NULL,
                agent_json TEXT,
                origin_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS turns (
                id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL REFERENCES threads(id),
                seq INTEGER NOT NULL,
                input_json TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at_ms INTEGER NOT NULL,
                ended_at_ms INTEGER,
                usage_json TEXT,
                checkpoint_before TEXT,
                checkpoint_after TEXT
            );

            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                thread_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                at_ms INTEGER NOT NULL,
                kind TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                UNIQUE(thread_id, seq)
            );

            CREATE TABLE IF NOT EXISTS schedules (
                id TEXT PRIMARY KEY,
                environment_id TEXT NOT NULL,
                definition_json TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_turns_thread_id ON turns(thread_id);
            CREATE INDEX IF NOT EXISTS idx_events_thread_seq ON events(thread_id, seq);
            CREATE INDEX IF NOT EXISTS idx_schedules_env ON schedules(environment_id);"
        )?;

        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (1, ?1)",
            [now_ms() as i64],
        )?;

        tx.commit()?;
    }

    Ok(())
}
