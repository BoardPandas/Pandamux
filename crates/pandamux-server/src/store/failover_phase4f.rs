use super::{Store, StoreError, checked_i64, checked_u64};
use pandamux_core::{
    AttemptBoundary, AttemptExecutionEvidence, CapacityInterruptionEvidence, ContinuationDelivery,
    ContinuationDeliveryKind, ContinuationDeliveryState, ContinuationId, ContinuationRecord,
    ContinuationState, EnvironmentId, ProviderInstanceId, RoutingDecision, SubscriptionKey,
    ThreadId, ToolLedgerEntry, ToolLedgerStatus, TurnAttempt, TurnAttemptId, TurnId, TurnInput,
    TurnStatus,
};
use rusqlite::{OptionalExtension, params};

const MAX_CONTINUATION_HANDOFF_BYTES: usize = 16 * 1024;

#[derive(Debug)]
struct RawTurnAttempt {
    id: String,
    turn_id: String,
    thread_id: String,
    ordinal: i64,
    environment_id: String,
    provider_instance_id: String,
    subscription_key: Option<String>,
    model: String,
    boundary: String,
    status: String,
    routing_json: String,
    execution_json: Option<String>,
    checkpoint: Option<String>,
    partial_output: String,
    capacity_evidence_json: Option<String>,
    started_at_ms: i64,
    ended_at_ms: Option<i64>,
}

#[derive(Debug)]
struct RawToolLedgerEntry {
    attempt_id: String,
    turn_id: String,
    provider_item_id: String,
    name: String,
    input_json: String,
    status: String,
    output: Option<String>,
    error: Option<String>,
    updated_at_ms: i64,
}

#[derive(Debug)]
struct RawContinuation {
    id: String,
    idempotency_key: String,
    thread_id: String,
    turn_id: String,
    source_attempt_id: String,
    target_provider_instance_id: Option<String>,
    target_turn_id: Option<String>,
    state: String,
    handoff: String,
    checkpoint: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(Debug)]
struct RawDelivery {
    delivery_key: String,
    continuation_id: Option<String>,
    thread_id: String,
    turn_id: String,
    kind: String,
    item_id: String,
    payload_sha256: String,
    payload_json: String,
    state: String,
    created_at_ms: i64,
    delivered_at_ms: Option<i64>,
}

impl Store {
    pub fn save_turn_attempt(&self, attempt: &TurnAttempt) -> Result<(), StoreError> {
        validate_attempt(attempt)?;
        let ordinal = i64::from(attempt.ordinal);
        let boundary = serde_json::to_string(&attempt.boundary)?;
        let status = serde_json::to_string(&attempt.status)?;
        let routing_json = serde_json::to_string(&attempt.routing)?;
        let execution_json = attempt
            .execution
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let evidence_json = attempt
            .capacity_evidence
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let started_at_ms = checked_i64(attempt.started_at_ms, "turn attempt started_at_ms")?;
        let ended_at_ms = attempt
            .ended_at_ms
            .map(|value| checked_i64(value, "turn attempt ended_at_ms"))
            .transpose()?;

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let existing = query_turn_attempt(&tx, attempt.id.as_str())?;
        if let Some(existing) = existing {
            let existing = decode_turn_attempt(existing)?;
            if existing.turn_id != attempt.turn_id
                || existing.thread_id != attempt.thread_id
                || existing.ordinal != attempt.ordinal
                || existing.environment_id != attempt.environment_id
                || existing.provider_instance_id != attempt.provider_instance_id
                || existing.subscription_key != attempt.subscription_key
                || existing.model != attempt.model
                || existing.execution != attempt.execution
                || existing.started_at_ms != attempt.started_at_ms
            {
                return Err(StoreError::Data(
                    "turn attempt identity cannot change after creation".to_string(),
                ));
            }
            if attempt.boundary < existing.boundary {
                return Err(StoreError::Data(
                    "turn attempt boundary cannot move backward".to_string(),
                ));
            }
            if !valid_attempt_status_transition(existing.status, attempt.status) {
                return Err(StoreError::Data(
                    "turn attempt status cannot move backward or change after completion"
                        .to_string(),
                ));
            }
        }

        tx.execute(
            "INSERT INTO turn_attempts (
                id, turn_id, thread_id, ordinal, environment_id,
                provider_instance_id, subscription_key, model, boundary, status,
                routing_json, execution_json, checkpoint, partial_output,
                capacity_evidence_json, started_at_ms, ended_at_ms
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16, ?17
             )
             ON CONFLICT(id) DO UPDATE SET
                boundary = excluded.boundary,
                status = excluded.status,
                routing_json = excluded.routing_json,
                checkpoint = excluded.checkpoint,
                partial_output = excluded.partial_output,
                capacity_evidence_json = excluded.capacity_evidence_json,
                ended_at_ms = excluded.ended_at_ms",
            params![
                attempt.id.as_str(),
                attempt.turn_id.as_str(),
                attempt.thread_id.as_str(),
                ordinal,
                attempt.environment_id.as_str(),
                attempt.provider_instance_id.as_str(),
                attempt
                    .subscription_key
                    .as_ref()
                    .map(SubscriptionKey::as_str),
                attempt.model,
                boundary,
                status,
                routing_json,
                execution_json,
                attempt.checkpoint,
                attempt.partial_output,
                evidence_json,
                started_at_ms,
                ended_at_ms,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_turn_attempt(&self, id: &TurnAttemptId) -> Result<Option<TurnAttempt>, StoreError> {
        let conn = self.conn.lock().unwrap();
        query_turn_attempt(&conn, id.as_str())?
            .map(decode_turn_attempt)
            .transpose()
    }

    pub fn list_turn_attempts(&self, turn_id: &TurnId) -> Result<Vec<TurnAttempt>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, turn_id, thread_id, ordinal, environment_id,
                    provider_instance_id, subscription_key, model, boundary, status,
                    routing_json, execution_json, checkpoint, partial_output,
                    capacity_evidence_json, started_at_ms, ended_at_ms
             FROM turn_attempts WHERE turn_id = ?1 ORDER BY ordinal ASC",
        )?;
        let rows = stmt.query_map([turn_id.as_str()], read_raw_turn_attempt)?;
        rows.map(|row| decode_turn_attempt(row?)).collect()
    }

