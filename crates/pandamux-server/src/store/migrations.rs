use rusqlite::types::Value;
use rusqlite::{Connection, Result, Transaction, params};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: i64 = 14;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug)]
struct LegacyUsageRow {
    id: Value,
    day: Value,
    hour_start: Value,
    environment_id: Value,
    provider: Value,
    model: Value,
    account_key: Value,
    agent_id: Value,
    schedule_id: Value,
    input_tokens: Value,
    cached_input_tokens: Value,
    cache_write_tokens: Value,
    output_tokens: Value,
    reasoning_tokens: Value,
    cost_usd_estimate: Value,
    records: Value,
    created_at_ms: Value,
}

struct NormalizedLegacyUsage {
    occurred_at_ms: i64,
    environment_id: String,
    provider: String,
    provider_instance_id: String,
    subscription_key: String,
    subscription_display_name: String,
    masked_account_label: String,
    observation_id: String,
    model: String,
    agent_id: Option<String>,
    schedule_id: Option<String>,
    input_tokens: i64,
    cached_input_tokens: i64,
    cache_write_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    source_record_count: i64,
    created_at_ms: i64,
}

fn backfill_legacy_usage(tx: &Transaction<'_>, migration_at_ms: i64) -> Result<()> {
    let rows = {
        let mut stmt = tx.prepare(
            "SELECT id, day, hour_start, environment_id, provider, model, account_key,
                    agent_id, schedule_id, input_tokens, cached_input_tokens,
                    cache_write_tokens, output_tokens, reasoning_tokens,
                    cost_usd_estimate, records, created_at_ms
             FROM usage_buckets ORDER BY id",
        )?;
        let mapped = stmt.query_map([], |row| {
            Ok(LegacyUsageRow {
                id: row.get(0)?,
                day: row.get(1)?,
                hour_start: row.get(2)?,
                environment_id: row.get(3)?,
                provider: row.get(4)?,
                model: row.get(5)?,
                account_key: row.get(6)?,
                agent_id: row.get(7)?,
                schedule_id: row.get(8)?,
                input_tokens: row.get(9)?,
                cached_input_tokens: row.get(10)?,
                cache_write_tokens: row.get(11)?,
                output_tokens: row.get(12)?,
                reasoning_tokens: row.get(13)?,
                cost_usd_estimate: row.get(14)?,
                records: row.get(15)?,
                created_at_ms: row.get(16)?,
            })
        })?;
        mapped.collect::<Result<Vec<_>>>()?
    };

    for row in rows {
        match normalize_legacy_usage(&row) {
            Ok(value) => insert_legacy_observation(tx, &value, migration_at_ms)?,
            Err((reason_code, reason_detail)) => {
                let legacy_id = value_i64(&row.id);
                tx.execute(
                    "INSERT INTO usage_quarantine (
                        legacy_bucket_id, source_kind, reason_code, reason_detail,
                        safe_excerpt_sha256, created_at_ms
                     ) VALUES (?1, 'legacy_bucket', ?2, ?3, ?4, ?5)",
                    params![
                        legacy_id,
                        reason_code,
                        reason_detail,
                        stable_hash(&[&legacy_row_material(&row)]),
                        migration_at_ms,
                    ],
                )?;
            }
        }
    }
    Ok(())
}

fn normalize_legacy_usage(
    row: &LegacyUsageRow,
) -> std::result::Result<NormalizedLegacyUsage, (&'static str, String)> {
    require_i64(&row.id, "legacy id")?;
    let day = require_text(&row.day, "day")?;
    let hour = match &row.hour_start {
        Value::Null => 0,
        value => {
            let hour = require_i64(value, "hour start")?;
            if !(0..=23).contains(&hour) {
                return Err((
                    "impossible_date",
                    "legacy hour start is outside 0 through 23".to_string(),
                ));
            }
            hour
        }
    };
    let occurred_at_ms = day_hour_to_epoch_ms(day, hour).ok_or_else(|| {
        (
            "impossible_date",
            "legacy day is not a valid UTC calendar date".to_string(),
        )
    })?;
    let environment_id = require_nonempty_text(&row.environment_id, "environment id")?;
    let provider_input = require_nonempty_text(&row.provider, "provider")?;
    let provider = normalize_provider(&provider_input).ok_or_else(|| {
        (
            "invalid_provider",
            "legacy provider is not recognized".to_string(),
        )
    })?;
    let model = require_nonempty_text(&row.model, "model")?;
    let account_key = require_nonempty_text(&row.account_key, "account identity")?;
    let input_tokens = require_nonnegative(&row.input_tokens, "input tokens")?;
    let cached_input_tokens = require_nonnegative(&row.cached_input_tokens, "cached input tokens")?;
    let cache_write_tokens = require_nonnegative(&row.cache_write_tokens, "cache write tokens")?;
    let output_tokens = require_nonnegative(&row.output_tokens, "output tokens")?;
    let reasoning_tokens = require_nonnegative(&row.reasoning_tokens, "reasoning tokens")?;
    let source_record_count = require_nonnegative(&row.records, "record count")?;
    if source_record_count == 0 {
        return Err((
            "invalid_record_count",
            "legacy record count must be greater than zero".to_string(),
        ));
    }
    let token_total = [
        input_tokens,
        cached_input_tokens,
        cache_write_tokens,
        output_tokens,
        reasoning_tokens,
    ]
    .into_iter()
    .try_fold(0_i64, |total, value| total.checked_add(value))
    .ok_or_else(|| {
        (
            "token_overflow",
            "legacy token categories overflow a signed 64-bit total".to_string(),
        )
    })?;
    if token_total == 0 {
        return Err((
            "empty_usage",
            "legacy row contains no token usage".to_string(),
        ));
    }
    let cost = require_real(&row.cost_usd_estimate, "cost estimate")?;
    if !cost.is_finite() || cost.is_sign_negative() {
        return Err((
            "invalid_cost_estimate",
            "legacy cost estimate must be finite and non-negative".to_string(),
        ));
    }
    let created_at_ms = require_nonnegative(&row.created_at_ms, "created at")?;
    let identity_hash = stable_hash(&[&environment_id, &provider, &account_key]);
    let observation_hash = stable_hash(&[&legacy_row_material(row)]);

    Ok(NormalizedLegacyUsage {
        occurred_at_ms,
        environment_id,
        provider: provider.clone(),
        provider_instance_id: format!("prov-legacy-{}", &identity_hash[..24]),
        subscription_key: format!("subkey-legacy-{}", &identity_hash[..24]),
        subscription_display_name: format!("Legacy {} quota", provider_display_name(&provider)),
        masked_account_label: mask_legacy_account(&account_key),
        observation_id: format!("usage-legacy-{}", &observation_hash[..32]),
        model,
        agent_id: optional_nonempty_text(&row.agent_id, "agent id")?,
        schedule_id: optional_nonempty_text(&row.schedule_id, "schedule id")?,
        input_tokens,
        cached_input_tokens,
        cache_write_tokens,
        output_tokens,
        reasoning_tokens,
        source_record_count,
        created_at_ms,
    })
}

fn insert_legacy_observation(
    tx: &Transaction<'_>,
    value: &NormalizedLegacyUsage,
    migration_at_ms: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO usage_subscriptions (
            subscription_key, provider, display_name, masked_account_label,
            created_at_ms, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(subscription_key) DO NOTHING",
        params![
            value.subscription_key,
            value.provider,
            value.subscription_display_name,
            value.masked_account_label,
            migration_at_ms,
        ],
    )?;
    tx.execute(
        "INSERT INTO provider_subscription_mappings (
            environment_id, provider_instance_id, subscription_key, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(environment_id, provider_instance_id) DO NOTHING",
        params![
            value.environment_id,
            value.provider_instance_id,
            value.subscription_key,
            migration_at_ms,
        ],
    )?;
    tx.execute(
        "INSERT INTO usage_observations (
            environment_id, observation_id, source_kind, parser_version,
            occurred_at_ms, provider, provider_instance_id, subscription_key,
            model, input_tokens, cached_input_tokens, cache_write_tokens,
            output_tokens, reasoning_tokens, other_tokens_json,
            provider_reported_micro_usd, source_record_count, agent_id,
            schedule_id, created_at_ms
         ) VALUES (
            ?1, ?2, 'legacy_bucket', 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
            ?11, ?12, '{}', NULL, ?13, ?14, ?15, ?16
         )",
        params![
            value.environment_id,
            value.observation_id,
            value.occurred_at_ms,
            value.provider,
            value.provider_instance_id,
            value.subscription_key,
            value.model,
            value.input_tokens,
            value.cached_input_tokens,
            value.cache_write_tokens,
            value.output_tokens,
            value.reasoning_tokens,
            value.source_record_count,
            value.agent_id,
            value.schedule_id,
            value.created_at_ms,
        ],
    )?;
    tx.execute(
        "INSERT INTO usage_outbox (environment_id, observation_id, enqueued_at_ms)
         VALUES (?1, ?2, ?3)",
        params![value.environment_id, value.observation_id, migration_at_ms],
    )?;
    Ok(())
}

fn require_i64(
    value: &Value,
    field: &'static str,
) -> std::result::Result<i64, (&'static str, String)> {
    value_i64(value).ok_or_else(|| {
        (
            "invalid_type",
            format!("legacy {field} does not contain an integer"),
        )
    })
}

fn require_nonnegative(
    value: &Value,
    field: &'static str,
) -> std::result::Result<i64, (&'static str, String)> {
    let value = require_i64(value, field)?;
    if value < 0 {
        return Err((
            "negative_integer",
            format!("legacy {field} contains a negative integer"),
        ));
    }
    Ok(value)
}

fn value_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Integer(value) => Some(*value),
        _ => None,
    }
}

fn require_text<'a>(
    value: &'a Value,
    field: &'static str,
) -> std::result::Result<&'a str, (&'static str, String)> {
    match value {
        Value::Text(value) => Ok(value),
        _ => Err((
            "invalid_type",
            format!("legacy {field} does not contain text"),
        )),
    }
}

fn require_nonempty_text(
    value: &Value,
    field: &'static str,
) -> std::result::Result<String, (&'static str, String)> {
    let value = require_text(value, field)?.trim();
    if value.is_empty() {
        return Err(("ambiguous_identity", format!("legacy {field} is empty")));
    }
    Ok(value.to_string())
}

fn optional_nonempty_text(
    value: &Value,
    field: &'static str,
) -> std::result::Result<Option<String>, (&'static str, String)> {
    match value {
        Value::Null => Ok(None),
        Value::Text(value) if value.trim().is_empty() => Ok(None),
        Value::Text(value) => Ok(Some(value.trim().to_string())),
        _ => Err((
            "invalid_type",
            format!("legacy {field} is neither text nor null"),
        )),
    }
}

fn require_real(
    value: &Value,
    field: &'static str,
) -> std::result::Result<f64, (&'static str, String)> {
    match value {
        Value::Real(value) => Ok(*value),
        Value::Integer(value) => Ok(*value as f64),
        _ => Err((
            "invalid_type",
            format!("legacy {field} does not contain a number"),
        )),
    }
}

fn normalize_provider(value: &str) -> Option<String> {
    let normalized = value.trim().trim_matches('"').to_ascii_lowercase();
    match normalized.as_str() {
        "claude" | "codex" | "antigravity" | "cursor" | "grok" | "opencode" | "custom" => {
            Some(normalized)
        }
        _ => None,
    }
}

fn provider_display_name(provider: &str) -> &str {
    match provider {
        "claude" => "Claude",
        "codex" => "Codex",
        "antigravity" => "Antigravity",
        "cursor" => "Cursor",
        "grok" => "Grok",
        "opencode" => "OpenCode",
        _ => "Custom",
    }
}

fn mask_legacy_account(value: &str) -> String {
    if let Some((local, domain)) = value.split_once('@') {
        let initial = local.chars().next().unwrap_or('*');
        return format!("{initial}***@{domain}");
    }
    let prefix: String = value.chars().take(2).collect();
    format!("{prefix}***")
}

fn legacy_row_material(row: &LegacyUsageRow) -> String {
    [
        &row.id,
        &row.day,
        &row.hour_start,
        &row.environment_id,
        &row.provider,
        &row.model,
        &row.account_key,
        &row.agent_id,
        &row.schedule_id,
        &row.input_tokens,
        &row.cached_input_tokens,
        &row.cache_write_tokens,
        &row.output_tokens,
        &row.reasoning_tokens,
        &row.cost_usd_estimate,
        &row.records,
        &row.created_at_ms,
    ]
    .into_iter()
    .map(value_material)
    .collect::<Vec<_>>()
    .join("\u{1f}")
}

fn value_material(value: &Value) -> String {
    match value {
        Value::Null => "n:".to_string(),
        Value::Integer(value) => format!("i:{value}"),
        Value::Real(value) => format!("r:{:016x}", value.to_bits()),
        Value::Text(value) => format!("t:{value}"),
        Value::Blob(value) => format!("b:{}", stable_hash_bytes(value)),
    }
}

fn stable_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    digest_hex(hasher.finalize().as_slice())
}

fn stable_hash_bytes(value: &[u8]) -> String {
    digest_hex(Sha256::digest(value).as_slice())
}

fn digest_hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn day_hour_to_epoch_ms(day: &str, hour: i64) -> Option<i64> {
    let bytes = day.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year = day[0..4].parse::<i64>().ok()?;
    let month = day[5..7].parse::<i64>().ok()?;
    let date = day[8..10].parse::<i64>().ok()?;
    if year < 1970 || !(1..=12).contains(&month) {
        return None;
    }
    let month_days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return None,
    };
    if !(1..=month_days).contains(&date) {
        return None;
    }
    let days = days_from_civil(year, month, date)?;
    days.checked_mul(86_400_000)?
        .checked_add(hour.checked_mul(3_600_000)?)
}

fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_from_civil(mut year: i64, month: i64, day: i64) -> Option<i64> {
    year -= i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153_i64.checked_mul(adjusted_month)? + 2) / 5 + day - 1;
    let day_of_era = year_of_era
        .checked_mul(365)?
        .checked_add(year_of_era / 4)?
        .checked_sub(year_of_era / 100)?
        .checked_add(day_of_year)?;
    era.checked_mul(146_097)?
        .checked_add(day_of_era)?
        .checked_sub(719_468)
}
/// Applies all pending versioned migrations inside a transaction.
pub fn apply_migrations(conn: &mut Connection) -> Result<()> {
    apply_migrations_to(conn, SCHEMA_VERSION)
}

