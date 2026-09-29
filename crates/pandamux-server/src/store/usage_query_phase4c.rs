use super::{StoreError, checked_u64};
use pandamux_core::{
    EnvironmentId, PricingVersionId, TokenCounts, UsagePricingRates, UsageSourceKind,
    estimate_usage_micro_usd,
};
use pandamux_protocol::{
    UsageAggregate, UsageCoverageGap, UsageEnvironmentSyncCoverage, UsageEnvironmentSyncState,
    UsageQueryParams,
};
use rusqlite::types::Value;
use rusqlite::{Connection, ToSql};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct UsageAggregateProjection {
    pub aggregate: UsageAggregate,
    pub included_environments: Vec<EnvironmentId>,
    pub observation_sources: Vec<UsageSourceKind>,
    pub latest_source_at_ms: Option<u64>,
    pub unknown_price_tokens: u64,
    pub priceable_tokens: u64,
    pub unknown_price_token_millionths: Option<u32>,
    pub environment_sync: Vec<UsageEnvironmentSyncCoverage>,
    pub quarantined_count: u64,
}

pub(super) fn load_usage_aggregate(
    conn: &Connection,
    params: &UsageQueryParams,
) -> Result<UsageAggregateProjection, StoreError> {
    let (where_sql, values) = usage_filter_where(params)?;
    let references = sql_references(&values);
    let totals_sql = format!(
        "SELECT COUNT(*),
                COALESCE(SUM(o.input_tokens), 0),
                COALESCE(SUM(o.cached_input_tokens), 0),
                COALESCE(SUM(o.cache_write_tokens), 0),
                COALESCE(SUM(o.output_tokens), 0),
                COALESCE(SUM(o.reasoning_tokens), 0),
                SUM(o.provider_reported_micro_usd),
                COUNT(o.provider_reported_micro_usd),
                MAX(o.occurred_at_ms)
         FROM usage_observations o {where_sql}"
    );
    let totals = conn.query_row(&totals_sql, references.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, Option<i64>>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, Option<i64>>(8)?,
        ))
    })?;

    let mut tokens = TokenCounts {
        input: checked_u64(totals.1, "aggregate input tokens")?,
        cached_input: checked_u64(totals.2, "aggregate cached input tokens")?,
        cache_write: checked_u64(totals.3, "aggregate cache write tokens")?,
        output: checked_u64(totals.4, "aggregate output tokens")?,
        reasoning: checked_u64(totals.5, "aggregate reasoning tokens")?,
        other: BTreeMap::new(),
    };
    load_other_token_totals(conn, &where_sql, &values, &mut tokens.other)?;
    let priceable_tokens = tokens
        .checked_total()
        .map_err(|error| StoreError::Data(error.to_string()))?;

    let pricing = load_pricing_projection(conn, &where_sql, &values)?;
    let unknown_price_token_millionths = if priceable_tokens == 0 {
        None
    } else {
        let numerator = u128::from(pricing.unknown_tokens)
            .checked_mul(1_000_000)
            .ok_or_else(|| {
                StoreError::Data("unknown-price share numerator overflow".to_string())
            })?;
        Some(
            u32::try_from(numerator / u128::from(priceable_tokens)).map_err(|_| {
                StoreError::Data("unknown-price share conversion overflow".to_string())
            })?,
        )
    };

    let included_environments = load_distinct_environments(conn, &where_sql, &values)?;
    let observation_sources = load_distinct_sources(conn, &where_sql, &values)?;
    let environment_sync = load_environment_sync(conn, params.environment_id.as_ref())?;
    let remote_latest_source_at_ms =
        load_remote_latest_source(conn, params.environment_id.as_ref())?;
    let fact_latest_source_at_ms = totals
        .8
        .map(|value| checked_u64(value, "latest usage source time"))
        .transpose()?;
    Ok(UsageAggregateProjection {
        aggregate: UsageAggregate {
            observation_count: checked_u64(totals.0, "usage observation count")?,
            tokens,
            provider_reported_micro_usd: totals
                .6
                .map(|value| checked_u64(value, "provider reported cost total"))
                .transpose()?,
            provider_reported_observation_count: checked_u64(
                totals.7,
                "provider reported observation count",
            )?,
            api_equivalent_estimated_micro_usd: if pricing.unknown_tokens == 0
                && priceable_tokens != 0
            {
                Some(pricing.estimated_micro_usd)
            } else {
                None
            },
            pricing_version_ids: pricing.version_ids.into_iter().collect(),
        },
        included_environments,
        observation_sources,
        latest_source_at_ms: fact_latest_source_at_ms.max(remote_latest_source_at_ms),
        unknown_price_tokens: pricing.unknown_tokens,
        priceable_tokens,
        unknown_price_token_millionths,
        environment_sync,
        quarantined_count: load_quarantined_count(conn, params.environment_id.as_ref())?,
    })
}