    /// Move the durable turn identity to the second route only while the turn is
    /// still running. Attempt one retains the original route and execution evidence.
    pub fn retarget_running_turn_for_safe_retry(
        &self,
        turn_id: &TurnId,
        expected_provider_instance_id: &ProviderInstanceId,
        provider_instance_id: &ProviderInstanceId,
        model: &str,
    ) -> Result<(), StoreError> {
        if provider_instance_id == expected_provider_instance_id || model.trim().is_empty() {
            return Err(StoreError::Data(
                "safe retry requires a distinct provider instance and model".to_string(),
            ));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let (stored_provider, input_json, status_json) = tx
            .query_row(
                "SELECT provider_instance_id, input_json, status FROM turns WHERE id = ?1",
                [turn_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::Data(format!("turn {turn_id} does not exist")))?;
        if stored_provider != expected_provider_instance_id.as_str() {
            return Err(StoreError::Data(
                "safe retry provider no longer matches the running turn".to_string(),
            ));
        }
        let status: TurnStatus = serde_json::from_str(&status_json)?;
        if status != TurnStatus::Running {
            return Err(StoreError::Data(
                "only a running turn can move to a safe retry route".to_string(),
            ));
        }
        let mut input: TurnInput = serde_json::from_str(&input_json)?;
        input.model = Some(model.to_string());
        let updated = tx.execute(
            "UPDATE turns
             SET provider_instance_id = ?2, input_json = ?3
             WHERE id = ?1 AND provider_instance_id = ?4 AND status = ?5",
            params![
                turn_id.as_str(),
                provider_instance_id.as_str(),
                serde_json::to_string(&input)?,
                expected_provider_instance_id.as_str(),
                serde_json::to_string(&TurnStatus::Running)?,
            ],
        )?;
        if updated != 1 {
            return Err(StoreError::Data(
                "running turn changed while selecting its safe retry route".to_string(),
            ));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn advance_turn_attempt_boundary(
        &self,
        id: &TurnAttemptId,
        boundary: AttemptBoundary,
    ) -> Result<TurnAttempt, StoreError> {
        let mut attempt = self
            .get_turn_attempt(id)?
            .ok_or_else(|| StoreError::Data(format!("turn attempt {id} does not exist")))?;
        if boundary < attempt.boundary {
            return Err(StoreError::Data(
                "turn attempt boundary cannot move backward".to_string(),
            ));
        }
        attempt.boundary = boundary;
        self.save_turn_attempt(&attempt)?;
        Ok(attempt)
    }

    pub fn append_turn_attempt_output(
        &self,
        id: &TurnAttemptId,
        delta: &str,
    ) -> Result<(), StoreError> {
        if delta.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE turn_attempts
             SET partial_output = partial_output || ?2
             WHERE id = ?1",
            params![id.as_str(), delta],
        )?;
        if updated != 1 {
            return Err(StoreError::Data(format!(
                "turn attempt {id} does not exist"
            )));
        }
        Ok(())
    }

    pub fn save_tool_ledger_entry(&self, entry: &ToolLedgerEntry) -> Result<(), StoreError> {
        if entry.provider_item_id.trim().is_empty() || entry.name.trim().is_empty() {
            return Err(StoreError::Data(
                "tool ledger item id and name must not be empty".to_string(),
            ));
        }
        let input_json = serde_json::to_string(&entry.input)?;
        let status = serde_json::to_string(&entry.status)?;
        let updated_at_ms = checked_i64(entry.updated_at_ms, "tool ledger updated_at_ms")?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let existing = query_tool_entry(&tx, entry.attempt_id.as_str(), &entry.provider_item_id)?;
        if let Some(existing) = existing {
            let existing = decode_tool_entry(existing)?;
            if existing.turn_id != entry.turn_id
                || existing.name != entry.name
                || existing.input != entry.input
            {
                return Err(StoreError::Data(
                    "tool ledger identity cannot change after creation".to_string(),
                ));
            }
            if existing.status != ToolLedgerStatus::Started {
                if existing.status == entry.status
                    && existing.output == entry.output
                    && existing.error == entry.error
                {
                    return Ok(());
                }
                return Err(StoreError::Data(
                    "completed tool ledger entries are immutable".to_string(),
                ));
            }
        }
        tx.execute(
            "INSERT INTO turn_tool_ledger (
                attempt_id, turn_id, provider_item_id, name, input_json,
                status, output, error, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(attempt_id, provider_item_id) DO UPDATE SET
                status = excluded.status,
                output = excluded.output,
                error = excluded.error,
                updated_at_ms = excluded.updated_at_ms",
            params![
                entry.attempt_id.as_str(),
                entry.turn_id.as_str(),
                entry.provider_item_id,
                entry.name,
                input_json,
                status,
                entry.output,
                entry.error,
                updated_at_ms,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_tool_ledger(&self, turn_id: &TurnId) -> Result<Vec<ToolLedgerEntry>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT attempt_id, turn_id, provider_item_id, name, input_json,
                    status, output, error, updated_at_ms
             FROM turn_tool_ledger WHERE turn_id = ?1
             ORDER BY updated_at_ms ASC, attempt_id ASC, provider_item_id ASC",
        )?;
        let rows = stmt.query_map([turn_id.as_str()], read_raw_tool_entry)?;
        rows.map(|row| decode_tool_entry(row?)).collect()
    }

    pub fn save_continuation(
        &self,
        continuation: &ContinuationRecord,
    ) -> Result<ContinuationRecord, StoreError> {
        validate_continuation(continuation)?;
        let state = serde_json::to_string(&continuation.state)?;
        let created_at_ms = checked_i64(continuation.created_at_ms, "continuation created_at_ms")?;
        let updated_at_ms = checked_i64(continuation.updated_at_ms, "continuation updated_at_ms")?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let existing = query_continuation_by_key(&tx, &continuation.idempotency_key)?;
        if let Some(existing) = existing {
            let existing = decode_continuation(existing)?;
            if existing == *continuation {
                return Ok(existing);
            }
            return Err(StoreError::Data(
                "continuation idempotency key already represents another request".to_string(),
            ));
        }
        tx.execute(
            "INSERT INTO continuations (
                id, idempotency_key, thread_id, turn_id, source_attempt_id,
                target_provider_instance_id, target_turn_id, state, handoff, checkpoint,
                created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                continuation.id.as_str(),
                continuation.idempotency_key,
                continuation.thread_id.as_str(),
                continuation.turn_id.as_str(),
                continuation.source_attempt_id.as_str(),
                continuation
                    .target_provider_instance_id
                    .as_ref()
                    .map(ProviderInstanceId::as_str),
                continuation.target_turn_id.as_ref().map(TurnId::as_str),
                state,
                continuation.handoff,
                continuation.checkpoint.as_deref(),
                created_at_ms,
                updated_at_ms,
            ],
        )?;
        tx.commit()?;
        Ok(continuation.clone())
    }

    pub fn get_continuation(
        &self,
        id: &ContinuationId,
    ) -> Result<Option<ContinuationRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        query_continuation_by_id(&conn, id.as_str())?
            .map(decode_continuation)
            .transpose()
    }

    pub fn list_continuations(
        &self,
        thread_id: &ThreadId,
    ) -> Result<Vec<ContinuationRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, idempotency_key, thread_id, turn_id, source_attempt_id,
                    target_provider_instance_id, target_turn_id, state, handoff, checkpoint,
                    created_at_ms, updated_at_ms
             FROM continuations WHERE thread_id = ?1 ORDER BY created_at_ms ASC, id ASC",
        )?;
        let rows = stmt.query_map([thread_id.as_str()], read_raw_continuation)?;
        rows.map(|row| decode_continuation(row?)).collect()
    }