pub(crate) fn apply_migrations_to(conn: &mut Connection, target_version: i64) -> Result<()> {
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

    if current < 1 && target_version >= 1 {
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

    if current < 2 && target_version >= 2 {
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

    if current < 3 && target_version >= 3 {
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

    if current < 4 && target_version >= 4 {
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

    if current < 5 && target_version >= 5 {
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

    if current < 6 && target_version >= 6 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE usage_store_meta (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                node_store_id TEXT NOT NULL UNIQUE,
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
            );

            CREATE TABLE usage_subscriptions (
                subscription_key TEXT PRIMARY KEY,
                provider TEXT NOT NULL CHECK(provider <> ''),
                display_name TEXT NOT NULL CHECK(display_name <> ''),
                masked_account_label TEXT,
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0)
            );

            CREATE TABLE provider_subscription_mappings (
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                provider_instance_id TEXT NOT NULL CHECK(provider_instance_id <> ''),
                subscription_key TEXT NOT NULL REFERENCES usage_subscriptions(subscription_key),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0),
                PRIMARY KEY(environment_id, provider_instance_id)
            );

            CREATE TABLE usage_observations (
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                observation_id TEXT NOT NULL CHECK(observation_id <> ''),
                source_kind TEXT NOT NULL CHECK(source_kind <> ''),
                parser_version INTEGER NOT NULL CHECK(parser_version > 0),
                provider_native_session_id TEXT,
                provider_native_turn_id TEXT,
                occurred_at_ms INTEGER CHECK(occurred_at_ms >= 0),
                provider TEXT NOT NULL CHECK(provider <> ''),
                provider_instance_id TEXT NOT NULL CHECK(provider_instance_id <> ''),
                subscription_key TEXT NOT NULL REFERENCES usage_subscriptions(subscription_key),
                model TEXT,
                input_tokens INTEGER NOT NULL CHECK(input_tokens >= 0),
                cached_input_tokens INTEGER NOT NULL CHECK(cached_input_tokens >= 0),
                cache_write_tokens INTEGER NOT NULL CHECK(cache_write_tokens >= 0),
                output_tokens INTEGER NOT NULL CHECK(output_tokens >= 0),
                reasoning_tokens INTEGER NOT NULL CHECK(reasoning_tokens >= 0),
                other_tokens_json TEXT NOT NULL DEFAULT '{}',
                provider_reported_micro_usd INTEGER CHECK(provider_reported_micro_usd >= 0),
                source_record_count INTEGER NOT NULL DEFAULT 1 CHECK(source_record_count > 0),
                thread_id TEXT,
                agent_id TEXT,
                schedule_id TEXT,
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                PRIMARY KEY(environment_id, observation_id)
            );

            CREATE INDEX idx_usage_observations_occurred
                ON usage_observations(occurred_at_ms, observation_id);
            CREATE INDEX idx_usage_observations_provider
                ON usage_observations(provider, provider_instance_id, occurred_at_ms);
            CREATE INDEX idx_usage_observations_subscription
                ON usage_observations(subscription_key, occurred_at_ms);
            CREATE INDEX idx_usage_observations_attribution
                ON usage_observations(agent_id, schedule_id, thread_id);

            CREATE TABLE usage_outbox (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                environment_id TEXT NOT NULL,
                observation_id TEXT NOT NULL,
                enqueued_at_ms INTEGER NOT NULL CHECK(enqueued_at_ms >= 0),
                acknowledged_at_ms INTEGER CHECK(acknowledged_at_ms >= 0),
                UNIQUE(environment_id, observation_id),
                FOREIGN KEY(environment_id, observation_id)
                    REFERENCES usage_observations(environment_id, observation_id)
                    ON DELETE CASCADE
            );

            CREATE TABLE usage_scanner_cursors (
                source_kind TEXT NOT NULL CHECK(source_kind <> ''),
                file_identity TEXT NOT NULL CHECK(file_identity <> ''),
                canonical_path TEXT NOT NULL,
                committed_offset INTEGER NOT NULL CHECK(committed_offset >= 0),
                parser_version INTEGER NOT NULL CHECK(parser_version > 0),
                observed_size INTEGER NOT NULL CHECK(observed_size >= 0),
                observed_modified_at_ms INTEGER CHECK(observed_modified_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0),
                PRIMARY KEY(source_kind, file_identity)
            );

            CREATE TABLE usage_hub_high_water (
                environment_id TEXT PRIMARY KEY CHECK(environment_id <> ''),
                node_store_id TEXT NOT NULL CHECK(node_store_id <> ''),
                high_water_seq INTEGER NOT NULL CHECK(high_water_seq >= 0),
                reconciliation_state TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0)
            );

            CREATE TABLE failover_pools (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                display_name TEXT NOT NULL CHECK(display_name <> ''),
                revision INTEGER NOT NULL CHECK(revision > 0),
                definition_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0)
            );

            CREATE TABLE organization_policies (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                display_name TEXT NOT NULL CHECK(display_name <> ''),
                revision INTEGER NOT NULL CHECK(revision > 0),
                policy_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0)
            );

            CREATE TABLE organization_project_assignments (
                project_id TEXT PRIMARY KEY CHECK(project_id <> ''),
                organization_id TEXT NOT NULL REFERENCES organization_policies(id)
                    ON DELETE CASCADE
            );

            CREATE TABLE usage_pricing_versions (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                provider TEXT NOT NULL CHECK(provider <> ''),
                effective_from_ms INTEGER NOT NULL CHECK(effective_from_ms >= 0),
                rules_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                UNIQUE(provider, effective_from_ms)
            );

            CREATE TABLE capacity_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subscription_key TEXT NOT NULL REFERENCES usage_subscriptions(subscription_key),
                window_start_ms INTEGER CHECK(window_start_ms >= 0),
                window_end_ms INTEGER CHECK(window_end_ms >= 0),
                used_millionths INTEGER CHECK(used_millionths BETWEEN 0 AND 1000000),
                remaining_millionths INTEGER CHECK(remaining_millionths BETWEEN 0 AND 1000000),
                reset_at_ms INTEGER CHECK(reset_at_ms >= 0),
                source TEXT NOT NULL CHECK(source <> ''),
                confidence TEXT NOT NULL CHECK(confidence <> ''),
                provider_version TEXT,
                as_of_ms INTEGER NOT NULL CHECK(as_of_ms >= 0),
                detail_json TEXT NOT NULL DEFAULT '{}'
            );

            CREATE UNIQUE INDEX idx_capacity_snapshot_identity
                ON capacity_snapshots(
                    subscription_key,
                    COALESCE(window_start_ms, -1),
                    COALESCE(window_end_ms, -1),
                    source,
                    as_of_ms
                );
            CREATE INDEX idx_capacity_subscription_as_of
                ON capacity_snapshots(subscription_key, as_of_ms DESC);

            CREATE TABLE usage_quarantine (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                legacy_bucket_id INTEGER UNIQUE,
                source_kind TEXT NOT NULL CHECK(source_kind <> ''),
                reason_code TEXT NOT NULL CHECK(reason_code <> ''),
                reason_detail TEXT NOT NULL,
                safe_excerpt_sha256 TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
            );",
        )?;

        let applied_at_ms = now_ms() as i64;
        tx.execute(
            "INSERT INTO usage_store_meta (singleton, node_store_id, created_at_ms)
             VALUES (1, ?1, ?2)",
            params![format!("store-{}", uuid::Uuid::new_v4()), applied_at_ms],
        )?;
        backfill_legacy_usage(&tx, applied_at_ms)?;
        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (6, ?1)",
            [applied_at_ms],
        )?;
        tx.commit()?;
    }

    if current < 7 && target_version >= 7 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "ALTER TABLE usage_observations ADD COLUMN reconciliation_key TEXT;
            ALTER TABLE usage_observations ADD COLUMN turn_id TEXT;
            ALTER TABLE usage_observations ADD COLUMN unknown_fields_json TEXT NOT NULL
                DEFAULT '[]';

            CREATE UNIQUE INDEX idx_usage_observations_reconciliation
                ON usage_observations(
                    environment_id,
                    provider_instance_id,
                    reconciliation_key
                )
                WHERE reconciliation_key IS NOT NULL;

            CREATE UNIQUE INDEX idx_usage_observations_managed_turn
                ON usage_observations(environment_id, turn_id)
                WHERE source_kind = 'managed_turn' AND turn_id IS NOT NULL;

            CREATE TABLE usage_scanner_file_cursors (
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                provider_instance_id TEXT NOT NULL CHECK(provider_instance_id <> ''),
                source_kind TEXT NOT NULL CHECK(source_kind <> ''),
                node_store_id TEXT NOT NULL CHECK(node_store_id <> ''),
                file_identity TEXT NOT NULL CHECK(file_identity <> ''),
                canonical_path TEXT NOT NULL,
                committed_offset INTEGER NOT NULL CHECK(committed_offset >= 0),
                committed_prefix_sha256 TEXT NOT NULL,
                parser_version INTEGER NOT NULL CHECK(parser_version > 0),
                parser_state_json TEXT NOT NULL,
                observed_size INTEGER NOT NULL CHECK(observed_size >= 0),
                observed_modified_at_ms INTEGER CHECK(observed_modified_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0),
                PRIMARY KEY(
                    environment_id,
                    provider_instance_id,
                    source_kind,
                    file_identity
                )
            );

            CREATE TABLE usage_scanner_diagnostics (
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                provider_instance_id TEXT NOT NULL CHECK(provider_instance_id <> ''),
                source_kind TEXT NOT NULL CHECK(source_kind <> ''),
                coverage_state TEXT NOT NULL CHECK(
                    coverage_state IN ('complete', 'partial', 'unavailable')
                ),
                parser_version INTEGER NOT NULL CHECK(parser_version > 0),
                last_scan_started_at_ms INTEGER NOT NULL CHECK(last_scan_started_at_ms >= 0),
                last_success_at_ms INTEGER CHECK(last_success_at_ms >= 0),
                latest_source_at_ms INTEGER CHECK(latest_source_at_ms >= 0),
                source_lag_ms INTEGER CHECK(source_lag_ms >= 0),
                files_scanned INTEGER NOT NULL CHECK(files_scanned >= 0),
                bytes_scanned INTEGER NOT NULL CHECK(bytes_scanned >= 0),
                quarantined_count INTEGER NOT NULL CHECK(quarantined_count >= 0),
                last_safe_error_code TEXT,
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0),
                PRIMARY KEY(environment_id, provider_instance_id, source_kind)
            );

            ALTER TABLE usage_quarantine ADD COLUMN environment_id TEXT;
            ALTER TABLE usage_quarantine ADD COLUMN provider_instance_id TEXT;
            ALTER TABLE usage_quarantine ADD COLUMN file_identity_sha256 TEXT;
            ALTER TABLE usage_quarantine ADD COLUMN record_offset INTEGER
                CHECK(record_offset >= 0);
            ALTER TABLE usage_quarantine ADD COLUMN parser_version INTEGER
                CHECK(parser_version > 0);

            CREATE UNIQUE INDEX idx_usage_quarantine_scanner_identity
                ON usage_quarantine(
                    environment_id,
                    provider_instance_id,
                    source_kind,
                    file_identity_sha256,
                    record_offset,
                    parser_version,
                    reason_code
                )
                WHERE legacy_bucket_id IS NULL;",
        )?;

        // A normalized fact can be revised in place when a later managed turn
        // supersedes its transcript copy. Preserve every revision notification
        // so a hub that already consumed the earlier sequence observes the
        // authoritative replacement under the same observation id.
        tx.execute_batch(
            "CREATE TABLE usage_outbox_v7 (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                environment_id TEXT NOT NULL,
                observation_id TEXT NOT NULL,
                enqueued_at_ms INTEGER NOT NULL CHECK(enqueued_at_ms >= 0),
                acknowledged_at_ms INTEGER CHECK(acknowledged_at_ms >= 0),
                FOREIGN KEY(environment_id, observation_id)
                    REFERENCES usage_observations(environment_id, observation_id)
                    ON DELETE CASCADE
            );

            INSERT INTO usage_outbox_v7 (
                seq, environment_id, observation_id, enqueued_at_ms, acknowledged_at_ms
            )
            SELECT seq, environment_id, observation_id, enqueued_at_ms, acknowledged_at_ms
            FROM usage_outbox;

            DROP TABLE usage_outbox;
            ALTER TABLE usage_outbox_v7 RENAME TO usage_outbox;

            CREATE INDEX idx_usage_outbox_observation
                ON usage_outbox(environment_id, observation_id, seq);
            CREATE INDEX idx_usage_outbox_acknowledgement
                ON usage_outbox(acknowledged_at_ms, seq);",
        )?;
        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (7, ?1)",
            [now_ms() as i64],
        )?;
        tx.commit()?;
    }

    if current < 8 && target_version >= 8 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "ALTER TABLE usage_observations ADD COLUMN source_node_store_id TEXT;
            ALTER TABLE usage_observations ADD COLUMN source_outbox_seq INTEGER
                CHECK(source_outbox_seq >= 0);
            ALTER TABLE usage_observations ADD COLUMN hub_sync_run_id TEXT;

            CREATE INDEX idx_usage_observations_environment_time
                ON usage_observations(environment_id, occurred_at_ms, observation_id);
            CREATE INDEX idx_usage_observations_model_time
                ON usage_observations(model, occurred_at_ms);
            CREATE INDEX idx_usage_observations_source_time
                ON usage_observations(source_kind, occurred_at_ms);
            CREATE INDEX idx_usage_observations_instance_time
                ON usage_observations(provider_instance_id, occurred_at_ms);
            CREATE INDEX idx_usage_observations_schedule_time
                ON usage_observations(schedule_id, occurred_at_ms);
            CREATE INDEX idx_usage_observations_remote_revision
                ON usage_observations(
                    environment_id,
                    source_node_store_id,
                    source_outbox_seq
                );
            CREATE INDEX idx_usage_observations_sync_run
                ON usage_observations(environment_id, hub_sync_run_id);

            ALTER TABLE usage_outbox ADD COLUMN payload_bytes INTEGER NOT NULL
                DEFAULT 0 CHECK(payload_bytes >= 0);
            UPDATE usage_outbox
            SET payload_bytes = COALESCE((
                SELECT 512 + 6 * (
                    length(CAST(o.observation_id AS BLOB))
                    + length(CAST(o.source_kind AS BLOB))
                    + length(CAST(COALESCE(o.provider_native_session_id, '') AS BLOB))
                    + length(CAST(COALESCE(o.provider_native_turn_id, '') AS BLOB))
                    + length(CAST(o.provider AS BLOB))
                    + length(CAST(o.provider_instance_id AS BLOB))
                    + length(CAST(o.subscription_key AS BLOB))
                    + length(CAST(COALESCE(o.model, '') AS BLOB))
                    + length(CAST(o.other_tokens_json AS BLOB))
                    + length(CAST(o.unknown_fields_json AS BLOB))
                    + length(CAST(COALESCE(o.thread_id, '') AS BLOB))
                    + length(CAST(COALESCE(o.turn_id, '') AS BLOB))
                    + length(CAST(COALESCE(o.agent_id, '') AS BLOB))
                    + length(CAST(COALESCE(o.schedule_id, '') AS BLOB))
                )
                FROM usage_observations o
                WHERE o.environment_id = usage_outbox.environment_id
                  AND o.observation_id = usage_outbox.observation_id
            ), 0);
            UPDATE usage_observations
            SET source_node_store_id = (
                    SELECT node_store_id
                    FROM usage_store_meta
                    WHERE singleton = 1
                ),
                source_outbox_seq = (
                    SELECT MAX(x.seq)
                    FROM usage_outbox x
                    WHERE x.environment_id = usage_observations.environment_id
                      AND x.observation_id = usage_observations.observation_id
                )
            WHERE source_outbox_seq IS NULL;
            CREATE INDEX idx_usage_outbox_retention
                ON usage_outbox(acknowledged_at_ms, enqueued_at_ms, seq);

            ALTER TABLE usage_hub_high_water ADD COLUMN last_attempt_at_ms INTEGER
                CHECK(last_attempt_at_ms >= 0);
            ALTER TABLE usage_hub_high_water ADD COLUMN last_success_at_ms INTEGER
                CHECK(last_success_at_ms >= 0);
            ALTER TABLE usage_hub_high_water ADD COLUMN last_safe_error_code TEXT;
            ALTER TABLE usage_hub_high_water ADD COLUMN gap_requested_after_seq INTEGER
                CHECK(gap_requested_after_seq >= 0);
            ALTER TABLE usage_hub_high_water ADD COLUMN gap_retained_from_seq INTEGER
                CHECK(gap_retained_from_seq >= 0);
            ALTER TABLE usage_hub_high_water ADD COLUMN gap_latest_seq INTEGER
                CHECK(gap_latest_seq >= 0);
            ALTER TABLE usage_hub_high_water ADD COLUMN backfill_run_id TEXT;
            ALTER TABLE usage_hub_high_water ADD COLUMN backfill_after_observation_id TEXT;
            ALTER TABLE usage_hub_high_water ADD COLUMN backfill_through_seq INTEGER
                CHECK(backfill_through_seq >= 0);
            ALTER TABLE usage_hub_high_water ADD COLUMN quarantined_count INTEGER NOT NULL
                DEFAULT 0 CHECK(quarantined_count >= 0);
            ALTER TABLE usage_hub_high_water ADD COLUMN latest_source_at_ms INTEGER
                CHECK(latest_source_at_ms >= 0);

            CREATE TABLE usage_hub_environment_status (
                environment_id TEXT PRIMARY KEY CHECK(environment_id <> ''),
                sync_state TEXT NOT NULL CHECK(sync_state IN (
                    'ready', 'reconciling', 'coverage_gap', 'stale', 'unreachable'
                )),
                last_attempt_at_ms INTEGER CHECK(last_attempt_at_ms >= 0),
                last_success_at_ms INTEGER CHECK(last_success_at_ms >= 0),
                last_safe_error_code TEXT,
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0)
            );
            INSERT INTO usage_hub_environment_status (
                environment_id, sync_state, last_attempt_at_ms,
                last_success_at_ms, last_safe_error_code, updated_at_ms
            )
            SELECT environment_id,
                   CASE
                       WHEN reconciliation_state IN (
                           'ready', 'reconciling', 'coverage_gap', 'stale', 'unreachable'
                       ) THEN reconciliation_state
                       ELSE 'stale'
                   END,
                   last_attempt_at_ms,
                   last_success_at_ms, last_safe_error_code, updated_at_ms
            FROM usage_hub_high_water;

            ALTER TABLE usage_pricing_versions ADD COLUMN source_published_at_ms INTEGER
                CHECK(source_published_at_ms >= 0);
            UPDATE usage_pricing_versions
            SET source_published_at_ms = effective_from_ms
            WHERE source_published_at_ms IS NULL;

            CREATE TABLE usage_pricing_rules (
                pricing_version_id TEXT NOT NULL
                    REFERENCES usage_pricing_versions(id) ON DELETE CASCADE,
                canonical_model_id TEXT NOT NULL CHECK(canonical_model_id <> ''),
                input_micro_usd_per_million INTEGER
                    CHECK(input_micro_usd_per_million >= 0),
                cached_input_micro_usd_per_million INTEGER
                    CHECK(cached_input_micro_usd_per_million >= 0),
                cache_write_micro_usd_per_million INTEGER
                    CHECK(cache_write_micro_usd_per_million >= 0),
                output_micro_usd_per_million INTEGER
                    CHECK(output_micro_usd_per_million >= 0),
                reasoning_micro_usd_per_million INTEGER
                    CHECK(reasoning_micro_usd_per_million >= 0),
                PRIMARY KEY(pricing_version_id, canonical_model_id),
                CHECK(
                    input_micro_usd_per_million IS NOT NULL OR
                    cached_input_micro_usd_per_million IS NOT NULL OR
                    cache_write_micro_usd_per_million IS NOT NULL OR
                    output_micro_usd_per_million IS NOT NULL OR
                    reasoning_micro_usd_per_million IS NOT NULL
                )
            );

            CREATE TABLE usage_pricing_aliases (
                pricing_version_id TEXT NOT NULL,
                alias_model_id TEXT NOT NULL CHECK(alias_model_id <> ''),
                canonical_model_id TEXT NOT NULL CHECK(canonical_model_id <> ''),
                PRIMARY KEY(pricing_version_id, alias_model_id),
                FOREIGN KEY(pricing_version_id, canonical_model_id)
                    REFERENCES usage_pricing_rules(
                        pricing_version_id,
                        canonical_model_id
                    ) ON DELETE CASCADE
            );

            CREATE INDEX idx_usage_pricing_versions_effective
                ON usage_pricing_versions(provider, effective_from_ms DESC, id);
            CREATE INDEX idx_usage_pricing_aliases_canonical
                ON usage_pricing_aliases(pricing_version_id, canonical_model_id);",
        )?;
        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (8, ?1)",
            [now_ms() as i64],
        )?;
        tx.commit()?;
    }

    if current < 9 && target_version >= 9 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "ALTER TABLE capacity_snapshots ADD COLUMN schema_version INTEGER NOT NULL
                DEFAULT 1 CHECK(schema_version = 1);
            ALTER TABLE capacity_snapshots ADD COLUMN environment_id TEXT NOT NULL
                DEFAULT 'env-legacy' CHECK(environment_id <> '');
            ALTER TABLE capacity_snapshots ADD COLUMN provider_instance_id TEXT;
            ALTER TABLE capacity_snapshots ADD COLUMN window_name TEXT;
            ALTER TABLE capacity_snapshots ADD COLUMN evidence_scope TEXT NOT NULL
                DEFAULT 'local' CHECK(evidence_scope IN ('local', 'hub'));
            ALTER TABLE capacity_snapshots ADD COLUMN hard_limit INTEGER NOT NULL
                DEFAULT 0 CHECK(hard_limit IN (0, 1));
            ALTER TABLE capacity_snapshots ADD COLUMN max_age_ms INTEGER NOT NULL
                DEFAULT 900000 CHECK(max_age_ms > 0);
            ALTER TABLE capacity_snapshots ADD COLUMN received_at_ms INTEGER NOT NULL
                DEFAULT 0 CHECK(received_at_ms >= 0);

            UPDATE capacity_snapshots
            SET environment_id = COALESCE((
                    SELECT MIN(m.environment_id)
                    FROM provider_subscription_mappings m
                    WHERE m.subscription_key = capacity_snapshots.subscription_key
                ), 'env-legacy'),
                received_at_ms = as_of_ms;

            DROP INDEX idx_capacity_snapshot_identity;
            CREATE UNIQUE INDEX idx_capacity_snapshot_revision
                ON capacity_snapshots(
                    subscription_key,
                    environment_id,
                    COALESCE(provider_instance_id, ''),
                    COALESCE(window_name, ''),
                    CASE WHEN window_name IS NULL
                        THEN COALESCE(window_start_ms, -1) ELSE -1 END,
                    CASE WHEN window_name IS NULL
                        THEN COALESCE(window_end_ms, -1) ELSE -1 END,
                    evidence_scope,
                    source,
                    as_of_ms
                );
            CREATE INDEX idx_capacity_environment_as_of
                ON capacity_snapshots(environment_id, as_of_ms DESC, id);
            CREATE INDEX idx_capacity_effective_window
                ON capacity_snapshots(
                    subscription_key,
                    COALESCE(window_name, ''),
                    as_of_ms DESC,
                    id
                );

            CREATE TABLE provider_health_snapshots (
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                provider_instance_id TEXT NOT NULL CHECK(provider_instance_id <> ''),
                report_json TEXT NOT NULL,
                last_success_at_ms INTEGER CHECK(last_success_at_ms >= 0),
                checked_at_ms INTEGER NOT NULL CHECK(checked_at_ms >= 0),
                expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms >= checked_at_ms),
                PRIMARY KEY(environment_id, provider_instance_id)
            );
            CREATE INDEX idx_provider_health_expiry
                ON provider_health_snapshots(expires_at_ms, environment_id);",
        )?;
        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (9, ?1)",
            [now_ms() as i64],
        )?;
        tx.commit()?;
    }

    if current < 10 && target_version >= 10 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE turn_attempts (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
                thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
                ordinal INTEGER NOT NULL CHECK(ordinal > 0),
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                provider_instance_id TEXT NOT NULL CHECK(provider_instance_id <> ''),
                subscription_key TEXT CHECK(subscription_key IS NULL OR subscription_key <> ''),
                model TEXT NOT NULL CHECK(model <> ''),
                boundary TEXT NOT NULL,
                status TEXT NOT NULL,
                routing_json TEXT NOT NULL,
                checkpoint TEXT,
                partial_output TEXT NOT NULL DEFAULT '',
                capacity_evidence_json TEXT,
                started_at_ms INTEGER NOT NULL CHECK(started_at_ms >= 0),
                ended_at_ms INTEGER CHECK(ended_at_ms >= started_at_ms),
                UNIQUE(turn_id, ordinal)
            );
            CREATE INDEX idx_turn_attempts_turn
                ON turn_attempts(turn_id, ordinal);

            CREATE TABLE turn_tool_ledger (
                attempt_id TEXT NOT NULL REFERENCES turn_attempts(id) ON DELETE CASCADE,
                turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
                provider_item_id TEXT NOT NULL CHECK(provider_item_id <> ''),
                name TEXT NOT NULL CHECK(name <> ''),
                input_json TEXT NOT NULL,
                status TEXT NOT NULL,
                output TEXT,
                error TEXT,
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0),
                PRIMARY KEY(attempt_id, provider_item_id)
            );
            CREATE INDEX idx_turn_tool_ledger_turn
                ON turn_tool_ledger(turn_id, attempt_id, provider_item_id);

            CREATE TABLE continuations (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                idempotency_key TEXT NOT NULL UNIQUE CHECK(idempotency_key <> ''),
                thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
                turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
                source_attempt_id TEXT NOT NULL REFERENCES turn_attempts(id),
                target_provider_instance_id TEXT,
                target_turn_id TEXT REFERENCES turns(id),
                state TEXT NOT NULL,
                handoff TEXT NOT NULL,
                checkpoint TEXT CHECK(checkpoint IS NULL OR checkpoint <> ''),
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
            );
            CREATE INDEX idx_continuations_turn
                ON continuations(turn_id, created_at_ms, id);
            CREATE UNIQUE INDEX idx_continuations_target_turn
                ON continuations(target_turn_id) WHERE target_turn_id IS NOT NULL;

            CREATE TABLE continuation_deliveries (
                delivery_key TEXT PRIMARY KEY CHECK(delivery_key <> ''),
                continuation_id TEXT REFERENCES continuations(id) ON DELETE CASCADE,
                thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
                turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
                kind TEXT NOT NULL,
                item_id TEXT NOT NULL CHECK(item_id <> ''),
                payload_sha256 TEXT NOT NULL CHECK(length(payload_sha256) = 64),
                payload_json TEXT NOT NULL,
                state TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                delivered_at_ms INTEGER CHECK(delivered_at_ms >= created_at_ms)
            );
            CREATE INDEX idx_continuation_deliveries_turn
                ON continuation_deliveries(turn_id, kind, item_id);",
        )?;
        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (10, ?1)",
            [now_ms() as i64],
        )?;
        tx.commit()?;
    }

    if current < 11 && target_version >= 11 {
        let tx = conn.transaction()?;
        tx.execute_batch("ALTER TABLE turn_attempts ADD COLUMN execution_json TEXT;")?;
        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (11, ?1)",
            [now_ms() as i64],
        )?;
        tx.commit()?;
    }

    if current < 12 && target_version >= 12 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "ALTER TABLE usage_observations ADD COLUMN fact_kind TEXT NOT NULL
                DEFAULT 'aggregate' CHECK(fact_kind <> '');

            UPDATE usage_observations
               SET fact_kind = CASE
                    WHEN input_tokens + cached_input_tokens + cache_write_tokens +
                         output_tokens + reasoning_tokens > 0
                        THEN 'token_component'
                    WHEN provider_reported_micro_usd IS NOT NULL
                        THEN 'turn_cost'
                    ELSE 'aggregate'
               END
             WHERE source_kind = 'claude_transcript';

            DROP INDEX idx_usage_observations_reconciliation;
            DROP INDEX idx_usage_observations_managed_turn;

            CREATE INDEX idx_usage_observations_reconciliation
                ON usage_observations(
                    environment_id,
                    reconciliation_key
                )
                WHERE reconciliation_key IS NOT NULL;

            CREATE INDEX idx_usage_observations_exact_component
                ON usage_observations(
                    environment_id,
                    fact_kind,
                    provider_native_session_id,
                    provider_native_turn_id
                )
                WHERE provider_native_turn_id IS NOT NULL;

            CREATE UNIQUE INDEX idx_usage_observations_managed_aggregate
                ON usage_observations(environment_id, turn_id)
                WHERE source_kind = 'managed_turn'
                  AND turn_id IS NOT NULL
                  AND fact_kind = 'aggregate';

            CREATE UNIQUE INDEX idx_usage_observations_managed_component
                ON usage_observations(
                    environment_id,
                    turn_id,
                    fact_kind,
                    provider_native_turn_id
                )
                WHERE source_kind = 'managed_turn'
                  AND turn_id IS NOT NULL
                  AND fact_kind IN ('token_component', 'turn_cost')
                  AND provider_native_turn_id IS NOT NULL;

            CREATE TABLE managed_usage_backfill_state (
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
                usage_json TEXT NOT NULL,
                expected_fact_count INTEGER NOT NULL CHECK(expected_fact_count > 0),
                completed_at_ms INTEGER NOT NULL CHECK(completed_at_ms >= 0),
                PRIMARY KEY(environment_id, turn_id)
            );

            CREATE TABLE managed_usage_backfill_facts (
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                turn_id TEXT NOT NULL,
                fact_kind TEXT NOT NULL CHECK(
                    fact_kind IN ('aggregate', 'token_component', 'turn_cost')
                ),
                provider_native_turn_id TEXT NOT NULL,
                PRIMARY KEY(
                    environment_id,
                    turn_id,
                    fact_kind,
                    provider_native_turn_id
                ),
                FOREIGN KEY(environment_id, turn_id)
                    REFERENCES managed_usage_backfill_state(environment_id, turn_id)
                    ON DELETE CASCADE
            );

            CREATE TABLE usage_reconciliation_mismatches (
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                managed_observation_id TEXT NOT NULL CHECK(managed_observation_id <> ''),
                transcript_observation_id TEXT NOT NULL CHECK(transcript_observation_id <> ''),
                provider_instance_id TEXT NOT NULL CHECK(provider_instance_id <> ''),
                source_kind TEXT NOT NULL CHECK(
                    source_kind IN ('claude_transcript', 'codex_transcript')
                ),
                detected_at_ms INTEGER NOT NULL CHECK(detected_at_ms >= 0),
                PRIMARY KEY(
                    environment_id,
                    managed_observation_id,
                    transcript_observation_id
                ),
                FOREIGN KEY(environment_id, managed_observation_id)
                    REFERENCES usage_observations(environment_id, observation_id)
                    ON DELETE CASCADE,
                FOREIGN KEY(environment_id, transcript_observation_id)
                    REFERENCES usage_observations(environment_id, observation_id)
                    ON DELETE CASCADE
            );

            CREATE TABLE usage_legacy_reconciliation_debt (
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                provider_instance_id TEXT NOT NULL CHECK(provider_instance_id <> ''),
                source_kind TEXT NOT NULL CHECK(source_kind = 'claude_transcript'),
                parser_version INTEGER NOT NULL CHECK(parser_version > 0),
                detected_at_ms INTEGER NOT NULL CHECK(detected_at_ms >= 0),
                latest_source_at_ms INTEGER CHECK(latest_source_at_ms >= 0),
                transcript_observation_count INTEGER NOT NULL
                    CHECK(transcript_observation_count > 0),
                managed_observation_count INTEGER NOT NULL
                    CHECK(managed_observation_count > 0),
                PRIMARY KEY(environment_id, provider_instance_id, source_kind)
            );

            INSERT INTO usage_legacy_reconciliation_debt (
                environment_id, provider_instance_id, source_kind,
                parser_version, detected_at_ms, latest_source_at_ms,
                transcript_observation_count, managed_observation_count
            )
            SELECT transcript.environment_id,
                   transcript.provider_instance_id,
                   transcript.source_kind,
                   MAX(transcript.parser_version),
                   MAX(transcript.created_at_ms),
                   MAX(transcript.occurred_at_ms),
                   COUNT(*),
                   (
                       SELECT COUNT(*)
                         FROM usage_observations managed
                        WHERE managed.environment_id = transcript.environment_id
                          AND managed.provider_instance_id = transcript.provider_instance_id
                          AND managed.provider = 'claude'
                          AND managed.source_kind = 'managed_turn'
                          AND managed.fact_kind = 'aggregate'
                   )
              FROM usage_observations transcript
             WHERE transcript.provider = 'claude'
               AND transcript.source_kind = 'claude_transcript'
               AND transcript.parser_version < 2
               AND EXISTS (
                   SELECT 1
                     FROM usage_observations managed
                    WHERE managed.environment_id = transcript.environment_id
                      AND managed.provider_instance_id = transcript.provider_instance_id
                      AND managed.provider = 'claude'
                      AND managed.source_kind = 'managed_turn'
                      AND managed.fact_kind = 'aggregate'
               )
             GROUP BY transcript.environment_id,
                      transcript.provider_instance_id,
                      transcript.source_kind;",
        )?;
        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (12, ?1)",
            [now_ms() as i64],
        )?;
        tx.commit()?;
    }

    if current < 13 && target_version >= 13 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "ALTER TABLE settings ADD COLUMN revision INTEGER NOT NULL
                DEFAULT 0 CHECK(revision >= 0);
            ALTER TABLE settings ADD COLUMN updated_at_ms INTEGER NOT NULL
                DEFAULT 0 CHECK(updated_at_ms >= 0);
            UPDATE settings SET revision = 1 WHERE key = 'user_settings';

            CREATE TABLE project_repositories (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                project_id TEXT NOT NULL CHECK(project_id <> ''),
                organization_id TEXT NOT NULL CHECK(organization_id <> ''),
                canonical_remote_identity TEXT NOT NULL CHECK(canonical_remote_identity <> ''),
                canonical_root_identity TEXT NOT NULL CHECK(canonical_root_identity <> ''),
                display_path TEXT NOT NULL,
                revision INTEGER NOT NULL CHECK(revision > 0),
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
                UNIQUE(project_id, organization_id)
            );

            CREATE TABLE project_layer_trust (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                organization_id TEXT NOT NULL CHECK(organization_id <> ''),
                project_id TEXT NOT NULL CHECK(project_id <> ''),
                repository_id TEXT NOT NULL REFERENCES project_repositories(id),
                project_content_sha256 TEXT NOT NULL
                    CHECK(length(project_content_sha256) = 64),
                approved_by TEXT NOT NULL CHECK(approved_by <> ''),
                approved_at_ms INTEGER NOT NULL CHECK(approved_at_ms >= 0),
                revoked_at_ms INTEGER CHECK(revoked_at_ms >= approved_at_ms)
            );
            CREATE UNIQUE INDEX idx_project_layer_trust_active
                ON project_layer_trust(
                    organization_id, project_id, repository_id, project_content_sha256
                ) WHERE revoked_at_ms IS NULL;

            CREATE TABLE personal_repo_state (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                head_source_commit_sha TEXT NOT NULL CHECK(
                    length(head_source_commit_sha) IN (40, 64)
                ),
                revision INTEGER NOT NULL CHECK(revision > 0),
                last_request_identity TEXT,
                last_request_digest TEXT CHECK(
                    last_request_digest IS NULL OR length(last_request_digest) = 64
                ),
                last_result_json TEXT,
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0)
            );

            CREATE TABLE instruction_sources (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                canonical_remote_identity TEXT NOT NULL UNIQUE
                    CHECK(canonical_remote_identity <> ''),
                tracking_kind TEXT NOT NULL CHECK(
                    tracking_kind IN ('branch', 'tag', 'commit')
                ),
                tracking_value TEXT NOT NULL CHECK(tracking_value <> ''),
                update_policy TEXT NOT NULL CHECK(
                    update_policy IN ('approve_each', 'trust_branch')
                ),
                require_signed_commit INTEGER NOT NULL DEFAULT 0
                    CHECK(require_signed_commit IN (0, 1)),
                revision INTEGER NOT NULL CHECK(revision > 0),
                last_checked_at_ms INTEGER CHECK(last_checked_at_ms >= 0),
                last_error_reason TEXT,
                active_source_commit_sha TEXT CHECK(
                    active_source_commit_sha IS NULL OR
                    length(active_source_commit_sha) IN (40, 64)
                ),
                active_bundle_sha TEXT CHECK(
                    active_bundle_sha IS NULL OR length(active_bundle_sha) = 64
                ),
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
            );
            CREATE INDEX idx_instruction_sources_active
                ON instruction_sources(active_bundle_sha, id);

            CREATE TABLE instruction_candidates (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                source_id TEXT NOT NULL REFERENCES instruction_sources(id) ON DELETE CASCADE,
                observed_source_commit_sha TEXT NOT NULL CHECK(
                    length(observed_source_commit_sha) IN (40, 64)
                ),
                observed_tag_object_sha TEXT CHECK(
                    observed_tag_object_sha IS NULL OR
                    length(observed_tag_object_sha) IN (40, 64)
                ),
                state TEXT NOT NULL CHECK(state IN (
                    'discovered', 'inspecting', 'blocked', 'awaiting_approval',
                    'approved', 'activated', 'superseded'
                )),
                safety TEXT NOT NULL CHECK(safety IN (
                    'ordinary', 'access_widening', 'scheduled_agent_change',
                    'history_rewrite', 'remote_identity_change', 'tracking_change'
                )),
                diff_digest TEXT NOT NULL CHECK(length(diff_digest) = 64),
                activatable INTEGER NOT NULL CHECK(activatable IN (0, 1)),
                inspection_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
                UNIQUE(source_id, observed_source_commit_sha)
            );
            CREATE INDEX idx_instruction_candidates_unresolved
                ON instruction_candidates(source_id, created_at_ms, id)
                WHERE state IN ('discovered', 'inspecting', 'awaiting_approval');

            CREATE TABLE configuration_audit (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                kind TEXT NOT NULL CHECK(kind <> ''),
                actor TEXT NOT NULL CHECK(actor <> ''),
                source_id TEXT REFERENCES instruction_sources(id),
                candidate_id TEXT REFERENCES instruction_candidates(id),
                bundle_sha TEXT CHECK(bundle_sha IS NULL OR length(bundle_sha) = 64),
                details_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
            );
            CREATE INDEX idx_configuration_audit_created
                ON configuration_audit(created_at_ms DESC, id);

            CREATE TABLE bundle_objects (
                bundle_sha TEXT PRIMARY KEY CHECK(length(bundle_sha) = 64),
                source_manifest_json TEXT NOT NULL,
                payload_sha256 TEXT NOT NULL CHECK(length(payload_sha256) = 64),
                archive_sha256 TEXT CHECK(
                    archive_sha256 IS NULL OR length(archive_sha256) = 64
                ),
                archive_bytes INTEGER CHECK(archive_bytes IS NULL OR archive_bytes >= 0),
                compiler_version TEXT NOT NULL CHECK(compiler_version <> ''),
                object_path TEXT NOT NULL CHECK(object_path <> ''),
                verified_at_ms INTEGER NOT NULL CHECK(verified_at_ms >= 0)
            );

            CREATE TABLE bundle_activations (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                bundle_sha TEXT NOT NULL REFERENCES bundle_objects(bundle_sha),
                candidate_id TEXT REFERENCES instruction_candidates(id),
                source_manifest_json TEXT NOT NULL,
                kind TEXT NOT NULL CHECK(kind IN (
                    'candidate_approval', 'trusted_branch', 'revert', 'recovery'
                )),
                actor TEXT NOT NULL CHECK(actor <> ''),
                prior_bundle_sha TEXT REFERENCES bundle_objects(bundle_sha),
                activated_at_ms INTEGER NOT NULL CHECK(activated_at_ms >= 0),
                deactivated_at_ms INTEGER CHECK(deactivated_at_ms >= activated_at_ms)
            );
            CREATE UNIQUE INDEX idx_bundle_activations_current
                ON bundle_activations((1)) WHERE deactivated_at_ms IS NULL;

            CREATE TABLE bundle_distribution (
                transfer_id TEXT PRIMARY KEY CHECK(transfer_id <> ''),
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                bundle_sha TEXT NOT NULL REFERENCES bundle_objects(bundle_sha),
                archive_sha256 TEXT NOT NULL CHECK(length(archive_sha256) = 64),
                expected_prior_bundle_sha TEXT REFERENCES bundle_objects(bundle_sha),
                state TEXT NOT NULL CHECK(state IN (
                    'pending', 'uploading', 'active', 'rejected'
                )),
                receipt_json TEXT,
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
                UNIQUE(environment_id, bundle_sha)
            );
            CREATE INDEX idx_bundle_distribution_incomplete
                ON bundle_distribution(state, created_at_ms, transfer_id)
                WHERE state IN ('pending', 'uploading');

            CREATE TABLE mutation_proposals (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                request_id TEXT NOT NULL CHECK(request_id <> ''),
                caller_identity TEXT NOT NULL CHECK(caller_identity <> ''),
                caller_json TEXT NOT NULL,
                tool_name TEXT NOT NULL CHECK(tool_name <> ''),
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                target_json TEXT NOT NULL,
                expected_revision INTEGER NOT NULL CHECK(expected_revision > 0),
                old_json TEXT NOT NULL,
                new_json TEXT NOT NULL,
                old_digest TEXT NOT NULL CHECK(length(old_digest) = 64),
                new_digest TEXT NOT NULL CHECK(length(new_digest) = 64),
                impact_json TEXT NOT NULL,
                resolution TEXT NOT NULL CHECK(resolution IN (
                    'pending', 'approved_applied', 'rejected', 'expired',
                    'cancelled_disconnected', 'superseded_revision'
                )),
                audit_id TEXT UNIQUE,
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms > created_at_ms),
                resolved_at_ms INTEGER CHECK(resolved_at_ms >= created_at_ms),
                UNIQUE(caller_identity, tool_name, request_id)
            );
            CREATE INDEX idx_mutation_proposals_pending_expiry
                ON mutation_proposals(expires_at_ms, id)
                WHERE resolution = 'pending';

            CREATE TABLE mutation_audit (
                id TEXT PRIMARY KEY CHECK(id <> ''),
                proposal_id TEXT NOT NULL UNIQUE REFERENCES mutation_proposals(id),
                target_json TEXT NOT NULL,
                old_digest TEXT NOT NULL CHECK(length(old_digest) = 64),
                new_digest TEXT NOT NULL CHECK(length(new_digest) = 64),
                resolver_json TEXT NOT NULL,
                applied_revision INTEGER NOT NULL CHECK(applied_revision > 0),
                applied_at_ms INTEGER NOT NULL CHECK(applied_at_ms >= 0)
            );",
        )?;
        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (13, ?1)",
            [now_ms() as i64],
        )?;
        tx.commit()?;
    }

    if current < 14 && target_version >= 14 {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE orchestration_runs (
                run_id TEXT PRIMARY KEY CHECK(run_id <> ''),
                organization_id TEXT NOT NULL CHECK(organization_id <> ''),
                project_id TEXT NOT NULL CHECK(project_id <> ''),
                repository_id TEXT NOT NULL REFERENCES project_repositories(id),
                snapshot_bytes BLOB NOT NULL CHECK(length(snapshot_bytes) > 0),
                snapshot_sha256 TEXT NOT NULL CHECK(length(snapshot_sha256) = 64),
                last_seq INTEGER NOT NULL CHECK(last_seq >= 0),
                status TEXT NOT NULL CHECK(status IN (
                    'draft', 'awaiting_approval', 'ready', 'running', 'paused',
                    'cancelling', 'succeeded', 'failed', 'cancelled'
                )),
                bundle_sha TEXT NOT NULL CHECK(length(bundle_sha) = 64),
                policy_revision TEXT NOT NULL CHECK(policy_revision <> ''),
                settings_revision INTEGER NOT NULL CHECK(settings_revision > 0),
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
                ended_at_ms INTEGER CHECK(ended_at_ms >= created_at_ms),
                CHECK(
                    (status IN ('succeeded', 'failed', 'cancelled')) =
                    (ended_at_ms IS NOT NULL)
                )
            );
            CREATE INDEX idx_orchestration_runs_nonterminal
                ON orchestration_runs(updated_at_ms, run_id)
                WHERE status NOT IN ('succeeded', 'failed', 'cancelled');

            CREATE TABLE orchestration_run_events (
                event_id TEXT PRIMARY KEY CHECK(event_id <> ''),
                run_id TEXT NOT NULL REFERENCES orchestration_runs(run_id) ON DELETE CASCADE,
                seq INTEGER NOT NULL CHECK(seq > 0),
                idempotency_key TEXT NOT NULL CHECK(idempotency_key <> ''),
                event_bytes BLOB NOT NULL CHECK(length(event_bytes) > 0),
                payload_sha256 TEXT NOT NULL CHECK(length(payload_sha256) = 64),
                at_ms INTEGER NOT NULL CHECK(at_ms >= 0),
                UNIQUE(run_id, seq),
                UNIQUE(run_id, idempotency_key)
            );
            CREATE INDEX idx_orchestration_run_events_replay
                ON orchestration_run_events(run_id, seq);
            CREATE TRIGGER orchestration_run_events_immutable_update
                BEFORE UPDATE ON orchestration_run_events
                BEGIN SELECT RAISE(ABORT, 'orchestration events are immutable'); END;
            CREATE TRIGGER orchestration_run_events_immutable_delete
                BEFORE DELETE ON orchestration_run_events
                BEGIN SELECT RAISE(ABORT, 'orchestration events are immutable'); END;

            CREATE TABLE orchestration_leases (
                lease_id TEXT PRIMARY KEY CHECK(lease_id <> ''),
                resource_kind TEXT NOT NULL CHECK(resource_kind IN (
                    'effect', 'capacity', 'worktree', 'remote_attempt'
                )),
                resource_id TEXT NOT NULL CHECK(resource_id <> ''),
                owner_incarnation_id TEXT NOT NULL CHECK(owner_incarnation_id <> ''),
                expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms >= 0),
                last_heartbeat_ms INTEGER NOT NULL CHECK(last_heartbeat_ms >= 0),
                revision INTEGER NOT NULL CHECK(revision > 0),
                CHECK(expires_at_ms >= last_heartbeat_ms),
                UNIQUE(resource_kind, resource_id)
            );
            CREATE INDEX idx_orchestration_leases_expiry
                ON orchestration_leases(expires_at_ms, resource_kind, resource_id);

            CREATE TABLE orchestration_effects (
                effect_id TEXT PRIMARY KEY CHECK(effect_id <> ''),
                run_id TEXT NOT NULL REFERENCES orchestration_runs(run_id) ON DELETE CASCADE,
                source_event_id TEXT NOT NULL REFERENCES orchestration_run_events(event_id),
                effect_bytes BLOB NOT NULL CHECK(length(effect_bytes) > 0),
                payload_sha256 TEXT NOT NULL CHECK(length(payload_sha256) = 64),
                delivery_state TEXT NOT NULL CHECK(delivery_state IN (
                    'pending', 'claimed', 'prepared', 'dispatch_started', 'in_flight',
                    'outcome_recorded', 'settled', 'cancelled', 'review_required'
                )),
                owner_incarnation_id TEXT,
                lease_id TEXT REFERENCES orchestration_leases(lease_id),
                claim_revision INTEGER NOT NULL DEFAULT 0 CHECK(claim_revision >= 0),
                prepared_receipt_digest TEXT CHECK(
                    prepared_receipt_digest IS NULL OR length(prepared_receipt_digest) = 64
                ),
                dispatch_started_at_ms INTEGER CHECK(dispatch_started_at_ms >= 0),
                result_digest TEXT CHECK(result_digest IS NULL OR length(result_digest) = 64),
                safe_error_code TEXT CHECK(safe_error_code IS NULL OR safe_error_code <> ''),
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
                CHECK((owner_incarnation_id IS NULL) = (lease_id IS NULL)),
                CHECK(
                    delivery_state NOT IN ('outcome_recorded', 'settled') OR
                    result_digest IS NOT NULL
                )
            );
            CREATE INDEX idx_orchestration_effects_pending
                ON orchestration_effects(delivery_state, created_at_ms, effect_id)
                WHERE delivery_state NOT IN ('settled', 'cancelled');
            CREATE TRIGGER orchestration_effect_identity_immutable
                BEFORE UPDATE ON orchestration_effects
                WHEN OLD.effect_id <> NEW.effect_id OR OLD.run_id <> NEW.run_id OR
                     OLD.source_event_id <> NEW.source_event_id OR
                     OLD.effect_bytes <> NEW.effect_bytes OR
                     OLD.payload_sha256 <> NEW.payload_sha256 OR
                     OLD.created_at_ms <> NEW.created_at_ms
                BEGIN SELECT RAISE(ABORT, 'orchestration effect identity is immutable'); END;
            CREATE TRIGGER orchestration_effects_immutable_delete
                BEFORE DELETE ON orchestration_effects
                BEGIN SELECT RAISE(ABORT, 'orchestration effects are immutable'); END;

            CREATE TABLE orchestration_attempt_dispatch (
                attempt_id TEXT PRIMARY KEY CHECK(attempt_id <> ''),
                run_id TEXT NOT NULL REFERENCES orchestration_runs(run_id) ON DELETE CASCADE,
                task_id TEXT NOT NULL CHECK(task_id <> ''),
                effect_id TEXT NOT NULL UNIQUE REFERENCES orchestration_effects(effect_id),
                thread_id TEXT NOT NULL UNIQUE CHECK(thread_id <> ''),
                turn_id TEXT NOT NULL UNIQUE CHECK(turn_id <> ''),
                turn_attempt_id TEXT NOT NULL UNIQUE CHECK(turn_attempt_id <> ''),
                provider_instance_id TEXT NOT NULL CHECK(provider_instance_id <> ''),
                route_bytes BLOB NOT NULL CHECK(length(route_bytes) > 0),
                route_sha256 TEXT NOT NULL CHECK(length(route_sha256) = 64),
                bundle_sha TEXT NOT NULL CHECK(length(bundle_sha) = 64),
                state TEXT NOT NULL CHECK(state IN (
                    'prepared', 'dispatch_started', 'accepted', 'not_started',
                    'ambiguous', 'terminal', 'cancel_requested', 'cancelled'
                )),
                last_boundary TEXT NOT NULL CHECK(last_boundary IN (
                    'safe_to_retry', 'request_submitted', 'visible_output_begun',
                    'tool_started', 'external_commit'
                )),
                last_progress_at_ms INTEGER NOT NULL CHECK(last_progress_at_ms >= 0),
                cancel_requested_at_ms INTEGER CHECK(cancel_requested_at_ms >= 0),
                ambiguity_reason TEXT,
                outcome_digest TEXT CHECK(outcome_digest IS NULL OR length(outcome_digest) = 64),
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
                CHECK(last_progress_at_ms >= created_at_ms),
                CHECK(cancel_requested_at_ms IS NULL OR cancel_requested_at_ms >= created_at_ms),
                CHECK(state <> 'ambiguous' OR ambiguity_reason IS NOT NULL),
                CHECK(state <> 'terminal' OR outcome_digest IS NOT NULL)
            );
            CREATE INDEX idx_orchestration_attempt_dispatch_active
                ON orchestration_attempt_dispatch(state, last_progress_at_ms, attempt_id)
                WHERE state NOT IN ('not_started', 'terminal', 'cancelled');

            CREATE TABLE orchestration_queue (
                queue_id TEXT PRIMARY KEY CHECK(queue_id <> ''),
                run_id TEXT NOT NULL CHECK(run_id <> ''),
                task_id TEXT NOT NULL CHECK(task_id <> ''),
                attempt_id TEXT NOT NULL CHECK(attempt_id <> ''),
                agent_id TEXT NOT NULL CHECK(agent_id <> ''),
                organization_id TEXT,
                agent_concurrency_limit INTEGER CHECK(
                    agent_concurrency_limit IS NULL OR agent_concurrency_limit > 0
                ),
                provider_instance_id TEXT NOT NULL CHECK(provider_instance_id <> ''),
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                candidate_sha256 TEXT NOT NULL CHECK(length(candidate_sha256) = 64),
                priority INTEGER NOT NULL,
                task_enqueued_at_ms INTEGER NOT NULL CHECK(task_enqueued_at_ms >= 0),
                enqueued_at_ms INTEGER NOT NULL CHECK(enqueued_at_ms >= task_enqueued_at_ms),
                not_before_ms INTEGER NOT NULL CHECK(not_before_ms >= enqueued_at_ms),
                state TEXT NOT NULL CHECK(state IN (
                    'queued', 'claimed', 'cancelled', 'dispatched'
                )),
                owner_incarnation_id TEXT,
                lease_id TEXT REFERENCES orchestration_leases(lease_id),
                claim_revision INTEGER NOT NULL DEFAULT 0 CHECK(claim_revision >= 0),
                interactive INTEGER NOT NULL DEFAULT 0 CHECK(interactive IN (0, 1)),
                claimed_at_ms INTEGER CHECK(
                    claimed_at_ms IS NULL OR claimed_at_ms >= enqueued_at_ms
                ),
                claim_order INTEGER UNIQUE CHECK(claim_order IS NULL OR claim_order > 0),
                interactive_streak INTEGER CHECK(
                    interactive_streak IS NULL OR interactive_streak BETWEEN 0 AND 4
                ),
                UNIQUE(run_id, task_id, attempt_id),
                CHECK((owner_incarnation_id IS NULL) = (lease_id IS NULL)),
                CHECK(interactive = 1 OR (
                    organization_id IS NOT NULL AND agent_concurrency_limit IS NOT NULL
                )),
                CHECK(interactive = 0 OR agent_concurrency_limit IS NULL),
                CHECK(
                    (claimed_at_ms IS NULL) = (claim_order IS NULL) AND
                    (claimed_at_ms IS NULL) = (interactive_streak IS NULL)
                )
            );
            CREATE INDEX idx_orchestration_queue_order
                ON orchestration_queue(
                    interactive DESC, enqueued_at_ms,
                    run_id, task_id, attempt_id
                ) WHERE state = 'queued';

            CREATE TABLE orchestration_worktrees (
                worktree_id TEXT PRIMARY KEY CHECK(worktree_id <> ''),
                repository_id TEXT NOT NULL REFERENCES project_repositories(id),
                run_id TEXT NOT NULL REFERENCES orchestration_runs(run_id) ON DELETE CASCADE,
                task_id TEXT NOT NULL CHECK(task_id <> ''),
                attempt_id TEXT NOT NULL UNIQUE CHECK(attempt_id <> ''),
                environment_id TEXT NOT NULL CHECK(environment_id <> ''),
                git_common_dir_identity TEXT NOT NULL CHECK(git_common_dir_identity <> ''),
                baseline_commit TEXT NOT NULL CHECK(baseline_commit <> ''),
                branch_ref TEXT NOT NULL UNIQUE CHECK(branch_ref <> ''),
                path TEXT NOT NULL UNIQUE CHECK(path <> ''),
                lease_id TEXT NOT NULL UNIQUE REFERENCES orchestration_leases(lease_id),
                checkpoint_ids_json TEXT NOT NULL,
                dirty_state TEXT NOT NULL CHECK(dirty_state IN (
                    'clean', 'dirty', 'locked', 'ambiguous'
                )),
                cleanup_eligibility TEXT NOT NULL CHECK(cleanup_eligibility IN (
                    'ineligible', 'eligible', 'retained'
                )),
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
            );
            CREATE INDEX idx_orchestration_worktrees_active
                ON orchestration_worktrees(cleanup_eligibility, updated_at_ms, worktree_id)
                WHERE cleanup_eligibility <> 'eligible';

            CREATE TABLE orchestration_budget_reservations (
                reservation_id TEXT PRIMARY KEY CHECK(reservation_id <> ''),
                run_id TEXT NOT NULL REFERENCES orchestration_runs(run_id) ON DELETE CASCADE,
                task_id TEXT NOT NULL CHECK(task_id <> ''),
                attempt_id TEXT NOT NULL UNIQUE CHECK(attempt_id <> ''),
                agent_id TEXT NOT NULL CHECK(agent_id <> ''),
                organization_id TEXT NOT NULL CHECK(organization_id <> ''),
                agent_limit_tokens INTEGER NOT NULL CHECK(agent_limit_tokens >= 0),
                agent_limit_micro_usd INTEGER NOT NULL CHECK(agent_limit_micro_usd >= 0),
                agent_limit_runtime_ms INTEGER NOT NULL CHECK(agent_limit_runtime_ms >= 0),
                ledger_utc_date TEXT NOT NULL CHECK(
                    length(ledger_utc_date) = 10 AND
                    substr(ledger_utc_date, 5, 1) = '-' AND
                    substr(ledger_utc_date, 8, 1) = '-'
                ),
                reserved_tokens INTEGER NOT NULL CHECK(reserved_tokens >= 0),
                reserved_micro_usd INTEGER NOT NULL CHECK(reserved_micro_usd >= 0),
                reserved_runtime_ms INTEGER NOT NULL CHECK(reserved_runtime_ms >= 0),
                actual_tokens INTEGER CHECK(actual_tokens >= 0),
                actual_micro_usd INTEGER CHECK(actual_micro_usd >= 0),
                actual_runtime_ms INTEGER CHECK(actual_runtime_ms >= 0),
                state TEXT NOT NULL CHECK(state IN (
                    'reserved', 'partially_reconciled', 'reconciled', 'released'
                )),
                grants_json TEXT NOT NULL CHECK(length(grants_json) > 0),
                reconciliation_json TEXT,
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
                CHECK(
                    state <> 'reconciled' OR
                    (actual_tokens IS NOT NULL AND actual_micro_usd IS NOT NULL AND
                     actual_runtime_ms IS NOT NULL)
                )
            );
            CREATE INDEX idx_orchestration_budget_incomplete
                ON orchestration_budget_reservations(state, updated_at_ms, reservation_id)
                WHERE state IN ('reserved', 'partially_reconciled');

            CREATE TABLE orchestration_daily_budgets (
                organization_id TEXT NOT NULL CHECK(organization_id <> ''),
                ledger_utc_date TEXT NOT NULL CHECK(
                    length(ledger_utc_date) = 10 AND
                    substr(ledger_utc_date, 5, 1) = '-' AND
                    substr(ledger_utc_date, 8, 1) = '-'
                ),
                limit_micro_usd INTEGER NOT NULL CHECK(limit_micro_usd >= 0),
                reserved_micro_usd INTEGER NOT NULL CHECK(reserved_micro_usd >= 0),
                reconciled_micro_usd INTEGER NOT NULL CHECK(reconciled_micro_usd >= 0),
                granted_micro_usd INTEGER NOT NULL CHECK(granted_micro_usd >= 0),
                revision INTEGER NOT NULL CHECK(revision > 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0),
                PRIMARY KEY(organization_id, ledger_utc_date)
            );

            CREATE TABLE orchestration_blackboard_artifacts (
                entry_id TEXT PRIMARY KEY CHECK(entry_id <> ''),
                run_id TEXT NOT NULL REFERENCES orchestration_runs(run_id) ON DELETE CASCADE,
                artifact_key TEXT NOT NULL CHECK(artifact_key <> ''),
                version INTEGER NOT NULL CHECK(version > 0),
                previous_entry_id TEXT REFERENCES orchestration_blackboard_artifacts(entry_id),
                producer_task_id TEXT NOT NULL CHECK(producer_task_id <> ''),
                consumers_json TEXT NOT NULL,
                output_contract_json TEXT NOT NULL,
                schema_sha256 TEXT NOT NULL CHECK(length(schema_sha256) = 64),
                canonical_json BLOB NOT NULL CHECK(length(canonical_json) <= 1048576),
                byte_len INTEGER NOT NULL CHECK(byte_len >= 0 AND byte_len <= 1048576),
                content_sha256 TEXT NOT NULL CHECK(length(content_sha256) = 64),
                created_event_id TEXT NOT NULL REFERENCES orchestration_run_events(event_id),
                provenance_json TEXT,
                CHECK(byte_len = length(canonical_json)),
                CHECK(
                    (version = 1 AND previous_entry_id IS NULL) OR
                    (version > 1 AND previous_entry_id IS NOT NULL)
                ),
                UNIQUE(run_id, artifact_key, version)
            );
            CREATE INDEX idx_orchestration_artifact_versions
                ON orchestration_blackboard_artifacts(run_id, artifact_key, version DESC);
            CREATE TRIGGER orchestration_blackboard_artifacts_immutable_update
                BEFORE UPDATE ON orchestration_blackboard_artifacts
                BEGIN SELECT RAISE(ABORT, 'orchestration artifacts are immutable'); END;
            CREATE TRIGGER orchestration_blackboard_artifacts_immutable_delete
                BEFORE DELETE ON orchestration_blackboard_artifacts
                BEGIN SELECT RAISE(ABORT, 'orchestration artifacts are immutable'); END;

            CREATE TABLE orchestration_attempt_progress (
                progress_id TEXT PRIMARY KEY CHECK(progress_id <> ''),
                run_id TEXT NOT NULL REFERENCES orchestration_runs(run_id) ON DELETE CASCADE,
                task_id TEXT NOT NULL CHECK(task_id <> ''),
                attempt_id TEXT NOT NULL CHECK(attempt_id <> ''),
                kind TEXT NOT NULL CHECK(kind IN (
                    'boundary_advance', 'stream_bytes', 'tool_lifecycle',
                    'approval_transition', 'checkpoint_created', 'usage_receipt',
                    'artifact_published', 'terminal_outcome'
                )),
                payload_digest TEXT NOT NULL CHECK(length(payload_digest) = 64),
                at_ms INTEGER NOT NULL CHECK(at_ms >= 0),
                cumulative_stream_bytes INTEGER CHECK(cumulative_stream_bytes >= 0),
                terminal INTEGER NOT NULL CHECK(terminal IN (0, 1)),
                CHECK((kind = 'stream_bytes') OR cumulative_stream_bytes IS NULL),
                CHECK((kind = 'terminal_outcome') = terminal)
            );
            CREATE INDEX idx_orchestration_attempt_progress_recent
                ON orchestration_attempt_progress(attempt_id, at_ms DESC, progress_id DESC);

            CREATE TABLE orchestration_recovery_reviews (
                review_id TEXT PRIMARY KEY CHECK(review_id <> ''),
                run_id TEXT NOT NULL REFERENCES orchestration_runs(run_id) ON DELETE CASCADE,
                task_id TEXT NOT NULL CHECK(task_id <> ''),
                attempt_id TEXT NOT NULL CHECK(attempt_id <> ''),
                effect_id TEXT REFERENCES orchestration_effects(effect_id),
                reason_code TEXT NOT NULL CHECK(reason_code <> ''),
                evidence_digest TEXT NOT NULL CHECK(length(evidence_digest) = 64),
                status TEXT NOT NULL CHECK(status IN ('pending', 'resolved')),
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                resolved_at_ms INTEGER CHECK(resolved_at_ms >= created_at_ms),
                resolution TEXT,
                CHECK(
                    (status = 'pending' AND resolved_at_ms IS NULL AND resolution IS NULL) OR
                    (status = 'resolved' AND resolved_at_ms IS NOT NULL AND resolution IS NOT NULL)
                )
            );
            CREATE UNIQUE INDEX idx_orchestration_recovery_review_identity
                ON orchestration_recovery_reviews(
                    run_id, task_id, attempt_id, ifnull(effect_id, ''),
                    reason_code, evidence_digest
                );
            CREATE INDEX idx_orchestration_recovery_reviews_unresolved
                ON orchestration_recovery_reviews(created_at_ms, review_id)
                WHERE status = 'pending';

            CREATE TABLE orchestration_node_receipts (
                receipt_id TEXT PRIMARY KEY CHECK(receipt_id <> ''),
                node_store_id TEXT NOT NULL CHECK(node_store_id <> ''),
                run_id TEXT NOT NULL CHECK(run_id <> ''),
                task_id TEXT NOT NULL CHECK(task_id <> ''),
                attempt_id TEXT NOT NULL CHECK(attempt_id <> ''),
                effect_id TEXT NOT NULL CHECK(effect_id <> ''),
                request_kind TEXT NOT NULL CHECK(request_kind IN (
                    'repository_prepare', 'attempt_prepare', 'attempt_dispatch',
                    'attempt_status', 'attempt_cancel', 'attempt_events', 'attempt_ack'
                )),
                idempotency_key TEXT NOT NULL CHECK(idempotency_key <> ''),
                request_sha256 TEXT NOT NULL CHECK(length(request_sha256) = 64),
                result_sha256 TEXT NOT NULL CHECK(length(result_sha256) = 64),
                result_bytes BLOB NOT NULL CHECK(length(result_bytes) > 0),
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
                UNIQUE(node_store_id, request_kind, idempotency_key)
            );
            CREATE INDEX idx_orchestration_node_receipts_updated
                ON orchestration_node_receipts(node_store_id, updated_at_ms, receipt_id);
            CREATE INDEX idx_orchestration_node_receipts_attempt
                ON orchestration_node_receipts(run_id, attempt_id, updated_at_ms, receipt_id);
            CREATE TRIGGER orchestration_node_receipts_immutable_update
                BEFORE UPDATE ON orchestration_node_receipts
                BEGIN SELECT RAISE(ABORT, 'orchestration node receipts are immutable'); END;
            CREATE TRIGGER orchestration_node_receipts_immutable_delete
                BEFORE DELETE ON orchestration_node_receipts
                BEGIN SELECT RAISE(ABORT, 'orchestration node receipts are immutable'); END;",
        )?;
        tx.execute(
            "INSERT INTO _migrations (version, applied_at_ms) VALUES (14, ?1)",
            [now_ms() as i64],
        )?;
        tx.commit()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tempfile::NamedTempFile;

    fn migration_version(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _migrations",
            [],
            |row| row.get(0),
        )
        .expect("migration version")
    }

    fn table_exists(conn: &Connection, table: &str) -> bool {
        conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
             )",
            [table],
            |row| row.get::<_, bool>(0),
        )
        .expect("table lookup")
    }

    fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("prepare column lookup");
        stmt.query_map([], |row| row.get::<_, String>(1))
            .expect("query columns")
            .any(|name| name.is_ok_and(|name| name == column))
    }

    fn insert_legacy_bucket(conn: &Connection, day: &str, provider: &str, input_tokens: i64) {
        conn.execute(
            "INSERT INTO usage_buckets (
                day, hour_start, environment_id, provider, model, account_key,
                agent_id, schedule_id, input_tokens, cached_input_tokens,
                cache_write_tokens, output_tokens, reasoning_tokens,
                cost_usd_estimate, records, created_at_ms
             ) VALUES (?1, 14, 'env-legacy', ?2, 'model-legacy',
                'person@example.com', 'agent-legacy', NULL, ?3, 20, 5, 30, 10,
                0.25, 2, 1700000000000)",
            params![day, provider, input_tokens],
        )
        .expect("insert legacy bucket");
    }

    fn existing_table_contents(conn: &Connection) -> BTreeMap<String, Vec<String>> {
        let table_names = {
            let mut statement = conn
                .prepare(
                    "SELECT name FROM sqlite_master
                      WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                        AND name <> '_migrations'
                      ORDER BY name",
                )
                .expect("prepare table inventory");
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .expect("query table inventory")
                .collect::<Result<Vec<_>>>()
                .expect("collect table inventory")
        };
        table_names
            .into_iter()
            .map(|table| {
                let escaped_table = table.replace('"', "\"\"");
                let columns = {
                    let mut statement = conn
                        .prepare(&format!("PRAGMA table_info(\"{escaped_table}\")"))
                        .expect("prepare column inventory");
                    statement
                        .query_map([], |row| row.get::<_, String>(1))
                        .expect("query column inventory")
                        .collect::<Result<Vec<_>>>()
                        .expect("collect column inventory")
                };
                let expression = columns
                    .iter()
                    .map(|column| format!("quote(\"{}\")", column.replace('"', "\"\"")))
                    .collect::<Vec<_>>()
                    .join(" || char(31) || ");
                let mut statement = conn
                    .prepare(&format!(
                        "SELECT {expression} FROM \"{escaped_table}\" ORDER BY 1"
                    ))
                    .expect("prepare table snapshot");
                let rows = statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .expect("query table snapshot")
                    .collect::<Result<Vec<_>>>()
                    .expect("collect table snapshot");
                (table, rows)
            })
            .collect()
    }

    #[test]
    fn upgrades_every_previous_schema_and_repeats_safely() {
        for prior_version in 1..SCHEMA_VERSION {
            let mut conn = Connection::open_in_memory().expect("open database");
            apply_migrations_to(&mut conn, prior_version).expect("apply prior migrations");
            assert_eq!(migration_version(&conn), prior_version);

            apply_migrations(&mut conn).expect("upgrade database");
            apply_migrations(&mut conn).expect("repeat migrations");

            assert_eq!(migration_version(&conn), SCHEMA_VERSION);
            assert_eq!(
                conn.query_row("SELECT COUNT(*) FROM _migrations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("migration count"),
                SCHEMA_VERSION
            );
            assert_eq!(
                conn.query_row("SELECT COUNT(*) FROM usage_store_meta", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("store metadata count"),
                1
            );
        }
    }

    #[test]
    fn migration_nine_preserves_capacity_revisions_and_adds_health_storage() {
        let mut conn = Connection::open_in_memory().expect("open database");
        apply_migrations_to(&mut conn, 8).expect("apply schema eight");
        conn.execute(
            "INSERT INTO usage_subscriptions (
                subscription_key, provider, display_name, created_at_ms, updated_at_ms
             ) VALUES ('subkey-capacity', 'codex', 'Capacity fixture', 1000, 1000)",
            [],
        )
        .expect("insert subscription");
        conn.execute(
            "INSERT INTO provider_subscription_mappings (
                environment_id, provider_instance_id, subscription_key, updated_at_ms
             ) VALUES ('env-capacity', 'codex-capacity', 'subkey-capacity', 1000)",
            [],
        )
        .expect("insert provider mapping");
        conn.execute(
            "INSERT INTO capacity_snapshots (
                subscription_key, window_start_ms, window_end_ms,
                used_millionths, remaining_millionths, reset_at_ms,
                source, confidence, provider_version, as_of_ms
             ) VALUES (
                'subkey-capacity', 1000, 2000, 250000, 750000, 2000,
                'legacy_fixture', 'reported', 'fixture', 1500
             )",
            [],
        )
        .expect("insert legacy capacity revision");
        conn.execute(
            "INSERT INTO capacity_snapshots (
                subscription_key, window_start_ms, window_end_ms,
                used_millionths, remaining_millionths, reset_at_ms,
                source, confidence, provider_version, as_of_ms
             ) VALUES (
                'subkey-capacity', 3000, 4000, 500000, 500000, 4000,
                'legacy_fixture', 'reported', 'fixture', 1500
             )",
            [],
        )
        .expect("insert second legacy capacity window");

        apply_migrations(&mut conn).expect("apply migration nine");

        let migrated = conn
            .query_row(
                "SELECT schema_version, environment_id, evidence_scope,
                        hard_limit, max_age_ms, received_at_ms
                 FROM capacity_snapshots",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .expect("query migrated capacity revision");
        assert_eq!(
            migrated,
            (
                1,
                "env-capacity".to_string(),
                "local".to_string(),
                0,
                900_000,
                1_500
            )
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM capacity_snapshots", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("migrated capacity revision count"),
            2
        );
        assert!(table_exists(&conn, "provider_health_snapshots"));

        apply_migrations(&mut conn).expect("repeat migration nine");
        assert_eq!(migration_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn migration_ten_adds_durable_failover_state() {
        let mut conn = Connection::open_in_memory().expect("open database");
        apply_migrations_to(&mut conn, 9).expect("apply schema nine");

        apply_migrations_to(&mut conn, 10).expect("apply migration ten");

        for table in [
            "turn_attempts",
            "turn_tool_ledger",
            "continuations",
            "continuation_deliveries",
        ] {
            assert!(table_exists(&conn, table), "missing {table}");
        }
        assert_eq!(migration_version(&conn), 10);
        apply_migrations_to(&mut conn, 10).expect("repeat migration ten");
        assert_eq!(migration_version(&conn), 10);
    }

    #[test]
    fn migration_eleven_adds_per_attempt_execution_evidence() {
        let mut conn = Connection::open_in_memory().expect("open database");
        apply_migrations_to(&mut conn, 10).expect("apply schema ten");
        assert!(!column_exists(&conn, "turn_attempts", "execution_json"));

        apply_migrations(&mut conn).expect("apply migration eleven");

        assert!(column_exists(&conn, "turn_attempts", "execution_json"));
        assert_eq!(migration_version(&conn), SCHEMA_VERSION);
        apply_migrations(&mut conn).expect("repeat migration eleven");
        assert_eq!(migration_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn migration_twelve_preserves_history_and_allows_managed_components() {
        let mut conn = Connection::open_in_memory().expect("open database");
        apply_migrations_to(&mut conn, 11).expect("apply schema eleven");
        conn.execute(
            "INSERT INTO usage_subscriptions (
                subscription_key, provider, display_name, created_at_ms, updated_at_ms
             ) VALUES ('subkey-p4c1', 'claude', 'P4-C1 fixture', 1, 1)",
            [],
        )
        .expect("insert subscription");
        conn.execute(
            "INSERT INTO usage_observations (
                environment_id, observation_id, source_kind, parser_version,
                provider_native_session_id, provider_native_turn_id, occurred_at_ms,
                provider, provider_instance_id, subscription_key, model,
                input_tokens, cached_input_tokens, cache_write_tokens, output_tokens,
                reasoning_tokens, other_tokens_json, provider_reported_micro_usd,
                created_at_ms
             ) VALUES (
                'env-p4c1', 'legacy-transcript', 'claude_transcript', 1,
                'session-p4c1', 'message-p4c1', 1000, 'claude', 'claude-p4c1',
                'subkey-p4c1', 'claude-actual', 11, 3, 2, 7, 0, '{}', 1234, 1000
             )",
            [],
        )
        .expect("insert legacy transcript observation");
        conn.execute(
            "INSERT INTO usage_observations (
                environment_id, observation_id, source_kind, parser_version,
                provider_native_session_id, provider_native_turn_id, occurred_at_ms,
                provider, provider_instance_id, subscription_key, model,
                input_tokens, cached_input_tokens, cache_write_tokens, output_tokens,
                reasoning_tokens, other_tokens_json, provider_reported_micro_usd,
                turn_id, created_at_ms
             ) VALUES (
                'env-p4c1', 'legacy-managed-result', 'managed_turn', 1,
                'session-p4c1', 'result-uuid', 1001, 'claude', 'claude-p4c1',
                'subkey-p4c1', 'selected-model', 11, 3, 2, 7, 0, '{}', 1234,
                'legacy-pandamux-turn', 1001
             )",
            [],
        )
        .expect("insert legacy managed result observation");

        apply_migrations(&mut conn).expect("apply migration twelve");

        let preserved: (String, i64, i64) = conn
            .query_row(
                "SELECT fact_kind, input_tokens, provider_reported_micro_usd
                   FROM usage_observations
                  WHERE observation_id = 'legacy-transcript'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("query classified legacy observation");
        assert_eq!(preserved, ("token_component".to_string(), 11, 1234));
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM usage_observations
                  WHERE observation_id IN ('legacy-transcript', 'legacy-managed-result')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("count ambiguous legacy pair"),
            2
        );
        assert_eq!(
            conn.query_row(
                "SELECT fact_kind FROM usage_observations
                  WHERE observation_id = 'legacy-managed-result'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("query legacy managed fact kind"),
            "aggregate"
        );
        assert!(table_exists(&conn, "managed_usage_backfill_state"));
        assert!(table_exists(&conn, "managed_usage_backfill_facts"));
        assert!(table_exists(&conn, "usage_reconciliation_mismatches"));
        assert!(table_exists(&conn, "usage_legacy_reconciliation_debt"));
        assert_eq!(
            conn.query_row(
                "SELECT transcript_observation_count, managed_observation_count
                   FROM usage_legacy_reconciliation_debt
                  WHERE environment_id = 'env-p4c1'
                    AND provider_instance_id = 'claude-p4c1'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .expect("query legacy reconciliation debt"),
            (1, 1)
        );
        let diagnostics = super::super::load_usage_scanner_diagnostics(
            &conn,
            &pandamux_protocol::UsageQueryParams::default(),
        )
        .expect("load legacy debt diagnostics");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].coverage_state,
            pandamux_protocol::UsageSourceCoverageState::Partial
        );
        assert_eq!(
            diagnostics[0].last_safe_error_code.as_deref(),
            Some("legacy_reconciliation_debt")
        );

        for (observation_id, message_id) in [
            ("managed-component-one", "message-one"),
            ("managed-component-two", "message-two"),
        ] {
            conn.execute(
                "INSERT INTO usage_observations (
                    environment_id, observation_id, source_kind, parser_version,
                    provider_native_session_id, provider_native_turn_id, occurred_at_ms,
                    provider, provider_instance_id, subscription_key, model,
                    input_tokens, cached_input_tokens, cache_write_tokens, output_tokens,
                    reasoning_tokens, other_tokens_json, provider_reported_micro_usd,
                    turn_id, created_at_ms, fact_kind
                 ) VALUES (
                    'env-p4c1', ?1, 'managed_turn', 2, 'session-p4c1', ?2, 1000,
                    'claude', 'claude-p4c1', 'subkey-p4c1', 'claude-actual',
                    5, 0, 0, 2, 0, '{}', NULL, 'pandamux-turn', 1000,
                    'token_component'
                 )",
                params![observation_id, message_id],
            )
            .expect("insert managed component");
        }
        assert_eq!(migration_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn migration_thirteen_preserves_existing_rows_and_adds_trusted_configuration() {
        let mut conn = Connection::open_in_memory().expect("open database");
        apply_migrations_to(&mut conn, 12).expect("apply schema twelve");
        let settings_json = r#"{"version":7,"fixture":"unchanged"}"#;
        conn.execute(
            "INSERT INTO settings (key, value_json) VALUES ('user_settings', ?1)",
            [settings_json],
        )
        .expect("insert settings fixture");
        let rows_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
            .expect("settings count before migration");

        apply_migrations_to(&mut conn, 13).expect("apply migration thirteen");

        let (stored_json, revision, updated_at_ms): (String, i64, i64) = conn
            .query_row(
                "SELECT value_json, revision, updated_at_ms
                   FROM settings WHERE key = 'user_settings'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read migrated settings");
        assert_eq!(stored_json, settings_json);
        assert_eq!(revision, 1);
        assert_eq!(updated_at_ms, 0);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM settings", [], |row| row
                .get::<_, i64>(0))
                .expect("settings count after migration"),
            rows_before
        );
        for table in [
            "project_repositories",
            "project_layer_trust",
            "personal_repo_state",
            "instruction_sources",
            "instruction_candidates",
            "configuration_audit",
            "bundle_objects",
            "bundle_activations",
            "bundle_distribution",
            "mutation_proposals",
            "mutation_audit",
        ] {
            assert!(table_exists(&conn, table), "missing {table}");
        }
        assert_eq!(migration_version(&conn), 13);

        apply_migrations_to(&mut conn, 13).expect("repeat migration thirteen");
        assert_eq!(migration_version(&conn), 13);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM settings", [], |row| row
                .get::<_, i64>(0))
                .expect("settings count after reopen equivalent"),
            rows_before
        );
    }

    #[test]
    fn migration_thirteen_failure_rolls_back_to_schema_twelve() {
        let mut conn = Connection::open_in_memory().expect("open database");
        apply_migrations_to(&mut conn, 12).expect("apply schema twelve");
        conn.execute_batch("CREATE TABLE project_repositories (conflict INTEGER);")
            .expect("create migration conflict");

        assert!(apply_migrations(&mut conn).is_err());
        assert_eq!(migration_version(&conn), 12);
        assert!(!column_exists(&conn, "settings", "revision"));
        assert!(!table_exists(&conn, "project_layer_trust"));
    }

    #[test]
    fn migration_fourteen_preserves_every_schema_thirteen_table_and_adds_execution_storage() {
        let mut conn = Connection::open_in_memory().expect("open database");
        apply_migrations_to(&mut conn, 13).expect("apply schema thirteen");
        conn.execute(
            "INSERT INTO settings (key, value_json, revision, updated_at_ms)
             VALUES ('phase5c-fixture', '{\"kept\":true}', 4, 1234)",
            [],
        )
        .expect("insert settings fixture");
        conn.execute(
            "INSERT INTO threads (
                id, project_id, environment_id, parent_thread_id, title,
                provider_instance_id, model, effort, access_mode, workspace_json,
                status, agent_json, origin_json, created_at_ms, updated_at_ms
             ) VALUES (
                'thread-phase5c', 'project-phase5c', 'env-phase5c', NULL,
                'Phase 5C fixture', 'provider-phase5c', 'model-phase5c', NULL,
                '\"workspace_only\"', '{}', '\"idle\"', NULL,
                '{\"type\":\"manual\"}', 1000, 1001
             )",
            [],
        )
        .expect("insert thread fixture");
        conn.execute(
            "INSERT INTO turns (
                id, thread_id, seq, input_json, status, started_at_ms
             ) VALUES ('turn-phase5c', 'thread-phase5c', 1, '{\"text\":\"kept\"}',
                '\"completed\"', 1000)",
            [],
        )
        .expect("insert turn fixture");
        conn.execute(
            "INSERT INTO failover_pools (
                id, display_name, revision, definition_json, updated_at_ms
             ) VALUES ('pool-phase5c', 'Phase 5C fixture', 1, '{}', 1000)",
            [],
        )
        .expect("insert failover fixture");
        conn.execute(
            "INSERT INTO project_repositories (
                id, project_id, organization_id, canonical_remote_identity,
                canonical_root_identity, display_path, revision, created_at_ms, updated_at_ms
             ) VALUES (
                'project-repository-phase5c', 'project-phase5c', 'org-phase5c',
                'remote-phase5c', 'root-phase5c', 'display-phase5c', 1, 1000, 1000
             )",
            [],
        )
        .expect("insert configuration fixture");
        let before = existing_table_contents(&conn);

        apply_migrations(&mut conn).expect("apply migration fourteen");

        assert_eq!(migration_version(&conn), 14);
        let after = existing_table_contents(&conn);
        for (table, rows) in before {
            assert_eq!(
                after.get(&table),
                Some(&rows),
                "changed legacy table {table}"
            );
        }
        for table in [
            "orchestration_runs",
            "orchestration_run_events",
            "orchestration_effects",
            "orchestration_attempt_dispatch",
            "orchestration_queue",
            "orchestration_leases",
            "orchestration_worktrees",
            "orchestration_budget_reservations",
            "orchestration_daily_budgets",
            "orchestration_blackboard_artifacts",
            "orchestration_attempt_progress",
            "orchestration_recovery_reviews",
            "orchestration_node_receipts",
        ] {
            assert!(table_exists(&conn, table), "missing {table}");
        }
        apply_migrations(&mut conn).expect("repeat migration fourteen");
        assert_eq!(migration_version(&conn), 14);
    }

    #[test]
    fn migration_fourteen_failure_rolls_back_every_execution_table() {
        let mut conn = Connection::open_in_memory().expect("open database");
        apply_migrations_to(&mut conn, 13).expect("apply schema thirteen");
        conn.execute_batch("CREATE TABLE orchestration_queue (conflict INTEGER);")
            .expect("inject migration conflict");

        assert!(apply_migrations(&mut conn).is_err());
        assert_eq!(migration_version(&conn), 13);
        assert!(!table_exists(&conn, "orchestration_runs"));
        assert!(!table_exists(&conn, "orchestration_run_events"));
        assert!(!table_exists(&conn, "orchestration_effects"));
        assert!(!table_exists(&conn, "orchestration_leases"));
    }

    #[test]
    fn migration_fourteen_enforces_each_runtime_table_and_declares_query_indexes() {
        let mut conn = Connection::open_in_memory().expect("open database");
        apply_migrations(&mut conn).expect("apply migration fourteen");
        conn.execute(
            "INSERT INTO project_repositories (
                id, project_id, organization_id, canonical_remote_identity,
                canonical_root_identity, display_path, revision, created_at_ms, updated_at_ms
             ) VALUES (
                'repository-db05', 'project-db05', 'organization-db05',
                'remote-db05', 'root-db05', 'display-db05', 1, 1, 1
             )",
            [],
        )
        .expect("insert repository prerequisite");
        conn.execute(
            "INSERT INTO orchestration_runs (
                run_id, organization_id, project_id, repository_id,
                snapshot_bytes, snapshot_sha256, last_seq, status, bundle_sha,
                policy_revision, settings_revision, created_at_ms, updated_at_ms, ended_at_ms
             ) VALUES (
                'run-db05', 'organization-db05', 'project-db05', 'repository-db05',
                x'00', lower(hex(zeroblob(32))), 1, 'running', lower(hex(zeroblob(32))),
                'policy-db05', 1, 1, 1, NULL
             )",
            [],
        )
        .expect("insert run prerequisite");
        conn.execute(
            "INSERT INTO orchestration_run_events (
                event_id, run_id, seq, idempotency_key, event_bytes, payload_sha256, at_ms
             ) VALUES (
                'event-db05', 'run-db05', 1, 'event-key-db05', x'00',
                lower(hex(zeroblob(32))), 1
             )",
            [],
        )
        .expect("insert event prerequisite");
        conn.execute(
            "INSERT INTO orchestration_leases (
                lease_id, resource_kind, resource_id, owner_incarnation_id,
                expires_at_ms, last_heartbeat_ms, revision
             ) VALUES ('lease-db05', 'capacity', 'capacity-db05', 'owner-db05', 2, 1, 1)",
            [],
        )
        .expect("insert lease prerequisite");
        conn.execute(
            "INSERT INTO orchestration_effects (
                effect_id, run_id, source_event_id, effect_bytes, payload_sha256,
                delivery_state, claim_revision, created_at_ms, updated_at_ms
             ) VALUES (
                'effect-db05', 'run-db05', 'event-db05', x'00',
                lower(hex(zeroblob(32))), 'pending', 0, 1, 1
             )",
            [],
        )
        .expect("insert effect prerequisite");

        let rejected = |sql: &str| {
            assert!(
                conn.execute(sql, []).is_err(),
                "migration constraint accepted malformed SQL: {sql}"
            );
        };
        rejected(
            "INSERT INTO orchestration_runs (
                run_id, organization_id, project_id, repository_id, snapshot_bytes,
                snapshot_sha256, last_seq, status, bundle_sha, policy_revision,
                settings_revision, created_at_ms, updated_at_ms
             ) VALUES ('', 'o', 'p', 'repository-db05', x'00',
                lower(hex(zeroblob(32))), 0, 'running', lower(hex(zeroblob(32))),
                'policy', 1, 1, 1)",
        );
        rejected(
            "INSERT INTO orchestration_run_events (
                event_id, run_id, seq, idempotency_key, event_bytes, payload_sha256, at_ms
             ) VALUES ('bad-event-db05', 'run-db05', 0, 'bad-event-key-db05', x'00',
                lower(hex(zeroblob(32))), 1)",
        );
        rejected(
            "INSERT INTO orchestration_leases (
                lease_id, resource_kind, resource_id, owner_incarnation_id,
                expires_at_ms, last_heartbeat_ms, revision
             ) VALUES ('bad-lease-db05', 'capacity', 'bad', 'owner', 1, 2, 1)",
        );
        rejected(
            "INSERT INTO orchestration_effects (
                effect_id, run_id, source_event_id, effect_bytes, payload_sha256,
                delivery_state, owner_incarnation_id, claim_revision, created_at_ms, updated_at_ms
             ) VALUES ('bad-effect-db05', 'run-db05', 'event-db05', x'00',
                lower(hex(zeroblob(32))), 'claimed', 'owner', 1, 1, 1)",
        );
        rejected(
            "INSERT INTO orchestration_attempt_dispatch (
                attempt_id, run_id, task_id, effect_id, thread_id, turn_id,
                turn_attempt_id, provider_instance_id, route_bytes, route_sha256,
                bundle_sha, state, last_boundary, last_progress_at_ms,
                created_at_ms, updated_at_ms
             ) VALUES (
                'bad-attempt-db05', 'run-db05', 'task-db05', 'effect-db05',
                'thread-db05', 'turn-db05', 'turn-attempt-db05', 'provider-db05',
                x'00', lower(hex(zeroblob(32))), lower(hex(zeroblob(32))),
                'terminal', 'safe_to_retry', 1, 1, 1
             )",
        );
        rejected(
            "INSERT INTO orchestration_queue (
                queue_id, run_id, task_id, attempt_id, agent_id, provider_instance_id,
                environment_id, candidate_sha256, priority, task_enqueued_at_ms,
                enqueued_at_ms, not_before_ms, state
             ) VALUES (
                'bad-queue-db05', 'run-db05', 'task-db05', 'attempt-db05',
                'agent-db05', 'provider-db05', 'environment-db05',
                lower(hex(zeroblob(32))), 0, 2, 1, 1, 'queued'
             )",
        );
        rejected(
            "INSERT INTO orchestration_worktrees (
                worktree_id, repository_id, run_id, task_id, attempt_id, environment_id,
                git_common_dir_identity, baseline_commit, branch_ref, path, lease_id,
                checkpoint_ids_json, dirty_state, cleanup_eligibility,
                created_at_ms, updated_at_ms
             ) VALUES (
                'bad-worktree-db05', 'repository-db05', 'run-db05', 'task-db05',
                'attempt-db05', 'environment-db05', 'common-db05', 'baseline-db05',
                'refs/heads/db05', '/db05', 'lease-db05', '[]', 'foreign',
                'retained', 1, 1
             )",
        );
        rejected(
            "INSERT INTO orchestration_budget_reservations (
                reservation_id, run_id, task_id, attempt_id, agent_id, organization_id,
                agent_limit_tokens, agent_limit_micro_usd, agent_limit_runtime_ms,
                ledger_utc_date, reserved_tokens, reserved_micro_usd, reserved_runtime_ms,
                state, grants_json, created_at_ms, updated_at_ms
             ) VALUES (
                'bad-budget-db05', 'run-db05', 'task-db05', 'attempt-db05',
                'agent-db05', 'organization-db05', 10, 10, 10,
                '2026-10-09', 1, 1, 1,
                'reconciled', '{}', 1, 1
             )",
        );
        rejected(
            "INSERT INTO orchestration_daily_budgets (
                organization_id, ledger_utc_date, limit_micro_usd, reserved_micro_usd,
                reconciled_micro_usd, granted_micro_usd, revision, updated_at_ms
             ) VALUES ('organization-db05', '2026-10-09', 1, 0, 0, 0, 0, 1)",
        );
        rejected(
            "INSERT INTO orchestration_blackboard_artifacts (
                entry_id, run_id, artifact_key, version, producer_task_id,
                consumers_json, output_contract_json, schema_sha256, canonical_json,
                byte_len, content_sha256, created_event_id
             ) VALUES (
                'bad-artifact-db05', 'run-db05', 'artifact-db05', 1, 'task-db05',
                '[]', '{}', lower(hex(zeroblob(32))), x'00', 2,
                lower(hex(zeroblob(32))), 'event-db05'
             )",
        );
        rejected(
            "INSERT INTO orchestration_attempt_progress (
                progress_id, run_id, task_id, attempt_id, kind, payload_digest,
                at_ms, terminal
             ) VALUES (
                'bad-progress-db05', 'run-db05', 'task-db05', 'attempt-db05',
                'terminal_outcome', lower(hex(zeroblob(32))), 1, 0
             )",
        );
        rejected(
            "INSERT INTO orchestration_recovery_reviews (
                review_id, run_id, task_id, attempt_id, reason_code,
                evidence_digest, status, created_at_ms
             ) VALUES (
                'bad-review-db05', 'run-db05', 'task-db05', 'attempt-db05',
                'reason', lower(hex(zeroblob(32))), 'resolved', 1
             )",
        );
        rejected(
            "INSERT INTO orchestration_node_receipts (
                receipt_id, node_store_id, run_id, task_id, attempt_id, effect_id,
                request_kind, idempotency_key, request_sha256, result_sha256,
                result_bytes, created_at_ms, updated_at_ms
             ) VALUES (
                'bad-receipt-db05', 'node-db05', 'run-db05', 'task-db05',
                'attempt-db05', 'effect-db05', 'attempt_status', '',
                lower(hex(zeroblob(32))), lower(hex(zeroblob(32))), x'00', 1, 1
             )",
        );

        for index in [
            "idx_orchestration_runs_nonterminal",
            "idx_orchestration_run_events_replay",
            "idx_orchestration_effects_pending",
            "idx_orchestration_attempt_dispatch_active",
            "idx_orchestration_queue_order",
            "idx_orchestration_leases_expiry",
            "idx_orchestration_worktrees_active",
            "idx_orchestration_budget_incomplete",
            "idx_orchestration_artifact_versions",
            "idx_orchestration_attempt_progress_recent",
            "idx_orchestration_recovery_review_identity",
            "idx_orchestration_recovery_reviews_unresolved",
            "idx_orchestration_node_receipts_updated",
            "idx_orchestration_node_receipts_attempt",
        ] {
            assert!(
                conn.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1
                     )",
                    [index],
                    |row| row.get::<_, bool>(0),
                )
                .expect("index lookup"),
                "missing runtime index {index}"
            );
        }
    }

    #[test]
    fn migration_six_backfills_valid_totals_and_quarantines_every_invalid_row() {
        let mut conn = Connection::open_in_memory().expect("open database");
        apply_migrations_to(&mut conn, 4).expect("apply legacy schema");
        insert_legacy_bucket(&conn, "2026-09-28", "claude", 100);
        insert_legacy_bucket(&conn, "2026-09-28", "future-provider", 100);
        insert_legacy_bucket(&conn, "2026-09-28", "codex", -1);
        insert_legacy_bucket(&conn, "2026-02-30", "codex", 100);
        insert_legacy_bucket(&conn, "2026-09-28", "codex", 100);
        conn.execute(
            "UPDATE usage_buckets SET account_key = '' WHERE id = last_insert_rowid()",
            [],
        )
        .expect("corrupt account identity");
        insert_legacy_bucket(&conn, "2026-09-28", "codex", 100);
        conn.execute(
            "UPDATE usage_buckets SET agent_id = x'00' WHERE id = last_insert_rowid()",
            [],
        )
        .expect("corrupt optional attribution");

        apply_migrations(&mut conn).expect("apply normalized migration");

        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_buckets", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("legacy count"),
            6
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_observations", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("observation count"),
            1
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_quarantine", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("quarantine count"),
            5
        );
        let reasons = {
            let mut stmt = conn
                .prepare("SELECT reason_code FROM usage_quarantine ORDER BY legacy_bucket_id")
                .expect("prepare quarantine reasons");
            stmt.query_map([], |row| row.get::<_, String>(0))
                .expect("query quarantine reasons")
                .collect::<Result<Vec<_>>>()
                .expect("collect quarantine reasons")
        };
        assert_eq!(
            reasons,
            vec![
                "invalid_provider",
                "negative_integer",
                "impossible_date",
                "ambiguous_identity",
                "invalid_type",
            ]
        );
        assert_eq!(
            conn.query_row(
                "SELECT input_tokens + cached_input_tokens + cache_write_tokens +
                        output_tokens + reasoning_tokens
                 FROM usage_observations",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("token total"),
            165
        );
        assert_eq!(
            conn.query_row(
                "SELECT source_record_count FROM usage_observations",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("source record count"),
            2
        );
        let (label, subscription_key): (String, String) = conn
            .query_row(
                "SELECT masked_account_label, subscription_key FROM usage_subscriptions",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("subscription");
        assert_eq!(label, "p***@example.com");
        assert!(subscription_key.starts_with("subkey-legacy-"));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_outbox", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("outbox count"),
            1
        );

        apply_migrations(&mut conn).expect("repeat normalized migration");
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_observations", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("observation count after repeat"),
            1
        );
    }

    #[test]
    fn interrupted_migration_rolls_back_and_can_retry() {
        let mut conn = Connection::open_in_memory().expect("open database");
        apply_migrations_to(&mut conn, 5).expect("apply schema five");
        conn.execute_batch("CREATE TABLE usage_observations (conflict INTEGER);")
            .expect("create migration conflict");

        assert!(apply_migrations(&mut conn).is_err());
        assert_eq!(migration_version(&conn), 5);
        assert!(!table_exists(&conn, "usage_store_meta"));

        conn.execute_batch("DROP TABLE usage_observations;")
            .expect("remove migration conflict");
        apply_migrations(&mut conn).expect("retry migration");
        assert_eq!(migration_version(&conn), SCHEMA_VERSION);
        assert!(table_exists(&conn, "usage_store_meta"));
    }

    #[test]
    fn database_reopen_keeps_one_node_store_identity() {
        let file = NamedTempFile::new().expect("temporary database");
        let path = file.path().to_path_buf();
        let node_store_id = {
            let mut conn = Connection::open(&path).expect("open database");
            apply_migrations(&mut conn).expect("apply migrations");
            conn.query_row(
                "SELECT node_store_id FROM usage_store_meta WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("node store id")
        };
        let mut reopened = Connection::open(&path).expect("reopen database");
        apply_migrations(&mut reopened).expect("reapply migrations");
        assert_eq!(
            reopened
                .query_row(
                    "SELECT node_store_id FROM usage_store_meta WHERE singleton = 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("reopened node store id"),
            node_store_id
        );
    }
}
