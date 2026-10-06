use super::{
    Store, StoreError, checked_i64, checked_u64, decode_usage_observation, load_usage_observation,
    optional_checked_i64, store_now_ms,
};
use pandamux_core::{
    EnvironmentId, NodeStoreId, ThreadId, TurnId, UsageObservation, UsagePricingVersion,
};
use pandamux_protocol::{
    UsageBackfillCursor, UsageBackfillItem, UsageBackfillParams, UsageBackfillResult,
    UsageCoverageGap, UsageExportResult, UsageNodeCoverageSnapshot, UsageQueryParams,
};
use rusqlite::{OptionalExtension, Transaction, params};

pub const DEFAULT_USAGE_OUTBOX_MAX_ROWS: u64 = 50_000;
pub const DEFAULT_USAGE_OUTBOX_MAX_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1_000;
pub const DEFAULT_USAGE_OUTBOX_MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsageOutboxRetention {
    pub max_rows: u64,
    pub max_age_ms: u64,
    pub max_bytes: u64,
}

impl Default for UsageOutboxRetention {
    fn default() -> Self {
        Self {
            max_rows: DEFAULT_USAGE_OUTBOX_MAX_ROWS,
            max_age_ms: DEFAULT_USAGE_OUTBOX_MAX_AGE_MS,
            max_bytes: DEFAULT_USAGE_OUTBOX_MAX_BYTES,
        }
    }
}