    pub fn claim_continuation(
        &self,
        id: &ContinuationId,
        target_provider_instance_id: &ProviderInstanceId,
        updated_at_ms: u64,
    ) -> Result<ContinuationRecord, StoreError> {
        let updated_at_ms = checked_i64(updated_at_ms, "continuation updated_at_ms")?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let raw = query_continuation_by_id(&tx, id.as_str())?
            .ok_or_else(|| StoreError::Data(format!("continuation {id} does not exist")))?;
        let mut continuation = decode_continuation(raw)?;
        if let Some(existing) = continuation.target_provider_instance_id.as_ref()
            && existing != target_provider_instance_id
        {
            return Err(StoreError::Data(
                "continuation was already claimed for another provider instance".to_string(),
            ));
        }
        if continuation.state == ContinuationState::Offered {
            continuation.state = ContinuationState::Claimed;
            continuation.target_provider_instance_id = Some(target_provider_instance_id.clone());
            continuation.updated_at_ms = checked_u64(updated_at_ms, "continuation updated_at_ms")?;
            tx.execute(
                "UPDATE continuations
                 SET target_provider_instance_id = ?2, state = ?3, updated_at_ms = ?4
                 WHERE id = ?1 AND state = ?5",
                params![
                    id.as_str(),
                    target_provider_instance_id.as_str(),
                    serde_json::to_string(&ContinuationState::Claimed)?,
                    updated_at_ms,
                    serde_json::to_string(&ContinuationState::Offered)?,
                ],
            )?;
        }
        tx.commit()?;
        Ok(continuation)
    }