fn usage_filter_where(params: &UsageQueryParams) -> Result<(String, Vec<Value>), StoreError> {
    let mut clauses = vec!["1 = 1".to_string()];
    let mut values = Vec::new();
    if let Some(value) = params.from_ms {
        clauses.push("o.occurred_at_ms >= ?".to_string());
        values.push(Value::Integer(i64::try_from(value).map_err(|_| {
            StoreError::Data("usage query from_ms exceeds the supported range".to_string())
        })?));
    }
    if let Some(value) = params.to_ms {
        clauses.push("o.occurred_at_ms <= ?".to_string());
        values.push(Value::Integer(i64::try_from(value).map_err(|_| {
            StoreError::Data("usage query to_ms exceeds the supported range".to_string())
        })?));
    }
    push_filter(
        &mut clauses,
        &mut values,
        "o.environment_id",
        params.environment_id.as_ref().map(EnvironmentId::as_str),
    );
    push_filter(
        &mut clauses,
        &mut values,
        "o.provider",
        params.provider.as_ref().map(|value| value.as_str()),
    );
    push_filter(
        &mut clauses,
        &mut values,
        "o.provider_instance_id",
        params
            .provider_instance_id
            .as_ref()
            .map(|value| value.as_str()),
    );
    push_filter(
        &mut clauses,
        &mut values,
        "o.subscription_key",
        params.subscription_key.as_ref().map(|value| value.as_str()),
    );
    push_filter(
        &mut clauses,
        &mut values,
        "o.agent_id",
        params.agent_id.as_ref().map(|value| value.as_str()),
    );
    push_filter(
        &mut clauses,
        &mut values,
        "o.schedule_id",
        params.schedule_id.as_ref().map(|value| value.as_str()),
    );
    push_filter(
        &mut clauses,
        &mut values,
        "o.model",
        params.model.as_deref(),
    );
    push_filter(
        &mut clauses,
        &mut values,
        "o.source_kind",
        params.source_kind.as_ref().map(|value| value.as_str()),
    );
    Ok((format!("WHERE {}", clauses.join(" AND ")), values))
}

fn push_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<Value>,
    column: &str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        clauses.push(format!("{column} = ?"));
        values.push(Value::Text(value.to_string()));
    }
}

fn sql_references(values: &[Value]) -> Vec<&dyn ToSql> {
    values.iter().map(|value| value as &dyn ToSql).collect()
}

fn load_other_token_totals(
    conn: &Connection,
    where_sql: &str,
    values: &[Value],
    other: &mut BTreeMap<String, u64>,
) -> Result<(), StoreError> {
    let sql = format!(
        "SELECT token.key, SUM(CAST(token.value AS INTEGER))
         FROM usage_observations o, json_each(o.other_tokens_json) token
         {where_sql}
         GROUP BY token.key ORDER BY token.key"
    );
    let references = sql_references(values);
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(references.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (category, value) = row?;
        if category.trim().is_empty() {
            return Err(StoreError::Data(
                "stored usage contains an empty token category".to_string(),
            ));
        }
        other.insert(category, checked_u64(value, "other token total")?);
    }
    Ok(())
}

struct PricingProjection {
    estimated_micro_usd: u64,
    unknown_tokens: u64,
    version_ids: BTreeSet<PricingVersionId>,
}

