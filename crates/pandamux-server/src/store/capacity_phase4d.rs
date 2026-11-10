#[cfg(test)]
use super::checked_u64;
use super::{Store, StoreError, checked_i64, decode_capacity_snapshot};
use pandamux_core::{CapacitySnapshot, EnvironmentId, ProviderInstanceId};
use pandamux_protocol::ProviderHealthReport;
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeSet;

impl Store {
    pub fn save_capacity_snapshots(
        &self,
        snapshots: &[CapacitySnapshot],
    ) -> Result<(), StoreError> {
        for snapshot in snapshots {
            self.save_capacity_snapshot(snapshot)?;
        }
        Ok(())
    }

    /// Returns only the newest revision for each evidence identity on an environment.
    pub fn latest_capacity_snapshots_for_environment(
        &self,
        environment_id: &EnvironmentId,
        limit: usize,
    ) -> Result<Vec<CapacitySnapshot>, StoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT subscription_key, window_start_ms, window_end_ms,
                    used_millionths, remaining_millionths, reset_at_ms, source,
                    confidence, provider_version, as_of_ms, schema_version,
                    environment_id, provider_instance_id, window_name, evidence_scope,
                    hard_limit, max_age_ms
             FROM capacity_snapshots
             WHERE environment_id = ?1
             ORDER BY as_of_ms DESC, id DESC",
        )?;
        let mut rows = stmt.query([environment_id.as_str()])?;
        let mut identities = BTreeSet::new();
        let mut snapshots = Vec::new();
        while let Some(row) = rows.next()? {
            let snapshot = decode_capacity_snapshot(row, 0)?;
            let identity = (
                snapshot.subscription_key.clone(),
                snapshot.provider_instance_id.clone(),
                snapshot.window_name.clone(),
                snapshot
                    .window_name
                    .is_none()
                    .then_some(snapshot.window_start_ms)
                    .flatten(),
                snapshot
                    .window_name
                    .is_none()
                    .then_some(snapshot.window_end_ms)
                    .flatten(),
                snapshot.evidence_scope,
                snapshot.source.clone(),
            );
            if identities.insert(identity) {
                snapshots.push(snapshot);
                if snapshots.len() == limit {
                    break;
                }
            }
        }
        snapshots.sort_by(|left, right| {
            left.subscription_key
                .cmp(&right.subscription_key)
                .then_with(|| left.window_name.cmp(&right.window_name))
                .then_with(|| left.window_start_ms.cmp(&right.window_start_ms))
                .then_with(|| left.window_end_ms.cmp(&right.window_end_ms))
                .then_with(|| left.evidence_scope.cmp(&right.evidence_scope))
                .then_with(|| left.source.cmp(&right.source))
        });
        Ok(snapshots)
    }

    pub fn latest_provider_health_report(
        &self,
        environment_id: &EnvironmentId,
        provider_instance_id: &ProviderInstanceId,
    ) -> Result<Option<ProviderHealthReport>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let report_json = conn
            .query_row(
                "SELECT report_json
                 FROM provider_health_snapshots
                 WHERE environment_id = ?1 AND provider_instance_id = ?2",
                params![environment_id.as_str(), provider_instance_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        report_json
            .map(|value| serde_json::from_str(&value).map_err(StoreError::from))
            .transpose()
    }

    pub fn save_provider_health_report(
        &self,
        report: &ProviderHealthReport,
    ) -> Result<(), StoreError> {
        if report.environment_id.as_str().trim().is_empty()
            || report.instance_id.as_str().trim().is_empty()
        {
            return Err(StoreError::Data(
                "provider health identity must not be empty".to_string(),
            ));
        }
        if report.expires_at_ms < report.checked_at_ms {
            return Err(StoreError::Data(
                "provider health expiry precedes its check time".to_string(),
            ));
        }
        let report_json = serde_json::to_string(report)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO provider_health_snapshots (
                environment_id, provider_instance_id, report_json,
                last_success_at_ms, checked_at_ms, expires_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(environment_id, provider_instance_id) DO UPDATE SET
                report_json = excluded.report_json,
                last_success_at_ms = excluded.last_success_at_ms,
                checked_at_ms = excluded.checked_at_ms,
                expires_at_ms = excluded.expires_at_ms
             WHERE excluded.checked_at_ms >= provider_health_snapshots.checked_at_ms",
            params![
                report.environment_id.as_str(),
                report.instance_id.as_str(),
                report_json,
                report
                    .last_success_at_ms
                    .map(|value| checked_i64(value, "provider health last success"))
                    .transpose()?,
                checked_i64(report.checked_at_ms, "provider health checked at")?,
                checked_i64(report.expires_at_ms, "provider health expiry")?,
            ],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn provider_health_snapshot_count(&self) -> Result<u64, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count = conn.query_row(
            "SELECT COUNT(*) FROM provider_health_snapshots",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        checked_u64(count, "provider health snapshot count")
    }
}