impl UsageOutboxRetention {
    fn validate(self) -> Result<Self, StoreError> {
        if self.max_rows == 0 || self.max_age_ms == 0 || self.max_bytes == 0 {
            return Err(StoreError::Data(
                "usage outbox retention limits must be greater than zero".to_string(),
            ));
        }
        checked_i64(self.max_rows, "usage outbox row limit")?;
        checked_i64(self.max_age_ms, "usage outbox age limit")?;
        checked_i64(self.max_bytes, "usage outbox byte limit")?;
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageOutboxCompaction {
    pub acknowledged: u64,
    pub compacted: u64,
    pub retained_from_seq: u64,
    pub latest_seq: u64,
    pub retained_rows: u64,
    pub retained_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageHubSyncState {
    pub environment_id: EnvironmentId,
    pub node_store_id: NodeStoreId,
    pub high_water_seq: u64,
    pub reconciliation_state: String,
    pub last_attempt_at_ms: Option<u64>,
    pub last_success_at_ms: Option<u64>,
    pub last_safe_error_code: Option<String>,
    pub coverage_gap: Option<UsageCoverageGap>,
    pub backfill_run_id: Option<String>,
    pub backfill_after_observation_id: Option<String>,
    pub backfill_through_seq: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageHubIngestOutcome {
    pub high_water_seq: u64,
    pub inserted: u64,
    pub updated: u64,
    pub duplicates: u64,
}

impl Store {
    pub fn usage_node_coverage_snapshot(&self) -> Result<UsageNodeCoverageSnapshot, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_node_coverage(&conn)
    }

    pub fn usage_outbox_window(&self) -> Result<(u64, u64), StoreError> {
        let conn = self.conn.lock().unwrap();
        usage_outbox_window(&conn)
    }

    pub fn export_usage_backfill(
        &self,
        params: &UsageBackfillParams,
    ) -> Result<UsageBackfillResult, StoreError> {
        params.validate().map_err(StoreError::Data)?;
        if params.after.is_some() && params.snapshot_through_seq.is_none() {
            return Err(StoreError::Data(
                "continued usage backfill requires snapshotThroughSeq".to_string(),
            ));
        }

        let conn = self.conn.lock().unwrap();
        let node_store_id = conn.query_row(
            "SELECT node_store_id FROM usage_store_meta WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )?;
        let (_, latest_seq) = usage_outbox_window(&conn)?;
        let snapshot_through_seq = params.snapshot_through_seq.unwrap_or(latest_seq);
        if snapshot_through_seq > latest_seq {
            return Err(StoreError::Data(
                "usage backfill snapshot exceeds the node outbox sequence".to_string(),
            ));
        }

        let after = params
            .after
            .as_ref()
            .map(|cursor| cursor.observation_id.as_str())
            .unwrap_or("");
        let mut stmt = conn.prepare(
            "SELECT source_outbox_seq,
                    observation_id, source_kind, parser_version,
                    provider_native_session_id, provider_native_turn_id,
                    occurred_at_ms, provider, provider_instance_id,
                    subscription_key, model, input_tokens,
                    cached_input_tokens, cache_write_tokens, output_tokens,
                    reasoning_tokens, other_tokens_json, unknown_fields_json,
                    provider_reported_micro_usd, source_record_count,
                    thread_id, turn_id, agent_id, schedule_id, fact_kind
             FROM usage_observations
             WHERE source_node_store_id = ?1
               AND source_outbox_seq <= ?2
               AND observation_id > ?3
             ORDER BY observation_id
             LIMIT ?4",
        )?;
        let mut rows = stmt.query(params![
            node_store_id,
            checked_i64(snapshot_through_seq, "usage backfill snapshot")?,
            after,
            i64::from(params.limit) + 1,
        ])?;
        let mut items = Vec::new();
        while let Some(row) = rows.next()? {
            items.push(UsageBackfillItem {
                latest_seq: checked_u64(row.get(0)?, "usage backfill source sequence")?,
                observation: decode_usage_observation(row, 1)?,
            });
        }
        let has_more = items.len() > params.limit as usize;
        if has_more {
            items.pop();
        }
        let next = items.last().map(|item| UsageBackfillCursor {
            observation_id: item.observation.id.clone(),
        });
        Ok(UsageBackfillResult {
            node_store_id: NodeStoreId::from(node_store_id),
            snapshot_through_seq,
            items,
            next,
            has_more,
            node_coverage: Some(load_node_coverage(&conn)?),
        })
    }

    pub fn acknowledge_and_compact_usage(
        &self,
        through_seq: u64,
        retention: UsageOutboxRetention,
    ) -> Result<UsageOutboxCompaction, StoreError> {
        let retention = retention.validate()?;
        let through_seq = checked_i64(through_seq, "usage acknowledgement sequence")?;
        let now_ms = store_now_ms();
        let acknowledged_at_ms = checked_i64(now_ms, "usage acknowledgement time")?;
        let age_cutoff = now_ms.saturating_sub(retention.max_age_ms);
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let acknowledged = tx.execute(
            "UPDATE usage_outbox
             SET acknowledged_at_ms = ?1
             WHERE seq <= ?2 AND acknowledged_at_ms IS NULL",
            params![acknowledged_at_ms, through_seq],
        )?;

        let mut compacted = tx.execute(
            "DELETE FROM usage_outbox
             WHERE acknowledged_at_ms IS NOT NULL AND enqueued_at_ms < ?1",
            [checked_i64(age_cutoff, "usage outbox age cutoff")?],
        )?;

        let row_count = tx.query_row("SELECT COUNT(*) FROM usage_outbox", [], |row| {
            row.get::<_, i64>(0)
        })?;
        let row_excess =
            row_count.saturating_sub(checked_i64(retention.max_rows, "usage outbox row limit")?);
        if row_excess > 0 {
            compacted += tx.execute(
                "DELETE FROM usage_outbox
                 WHERE seq IN (
                    SELECT seq FROM usage_outbox
                    WHERE acknowledged_at_ms IS NOT NULL
                    ORDER BY seq
                    LIMIT ?1
                 )",
                [row_excess],
            )?;
        }

        let retained_bytes = tx.query_row(
            "SELECT COALESCE(SUM(payload_bytes), 0) FROM usage_outbox",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        let max_bytes = checked_i64(retention.max_bytes, "usage outbox byte limit")?;
        if retained_bytes > max_bytes {
            let mut statement = tx.prepare(
                "SELECT seq, payload_bytes FROM usage_outbox
                 WHERE acknowledged_at_ms IS NOT NULL ORDER BY seq",
            )?;
            let rows = statement
                .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
            let mut bytes_to_remove = retained_bytes - max_bytes;
            let mut through_delete_seq = None;
            for row in rows {
                let (seq, bytes) = row?;
                through_delete_seq = Some(seq);
                bytes_to_remove = bytes_to_remove.saturating_sub(bytes);
                if bytes_to_remove == 0 {
                    break;
                }
            }
            drop(statement);
            if let Some(seq) = through_delete_seq {
                compacted += tx.execute(
                    "DELETE FROM usage_outbox
                     WHERE acknowledged_at_ms IS NOT NULL AND seq <= ?1",
                    [seq],
                )?;
            }
        }

        let (retained_from_seq, latest_seq) = usage_outbox_window(&tx)?;
        let (retained_rows, retained_bytes) = tx.query_row(
            "SELECT COUNT(*), COALESCE(SUM(payload_bytes), 0) FROM usage_outbox",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )?;
        tx.commit()?;
        Ok(UsageOutboxCompaction {
            acknowledged: u64::try_from(acknowledged).map_err(|_| {
                StoreError::Data("usage acknowledgement count overflow".to_string())
            })?,
            compacted: u64::try_from(compacted)
                .map_err(|_| StoreError::Data("usage compaction count overflow".to_string()))?,
            retained_from_seq,
            latest_seq,
            retained_rows: checked_u64(retained_rows, "usage retained row count")?,
            retained_bytes: checked_u64(retained_bytes, "usage retained bytes")?,
        })
    }

    pub fn usage_hub_sync_state(
        &self,
        environment_id: &EnvironmentId,
    ) -> Result<Option<UsageHubSyncState>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_hub_sync_state(&conn, environment_id)
    }

    pub fn begin_usage_backfill(
        &self,
        environment_id: &EnvironmentId,
        node_store_id: &NodeStoreId,
        snapshot_through_seq: u64,
        coverage_gap: Option<&UsageCoverageGap>,
    ) -> Result<UsageHubSyncState, StoreError> {
        validate_hub_scope(environment_id, node_store_id)?;
        let now_ms = checked_i64(store_now_ms(), "usage sync attempt time")?;
        let run_id = format!("usage-sync-{}", uuid::Uuid::new_v4());
        let gap = coverage_gap.cloned();
        let reconciliation_state = if gap.is_some() {
            "coverage_gap"
        } else {
            "reconciling"
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO usage_hub_high_water (
                environment_id, node_store_id, high_water_seq,
                reconciliation_state, updated_at_ms, last_attempt_at_ms,
                last_success_at_ms, last_safe_error_code,
                gap_requested_after_seq, gap_retained_from_seq, gap_latest_seq,
                backfill_run_id, backfill_after_observation_id, backfill_through_seq
             ) VALUES (?1, ?2, 0, ?9, ?3, ?3, NULL, NULL,
                       ?4, ?5, ?6, ?7, NULL, ?8)
             ON CONFLICT(environment_id) DO UPDATE SET
                node_store_id = excluded.node_store_id,
                high_water_seq = 0,
                reconciliation_state = excluded.reconciliation_state,
                updated_at_ms = excluded.updated_at_ms,
                last_attempt_at_ms = excluded.last_attempt_at_ms,
                last_safe_error_code = NULL,
                gap_requested_after_seq = excluded.gap_requested_after_seq,
                gap_retained_from_seq = excluded.gap_retained_from_seq,
                gap_latest_seq = excluded.gap_latest_seq,
                backfill_run_id = excluded.backfill_run_id,
                backfill_after_observation_id = NULL,
                backfill_through_seq = excluded.backfill_through_seq",
            params![
                environment_id.as_str(),
                node_store_id.as_str(),
                now_ms,
                gap.as_ref()
                    .map(|value| checked_i64(
                        value.requested_after_seq,
                        "usage coverage gap requested sequence"
                    ))
                    .transpose()?,
                gap.as_ref()
                    .map(|value| checked_i64(
                        value.retained_from_seq,
                        "usage coverage gap retained sequence"
                    ))
                    .transpose()?,
                gap.as_ref()
                    .map(|value| checked_i64(
                        value.latest_seq,
                        "usage coverage gap latest sequence"
                    ))
                    .transpose()?,
                run_id,
                checked_i64(snapshot_through_seq, "usage backfill snapshot")?,
                reconciliation_state,
            ],
        )?;
        upsert_hub_environment_status(
            &conn,
            environment_id,
            reconciliation_state,
            now_ms,
            None,
            None,
        )?;
        load_hub_sync_state(&conn, environment_id)?.ok_or_else(|| {
            StoreError::Data("usage backfill state disappeared after insert".to_string())
        })
    }

    pub fn ingest_usage_export_page(
        &self,
        environment_id: &EnvironmentId,
        page: &UsageExportResult,
    ) -> Result<UsageHubIngestOutcome, StoreError> {
        validate_hub_scope(environment_id, &page.node_store_id)?;
        if page.coverage_gap.is_some() {
            return Err(StoreError::Data(
                "coverage-gap export must enter bounded backfill".to_string(),
            ));
        }

        let mut items = page.items.clone();
        items.sort_by_key(|item| item.seq);
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let current = load_hub_sync_state(&tx, environment_id)?;
        if let Some(current) = &current
            && current.node_store_id != page.node_store_id
        {
            return Err(StoreError::Data(
                "node store identity changed before incremental ingestion".to_string(),
            ));
        }
        let mut high_water = current.as_ref().map_or(0, |state| state.high_water_seq);
        let starting_high_water = high_water;
        let mut outcome = UsageHubIngestOutcome {
            high_water_seq: high_water,
            inserted: 0,
            updated: 0,
            duplicates: 0,
        };
        for item in &items {
            if item.seq > page.latest_seq {
                return Err(StoreError::Data(
                    "usage export item exceeds the node latest sequence".to_string(),
                ));
            }
            if item.seq <= high_water {
                outcome.duplicates = outcome.duplicates.checked_add(1).ok_or_else(|| {
                    StoreError::Data("usage duplicate count overflow".to_string())
                })?;
                continue;
            }
            let expected_seq = high_water.checked_add(1).ok_or_else(|| {
                StoreError::Data("usage hub high-water sequence overflow".to_string())
            })?;
            if item.seq != expected_seq {
                return Err(StoreError::Data(format!(
                    "usage export page is out of order: expected sequence {expected_seq}"
                )));
            }
            let change = upsert_remote_observation(
                &tx,
                environment_id,
                &page.node_store_id,
                item.seq,
                None,
                &item.observation,
            )?;
            match change {
                RemoteObservationChange::Inserted => outcome.inserted += 1,
                RemoteObservationChange::Updated => outcome.updated += 1,
                RemoteObservationChange::Duplicate => outcome.duplicates += 1,
            }
            high_water = high_water.max(item.seq);
        }
        if (high_water > starting_high_water && page.next_after_seq != high_water)
            || page.next_after_seq > high_water
        {
            return Err(StoreError::Data(
                "usage export cursor does not match its page contents".to_string(),
            ));
        }
        persist_remote_scanner_diagnostics(&tx, environment_id, page.node_coverage.as_ref())?;
        let quarantined_count = page
            .node_coverage
            .as_ref()
            .map_or(0, |coverage| coverage.quarantined_count);
        let latest_source_at_ms = page
            .node_coverage
            .as_ref()
            .and_then(|coverage| coverage.latest_source_at_ms);
        let now_ms = checked_i64(store_now_ms(), "usage sync success time")?;
        tx.execute(
            "INSERT INTO usage_hub_high_water (
                environment_id, node_store_id, high_water_seq,
                reconciliation_state, updated_at_ms, last_attempt_at_ms,
                last_success_at_ms, last_safe_error_code,
                quarantined_count, latest_source_at_ms
             ) VALUES (?1, ?2, ?3, 'ready', ?4, ?4, ?4, NULL, ?5, ?6)
             ON CONFLICT(environment_id) DO UPDATE SET
                node_store_id = excluded.node_store_id,
                high_water_seq = MAX(
                    usage_hub_high_water.high_water_seq,
                    excluded.high_water_seq
                ),
                reconciliation_state = 'ready',
                updated_at_ms = excluded.updated_at_ms,
                last_attempt_at_ms = excluded.last_attempt_at_ms,
                last_success_at_ms = excluded.last_success_at_ms,
                last_safe_error_code = NULL,
                gap_requested_after_seq = NULL,
                gap_retained_from_seq = NULL,
                gap_latest_seq = NULL,
                backfill_run_id = NULL,
                backfill_after_observation_id = NULL,
                backfill_through_seq = NULL,
                quarantined_count = excluded.quarantined_count,
                latest_source_at_ms = excluded.latest_source_at_ms",
            params![
                environment_id.as_str(),
                page.node_store_id.as_str(),
                checked_i64(high_water, "usage hub high-water sequence")?,
                now_ms,
                checked_i64(quarantined_count, "remote quarantine count")?,
                optional_checked_i64(latest_source_at_ms, "remote latest source time")?,
            ],
        )?;
        upsert_hub_environment_status(&tx, environment_id, "ready", now_ms, Some(now_ms), None)?;
        tx.commit()?;
        outcome.high_water_seq = high_water;
        Ok(outcome)
    }

    pub fn ingest_usage_backfill_page(
        &self,
        environment_id: &EnvironmentId,
        page: &UsageBackfillResult,
    ) -> Result<UsageHubIngestOutcome, StoreError> {
        validate_hub_scope(environment_id, &page.node_store_id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let state = load_hub_sync_state(&tx, environment_id)?.ok_or_else(|| {
            StoreError::Data("usage backfill has no durable reconciliation state".to_string())
        })?;
        if state.node_store_id != page.node_store_id
            || state.backfill_through_seq != Some(page.snapshot_through_seq)
        {
            return Err(StoreError::Data(
                "usage backfill page does not match durable reconciliation state".to_string(),
            ));
        }
        let run_id = state.backfill_run_id.as_deref().ok_or_else(|| {
            StoreError::Data("usage backfill run identity is missing".to_string())
        })?;
        let mut outcome = UsageHubIngestOutcome {
            high_water_seq: state.high_water_seq,
            inserted: 0,
            updated: 0,
            duplicates: 0,
        };
        for item in &page.items {
            if item.latest_seq > page.snapshot_through_seq {
                return Err(StoreError::Data(
                    "usage backfill item exceeds its snapshot sequence".to_string(),
                ));
            }
            let change = upsert_remote_observation(
                &tx,
                environment_id,
                &page.node_store_id,
                item.latest_seq,
                Some(run_id),
                &item.observation,
            )?;
            match change {
                RemoteObservationChange::Inserted => outcome.inserted += 1,
                RemoteObservationChange::Updated => outcome.updated += 1,
                RemoteObservationChange::Duplicate => outcome.duplicates += 1,
            }
        }
        persist_remote_scanner_diagnostics(&tx, environment_id, page.node_coverage.as_ref())?;
        let quarantined_count = page
            .node_coverage
            .as_ref()
            .map_or(0, |coverage| coverage.quarantined_count);
        let latest_source_at_ms = page
            .node_coverage
            .as_ref()
            .and_then(|coverage| coverage.latest_source_at_ms);

        let next = page
            .next
            .as_ref()
            .map(|cursor| cursor.observation_id.as_str());
        if page.has_more && next.is_none() {
            return Err(StoreError::Data(
                "usage backfill page with more data requires a cursor".to_string(),
            ));
        }
        let now_ms = checked_i64(store_now_ms(), "usage backfill update time")?;
        if page.has_more {
            tx.execute(
                "UPDATE usage_hub_high_water
                 SET reconciliation_state = 'reconciling',
                     backfill_after_observation_id = ?1,
                     updated_at_ms = ?2,
                     last_attempt_at_ms = ?2,
                     last_safe_error_code = NULL,
                     quarantined_count = ?3,
                     latest_source_at_ms = ?4
                 WHERE environment_id = ?5",
                params![
                    next,
                    now_ms,
                    checked_i64(quarantined_count, "remote quarantine count")?,
                    optional_checked_i64(latest_source_at_ms, "remote latest source time")?,
                    environment_id.as_str(),
                ],
            )?;
            upsert_hub_environment_status(&tx, environment_id, "reconciling", now_ms, None, None)?;
        } else {
            tx.execute(
                "DELETE FROM usage_observations
                 WHERE environment_id = ?1
                   AND (
                       source_node_store_id <> ?2 OR
                       source_node_store_id IS NULL OR
                       hub_sync_run_id <> ?3 OR
                       hub_sync_run_id IS NULL
                   )",
                params![environment_id.as_str(), page.node_store_id.as_str(), run_id],
            )?;
            tx.execute(
                "UPDATE usage_hub_high_water
                 SET high_water_seq = ?1,
                     reconciliation_state = 'ready',
                     updated_at_ms = ?2,
                     last_attempt_at_ms = ?2,
                     last_success_at_ms = ?2,
                     last_safe_error_code = NULL,
                     gap_requested_after_seq = NULL,
                     gap_retained_from_seq = NULL,
                     gap_latest_seq = NULL,
                     backfill_run_id = NULL,
                     backfill_after_observation_id = NULL,
                     backfill_through_seq = NULL,
                     quarantined_count = ?3,
                     latest_source_at_ms = ?4
                 WHERE environment_id = ?5",
                params![
                    checked_i64(page.snapshot_through_seq, "usage backfill high-water")?,
                    now_ms,
                    checked_i64(quarantined_count, "remote quarantine count")?,
                    optional_checked_i64(latest_source_at_ms, "remote latest source time")?,
                    environment_id.as_str(),
                ],
            )?;
            upsert_hub_environment_status(
                &tx,
                environment_id,
                "ready",
                now_ms,
                Some(now_ms),
                None,
            )?;
            outcome.high_water_seq = page.snapshot_through_seq;
        }
        tx.commit()?;
        Ok(outcome)
    }

    pub fn record_usage_sync_failure(
        &self,
        environment_id: &EnvironmentId,
        unreachable: bool,
        safe_error_code: &str,
    ) -> Result<(), StoreError> {
        if environment_id.as_str().trim().is_empty()
            || !super::is_safe_diagnostic_code(safe_error_code)
        {
            return Err(StoreError::Data(
                "usage sync failure scope and safe code must be bounded".to_string(),
            ));
        }
        let conn = self.conn.lock().unwrap();
        let current = load_hub_sync_state(&conn, environment_id)?;
        let has_backfill = current
            .as_ref()
            .is_some_and(|current| current.backfill_run_id.is_some());
        let high_water_state = if has_backfill {
            "reconciling"
        } else if unreachable {
            "unreachable"
        } else {
            "stale"
        };
        let coverage_state = if unreachable {
            "unreachable"
        } else if has_backfill {
            "reconciling"
        } else {
            "stale"
        };
        let now_ms = checked_i64(store_now_ms(), "usage sync failure time")?;
        if current.is_some() {
            conn.execute(
                "UPDATE usage_hub_high_water
                 SET reconciliation_state = ?1,
                     updated_at_ms = ?2,
                     last_attempt_at_ms = ?2,
                     last_safe_error_code = ?3
                 WHERE environment_id = ?4",
                params![
                    high_water_state,
                    now_ms,
                    safe_error_code,
                    environment_id.as_str()
                ],
            )?;
        }
        upsert_hub_environment_status(
            &conn,
            environment_id,
            coverage_state,
            now_ms,
            None,
            Some(safe_error_code),
        )?;
        Ok(())
    }

    pub fn save_usage_pricing_version(
        &self,
        version: &UsagePricingVersion,
    ) -> Result<(), StoreError> {
        version.validate().map_err(StoreError::Data)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let rules_json = serde_json::to_string(&version.rules)?;
        tx.execute(
            "INSERT INTO usage_pricing_versions (
                id, provider, effective_from_ms, rules_json, created_at_ms,
                source_published_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                provider = excluded.provider,
                effective_from_ms = excluded.effective_from_ms,
                rules_json = excluded.rules_json,
                source_published_at_ms = excluded.source_published_at_ms",
            params![
                version.id.as_str(),
                version.provider.as_str(),
                checked_i64(version.effective_from_ms, "pricing effective time")?,
                rules_json,
                checked_i64(store_now_ms(), "pricing created time")?,
                checked_i64(version.source_published_at_ms, "pricing source time")?,
            ],
        )?;
        tx.execute(
            "DELETE FROM usage_pricing_rules WHERE pricing_version_id = ?1",
            [version.id.as_str()],
        )?;
        for rule in &version.rules {
            tx.execute(
                "INSERT INTO usage_pricing_rules (
                    pricing_version_id, canonical_model_id,
                    input_micro_usd_per_million,
                    cached_input_micro_usd_per_million,
                    cache_write_micro_usd_per_million,
                    output_micro_usd_per_million,
                    reasoning_micro_usd_per_million
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    version.id.as_str(),
                    rule.canonical_model_id,
                    optional_checked_i64(
                        rule.rates.input_micro_usd_per_million,
                        "pricing input rate"
                    )?,
                    optional_checked_i64(
                        rule.rates.cached_input_micro_usd_per_million,
                        "pricing cached input rate"
                    )?,
                    optional_checked_i64(
                        rule.rates.cache_write_micro_usd_per_million,
                        "pricing cache write rate"
                    )?,
                    optional_checked_i64(
                        rule.rates.output_micro_usd_per_million,
                        "pricing output rate"
                    )?,
                    optional_checked_i64(
                        rule.rates.reasoning_micro_usd_per_million,
                        "pricing reasoning rate"
                    )?,
                ],
            )?;
            for alias in &rule.aliases {
                tx.execute(
                    "INSERT INTO usage_pricing_aliases (
                        pricing_version_id, alias_model_id, canonical_model_id
                     ) VALUES (?1, ?2, ?3)",
                    params![version.id.as_str(), alias, rule.canonical_model_id],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

fn usage_outbox_window(conn: &rusqlite::Connection) -> Result<(u64, u64), StoreError> {
    let latest_seq = conn
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'usage_outbox'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(0);
    let retained_from_seq = conn
        .query_row("SELECT MIN(seq) FROM usage_outbox", [], |row| {
            row.get::<_, Option<i64>>(0)
        })?
        .unwrap_or_else(|| latest_seq.saturating_add(1));
    Ok((
        checked_u64(retained_from_seq, "usage retained sequence")?,
        checked_u64(latest_seq, "usage latest sequence")?,
    ))
}

fn load_node_coverage(
    conn: &rusqlite::Connection,
) -> Result<UsageNodeCoverageSnapshot, StoreError> {
    let quarantined_count = checked_u64(
        conn.query_row("SELECT COUNT(*) FROM usage_quarantine", [], |row| {
            row.get(0)
        })?,
        "usage quarantine count",
    )?;
    let observation_latest = conn
        .query_row(
            "SELECT MAX(occurred_at_ms) FROM usage_observations",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )?
        .map(|value| checked_u64(value, "latest usage observation time"))
        .transpose()?;
    let diagnostic_latest = conn
        .query_row(
            "SELECT MAX(latest_source_at_ms) FROM usage_scanner_diagnostics",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )?
        .map(|value| checked_u64(value, "latest scanner source time"))
        .transpose()?;
    Ok(UsageNodeCoverageSnapshot {
        quarantined_count,
        latest_source_at_ms: observation_latest.max(diagnostic_latest),
        scanner_diagnostics: super::load_usage_scanner_diagnostics(
            conn,
            &UsageQueryParams::default(),
        )?,
    })
}

fn load_hub_sync_state(
    conn: &rusqlite::Connection,
    environment_id: &EnvironmentId,
) -> Result<Option<UsageHubSyncState>, StoreError> {
    let row = conn
        .query_row(
            "SELECT node_store_id, high_water_seq, reconciliation_state,
                    last_attempt_at_ms, last_success_at_ms, last_safe_error_code,
                    gap_requested_after_seq, gap_retained_from_seq, gap_latest_seq,
                    backfill_run_id, backfill_after_observation_id,
                    backfill_through_seq
             FROM usage_hub_high_water WHERE environment_id = ?1",
            [environment_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                ))
            },
        )
        .optional()?;
    row.map(|row| {
        let coverage_gap = match (row.6, row.7, row.8) {
            (Some(requested_after_seq), Some(retained_from_seq), Some(latest_seq)) => {
                Some(UsageCoverageGap {
                    requested_after_seq: checked_u64(
                        requested_after_seq,
                        "usage gap requested sequence",
                    )?,
                    retained_from_seq: checked_u64(
                        retained_from_seq,
                        "usage gap retained sequence",
                    )?,
                    latest_seq: checked_u64(latest_seq, "usage gap latest sequence")?,
                })
            }
            (None, None, None) => None,
            _ => {
                return Err(StoreError::Data(
                    "usage coverage gap state is incomplete".to_string(),
                ));
            }
        };
        Ok(UsageHubSyncState {
            environment_id: environment_id.clone(),
            node_store_id: NodeStoreId::from(row.0),
            high_water_seq: checked_u64(row.1, "usage hub high-water sequence")?,
            reconciliation_state: row.2,
            last_attempt_at_ms: row
                .3
                .map(|value| checked_u64(value, "usage sync attempt time"))
                .transpose()?,
            last_success_at_ms: row
                .4
                .map(|value| checked_u64(value, "usage sync success time"))
                .transpose()?,
            last_safe_error_code: row.5,
            coverage_gap,
            backfill_run_id: row.9,
            backfill_after_observation_id: row.10,
            backfill_through_seq: row
                .11
                .map(|value| checked_u64(value, "usage backfill snapshot"))
                .transpose()?,
        })
    })
    .transpose()
}

fn validate_hub_scope(
    environment_id: &EnvironmentId,
    node_store_id: &NodeStoreId,
) -> Result<(), StoreError> {
    if environment_id.as_str().trim().is_empty() || node_store_id.as_str().trim().is_empty() {
        return Err(StoreError::Data(
            "usage hub environment and node store identity must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn upsert_hub_environment_status(
    conn: &rusqlite::Connection,
    environment_id: &EnvironmentId,
    state: &str,
    attempt_at_ms: i64,
    success_at_ms: Option<i64>,
    safe_error_code: Option<&str>,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO usage_hub_environment_status (
            environment_id, sync_state, last_attempt_at_ms,
            last_success_at_ms, last_safe_error_code, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?3)
         ON CONFLICT(environment_id) DO UPDATE SET
            sync_state = excluded.sync_state,
            last_attempt_at_ms = excluded.last_attempt_at_ms,
            last_success_at_ms = COALESCE(
                excluded.last_success_at_ms,
                usage_hub_environment_status.last_success_at_ms
            ),
            last_safe_error_code = excluded.last_safe_error_code,
            updated_at_ms = excluded.updated_at_ms",
        params![
            environment_id.as_str(),
            state,
            attempt_at_ms,
            success_at_ms,
            safe_error_code,
        ],
    )?;
    Ok(())
}

fn persist_remote_scanner_diagnostics(
    tx: &Transaction<'_>,
    environment_id: &EnvironmentId,
    coverage: Option<&UsageNodeCoverageSnapshot>,
) -> Result<(), StoreError> {
    let Some(coverage) = coverage else {
        return Ok(());
    };
    tx.execute(
        "DELETE FROM usage_scanner_diagnostics WHERE environment_id = ?1",
        [environment_id.as_str()],
    )?;
    for diagnostic in &coverage.scanner_diagnostics {
        let mut diagnostic = diagnostic.clone();
        diagnostic.environment_id = environment_id.clone();
        super::validate_usage_diagnostic(&diagnostic)?;
        super::upsert_usage_diagnostic(tx, &diagnostic)?;
    }
    Ok(())
}

enum RemoteObservationChange {
    Inserted,
    Updated,
    Duplicate,
}

fn upsert_remote_observation(
    tx: &Transaction<'_>,
    environment_id: &EnvironmentId,
    node_store_id: &NodeStoreId,
    outbox_seq: u64,
    sync_run_id: Option<&str>,
    observation: &UsageObservation,
) -> Result<RemoteObservationChange, StoreError> {
    observation
        .validate()
        .map_err(|error| StoreError::Data(error.to_string()))?;
    let existing = load_usage_observation(tx, environment_id, &observation.id)?;
    let existing_revision = tx
        .query_row(
            "SELECT source_node_store_id, source_outbox_seq
             FROM usage_observations
             WHERE environment_id = ?1 AND observation_id = ?2",
            params![environment_id.as_str(), observation.id.as_str()],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                ))
            },
        )
        .optional()?;
    if let (Some(existing), Some((Some(existing_store), Some(existing_seq)))) =
        (&existing, &existing_revision)
        && existing_store == node_store_id.as_str()
    {
        let existing_seq = checked_u64(*existing_seq, "remote usage revision")?;
        if existing_seq > outbox_seq {
            return Ok(RemoteObservationChange::Duplicate);
        }
        if existing_seq == outbox_seq {
            if existing != observation {
                return Err(StoreError::Data(format!(
                    "remote usage revision collision for {}",
                    observation.id
                )));
            }
            if let Some(run_id) = sync_run_id {
                tx.execute(
                    "UPDATE usage_observations SET hub_sync_run_id = ?1
                     WHERE environment_id = ?2 AND observation_id = ?3",
                    params![run_id, environment_id.as_str(), observation.id.as_str()],
                )?;
            }
            return Ok(RemoteObservationChange::Duplicate);
        }
    }

    ensure_remote_subscription(tx, observation)?;
    let other_tokens_json = serde_json::to_string(&observation.tokens.other)?;
    let unknown_fields_json = serde_json::to_string(&observation.unknown_fields)?;
    tx.execute(
        "INSERT INTO usage_observations (
            environment_id, observation_id, source_kind, parser_version,
            provider_native_session_id, provider_native_turn_id, occurred_at_ms,
            provider, provider_instance_id, subscription_key, model,
            input_tokens, cached_input_tokens, cache_write_tokens, output_tokens,
            reasoning_tokens, other_tokens_json, unknown_fields_json,
            provider_reported_micro_usd, source_record_count, thread_id,
            turn_id, agent_id, schedule_id, fact_kind, created_at_ms, reconciliation_key,
            source_node_store_id, source_outbox_seq, hub_sync_run_id
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
            ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25,
            ?26, NULL, ?27, ?28, ?29
         ) ON CONFLICT(environment_id, observation_id) DO UPDATE SET
            source_kind = excluded.source_kind,
            parser_version = excluded.parser_version,
            provider_native_session_id = excluded.provider_native_session_id,
            provider_native_turn_id = excluded.provider_native_turn_id,
            occurred_at_ms = excluded.occurred_at_ms,
            provider = excluded.provider,
            provider_instance_id = excluded.provider_instance_id,
            subscription_key = excluded.subscription_key,
            model = excluded.model,
            input_tokens = excluded.input_tokens,
            cached_input_tokens = excluded.cached_input_tokens,
            cache_write_tokens = excluded.cache_write_tokens,
            output_tokens = excluded.output_tokens,
            reasoning_tokens = excluded.reasoning_tokens,
            other_tokens_json = excluded.other_tokens_json,
            unknown_fields_json = excluded.unknown_fields_json,
            provider_reported_micro_usd = excluded.provider_reported_micro_usd,
            source_record_count = excluded.source_record_count,
            thread_id = excluded.thread_id,
            turn_id = excluded.turn_id,
            agent_id = excluded.agent_id,
            schedule_id = excluded.schedule_id,
            fact_kind = excluded.fact_kind,
            created_at_ms = excluded.created_at_ms,
            reconciliation_key = NULL,
            source_node_store_id = excluded.source_node_store_id,
            source_outbox_seq = excluded.source_outbox_seq,
            hub_sync_run_id = excluded.hub_sync_run_id",
        params![
            environment_id.as_str(),
            observation.id.as_str(),
            observation.source_kind.as_str(),
            i64::from(observation.parser_version),
            observation.provider_native_session_id,
            observation.provider_native_turn_id,
            optional_checked_i64(observation.occurred_at_ms, "remote usage occurred time")?,
            observation.provider.as_str(),
            observation.provider_instance_id.as_str(),
            observation.subscription_key.as_str(),
            observation.model,
            checked_i64(observation.tokens.input, "remote input tokens")?,
            checked_i64(
                observation.tokens.cached_input,
                "remote cached input tokens"
            )?,
            checked_i64(observation.tokens.cache_write, "remote cache write tokens")?,
            checked_i64(observation.tokens.output, "remote output tokens")?,
            checked_i64(observation.tokens.reasoning, "remote reasoning tokens")?,
            other_tokens_json,
            unknown_fields_json,
            optional_checked_i64(
                observation.provider_reported_micro_usd,
                "remote provider reported cost"
            )?,
            checked_i64(
                observation.source_record_count,
                "remote source record count"
            )?,
            observation.thread_id.as_ref().map(ThreadId::as_str),
            observation.turn_id.as_ref().map(TurnId::as_str),
            observation.agent_id.as_ref().map(|value| value.as_str()),
            observation.schedule_id.as_ref().map(|value| value.as_str()),
            observation.fact_kind.as_str(),
            checked_i64(store_now_ms(), "remote usage ingestion time")?,
            node_store_id.as_str(),
            checked_i64(outbox_seq, "remote outbox sequence")?,
            sync_run_id,
        ],
    )?;
    Ok(if existing.is_some() {
        RemoteObservationChange::Updated
    } else {
        RemoteObservationChange::Inserted
    })
}

fn ensure_remote_subscription(
    tx: &Transaction<'_>,
    observation: &UsageObservation,
) -> Result<(), StoreError> {
    let now_ms = checked_i64(store_now_ms(), "remote subscription time")?;
    tx.execute(
        "INSERT INTO usage_subscriptions (
            subscription_key, provider, display_name, masked_account_label,
            created_at_ms, updated_at_ms
         ) VALUES (?1, ?2, ?3, NULL, ?4, ?4)
         ON CONFLICT(subscription_key) DO NOTHING",
        params![
            observation.subscription_key.as_str(),
            observation.provider.as_str(),
            format!("{} subscription", observation.provider.as_str()),
            now_ms,
        ],
    )?;
    let provider = tx.query_row(
        "SELECT provider FROM usage_subscriptions WHERE subscription_key = ?1",
        [observation.subscription_key.as_str()],
        |row| row.get::<_, String>(0),
    )?;
    if provider != observation.provider.as_str() {
        return Err(StoreError::Data(
            "remote usage subscription provider mismatch".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{
        AgentId, PricingVersionId, ProviderInstanceId, ScheduleId, SubscriptionKey, TokenCounts,
        UsageFactKind, UsageObservationId, UsagePricingRates, UsagePricingRule, UsageProvider,
        UsageSourceKind, UsageSubscription, UsageUnknownField,
    };
    use pandamux_protocol::{
        UsageBackfillCursor, UsageExportItem, UsageExportParams, UsageNodeCoverageSnapshot,
        UsageQueryParams, UsageScannerDiagnostic, UsageSourceCoverageState,
    };
    use std::collections::BTreeSet;
    use tempfile::NamedTempFile;

    fn configure_store(store: &Store, environment_id: &EnvironmentId) {
        store
            .save_usage_subscription(
                &UsageSubscription {
                    key: SubscriptionKey::new("subkey-test"),
                    provider: UsageProvider::Claude,
                    display_name: "Test subscription".to_string(),
                    masked_account_label: Some("t***@example.com".to_string()),
                },
                1,
            )
            .expect("save usage subscription");
        store
            .map_provider_subscription(
                environment_id,
                &ProviderInstanceId::new("claude-test"),
                &SubscriptionKey::new("subkey-test"),
                1,
            )
            .expect("map usage subscription");
    }

    fn observation(id: &str, occurred_at_ms: u64, input: u64) -> UsageObservation {
        UsageObservation {
            id: UsageObservationId::new(id),
            source_kind: UsageSourceKind::ClaudeTranscript,
            parser_version: 1,
            provider_native_session_id: Some("session-test".to_string()),
            provider_native_turn_id: Some(format!("turn-{id}")),
            occurred_at_ms: Some(occurred_at_ms),
            provider: UsageProvider::Claude,
            provider_instance_id: ProviderInstanceId::new("claude-test"),
            subscription_key: SubscriptionKey::new("subkey-test"),
            model: Some("claude-model".to_string()),
            tokens: TokenCounts {
                input,
                output: 5,
                ..TokenCounts::default()
            },
            unknown_fields: BTreeSet::<UsageUnknownField>::new(),
            provider_reported_micro_usd: None,
            source_record_count: 1,
            thread_id: None,
            turn_id: None,
            agent_id: None,
            schedule_id: None,
            fact_kind: UsageFactKind::TokenComponent,
        }
    }

    #[test]
    fn acknowledged_retention_reports_gap_and_backfills_all_facts() {
        let store = Store::in_memory().expect("open node store");
        let environment_id = EnvironmentId::new("env-node");
        configure_store(&store, &environment_id);
        for index in 1..=3 {
            store
                .record_usage_batch(
                    &environment_id,
                    &[observation(&format!("usage-{index}"), 1_000 + index, index)],
                    None,
                )
                .expect("record usage observation");
        }

        let compacted = store
            .acknowledge_and_compact_usage(
                2,
                UsageOutboxRetention {
                    max_rows: 1,
                    max_age_ms: u64::MAX / 2,
                    max_bytes: u64::MAX / 2,
                },
            )
            .expect("compact acknowledged outbox");
        assert_eq!(compacted.acknowledged, 2);
        assert_eq!(compacted.compacted, 2);
        assert_eq!(compacted.retained_from_seq, 3);

        let incremental = store
            .export_usage(&UsageExportParams {
                after_seq: 0,
                limit: 10,
            })
            .expect("export retained window");
        assert_eq!(
            incremental.coverage_gap,
            Some(UsageCoverageGap {
                requested_after_seq: 0,
                retained_from_seq: 3,
                latest_seq: 3,
            })
        );
        assert!(incremental.items.is_empty());
        let wire = serde_json::to_string(&incremental).expect("serialize gap export");
        assert!(!wire.contains("Test subscription"));
        assert!(!wire.contains("example.com"));
        assert!(!wire.contains("canonicalPath"));
        assert!(!wire.contains("profile"));

        let first = store
            .export_usage_backfill(&UsageBackfillParams {
                snapshot_through_seq: None,
                after: None,
                limit: 2,
            })
            .expect("first backfill page");
        assert_eq!(first.items.len(), 2);
        assert!(first.has_more);
        let second = store
            .export_usage_backfill(&UsageBackfillParams {
                snapshot_through_seq: Some(first.snapshot_through_seq),
                after: first.next.clone(),
                limit: 2,
            })
            .expect("second backfill page");
        assert_eq!(second.items.len(), 1);
        assert!(!second.has_more);
    }

    #[test]
    fn outbox_age_and_byte_limits_never_compact_unacknowledged_rows() {
        let store = Store::in_memory().expect("open retention store");
        let environment_id = EnvironmentId::new("env-node");
        configure_store(&store, &environment_id);
        store
            .record_usage_batch(
                &environment_id,
                &[
                    observation("usage-age", 1_000, 1),
                    observation("usage-bytes", 2_000, 2),
                ],
                None,
            )
            .expect("record retention fixtures");
        store
            .conn
            .lock()
            .expect("store lock")
            .execute(
                "UPDATE usage_outbox SET enqueued_at_ms = 0 WHERE seq = 1",
                [],
            )
            .expect("age first outbox row");
        let aged = store
            .acknowledge_and_compact_usage(
                1,
                UsageOutboxRetention {
                    max_rows: 10,
                    max_age_ms: 1,
                    max_bytes: u64::MAX / 2,
                },
            )
            .expect("compact aged acknowledgement");
        assert_eq!(aged.compacted, 1);
        assert_eq!(aged.retained_rows, 1);
        assert_eq!(aged.retained_from_seq, 2);

        let bytes = store
            .acknowledge_and_compact_usage(
                2,
                UsageOutboxRetention {
                    max_rows: 10,
                    max_age_ms: u64::MAX / 2,
                    max_bytes: 1,
                },
            )
            .expect("compact byte-bounded acknowledgement");
        assert_eq!(bytes.compacted, 1);
        assert_eq!(bytes.retained_rows, 0);
        assert_eq!(bytes.retained_from_seq, 3);
        assert_eq!(bytes.latest_seq, 2);
    }

    #[test]
    fn hub_ingestion_is_idempotent_for_duplicate_and_reordered_pages() {
        let hub = Store::in_memory().expect("open hub store");
        let environment_id = EnvironmentId::new("env-remote");
        let node_store_id = NodeStoreId::new("store-remote");
        let page = UsageExportResult {
            node_store_id: node_store_id.clone(),
            items: vec![
                UsageExportItem {
                    seq: 2,
                    observation: observation("usage-two", 2_000, 20),
                },
                UsageExportItem {
                    seq: 1,
                    observation: observation("usage-one", 1_000, 10),
                },
            ],
            next_after_seq: 2,
            has_more: false,
            retained_from_seq: 1,
            latest_seq: 2,
            coverage_gap: None,
            node_coverage: Some(UsageNodeCoverageSnapshot {
                quarantined_count: 7,
                latest_source_at_ms: Some(2_500),
                scanner_diagnostics: vec![UsageScannerDiagnostic {
                    environment_id: EnvironmentId::new("env-forged"),
                    provider_instance_id: ProviderInstanceId::new("claude-test"),
                    source_kind: UsageSourceKind::ClaudeTranscript,
                    coverage_state: UsageSourceCoverageState::Partial,
                    parser_version: 1,
                    last_scan_started_at_ms: 2_400,
                    last_success_at_ms: Some(2_300),
                    latest_source_at_ms: Some(2_500),
                    source_lag_ms: Some(10),
                    files_scanned: 2,
                    bytes_scanned: 100,
                    quarantined_count: 7,
                    last_safe_error_code: Some("usage_record_malformed".to_string()),
                }],
            }),
        };
        let first = hub
            .ingest_usage_export_page(&environment_id, &page)
            .expect("ingest reordered page");
        assert_eq!(first.inserted, 2);
        assert_eq!(first.high_water_seq, 2);
        let duplicate = hub
            .ingest_usage_export_page(&environment_id, &page)
            .expect("replay duplicate page");
        assert_eq!(duplicate.duplicates, 2);
        assert_eq!(duplicate.inserted, 0);

        let query = hub
            .query_usage(&UsageQueryParams {
                environment_id: Some(environment_id),
                ..UsageQueryParams::default()
            })
            .expect("query synchronized usage");
        assert_eq!(query.aggregate.observation_count, 2);
        assert_eq!(query.aggregate.tokens.input, 30);
        assert_eq!(query.items.len(), 2);
        assert!(
            query
                .items
                .iter()
                .all(|item| item.observation.fact_kind == UsageFactKind::TokenComponent)
        );
        assert_eq!(query.coverage.quarantined_count, 7);
        assert_eq!(query.coverage.latest_source_at_ms, Some(2_500));
        assert_eq!(query.coverage.scanner_diagnostics.len(), 1);
        assert_eq!(
            query.coverage.scanner_diagnostics[0].environment_id,
            EnvironmentId::new("env-remote")
        );
    }

    #[test]
    fn reordered_pages_wait_for_the_missing_sequence_without_losing_facts() {
        let hub = Store::in_memory().expect("open reordered hub");
        let environment_id = EnvironmentId::new("env-remote");
        let node_store_id = NodeStoreId::new("store-remote");
        let page = |seq: u64, id: &str, input: u64| UsageExportResult {
            node_store_id: node_store_id.clone(),
            items: vec![UsageExportItem {
                seq,
                observation: observation(id, seq * 1_000, input),
            }],
            next_after_seq: seq,
            has_more: seq == 1,
            retained_from_seq: 1,
            latest_seq: 2,
            coverage_gap: None,
            node_coverage: None,
        };
        assert!(
            hub.ingest_usage_export_page(&environment_id, &page(2, "usage-two", 20),)
                .is_err()
        );
        hub.ingest_usage_export_page(&environment_id, &page(1, "usage-one", 10))
            .expect("ingest missing first page");
        hub.ingest_usage_export_page(&environment_id, &page(2, "usage-two", 20))
            .expect("retry second page");
        let query = hub
            .query_usage(&UsageQueryParams::default())
            .expect("query reordered pages");
        assert_eq!(query.aggregate.observation_count, 2);
        assert_eq!(query.aggregate.tokens.input, 30);
    }

    #[test]
    fn unreachable_environment_is_visible_before_its_first_successful_sync() {
        let hub = Store::in_memory().expect("open hub store");
        let environment_id = EnvironmentId::new("env-offline");
        hub.record_usage_sync_failure(&environment_id, true, "usage_sync_unreachable")
            .expect("record unreachable environment");
        let query = hub
            .query_usage(&UsageQueryParams::default())
            .expect("query empty hub coverage");
        assert_eq!(query.aggregate.observation_count, 0);
        assert_eq!(query.coverage.environment_sync.len(), 1);
        assert_eq!(
            query.coverage.environment_sync[0].state,
            pandamux_protocol::UsageEnvironmentSyncState::Unreachable
        );
        assert_eq!(
            query.coverage.environment_sync[0]
                .last_safe_error_code
                .as_deref(),
            Some("usage_sync_unreachable")
        );
    }

    #[test]
    fn coverage_gap_remains_explicit_while_bounded_backfill_is_pending() {
        let hub = Store::in_memory().expect("open gap hub");
        let environment_id = EnvironmentId::new("env-gap");
        let gap = UsageCoverageGap {
            requested_after_seq: 4,
            retained_from_seq: 8,
            latest_seq: 10,
        };
        hub.begin_usage_backfill(
            &environment_id,
            &NodeStoreId::new("store-gap"),
            10,
            Some(&gap),
        )
        .expect("persist coverage gap");
        let query = hub
            .query_usage(&UsageQueryParams::default())
            .expect("query pending gap");
        assert_eq!(query.coverage.environment_sync.len(), 1);
        assert_eq!(
            query.coverage.environment_sync[0].state,
            pandamux_protocol::UsageEnvironmentSyncState::CoverageGap
        );
        assert_eq!(
            query.coverage.environment_sync[0].coverage_gap.as_ref(),
            Some(&gap)
        );
    }

    #[test]
    fn sql_aggregate_applies_every_usage_filter() {
        let hub = Store::in_memory().expect("open filter hub");
        let utc_day_start_ms = 1_704_067_200_000;
        let mut target = observation("usage-target", utc_day_start_ms, 11);
        target.source_kind = UsageSourceKind::ManagedTurn;
        target.provider_instance_id = ProviderInstanceId::new("claude-target");
        target.subscription_key = SubscriptionKey::new("subkey-target");
        target.model = Some("model-target".to_string());
        target.agent_id = Some(AgentId::new("agent-target"));
        target.schedule_id = Some(ScheduleId::new("schedule-target"));

        let mut decoy = observation("usage-decoy", utc_day_start_ms - 1, 22);
        decoy.source_kind = UsageSourceKind::CodexTranscript;
        decoy.provider = UsageProvider::Codex;
        decoy.provider_instance_id = ProviderInstanceId::new("codex-decoy");
        decoy.subscription_key = SubscriptionKey::new("subkey-decoy");
        decoy.model = Some("model-decoy".to_string());
        decoy.agent_id = Some(AgentId::new("agent-decoy"));
        decoy.schedule_id = Some(ScheduleId::new("schedule-decoy"));

        for (environment_id, node_store_id, value) in [
            (
                EnvironmentId::new("env-target"),
                NodeStoreId::new("store-target"),
                target,
            ),
            (
                EnvironmentId::new("env-decoy"),
                NodeStoreId::new("store-decoy"),
                decoy,
            ),
        ] {
            hub.ingest_usage_export_page(
                &environment_id,
                &UsageExportResult {
                    node_store_id,
                    items: vec![UsageExportItem {
                        seq: 1,
                        observation: value,
                    }],
                    next_after_seq: 1,
                    has_more: false,
                    retained_from_seq: 1,
                    latest_seq: 1,
                    coverage_gap: None,
                    node_coverage: None,
                },
            )
            .expect("ingest filter fixture");
        }

        let filters = [
            UsageQueryParams {
                from_ms: Some(utc_day_start_ms),
                to_ms: Some(utc_day_start_ms),
                ..UsageQueryParams::default()
            },
            UsageQueryParams {
                environment_id: Some(EnvironmentId::new("env-target")),
                ..UsageQueryParams::default()
            },
            UsageQueryParams {
                provider: Some(UsageProvider::Claude),
                ..UsageQueryParams::default()
            },
            UsageQueryParams {
                provider_instance_id: Some(ProviderInstanceId::new("claude-target")),
                ..UsageQueryParams::default()
            },
            UsageQueryParams {
                subscription_key: Some(SubscriptionKey::new("subkey-target")),
                ..UsageQueryParams::default()
            },
            UsageQueryParams {
                agent_id: Some(AgentId::new("agent-target")),
                ..UsageQueryParams::default()
            },
            UsageQueryParams {
                schedule_id: Some(ScheduleId::new("schedule-target")),
                ..UsageQueryParams::default()
            },
            UsageQueryParams {
                model: Some("model-target".to_string()),
                ..UsageQueryParams::default()
            },
            UsageQueryParams {
                source_kind: Some(UsageSourceKind::ManagedTurn),
                ..UsageQueryParams::default()
            },
        ];
        for params in filters {
            let result = hub.query_usage(&params).expect("query one filter");
            assert_eq!(result.aggregate.observation_count, 1, "params: {params:?}");
            assert_eq!(result.aggregate.tokens.input, 11, "params: {params:?}");
        }
    }

    #[test]
    fn durable_backfill_resumes_after_reopen_and_removes_old_store_facts() {
        let file = NamedTempFile::new().expect("temporary hub database");
        let path = file.path().to_path_buf();
        let environment_id = EnvironmentId::new("env-remote");
        let old_store = NodeStoreId::new("store-old");
        let new_store = NodeStoreId::new("store-new");
        {
            let hub = Store::open(&path).expect("open hub store");
            hub.ingest_usage_export_page(
                &environment_id,
                &UsageExportResult {
                    node_store_id: old_store,
                    items: vec![UsageExportItem {
                        seq: 1,
                        observation: observation("usage-old", 500, 5),
                    }],
                    next_after_seq: 1,
                    has_more: false,
                    retained_from_seq: 1,
                    latest_seq: 1,
                    coverage_gap: None,
                    node_coverage: None,
                },
            )
            .expect("ingest old-store fact");
            hub.begin_usage_backfill(&environment_id, &new_store, 2, None)
                .expect("begin store-reset backfill");
            hub.ingest_usage_backfill_page(
                &environment_id,
                &UsageBackfillResult {
                    node_store_id: new_store.clone(),
                    snapshot_through_seq: 2,
                    items: vec![UsageBackfillItem {
                        latest_seq: 1,
                        observation: observation("usage-new-one", 1_000, 10),
                    }],
                    next: Some(UsageBackfillCursor {
                        observation_id: UsageObservationId::new("usage-new-one"),
                    }),
                    has_more: true,
                    node_coverage: None,
                },
            )
            .expect("persist first backfill page");
        }

        let hub = Store::open(&path).expect("reopen hub store");
        let state = hub
            .usage_hub_sync_state(&environment_id)
            .expect("load backfill state")
            .expect("backfill state");
        assert_eq!(
            state.backfill_after_observation_id.as_deref(),
            Some("usage-new-one")
        );
        hub.ingest_usage_backfill_page(
            &environment_id,
            &UsageBackfillResult {
                node_store_id: new_store,
                snapshot_through_seq: 2,
                items: vec![UsageBackfillItem {
                    latest_seq: 2,
                    observation: observation("usage-new-two", 2_000, 20),
                }],
                next: Some(UsageBackfillCursor {
                    observation_id: UsageObservationId::new("usage-new-two"),
                }),
                has_more: false,
                node_coverage: None,
            },
        )
        .expect("complete resumed backfill");
        let query = hub
            .query_usage(&UsageQueryParams {
                environment_id: Some(environment_id.clone()),
                ..UsageQueryParams::default()
            })
            .expect("query reconciled environment");
        let ids = query
            .items
            .iter()
            .map(|item| item.observation.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["usage-new-one", "usage-new-two"]);
        assert_eq!(
            hub.usage_hub_sync_state(&environment_id)
                .expect("load completed state")
                .expect("completed state")
                .high_water_seq,
            2
        );
    }

    #[test]
    fn effective_pricing_revalues_projection_without_changing_facts() {
        let store = Store::in_memory().expect("open pricing store");
        let environment_id = EnvironmentId::new("env-local");
        configure_store(&store, &environment_id);
        let mut first = observation("usage-first", 1_000, 10);
        first.model = Some("claude-alias".to_string());
        first.provider_reported_micro_usd = Some(99);
        let second = observation("usage-second", 2_000, 10);
        store
            .record_usage_batch(&environment_id, &[first, second], None)
            .expect("record pricing fixtures");

        store
            .save_usage_pricing_version(&pricing_version("price-one", 0, 1_000_000, 2_000_000))
            .expect("save first pricing version");
        store
            .save_usage_pricing_version(&pricing_version("price-two", 2_000, 2_000_000, 4_000_000))
            .expect("save second pricing version");

        let initial = store
            .query_usage(&UsageQueryParams::default())
            .expect("query initial pricing");
        assert_eq!(initial.aggregate.tokens.input, 20);
        assert_eq!(initial.aggregate.tokens.output, 10);
        assert_eq!(
            initial.aggregate.api_equivalent_estimated_micro_usd,
            Some(60)
        );
        assert_eq!(initial.aggregate.provider_reported_micro_usd, Some(99));
        assert_eq!(initial.aggregate.provider_reported_observation_count, 1);
        assert_eq!(
            initial
                .aggregate
                .pricing_version_ids
                .iter()
                .map(|value| value.as_str())
                .collect::<Vec<_>>(),
            vec!["price-one", "price-two"]
        );
        assert_eq!(initial.coverage.unknown_price_tokens, 0);

        store
            .save_usage_pricing_version(&pricing_version("price-two", 2_000, 3_000_000, 6_000_000))
            .expect("replace second pricing projection");
        let revalued = store
            .query_usage(&UsageQueryParams::default())
            .expect("query revalued pricing");
        assert_eq!(
            revalued.aggregate.api_equivalent_estimated_micro_usd,
            Some(80)
        );
        assert_eq!(revalued.aggregate.tokens, initial.aggregate.tokens);
        assert_eq!(revalued.aggregate.observation_count, 2);

        let mut unknown = observation("usage-unknown-price", 3_000, 10);
        unknown.model = Some("unpriced-model".to_string());
        store
            .record_usage_batch(&environment_id, &[unknown], None)
            .expect("record unknown-price fact");
        let partial = store
            .query_usage(&UsageQueryParams::default())
            .expect("query partial pricing coverage");
        assert_eq!(partial.aggregate.api_equivalent_estimated_micro_usd, None);
        assert_eq!(partial.coverage.unknown_price_tokens, 15);
        assert_eq!(partial.coverage.priceable_tokens, 45);
        assert_eq!(
            partial.coverage.unknown_price_token_millionths,
            Some(333_333)
        );

        let first_only = store
            .query_usage(&UsageQueryParams {
                from_ms: Some(1_000),
                to_ms: Some(1_999),
                model: Some("claude-alias".to_string()),
                ..UsageQueryParams::default()
            })
            .expect("query exact time and alias filter");
        assert_eq!(first_only.aggregate.observation_count, 1);
        assert_eq!(
            first_only.aggregate.api_equivalent_estimated_micro_usd,
            Some(20)
        );
    }

    #[test]
    fn aggregation_conserves_generated_partitions() {
        let store = Store::in_memory().expect("open aggregate store");
        let environment_id = EnvironmentId::new("env-local");
        configure_store(&store, &environment_id);
        let mut expected_a = TokenCounts::default();
        let mut expected_b = TokenCounts::default();
        for index in 0..64_u64 {
            let mut value = observation(
                &format!("usage-generated-{index:02}"),
                10_000 + index,
                index + 1,
            );
            value.model = Some(if index % 2 == 0 {
                "model-a".to_string()
            } else {
                "model-b".to_string()
            });
            if index % 3 == 0 {
                value.tokens.other.insert("future".to_string(), index);
            }
            if index % 2 == 0 {
                expected_a
                    .checked_accumulate(&value.tokens)
                    .expect("accumulate model a");
            } else {
                expected_b
                    .checked_accumulate(&value.tokens)
                    .expect("accumulate model b");
            }
            store
                .record_usage_batch(&environment_id, &[value], None)
                .expect("record generated observation");
        }
        let all = store
            .query_usage(&UsageQueryParams::default())
            .expect("query generated total");
        let model_a = store
            .query_usage(&UsageQueryParams {
                model: Some("model-a".to_string()),
                ..UsageQueryParams::default()
            })
            .expect("query model a");
        let model_b = store
            .query_usage(&UsageQueryParams {
                model: Some("model-b".to_string()),
                ..UsageQueryParams::default()
            })
            .expect("query model b");
        assert_eq!(model_a.aggregate.tokens, expected_a);
        assert_eq!(model_b.aggregate.tokens, expected_b);
        assert_eq!(
            model_a
                .aggregate
                .tokens
                .checked_add(&model_b.aggregate.tokens)
                .expect("combine partitions"),
            all.aggregate.tokens
        );
    }

    #[test]
    fn sql_aggregate_rejects_integer_overflow_instead_of_wrapping() {
        let store = Store::in_memory().expect("open overflow store");
        let environment_id = EnvironmentId::new("env-local");
        configure_store(&store, &environment_id);
        let input = u64::try_from(i64::MAX).expect("positive i64") - 5;
        store
            .record_usage_batch(
                &environment_id,
                &[
                    observation("usage-overflow-one", 1, input),
                    observation("usage-overflow-two", 2, input),
                ],
                None,
            )
            .expect("record individually valid large facts");
        assert!(
            store.query_usage(&UsageQueryParams::default()).is_err(),
            "SQLite checked SUM must reject an aggregate outside its integer range"
        );
    }

    fn pricing_version(
        id: &str,
        effective_from_ms: u64,
        input_rate: u64,
        output_rate: u64,
    ) -> UsagePricingVersion {
        UsagePricingVersion {
            id: PricingVersionId::new(id),
            provider: UsageProvider::Claude,
            source_published_at_ms: effective_from_ms,
            effective_from_ms,
            rules: vec![UsagePricingRule {
                canonical_model_id: "claude-model".to_string(),
                aliases: BTreeSet::from(["claude-alias".to_string()]),
                rates: UsagePricingRates {
                    input_micro_usd_per_million: Some(input_rate),
                    output_micro_usd_per_million: Some(output_rate),
                    ..UsagePricingRates::default()
                },
            }],
        }
    }
}
