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
        );",
    )?;

    let current_version: Option<i64> = conn
        .query_row("SELECT MAX(version) FROM _migrations", [], |row| row.get(0))
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
            CREATE INDEX IF NOT EXISTS idx_schedules_env ON schedules(environment_id);",
        )?;

        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (1, ?1)",
            [now_ms() as i64],
        )?;

        tx.commit()?;
    }

    if current < 2 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS thread_resumes (
                thread_id TEXT PRIMARY KEY REFERENCES threads(id),
                resume_token TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );",
        )?;

        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (2, ?1)",
            [now_ms() as i64],
        )?;

        tx.commit()?;
    }

    if current < 3 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS attachments (
                id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL REFERENCES threads(id),
                file_name TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                file_path TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_attachments_thread ON attachments(thread_id);",
        )?;

        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (3, ?1)",
            [now_ms() as i64],
        )?;

        tx.commit()?;
    }

    if current < 4 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS usage_buckets (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                day TEXT NOT NULL,
                hour_start INTEGER,
                environment_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                account_key TEXT NOT NULL,
                agent_id TEXT,
                schedule_id TEXT,
                input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                cache_write_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                reasoning_tokens INTEGER NOT NULL,
                cost_usd_estimate REAL NOT NULL,
                records INTEGER NOT NULL,
                created_at_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_usage_day ON usage_buckets(day);
            CREATE INDEX IF NOT EXISTS idx_usage_account ON usage_buckets(account_key);
            CREATE INDEX IF NOT EXISTS idx_usage_provider_model ON usage_buckets(provider, model);
            CREATE INDEX IF NOT EXISTS idx_usage_env ON usage_buckets(environment_id);",
        )?;

        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (4, ?1)",
            [now_ms() as i64],
        )?;

        tx.commit()?;
    }

    if current < 5 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "ALTER TABLE turns ADD COLUMN environment_id TEXT NOT NULL DEFAULT '';
            ALTER TABLE turns ADD COLUMN provider_instance_id TEXT NOT NULL DEFAULT '';

            UPDATE turns
            SET environment_id = COALESCE(
                    (SELECT threads.environment_id FROM threads WHERE threads.id = turns.thread_id),
                    ''
                ),
                provider_instance_id = COALESCE(
                    (SELECT threads.provider_instance_id FROM threads WHERE threads.id = turns.thread_id),
                    ''
                );

            CREATE TABLE provider_execution_audit (
                turn_id TEXT PRIMARY KEY REFERENCES turns(id),
                thread_id TEXT NOT NULL REFERENCES threads(id),
                environment_id TEXT NOT NULL,
                provider_instance_id TEXT NOT NULL,
                provider_kind TEXT NOT NULL,
                profile_dir TEXT NOT NULL,
                binary_path TEXT,
                capabilities_json TEXT NOT NULL,
                model TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );

            CREATE INDEX idx_provider_execution_audit_instance
                ON provider_execution_audit(environment_id, provider_instance_id);",
        )?;

        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (5, ?1)",
            [now_ms() as i64],
        )?;

        tx.commit()?;
    }

    Ok(())
}