    pub fn set_continuation_state(
        &self,
        id: &ContinuationId,
        state: ContinuationState,
        updated_at_ms: u64,
    ) -> Result<ContinuationRecord, StoreError> {
        let mut continuation = self
            .get_continuation(id)?
            .ok_or_else(|| StoreError::Data(format!("continuation {id} does not exist")))?;
        if matches!(
            continuation.state,
            ContinuationState::Completed | ContinuationState::Failed
        ) {
            if continuation.state == state {
                return Ok(continuation);
            }
            return Err(StoreError::Data(
                "completed continuation state is immutable".to_string(),
            ));
        }
        if continuation_state_rank(state) < continuation_state_rank(continuation.state) {
            return Err(StoreError::Data(
                "continuation state cannot move backward".to_string(),
            ));
        }
        continuation.state = state;
        continuation.updated_at_ms = updated_at_ms;
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE continuations SET state = ?2, updated_at_ms = ?3 WHERE id = ?1",
            params![
                id.as_str(),
                serde_json::to_string(&state)?,
                checked_i64(updated_at_ms, "continuation updated_at_ms")?,
            ],
        )?;
        if updated != 1 {
            return Err(StoreError::Data(format!(
                "continuation {id} does not exist"
            )));
        }
        Ok(continuation)
    }

    pub fn link_continuation_target_turn(
        &self,
        id: &ContinuationId,
        target_turn_id: &TurnId,
        updated_at_ms: u64,
    ) -> Result<ContinuationRecord, StoreError> {
        let updated_at_ms = checked_i64(updated_at_ms, "continuation updated_at_ms")?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let raw = query_continuation_by_id(&tx, id.as_str())?
            .ok_or_else(|| StoreError::Data(format!("continuation {id} does not exist")))?;
        let mut continuation = decode_continuation(raw)?;
        if let Some(existing) = continuation.target_turn_id.as_ref()
            && existing != target_turn_id
        {
            return Err(StoreError::Data(
                "continuation already targets another turn".to_string(),
            ));
        }
        if continuation.target_turn_id.as_ref() == Some(target_turn_id)
            && continuation.state == ContinuationState::Running
        {
            return Ok(continuation);
        }
        if continuation.state != ContinuationState::Claimed {
            return Err(StoreError::Data(
                "only a claimed continuation can start a target turn".to_string(),
            ));
        }
        tx.execute(
            "UPDATE continuations
             SET target_turn_id = ?2, state = ?3, updated_at_ms = ?4
             WHERE id = ?1",
            params![
                id.as_str(),
                target_turn_id.as_str(),
                serde_json::to_string(&ContinuationState::Running)?,
                updated_at_ms,
            ],
        )?;
        tx.commit()?;
        continuation.target_turn_id = Some(target_turn_id.clone());
        continuation.state = ContinuationState::Running;
        continuation.updated_at_ms = checked_u64(updated_at_ms, "continuation updated_at_ms")?;
        Ok(continuation)
    }

    pub fn continuation_for_target_turn(
        &self,
        target_turn_id: &TurnId,
    ) -> Result<Option<ContinuationRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, idempotency_key, thread_id, turn_id, source_attempt_id,
                    target_provider_instance_id, target_turn_id, state, handoff, checkpoint,
                    created_at_ms, updated_at_ms
             FROM continuations WHERE target_turn_id = ?1",
            [target_turn_id.as_str()],
            read_raw_continuation,
        )
        .optional()
        .map_err(StoreError::from)?
        .map(decode_continuation)
        .transpose()
    }

    pub fn prepare_continuation_delivery(
        &self,
        delivery: &ContinuationDelivery,
    ) -> Result<ContinuationDelivery, StoreError> {
        validate_delivery(delivery)?;
        let kind = serde_json::to_string(&delivery.kind)?;
        let state = serde_json::to_string(&delivery.state)?;
        let created_at_ms = checked_i64(delivery.created_at_ms, "delivery created_at_ms")?;
        let delivered_at_ms = delivery
            .delivered_at_ms
            .map(|value| checked_i64(value, "delivery delivered_at_ms"))
            .transpose()?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if let Some(existing) = query_delivery(&tx, &delivery.delivery_key)? {
            let existing = decode_delivery(existing)?;
            if existing.kind != delivery.kind
                || existing.item_id != delivery.item_id
                || existing.payload_sha256 != delivery.payload_sha256
                || existing.payload_json != delivery.payload_json
                || existing.turn_id != delivery.turn_id
                || existing.thread_id != delivery.thread_id
                || existing.continuation_id != delivery.continuation_id
            {
                return Err(StoreError::Data(
                    "delivery key already represents another payload".to_string(),
                ));
            }
            return Ok(existing);
        }
        tx.execute(
            "INSERT INTO continuation_deliveries (
                delivery_key, continuation_id, thread_id, turn_id, kind, item_id,
                payload_sha256, payload_json, state, created_at_ms, delivered_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                delivery.delivery_key,
                delivery
                    .continuation_id
                    .as_ref()
                    .map(ContinuationId::as_str),
                delivery.thread_id.as_str(),
                delivery.turn_id.as_str(),
                kind,
                delivery.item_id,
                delivery.payload_sha256,
                delivery.payload_json,
                state,
                created_at_ms,
                delivered_at_ms,
            ],
        )?;
        tx.commit()?;
        Ok(delivery.clone())
    }

    pub fn get_continuation_delivery(
        &self,
        delivery_key: &str,
    ) -> Result<Option<ContinuationDelivery>, StoreError> {
        let conn = self.conn.lock().unwrap();
        query_delivery(&conn, delivery_key)?
            .map(decode_delivery)
            .transpose()
    }

    pub fn latest_continuation_delivery(
        &self,
        thread_id: &ThreadId,
        kind: ContinuationDeliveryKind,
        item_id: &str,
        payload_sha256: &str,
        payload_json: &str,
    ) -> Result<Option<ContinuationDelivery>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let kind = serde_json::to_string(&kind)?;
        conn.query_row(
            "SELECT delivery_key, continuation_id, thread_id, turn_id, kind, item_id,
                    payload_sha256, payload_json, state, created_at_ms, delivered_at_ms
             FROM continuation_deliveries
             WHERE thread_id = ?1 AND kind = ?2 AND item_id = ?3
             ORDER BY CASE
                        WHEN payload_sha256 = ?4 AND payload_json = ?5 THEN 0
                        ELSE 1
                      END,
                      created_at_ms DESC, delivery_key DESC
             LIMIT 1",
            params![
                thread_id.as_str(),
                kind,
                item_id,
                payload_sha256,
                payload_json
            ],
            read_raw_delivery,
        )
        .optional()
        .map_err(StoreError::from)?
        .map(decode_delivery)
        .transpose()
    }

    pub fn mark_continuation_delivery_delivered(
        &self,
        delivery_key: &str,
        delivered_at_ms: u64,
    ) -> Result<ContinuationDelivery, StoreError> {
        let delivered_at_ms = checked_i64(delivered_at_ms, "delivery delivered_at_ms")?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let raw = query_delivery(&tx, delivery_key)?
            .ok_or_else(|| StoreError::Data("delivery does not exist".to_string()))?;
        let mut delivery = decode_delivery(raw)?;
        if delivery.state == ContinuationDeliveryState::Delivered {
            return Ok(delivery);
        }
        tx.execute(
            "UPDATE continuation_deliveries
             SET state = ?2, delivered_at_ms = ?3 WHERE delivery_key = ?1",
            params![
                delivery_key,
                serde_json::to_string(&ContinuationDeliveryState::Delivered)?,
                delivered_at_ms,
            ],
        )?;
        tx.commit()?;
        delivery.state = ContinuationDeliveryState::Delivered;
        delivery.delivered_at_ms = Some(checked_u64(delivered_at_ms, "delivery delivered_at_ms")?);
        Ok(delivery)
    }
}

fn validate_attempt(attempt: &TurnAttempt) -> Result<(), StoreError> {
    if attempt.ordinal == 0
        || attempt.model.trim().is_empty()
        || attempt.provider_instance_id.as_str().trim().is_empty()
        || attempt
            .subscription_key
            .as_ref()
            .is_some_and(|key| key.as_str().trim().is_empty())
    {
        return Err(StoreError::Data(
            "turn attempt ordinal, provider, subscription, and model must be valid".to_string(),
        ));
    }
    if attempt
        .ended_at_ms
        .is_some_and(|ended| ended < attempt.started_at_ms)
    {
        return Err(StoreError::Data(
            "turn attempt cannot end before it starts".to_string(),
        ));
    }
    Ok(())
}

fn valid_attempt_status_transition(
    previous: pandamux_core::TurnAttemptStatus,
    next: pandamux_core::TurnAttemptStatus,
) -> bool {
    use pandamux_core::TurnAttemptStatus;

    previous == next
        || matches!(
            (previous, next),
            (
                TurnAttemptStatus::Running,
                TurnAttemptStatus::Retrying
                    | TurnAttemptStatus::InterruptedForCapacity
                    | TurnAttemptStatus::Completed
                    | TurnAttemptStatus::Failed
            ) | (
                TurnAttemptStatus::Retrying,
                TurnAttemptStatus::InterruptedForCapacity | TurnAttemptStatus::Failed
            )
        )
}

