use rusqlite::types::Value;
use rusqlite::{Connection, Result, Transaction, params};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: i64 = 11;

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

fn apply_migrations_to(conn: &mut Connection, target_version: i64) -> Result<()> {
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

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