fn load_pricing_projection(
    conn: &Connection,
    where_sql: &str,
    values: &[Value],
) -> Result<PricingProjection, StoreError> {
    let sql = format!(
        "WITH filtered AS (
            SELECT o.*,
                   (
                       SELECT version.id
                       FROM usage_pricing_versions version
                       WHERE version.provider = o.provider
                         AND o.occurred_at_ms IS NOT NULL
                         AND version.effective_from_ms <= o.occurred_at_ms
                       ORDER BY version.effective_from_ms DESC, version.id DESC
                       LIMIT 1
                   ) AS pricing_version_id
            FROM usage_observations o {where_sql}
         ), resolved AS (
            SELECT filtered.*,
                   rule.input_micro_usd_per_million,
                   rule.cached_input_micro_usd_per_million,
                   rule.cache_write_micro_usd_per_million,
                   rule.output_micro_usd_per_million,
                   rule.reasoning_micro_usd_per_million,
                   COALESCE((
                       SELECT SUM(CAST(value AS INTEGER))
                       FROM json_each(filtered.other_tokens_json)
                   ), 0) AS other_token_total
            FROM filtered
            LEFT JOIN usage_pricing_aliases model_alias
              ON model_alias.pricing_version_id = filtered.pricing_version_id
             AND model_alias.alias_model_id = filtered.model
            LEFT JOIN usage_pricing_rules rule
              ON rule.pricing_version_id = filtered.pricing_version_id
             AND rule.canonical_model_id = COALESCE(
                    model_alias.canonical_model_id,
                    filtered.model
                 )
         )
         SELECT pricing_version_id,
                input_micro_usd_per_million,
                cached_input_micro_usd_per_million,
                cache_write_micro_usd_per_million,
                output_micro_usd_per_million,
                reasoning_micro_usd_per_million,
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(cached_input_tokens), 0),
                COALESCE(SUM(cache_write_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(reasoning_tokens), 0),
                COALESCE(SUM(other_token_total), 0)
         FROM resolved
         GROUP BY pricing_version_id,
                  input_micro_usd_per_million,
                  cached_input_micro_usd_per_million,
                  cache_write_micro_usd_per_million,
                  output_micro_usd_per_million,
                  reasoning_micro_usd_per_million"
    );
    let references = sql_references(values);
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(references.as_slice(), |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<i64>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, i64>(10)?,
            row.get::<_, i64>(11)?,
        ))
    })?;

    let mut estimated_micro_usd = 0_u64;
    let mut unknown_tokens = 0_u64;
    let mut version_ids = BTreeSet::new();
    for row in rows {
        let row = row?;
        let other_token_total = checked_u64(row.11, "priced other tokens")?;
        let tokens = TokenCounts {
            input: checked_u64(row.6, "priced input tokens")?,
            cached_input: checked_u64(row.7, "priced cached input tokens")?,
            cache_write: checked_u64(row.8, "priced cache write tokens")?,
            output: checked_u64(row.9, "priced output tokens")?,
            reasoning: checked_u64(row.10, "priced reasoning tokens")?,
            other: if other_token_total != 0 {
                BTreeMap::from([("unpricedOther".to_string(), other_token_total)])
            } else {
                BTreeMap::new()
            },
        };
        let total = tokens
            .checked_total()
            .map_err(|error| StoreError::Data(error.to_string()))?;
        let rates = UsagePricingRates {
            input_micro_usd_per_million: optional_rate(row.1, "input pricing rate")?,
            cached_input_micro_usd_per_million: optional_rate(row.2, "cached input pricing rate")?,
            cache_write_micro_usd_per_million: optional_rate(row.3, "cache write pricing rate")?,
            output_micro_usd_per_million: optional_rate(row.4, "output pricing rate")?,
            reasoning_micro_usd_per_million: optional_rate(row.5, "reasoning pricing rate")?,
        };
        let estimate = estimate_usage_micro_usd(&tokens, &rates)
            .map_err(|error| StoreError::Data(error.to_string()))?;
        match (estimate, row.0) {
            (Some(cost), Some(version_id)) => {
                estimated_micro_usd = estimated_micro_usd
                    .checked_add(cost)
                    .ok_or_else(|| StoreError::Data("usage price total overflow".to_string()))?;
                version_ids.insert(PricingVersionId::from(version_id));
            }
            _ => {
                unknown_tokens = unknown_tokens.checked_add(total).ok_or_else(|| {
                    StoreError::Data("unknown-price token total overflow".to_string())
                })?;
            }
        }
    }
    Ok(PricingProjection {
        estimated_micro_usd,
        unknown_tokens,
        version_ids,
    })
}

fn optional_rate(value: Option<i64>, field: &str) -> Result<Option<u64>, StoreError> {
    value.map(|value| checked_u64(value, field)).transpose()
}

fn load_distinct_environments(
    conn: &Connection,
    where_sql: &str,
    values: &[Value],
) -> Result<Vec<EnvironmentId>, StoreError> {
    let sql = format!(
        "SELECT DISTINCT o.environment_id FROM usage_observations o
         {where_sql} ORDER BY o.environment_id"
    );
    let references = sql_references(values);
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(references.as_slice(), |row| row.get::<_, String>(0))?;
    rows.map(|row| row.map(EnvironmentId::from).map_err(StoreError::from))
        .collect()
}

fn load_distinct_sources(
    conn: &Connection,
    where_sql: &str,
    values: &[Value],
) -> Result<Vec<UsageSourceKind>, StoreError> {
    let sql = format!(
        "SELECT DISTINCT o.source_kind FROM usage_observations o
         {where_sql} ORDER BY o.source_kind"
    );
    let references = sql_references(values);
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(references.as_slice(), |row| row.get::<_, String>(0))?;
    rows.map(|row| row.map(UsageSourceKind::from).map_err(StoreError::from))
        .collect()
}