fn validate_continuation(continuation: &ContinuationRecord) -> Result<(), StoreError> {
    if continuation.idempotency_key.trim().is_empty()
        || continuation.handoff.trim().is_empty()
        || continuation.handoff.len() > MAX_CONTINUATION_HANDOFF_BYTES
        || continuation
            .checkpoint
            .as_deref()
            .is_some_and(|checkpoint| checkpoint.trim().is_empty())
        || continuation.updated_at_ms < continuation.created_at_ms
    {
        return Err(StoreError::Data(
            "continuation requires an idempotency key, bounded handoff, and valid timestamps"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_delivery(delivery: &ContinuationDelivery) -> Result<(), StoreError> {
    if delivery.delivery_key.trim().is_empty()
        || delivery.item_id.trim().is_empty()
        || delivery.payload_sha256.len() != 64
        || delivery
            .delivered_at_ms
            .is_some_and(|value| value < delivery.created_at_ms)
    {
        return Err(StoreError::Data(
            "continuation delivery identity, digest, or timestamps are invalid".to_string(),
        ));
    }
    Ok(())
}

fn continuation_state_rank(state: ContinuationState) -> u8 {
    match state {
        ContinuationState::Offered => 0,
        ContinuationState::Claimed => 1,
        ContinuationState::Running => 2,
        ContinuationState::Completed | ContinuationState::Failed => 3,
    }
}

fn query_turn_attempt(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<Option<RawTurnAttempt>, StoreError> {
    conn.query_row(
        "SELECT id, turn_id, thread_id, ordinal, environment_id,
                provider_instance_id, subscription_key, model, boundary, status,
                routing_json, execution_json, checkpoint, partial_output,
                capacity_evidence_json, started_at_ms, ended_at_ms
         FROM turn_attempts WHERE id = ?1",
        [id],
        read_raw_turn_attempt,
    )
    .optional()
    .map_err(StoreError::from)
}

fn read_raw_turn_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawTurnAttempt> {
    Ok(RawTurnAttempt {
        id: row.get(0)?,
        turn_id: row.get(1)?,
        thread_id: row.get(2)?,
        ordinal: row.get(3)?,
        environment_id: row.get(4)?,
        provider_instance_id: row.get(5)?,
        subscription_key: row.get(6)?,
        model: row.get(7)?,
        boundary: row.get(8)?,
        status: row.get(9)?,
        routing_json: row.get(10)?,
        execution_json: row.get(11)?,
        checkpoint: row.get(12)?,
        partial_output: row.get(13)?,
        capacity_evidence_json: row.get(14)?,
        started_at_ms: row.get(15)?,
        ended_at_ms: row.get(16)?,
    })
}

fn decode_turn_attempt(raw: RawTurnAttempt) -> Result<TurnAttempt, StoreError> {
    Ok(TurnAttempt {
        id: TurnAttemptId::from(raw.id),
        turn_id: TurnId::from(raw.turn_id),
        thread_id: ThreadId::from(raw.thread_id),
        ordinal: u32::try_from(raw.ordinal)
            .map_err(|_| StoreError::Data("turn attempt ordinal is invalid".to_string()))?,
        environment_id: EnvironmentId::from(raw.environment_id),
        provider_instance_id: ProviderInstanceId::from(raw.provider_instance_id),
        subscription_key: raw.subscription_key.map(SubscriptionKey::from),
        model: raw.model,
        boundary: serde_json::from_str(&raw.boundary)?,
        status: serde_json::from_str(&raw.status)?,
        routing: serde_json::from_str::<RoutingDecision>(&raw.routing_json)?,
        execution: raw
            .execution_json
            .as_deref()
            .map(serde_json::from_str::<AttemptExecutionEvidence>)
            .transpose()?,
        checkpoint: raw.checkpoint,
        partial_output: raw.partial_output,
        capacity_evidence: raw
            .capacity_evidence_json
            .as_deref()
            .map(serde_json::from_str::<CapacityInterruptionEvidence>)
            .transpose()?,
        started_at_ms: checked_u64(raw.started_at_ms, "turn attempt started_at_ms")?,
        ended_at_ms: raw
            .ended_at_ms
            .map(|value| checked_u64(value, "turn attempt ended_at_ms"))
            .transpose()?,
    })
}

fn query_tool_entry(
    conn: &rusqlite::Connection,
    attempt_id: &str,
    provider_item_id: &str,
) -> Result<Option<RawToolLedgerEntry>, StoreError> {
    conn.query_row(
        "SELECT attempt_id, turn_id, provider_item_id, name, input_json,
                status, output, error, updated_at_ms
         FROM turn_tool_ledger WHERE attempt_id = ?1 AND provider_item_id = ?2",
        params![attempt_id, provider_item_id],
        read_raw_tool_entry,
    )
    .optional()
    .map_err(StoreError::from)
}

fn read_raw_tool_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawToolLedgerEntry> {
    Ok(RawToolLedgerEntry {
        attempt_id: row.get(0)?,
        turn_id: row.get(1)?,
        provider_item_id: row.get(2)?,
        name: row.get(3)?,
        input_json: row.get(4)?,
        status: row.get(5)?,
        output: row.get(6)?,
        error: row.get(7)?,
        updated_at_ms: row.get(8)?,
    })
}

fn decode_tool_entry(raw: RawToolLedgerEntry) -> Result<ToolLedgerEntry, StoreError> {
    Ok(ToolLedgerEntry {
        attempt_id: TurnAttemptId::from(raw.attempt_id),
        turn_id: TurnId::from(raw.turn_id),
        provider_item_id: raw.provider_item_id,
        name: raw.name,
        input: serde_json::from_str(&raw.input_json)?,
        status: serde_json::from_str(&raw.status)?,
        output: raw.output,
        error: raw.error,
        updated_at_ms: checked_u64(raw.updated_at_ms, "tool ledger updated_at_ms")?,
    })
}

fn query_continuation_by_id(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<Option<RawContinuation>, StoreError> {
    query_continuation(conn, "id", id)
}

fn query_continuation_by_key(
    conn: &rusqlite::Connection,
    key: &str,
) -> Result<Option<RawContinuation>, StoreError> {
    query_continuation(conn, "idempotency_key", key)
}

fn query_continuation(
    conn: &rusqlite::Connection,
    column: &str,
    value: &str,
) -> Result<Option<RawContinuation>, StoreError> {
    let sql = format!(
        "SELECT id, idempotency_key, thread_id, turn_id, source_attempt_id,
                target_provider_instance_id, target_turn_id, state, handoff, checkpoint,
                created_at_ms, updated_at_ms
         FROM continuations WHERE {column} = ?1"
    );
    conn.query_row(&sql, [value], read_raw_continuation)
        .optional()
        .map_err(StoreError::from)
}

fn read_raw_continuation(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawContinuation> {
    Ok(RawContinuation {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        thread_id: row.get(2)?,
        turn_id: row.get(3)?,
        source_attempt_id: row.get(4)?,
        target_provider_instance_id: row.get(5)?,
        target_turn_id: row.get(6)?,
        state: row.get(7)?,
        handoff: row.get(8)?,
        checkpoint: row.get(9)?,
        created_at_ms: row.get(10)?,
        updated_at_ms: row.get(11)?,
    })
}

fn decode_continuation(raw: RawContinuation) -> Result<ContinuationRecord, StoreError> {
    Ok(ContinuationRecord {
        id: ContinuationId::from(raw.id),
        idempotency_key: raw.idempotency_key,
        thread_id: ThreadId::from(raw.thread_id),
        turn_id: TurnId::from(raw.turn_id),
        source_attempt_id: TurnAttemptId::from(raw.source_attempt_id),
        target_provider_instance_id: raw
            .target_provider_instance_id
            .map(ProviderInstanceId::from),
        target_turn_id: raw.target_turn_id.map(TurnId::from),
        state: serde_json::from_str(&raw.state)?,
        handoff: raw.handoff,
        checkpoint: raw.checkpoint,
        created_at_ms: checked_u64(raw.created_at_ms, "continuation created_at_ms")?,
        updated_at_ms: checked_u64(raw.updated_at_ms, "continuation updated_at_ms")?,
    })
}

fn query_delivery(
    conn: &rusqlite::Connection,
    delivery_key: &str,
) -> Result<Option<RawDelivery>, StoreError> {
    conn.query_row(
        "SELECT delivery_key, continuation_id, thread_id, turn_id, kind, item_id,
                payload_sha256, payload_json, state, created_at_ms, delivered_at_ms
         FROM continuation_deliveries WHERE delivery_key = ?1",
        [delivery_key],
        read_raw_delivery,
    )
    .optional()
    .map_err(StoreError::from)
}

fn read_raw_delivery(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawDelivery> {
    Ok(RawDelivery {
        delivery_key: row.get(0)?,
        continuation_id: row.get(1)?,
        thread_id: row.get(2)?,
        turn_id: row.get(3)?,
        kind: row.get(4)?,
        item_id: row.get(5)?,
        payload_sha256: row.get(6)?,
        payload_json: row.get(7)?,
        state: row.get(8)?,
        created_at_ms: row.get(9)?,
        delivered_at_ms: row.get(10)?,
    })
}

fn decode_delivery(raw: RawDelivery) -> Result<ContinuationDelivery, StoreError> {
    Ok(ContinuationDelivery {
        delivery_key: raw.delivery_key,
        continuation_id: raw.continuation_id.map(ContinuationId::from),
        thread_id: ThreadId::from(raw.thread_id),
        turn_id: TurnId::from(raw.turn_id),
        kind: serde_json::from_str::<ContinuationDeliveryKind>(&raw.kind)?,
        item_id: raw.item_id,
        payload_sha256: raw.payload_sha256,
        payload_json: raw.payload_json,
        state: serde_json::from_str(&raw.state)?,
        created_at_ms: checked_u64(raw.created_at_ms, "delivery created_at_ms")?,
        delivered_at_ms: raw
            .delivered_at_ms
            .map(|value| checked_u64(value, "delivery delivered_at_ms"))
            .transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{
        AccessMode, ProviderCapabilities, ProviderKind, RoutingCapabilityRequirements,
        RoutingEvaluation, RoutingScore, Thread, ThreadOrigin, ThreadStatus, ThreadWorkspace, Turn,
        TurnAttemptStatus, TurnInput, TurnStatus,
    };

    fn seed_turn(store: &Store) -> (Thread, Turn) {
        let thread = Thread {
            id: ThreadId::new("thread-failover"),
            project_id: None,
            environment_id: EnvironmentId::new("env-local"),
            parent_thread_id: None,
            title: "Failover".to_string(),
            provider_instance_id: ProviderInstanceId::new("provider-a"),
            model: "model-a".to_string(),
            effort: None,
            access_mode: AccessMode::WorkspaceOnly,
            workspace: ThreadWorkspace {
                cwd: "/work".to_string(),
                worktree: None,
            },
            status: ThreadStatus::Working,
            agent: None,
            origin: ThreadOrigin::default(),
            created_at_ms: 100,
            updated_at_ms: 100,
        };
        store.save_thread(&thread).expect("save thread");
        let turn = Turn {
            id: TurnId::new("turn-failover"),
            thread_id: thread.id.clone(),
            environment_id: thread.environment_id.clone(),
            provider_instance_id: thread.provider_instance_id.clone(),
            seq: 1,
            input: TurnInput {
                text: "work".to_string(),
                attachment_ids: Vec::new(),
                attachments: Vec::new(),
                model: Some("model-a".to_string()),
                effort: None,
            },
            status: TurnStatus::Running,
            started_at_ms: 100,
            ended_at_ms: None,
            usage: None,
            checkpoint_before: Some("checkpoint-a".to_string()),
            checkpoint_after: None,
        };
        store.save_turn(&turn).expect("save turn");
        (thread, turn)
    }

    fn attempt(thread: &Thread, turn: &Turn) -> TurnAttempt {
        TurnAttempt {
            id: TurnAttemptId::new("attempt-one"),
            turn_id: turn.id.clone(),
            thread_id: thread.id.clone(),
            ordinal: 1,
            environment_id: thread.environment_id.clone(),
            provider_instance_id: thread.provider_instance_id.clone(),
            subscription_key: Some(SubscriptionKey::new("subkey-a")),
            model: "model-a".to_string(),
            boundary: AttemptBoundary::SafeToRetry,
            status: TurnAttemptStatus::Running,
            routing: RoutingDecision {
                selected_provider_instance_id: Some(thread.provider_instance_id.clone()),
                selected_subscription_key: Some(SubscriptionKey::new("subkey-a")),
                selected_model: Some("model-a".to_string()),
                evaluations: vec![RoutingEvaluation {
                    provider_instance_id: thread.provider_instance_id.clone(),
                    subscription_key: SubscriptionKey::new("subkey-a"),
                    model: "model-a".to_string(),
                    eligible: true,
                    rejection_codes: Vec::new(),
                    score: Some(RoutingScore {
                        total: 1,
                        ..RoutingScore::default()
                    }),
                    explanation: "eligible".to_string(),
                }],
            },
            execution: Some(AttemptExecutionEvidence {
                provider_kind: ProviderKind::Custom,
                profile_dir: "/profiles/provider-a".to_string(),
                binary_path: Some("/bin/provider-a".to_string()),
                capabilities: ProviderCapabilities::default(),
                provider_version: Some("fixture-1".to_string()),
            }),
            checkpoint: turn.checkpoint_before.clone(),
            partial_output: String::new(),
            capacity_evidence: None,
            started_at_ms: 100,
            ended_at_ms: None,
        }
    }

    #[test]
    fn attempt_boundary_output_and_tool_ledger_are_durable() {
        let store = Store::in_memory().expect("store");
        let (thread, turn) = seed_turn(&store);
        let value = attempt(&thread, &turn);
        store.save_turn_attempt(&value).expect("save attempt");
        store
            .advance_turn_attempt_boundary(&value.id, AttemptBoundary::VisibleOutputBegun)
            .expect("advance boundary");
        store
            .append_turn_attempt_output(&value.id, "partial")
            .expect("append output");
        let entry = ToolLedgerEntry {
            attempt_id: value.id.clone(),
            turn_id: turn.id.clone(),
            provider_item_id: "tool-one".to_string(),
            name: "write".to_string(),
            input: serde_json::json!({"path":"file.txt"}),
            status: ToolLedgerStatus::Started,
            output: None,
            error: None,
            updated_at_ms: 110,
        };
        store
            .save_tool_ledger_entry(&entry)
            .expect("save tool entry");
        let mut finished = entry.clone();
        finished.status = ToolLedgerStatus::Succeeded;
        finished.output = Some("done".to_string());
        finished.updated_at_ms = 120;
        store
            .save_tool_ledger_entry(&finished)
            .expect("finish tool entry");
        let mut duplicate = finished.clone();
        duplicate.updated_at_ms = 130;
        store
            .save_tool_ledger_entry(&duplicate)
            .expect("repeat finished tool entry");

        let restored = store
            .get_turn_attempt(&value.id)
            .expect("get attempt")
            .expect("attempt exists");
        assert_eq!(restored.boundary, AttemptBoundary::VisibleOutputBegun);
        assert_eq!(restored.partial_output, "partial");
        assert_eq!(
            store.list_tool_ledger(&turn.id).expect("ledger"),
            vec![finished]
        );
        assert!(
            store
                .advance_turn_attempt_boundary(&value.id, AttemptBoundary::SafeToRetry)
                .is_err()
        );
        let mut terminal = restored;
        terminal.status = TurnAttemptStatus::Completed;
        terminal.ended_at_ms = Some(130);
        store
            .save_turn_attempt(&terminal)
            .expect("complete attempt");
        terminal.status = TurnAttemptStatus::Running;
        assert!(store.save_turn_attempt(&terminal).is_err());
    }

    #[test]
    fn safe_retry_retargets_only_the_running_turn() {
        let store = Store::in_memory().expect("store");
        let (thread, turn) = seed_turn(&store);
        let value = attempt(&thread, &turn);
        store.save_turn_attempt(&value).expect("save attempt");

        store
            .retarget_running_turn_for_safe_retry(
                &turn.id,
                &thread.provider_instance_id,
                &ProviderInstanceId::new("provider-b"),
                "model-b",
            )
            .expect("retarget running turn");

        let updated = store
            .get_turn(&turn.id)
            .expect("get retargeted turn")
            .expect("turn exists");
        assert_eq!(updated.provider_instance_id.as_str(), "provider-b");
        assert_eq!(updated.input.model.as_deref(), Some("model-b"));
        assert_eq!(
            store
                .get_turn_attempt(&value.id)
                .expect("get original attempt")
                .expect("attempt exists"),
            value
        );
        assert!(
            store
                .retarget_running_turn_for_safe_retry(
                    &turn.id,
                    &thread.provider_instance_id,
                    &ProviderInstanceId::new("provider-c"),
                    "model-c",
                )
                .is_err()
        );
    }

    #[test]
    fn continuation_and_delivery_are_idempotent() {
        let store = Store::in_memory().expect("store");
        let (thread, turn) = seed_turn(&store);
        let value = attempt(&thread, &turn);
        store.save_turn_attempt(&value).expect("save attempt");
        let continuation = ContinuationRecord {
            id: ContinuationId::new("continuation-one"),
            idempotency_key: "continue-turn-failover".to_string(),
            thread_id: thread.id.clone(),
            turn_id: turn.id.clone(),
            source_attempt_id: value.id.clone(),
            target_provider_instance_id: None,
            target_turn_id: None,
            state: ContinuationState::Offered,
            handoff: "Continue from the saved checkpoint.".to_string(),
            checkpoint: Some("checkpoint-a".to_string()),
            created_at_ms: 120,
            updated_at_ms: 120,
        };
        let mut oversized = continuation.clone();
        oversized.id = ContinuationId::new("continuation-oversized");
        oversized.idempotency_key = "continue-turn-oversized".to_string();
        oversized.handoff = "x".repeat(MAX_CONTINUATION_HANDOFF_BYTES + 1);
        assert!(store.save_continuation(&oversized).is_err());
        assert_eq!(
            store
                .save_continuation(&continuation)
                .expect("save continuation"),
            continuation
        );
        assert_eq!(
            store
                .save_continuation(&continuation)
                .expect("repeat continuation"),
            continuation
        );
        let claimed = store
            .claim_continuation(
                &continuation.id,
                &ProviderInstanceId::new("provider-b"),
                130,
            )
            .expect("claim continuation");
        assert_eq!(claimed.state, ContinuationState::Claimed);
        assert_eq!(
            store
                .claim_continuation(
                    &continuation.id,
                    &ProviderInstanceId::new("provider-b"),
                    140,
                )
                .expect("repeat claim"),
            claimed
        );

        let delivery = ContinuationDelivery {
            delivery_key: "approval-turn-failover-request-one".to_string(),
            continuation_id: Some(continuation.id.clone()),
            thread_id: thread.id,
            turn_id: turn.id,
            kind: ContinuationDeliveryKind::Approval,
            item_id: "request-one".to_string(),
            payload_sha256: "a".repeat(64),
            payload_json: "{\"decision\":\"allow\"}".to_string(),
            state: ContinuationDeliveryState::Pending,
            created_at_ms: 140,
            delivered_at_ms: None,
        };
        assert_eq!(
            store
                .prepare_continuation_delivery(&delivery)
                .expect("prepare delivery"),
            delivery
        );
        let delivered = store
            .mark_continuation_delivery_delivered(&delivery.delivery_key, 150)
            .expect("mark delivery");
        assert_eq!(delivered.state, ContinuationDeliveryState::Delivered);
        assert_eq!(
            store
                .prepare_continuation_delivery(&delivery)
                .expect("repeat prepare")
                .state,
            ContinuationDeliveryState::Delivered
        );
    }

    #[test]
    fn delivery_rejects_same_key_with_different_payload() {
        let store = Store::in_memory().expect("store");
        let (thread, turn) = seed_turn(&store);
        let value = attempt(&thread, &turn);
        store.save_turn_attempt(&value).expect("save attempt");
        let first = ContinuationDelivery {
            delivery_key: "tool-result-one".to_string(),
            continuation_id: None,
            thread_id: thread.id,
            turn_id: turn.id,
            kind: ContinuationDeliveryKind::ToolResult,
            item_id: "tool-one".to_string(),
            payload_sha256: "b".repeat(64),
            payload_json: "{\"result\":1}".to_string(),
            state: ContinuationDeliveryState::Pending,
            created_at_ms: 160,
            delivered_at_ms: None,
        };
        store
            .prepare_continuation_delivery(&first)
            .expect("prepare first");
        let mut conflicting = first;
        conflicting.payload_sha256 = "c".repeat(64);
        assert!(store.prepare_continuation_delivery(&conflicting).is_err());
    }

    #[test]
    fn continuation_and_tool_result_delivery_remain_idempotent_after_database_restart() {
        let temp = tempfile::tempdir().expect("temporary store directory");
        let path = temp.path().join("pandamux.db");
        let (continuation, delivery) = {
            let store = Store::open(&path).expect("open first store");
            let (thread, turn) = seed_turn(&store);
            let value = attempt(&thread, &turn);
            store.save_turn_attempt(&value).expect("save attempt");
            let continuation = ContinuationRecord {
                id: ContinuationId::new("continuation-restart"),
                idempotency_key: "continue-after-restart".to_string(),
                thread_id: thread.id.clone(),
                turn_id: turn.id.clone(),
                source_attempt_id: value.id,
                target_provider_instance_id: None,
                target_turn_id: None,
                state: ContinuationState::Offered,
                handoff: "Continue from the durable checkpoint.".to_string(),
                checkpoint: Some("checkpoint-a".to_string()),
                created_at_ms: 200,
                updated_at_ms: 200,
            };
            store
                .save_continuation(&continuation)
                .expect("save restart continuation");
            let delivery = ContinuationDelivery {
                delivery_key: "tool-result-restart".to_string(),
                continuation_id: Some(continuation.id.clone()),
                thread_id: thread.id,
                turn_id: turn.id,
                kind: ContinuationDeliveryKind::ToolResult,
                item_id: "tool-restart".to_string(),
                payload_sha256: "d".repeat(64),
                payload_json: "{\"result\":\"durable\"}".to_string(),
                state: ContinuationDeliveryState::Pending,
                created_at_ms: 210,
                delivered_at_ms: None,
            };
            store
                .prepare_continuation_delivery(&delivery)
                .expect("prepare restart delivery");
            store
                .mark_continuation_delivery_delivered(&delivery.delivery_key, 220)
                .expect("deliver before restart");
            (continuation, delivery)
        };

        let reopened = Store::open(&path).expect("reopen store");
        assert_eq!(
            reopened
                .get_continuation(&continuation.id)
                .expect("load continuation after restart"),
            Some(continuation)
        );
        let repeated = reopened
            .prepare_continuation_delivery(&delivery)
            .expect("repeat tool result after restart");
        assert_eq!(repeated.state, ContinuationDeliveryState::Delivered);
        assert_eq!(repeated.delivered_at_ms, Some(220));
        let mut conflicting = delivery;
        conflicting.payload_sha256 = "e".repeat(64);
        assert!(
            reopened
                .prepare_continuation_delivery(&conflicting)
                .is_err()
        );
    }

    #[test]
    fn restart_reconciliation_closes_attempts_and_linked_continuations() {
        let store = Store::in_memory().expect("store");
        let (thread, turn) = seed_turn(&store);
        let value = attempt(&thread, &turn);
        store.save_turn_attempt(&value).expect("save attempt");
        let continuation = ContinuationRecord {
            id: ContinuationId::new("continuation-active-at-restart"),
            idempotency_key: "continue-active-at-restart".to_string(),
            thread_id: thread.id,
            turn_id: turn.id.clone(),
            source_attempt_id: value.id.clone(),
            target_provider_instance_id: None,
            target_turn_id: None,
            state: ContinuationState::Offered,
            handoff: "Continue from the durable checkpoint.".to_string(),
            checkpoint: Some("checkpoint-a".to_string()),
            created_at_ms: 120,
            updated_at_ms: 120,
        };
        store
            .save_continuation(&continuation)
            .expect("save continuation");
        store
            .claim_continuation(
                &continuation.id,
                &ProviderInstanceId::new("provider-b"),
                130,
            )
            .expect("claim continuation");
        store
            .link_continuation_target_turn(&continuation.id, &turn.id, 140)
            .expect("link continuation target");

        assert_eq!(store.reconcile_active_turns().expect("reconcile"), 1);
        let reconciled_attempt = store
            .get_turn_attempt(&value.id)
            .expect("get attempt")
            .expect("attempt exists");
        assert_eq!(reconciled_attempt.status, TurnAttemptStatus::Failed);
        assert!(reconciled_attempt.ended_at_ms.is_some());
        assert_eq!(
            store
                .get_continuation(&continuation.id)
                .expect("get continuation")
                .expect("continuation exists")
                .state,
            ContinuationState::Failed
        );
    }

    #[test]
    fn requirements_type_remains_serializable_for_store_fixtures() {
        serde_json::to_string(&RoutingCapabilityRequirements::default())
            .expect("serialize requirements");
        assert_eq!(ProviderKind::Custom.as_str(), "custom");
    }
}