fn load_environment_sync(
    conn: &Connection,
    environment_filter: Option<&EnvironmentId>,
) -> Result<Vec<UsageEnvironmentSyncCoverage>, StoreError> {
    let mut sql = String::from(
        "SELECT status.environment_id, status.sync_state, status.last_success_at_ms,
                status.last_safe_error_code, high_water.gap_requested_after_seq,
                high_water.gap_retained_from_seq, high_water.gap_latest_seq
         FROM usage_hub_environment_status status
         LEFT JOIN usage_hub_high_water high_water
           ON high_water.environment_id = status.environment_id",
    );
    let mut values = Vec::new();
    if let Some(environment_id) = environment_filter {
        sql.push_str(" WHERE status.environment_id = ?1");
        values.push(Value::Text(environment_id.as_str().to_string()));
    }
    sql.push_str(" ORDER BY status.environment_id");
    let references = sql_references(&values);
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(references.as_slice(), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, Option<i64>>(6)?,
        ))
    })?;
    let mut coverage = Vec::new();
    for row in rows {
        let row = row?;
        let state = match row.1.as_str() {
            "ready" => UsageEnvironmentSyncState::Ready,
            "reconciling" => UsageEnvironmentSyncState::Reconciling,
            "coverage_gap" => UsageEnvironmentSyncState::CoverageGap,
            "stale" => UsageEnvironmentSyncState::Stale,
            "unreachable" => UsageEnvironmentSyncState::Unreachable,
            other => {
                return Err(StoreError::Data(format!(
                    "unknown usage synchronization state {other}"
                )));
            }
        };
        let gap = match (row.4, row.5, row.6) {
            (Some(requested_after_seq), Some(retained_from_seq), Some(latest_seq)) => {
                Some(UsageCoverageGap {
                    requested_after_seq: checked_u64(
                        requested_after_seq,
                        "coverage gap requested sequence",
                    )?,
                    retained_from_seq: checked_u64(
                        retained_from_seq,
                        "coverage gap retained sequence",
                    )?,
                    latest_seq: checked_u64(latest_seq, "coverage gap latest sequence")?,
                })
            }
            (None, None, None) => None,
            _ => {
                return Err(StoreError::Data(
                    "usage environment coverage gap is incomplete".to_string(),
                ));
            }
        };
        coverage.push(UsageEnvironmentSyncCoverage {
            environment_id: EnvironmentId::from(row.0),
            state,
            last_success_at_ms: row
                .2
                .map(|value| checked_u64(value, "usage sync success time"))
                .transpose()?,
            last_safe_error_code: row.3,
            coverage_gap: gap,
        });
    }
    Ok(coverage)
}

fn load_quarantined_count(
    conn: &Connection,
    environment_filter: Option<&EnvironmentId>,
) -> Result<u64, StoreError> {
    let (local, remote) = if let Some(environment_id) = environment_filter {
        (
            conn.query_row(
                "SELECT COUNT(*) FROM usage_quarantine WHERE environment_id = ?1",
                [environment_id.as_str()],
                |row| row.get::<_, i64>(0),
            )?,
            conn.query_row(
                "SELECT COALESCE(SUM(quarantined_count), 0)
                 FROM usage_hub_high_water WHERE environment_id = ?1",
                [environment_id.as_str()],
                |row| row.get::<_, i64>(0),
            )?,
        )
    } else {
        (
            conn.query_row("SELECT COUNT(*) FROM usage_quarantine", [], |row| {
                row.get::<_, i64>(0)
            })?,
            conn.query_row(
                "SELECT COALESCE(SUM(quarantined_count), 0) FROM usage_hub_high_water",
                [],
                |row| row.get::<_, i64>(0),
            )?,
        )
    };
    checked_u64(local, "local usage quarantine count")?
        .checked_add(checked_u64(remote, "remote usage quarantine count")?)
        .ok_or_else(|| StoreError::Data("usage quarantine total overflow".to_string()))
}

fn load_remote_latest_source(
    conn: &Connection,
    environment_filter: Option<&EnvironmentId>,
) -> Result<Option<u64>, StoreError> {
    let value = if let Some(environment_id) = environment_filter {
        conn.query_row(
            "SELECT MAX(latest_source_at_ms) FROM usage_hub_high_water
             WHERE environment_id = ?1",
            [environment_id.as_str()],
            |row| row.get::<_, Option<i64>>(0),
        )?
    } else {
        conn.query_row(
            "SELECT MAX(latest_source_at_ms) FROM usage_hub_high_water",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )?
    };
    value
        .map(|value| checked_u64(value, "remote latest source time"))
        .transpose()
}
