use pandamux_core::TokenCounts;
pub use pandamux_core::UsageUnknownField as UnknownUsageField;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const CLAUDE_TRANSCRIPT_PARSER_VERSION: u32 = 2;
pub const CODEX_TRANSCRIPT_PARSER_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptProvider {
    Claude,
    Codex,
}

impl TranscriptProvider {
    pub fn parser_version(self) -> u32 {
        match self {
            Self::Claude => CLAUDE_TRANSCRIPT_PARSER_VERSION,
            Self::Codex => CODEX_TRANSCRIPT_PARSER_VERSION,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedTranscriptUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_native_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_native_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub tokens: TokenCounts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reported_micro_usd: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub unknown_fields: BTreeSet<UnknownUsageField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptRecordError {
    pub code: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptRecordOutcome {
    Ignored,
    Usage(ParsedTranscriptUsage),
    Malformed(TranscriptRecordError),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptParser {
    provider: TranscriptProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codex_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codex_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codex_latest_usage: Option<ParsedTranscriptUsage>,
}

impl TranscriptParser {
    pub fn new(provider: TranscriptProvider) -> Self {
        Self {
            provider,
            session_id: None,
            codex_turn_id: None,
            codex_model: None,
            codex_latest_usage: None,
        }
    }

    pub fn provider(&self) -> TranscriptProvider {
        self.provider
    }

    pub fn parser_version(&self) -> u32 {
        self.provider.parser_version()
    }

    pub fn parse_complete_record(&mut self, bytes: &[u8]) -> TranscriptRecordOutcome {
        let value = match serde_json::from_slice::<Value>(bytes) {
            Ok(value) => value,
            Err(_) => {
                return TranscriptRecordOutcome::Malformed(TranscriptRecordError {
                    code: "invalid_json",
                });
            }
        };
        match self.provider {
            TranscriptProvider::Claude => self.parse_claude(&value),
            TranscriptProvider::Codex => self.parse_codex(&value),
        }
    }

    fn parse_claude(&mut self, value: &Value) -> TranscriptRecordOutcome {
        if let Some(session_id) =
            first_nonempty_string(value, &["/sessionId", "/session_id", "/message/session_id"])
        {
            self.session_id = Some(session_id);
        }

        let usage = value
            .pointer("/usage")
            .or_else(|| value.pointer("/message/usage"));
        let Some(usage) = usage else {
            return TranscriptRecordOutcome::Ignored;
        };
        let tokens = match parse_token_counts(usage, TranscriptProvider::Claude) {
            Ok(tokens) => tokens,
            Err(error) => return TranscriptRecordOutcome::Malformed(error),
        };
        let provider_reported_micro_usd = match value
            .pointer("/total_cost_usd")
            .or_else(|| value.pointer("/cost_usd"))
            .or_else(|| usage.get("cost_usd"))
        {
            Some(cost) => match parse_provider_reported_micro_usd(cost) {
                Some(cost) => Some(cost),
                None => {
                    return TranscriptRecordOutcome::Malformed(TranscriptRecordError {
                        code: "invalid_cost",
                    });
                }
            },
            None => None,
        };
        if tokens.counts.is_zero() && provider_reported_micro_usd.is_none() {
            return TranscriptRecordOutcome::Ignored;
        }

        let occurred_at_ms =
            first_timestamp_ms(value, &["/timestamp", "/created_at", "/message/created_at"]);
        let model = first_nonempty_string(value, &["/model", "/message/model"]);
        let provider_native_turn_id =
            first_nonempty_string(value, &["/message/id", "/turnId", "/turn_id", "/uuid"]);
        let mut unknown_fields = tokens.unknown_fields;
        record_unknowns(
            &mut unknown_fields,
            occurred_at_ms,
            model.as_deref(),
            provider_native_turn_id.as_deref(),
        );

        TranscriptRecordOutcome::Usage(ParsedTranscriptUsage {
            provider_native_session_id: self.session_id.clone(),
            provider_native_turn_id,
            occurred_at_ms,
            model,
            tokens: tokens.counts,
            provider_reported_micro_usd,
            unknown_fields,
        })
    }

    fn parse_codex(&mut self, value: &Value) -> TranscriptRecordOutcome {
        let record_type = value.get("type").and_then(Value::as_str);
        let payload = value.get("payload").unwrap_or(value);
        let payload_type = payload.get("type").and_then(Value::as_str);

        if record_type == Some("session_meta") {
            self.session_id = first_nonempty_string(payload, &["/id", "/session_id"]);
            self.codex_turn_id = None;
            self.codex_model = None;
            self.codex_latest_usage = None;
            return TranscriptRecordOutcome::Ignored;
        }

        if record_type == Some("turn_context") {
            self.codex_turn_id = first_nonempty_string(payload, &["/turn_id", "/turnId", "/id"]);
            self.codex_model = first_nonempty_string(payload, &["/model"]);
            self.codex_latest_usage = None;
            return TranscriptRecordOutcome::Ignored;
        }

        let is_token_record = matches!(
            payload_type,
            Some("token_count" | "token_usage" | "token_usage_updated")
        ) || value.get("tokenUsage").is_some();
        if is_token_record {
            let usage = payload
                .pointer("/info/last_token_usage")
                .or_else(|| payload.pointer("/info/lastTokenUsage"))
                .or_else(|| payload.pointer("/tokenUsage/last"))
                .or_else(|| payload.get("usage"))
                .or_else(|| value.pointer("/tokenUsage/last"));
            let Some(usage) = usage else {
                return TranscriptRecordOutcome::Malformed(TranscriptRecordError {
                    code: "missing_usage_payload",
                });
            };
            let tokens = match parse_token_counts(usage, TranscriptProvider::Codex) {
                Ok(tokens) => tokens,
                Err(error) => return TranscriptRecordOutcome::Malformed(error),
            };
            if tokens.counts.is_zero() {
                return TranscriptRecordOutcome::Ignored;
            }
            let occurred_at_ms = first_timestamp_ms(value, &["/timestamp", "/time"]);
            let model =
                first_nonempty_string(payload, &["/model"]).or_else(|| self.codex_model.clone());
            let provider_native_turn_id = first_nonempty_string(payload, &["/turn_id", "/turnId"])
                .or_else(|| self.codex_turn_id.clone());
            let mut unknown_fields = tokens.unknown_fields;
            record_unknowns(
                &mut unknown_fields,
                occurred_at_ms,
                model.as_deref(),
                provider_native_turn_id.as_deref(),
            );
            self.codex_latest_usage = Some(ParsedTranscriptUsage {
                provider_native_session_id: self.session_id.clone(),
                provider_native_turn_id,
                occurred_at_ms,
                model,
                tokens: tokens.counts,
                provider_reported_micro_usd: None,
                unknown_fields,
            });
            return TranscriptRecordOutcome::Ignored;
        }

        let is_completion = matches!(
            payload_type,
            Some("task_complete" | "turn_complete" | "turn_completed")
        );
        if !is_completion {
            return TranscriptRecordOutcome::Ignored;
        }
        let Some(mut usage) = self.codex_latest_usage.take() else {
            self.codex_turn_id = None;
            return TranscriptRecordOutcome::Ignored;
        };
        if let Some(turn_id) = first_nonempty_string(payload, &["/turn_id", "/turnId", "/id"]) {
            usage.provider_native_turn_id = Some(turn_id);
            usage.unknown_fields.remove(&UnknownUsageField::Identity);
        }
        if let Some(completed_at_ms) = first_timestamp_ms(value, &["/timestamp", "/time"]) {
            usage.occurred_at_ms = Some(completed_at_ms);
            usage.unknown_fields.remove(&UnknownUsageField::OccurredAt);
        }
        if let Some(model) = first_nonempty_string(payload, &["/model"]) {
            usage.model = Some(model);
            usage.unknown_fields.remove(&UnknownUsageField::Model);
        }
        self.codex_turn_id = None;
        TranscriptRecordOutcome::Usage(usage)
    }
}

struct ParsedTokenCounts {
    counts: TokenCounts,
    unknown_fields: BTreeSet<UnknownUsageField>,
}

fn parse_token_counts(
    value: &Value,
    provider: TranscriptProvider,
) -> Result<ParsedTokenCounts, TranscriptRecordError> {
    let input_keys: &[&str] = match provider {
        TranscriptProvider::Claude => &["input_tokens"],
        TranscriptProvider::Codex => &["input_tokens", "inputTokens"],
    };
    let output_keys: &[&str] = match provider {
        TranscriptProvider::Claude => &["output_tokens"],
        TranscriptProvider::Codex => &["output_tokens", "outputTokens"],
    };
    let input = parse_optional_u64(value, input_keys)?;
    let output = parse_optional_u64(value, output_keys)?;
    let cached_input = parse_optional_u64(
        value,
        &[
            "cache_read_input_tokens",
            "cached_input_tokens",
            "cachedInputTokens",
        ],
    )?;
    let cache_write = parse_optional_u64(
        value,
        &[
            "cache_creation_input_tokens",
            "cache_write_input_tokens",
            "cacheWriteInputTokens",
        ],
    )?;
    let reasoning =
        parse_optional_u64(value, &["reasoning_tokens", "reasoningTokens"])?.unwrap_or(0);

    let mut unknown_fields = BTreeSet::new();
    if input.is_none() {
        unknown_fields.insert(UnknownUsageField::InputTokens);
    }
    if output.is_none() {
        unknown_fields.insert(UnknownUsageField::OutputTokens);
    }
    if provider == TranscriptProvider::Claude && cached_input.is_none() {
        unknown_fields.insert(UnknownUsageField::CacheReadTokens);
    }
    if provider == TranscriptProvider::Claude && cache_write.is_none() {
        unknown_fields.insert(UnknownUsageField::CacheWriteTokens);
    }
    Ok(ParsedTokenCounts {
        counts: TokenCounts {
            input: input.unwrap_or(0),
            cached_input: cached_input.unwrap_or(0),
            cache_write: cache_write.unwrap_or(0),
            output: output.unwrap_or(0),
            reasoning,
            ..TokenCounts::default()
        },
        unknown_fields,
    })
}

fn parse_optional_u64(value: &Value, keys: &[&str]) -> Result<Option<u64>, TranscriptRecordError> {
    for key in keys {
        if let Some(candidate) = value.get(*key) {
            return candidate.as_u64().map(Some).ok_or(TranscriptRecordError {
                code: "invalid_token_count",
            });
        }
    }
    Ok(None)
}

fn record_unknowns(
    unknown_fields: &mut BTreeSet<UnknownUsageField>,
    occurred_at_ms: Option<u64>,
    model: Option<&str>,
    turn_id: Option<&str>,
) {
    if occurred_at_ms.is_none() {
        unknown_fields.insert(UnknownUsageField::OccurredAt);
    }
    if model.is_none() {
        unknown_fields.insert(UnknownUsageField::Model);
    }
    if turn_id.is_none() {
        unknown_fields.insert(UnknownUsageField::Identity);
    }
}

fn first_nonempty_string(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn first_timestamp_ms(value: &Value, pointers: &[&str]) -> Option<u64> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(parse_provider_timestamp_ms))
}

pub fn parse_provider_timestamp_ms(value: &Value) -> Option<u64> {
    if let Some(value) = value.as_u64() {
        return normalize_epoch(value);
    }
    let value = value.as_str()?.trim();
    if let Ok(value) = value.parse::<u64>() {
        return normalize_epoch(value);
    }
    parse_rfc3339_ms(value)
}

fn normalize_epoch(value: u64) -> Option<u64> {
    if value < 100_000_000_000 {
        value.checked_mul(1_000)
    } else {
        Some(value)
    }
}

pub fn parse_provider_reported_micro_usd(value: &Value) -> Option<u64> {
    let text = match value {
        Value::Number(number) => number.to_string(),
        Value::String(value) => value.trim().to_string(),
        _ => return None,
    };
    decimal_to_scaled_u64(&text, 6)
}

fn decimal_to_scaled_u64(value: &str, scale: usize) -> Option<u64> {
    if value.is_empty() || value.starts_with('-') || value.contains(['e', 'E']) {
        return None;
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let whole: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let scale_factor = 10_u64.checked_pow(u32::try_from(scale).ok()?)?;
    let mut result = whole.checked_mul(scale_factor)?;
    let mut digits = fraction.bytes();
    for index in 0..scale {
        let digit = digits.next().unwrap_or(b'0');
        if !digit.is_ascii_digit() {
            return None;
        }
        let place = 10_u64.checked_pow(u32::try_from(scale - index - 1).ok()?)?;
        result = result.checked_add(u64::from(digit - b'0').checked_mul(place)?)?;
    }
    if let Some(rounding_digit) = digits.next() {
        if !rounding_digit.is_ascii_digit() || !digits.all(|digit| digit.is_ascii_digit()) {
            return None;
        }
        if rounding_digit >= b'5' {
            result = result.checked_add(1)?;
        }
    }
    Some(result)
}

fn parse_rfc3339_ms(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !matches!(bytes.get(10), Some(b'T' | b't' | b' '))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let year = parse_digits(bytes.get(0..4)?)?;
    let month = parse_digits(bytes.get(5..7)?)?;
    let day = parse_digits(bytes.get(8..10)?)?;
    let hour = parse_digits(bytes.get(11..13)?)?;
    let minute = parse_digits(bytes.get(14..16)?)?;
    let second = parse_digits(bytes.get(17..19)?)?;
    if year < 1970 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let month_days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return None,
    };
    if !(1..=month_days).contains(&day) {
        return None;
    }

    let mut cursor = 19;
    let mut millis = 0_u64;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == start {
            return None;
        }
        let fraction = &bytes[start..cursor];
        for index in 0..3 {
            let digit = fraction.get(index).copied().unwrap_or(b'0');
            millis = millis
                .checked_mul(10)?
                .checked_add(u64::from(digit - b'0'))?;
        }
        if fraction.get(3).is_some_and(|digit| *digit >= b'5') {
            millis = millis.checked_add(1)?;
        }
    }

    let offset_seconds = match bytes.get(cursor) {
        Some(b'Z' | b'z') if cursor + 1 == bytes.len() => 0_i64,
        Some(sign @ (b'+' | b'-')) if cursor + 6 == bytes.len() => {
            if bytes.get(cursor + 3) != Some(&b':') {
                return None;
            }
            let offset_hour = parse_digits(bytes.get(cursor + 1..cursor + 3)?)?;
            let offset_minute = parse_digits(bytes.get(cursor + 4..cursor + 6)?)?;
            if offset_hour > 23 || offset_minute > 59 {
                return None;
            }
            let seconds = i64::from(offset_hour)
                .checked_mul(3_600)?
                .checked_add(i64::from(offset_minute).checked_mul(60)?)?;
            if *sign == b'-' { -seconds } else { seconds }
        }
        _ => return None,
    };

    let days = days_from_civil(i64::from(year), i64::from(month), i64::from(day))?;
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour).checked_mul(3_600)?)?
        .checked_add(i64::from(minute).checked_mul(60)?)?
        .checked_add(i64::from(second))?
        .checked_sub(offset_seconds)?;
    if seconds < 0 {
        return None;
    }
    u64::try_from(seconds)
        .ok()?
        .checked_mul(1_000)?
        .checked_add(millis)
}

fn parse_digits(bytes: &[u8]) -> Option<u32> {
    bytes.iter().try_fold(0_u32, |value, digit| {
        if digit.is_ascii_digit() {
            value.checked_mul(10)?.checked_add(u32::from(*digit - b'0'))
        } else {
            None
        }
    })
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_fixture_preserves_provenance_without_fallbacks() {
        let mut parser = TranscriptParser::new(TranscriptProvider::Claude);
        let outcome = parser.parse_complete_record(
            br#"{"type":"assistant","sessionId":"session-1","uuid":"row-1","timestamp":"2026-09-29T14:15:16.789Z","message":{"id":"msg-1","model":"claude-sonnet","usage":{"input_tokens":11,"output_tokens":7,"cache_read_input_tokens":3,"cache_creation_input_tokens":2}},"total_cost_usd":"0.0123456"}"#,
        );
        let TranscriptRecordOutcome::Usage(usage) = outcome else {
            panic!("expected parsed usage");
        };
        assert_eq!(
            usage.provider_native_session_id.as_deref(),
            Some("session-1")
        );
        assert_eq!(usage.provider_native_turn_id.as_deref(), Some("msg-1"));
        assert_eq!(usage.occurred_at_ms, Some(1_790_691_316_789));
        assert_eq!(usage.model.as_deref(), Some("claude-sonnet"));
        assert_eq!(usage.tokens.input, 11);
        assert_eq!(usage.tokens.output, 7);
        assert_eq!(usage.tokens.cached_input, 3);
        assert_eq!(usage.tokens.cache_write, 2);
        assert_eq!(usage.provider_reported_micro_usd, Some(12_346));
        assert!(usage.unknown_fields.is_empty());
    }

    #[test]
    fn claude_unknown_fields_remain_unknown() {
        let mut parser = TranscriptParser::new(TranscriptProvider::Claude);
        let outcome = parser.parse_complete_record(
            br#"{"type":"assistant","message":{"usage":{"input_tokens":4,"output_tokens":2}}}"#,
        );
        let TranscriptRecordOutcome::Usage(usage) = outcome else {
            panic!("expected parsed usage");
        };
        assert_eq!(usage.occurred_at_ms, None);
        assert_eq!(usage.model, None);
        assert_eq!(usage.provider_native_turn_id, None);
        assert_eq!(
            usage.unknown_fields,
            BTreeSet::from([
                UnknownUsageField::OccurredAt,
                UnknownUsageField::Model,
                UnknownUsageField::Identity,
                UnknownUsageField::CacheReadTokens,
                UnknownUsageField::CacheWriteTokens,
            ])
        );
    }

    #[test]
    fn claude_message_identity_wins_over_result_and_row_identifiers() {
        let mut parser = TranscriptParser::new(TranscriptProvider::Claude);
        assert_eq!(parser.parser_version(), 2);
        let outcome = parser.parse_complete_record(
            br#"{"type":"assistant","turnId":"result-turn","uuid":"row-uuid","message":{"id":"assistant-message","model":"claude-actual","usage":{"input_tokens":4,"output_tokens":2}}}"#,
        );
        let TranscriptRecordOutcome::Usage(usage) = outcome else {
            panic!("expected parsed usage");
        };
        assert_eq!(
            usage.provider_native_turn_id.as_deref(),
            Some("assistant-message")
        );
    }

    #[test]
    fn codex_fixture_emits_only_at_completion_and_round_trips_state() {
        let mut parser = TranscriptParser::new(TranscriptProvider::Codex);
        assert_eq!(
            parser.parse_complete_record(
                br#"{"timestamp":"2026-09-29T10:00:00Z","type":"session_meta","payload":{"id":"codex-session"}}"#
            ),
            TranscriptRecordOutcome::Ignored
        );
        assert_eq!(
            parser.parse_complete_record(
                br#"{"timestamp":"2026-09-29T10:00:01Z","type":"turn_context","payload":{"turn_id":"turn-7","model":"gpt-5"}}"#
            ),
            TranscriptRecordOutcome::Ignored
        );
        assert_eq!(
            parser.parse_complete_record(
                br#"{"timestamp":"2026-09-29T10:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":21,"output_tokens":8,"cached_input_tokens":5,"reasoning_tokens":3}}}}"#
            ),
            TranscriptRecordOutcome::Ignored
        );
        let encoded = serde_json::to_string(&parser).expect("serialize parser state");
        let mut parser: TranscriptParser =
            serde_json::from_str(&encoded).expect("restore parser state");
        let outcome = parser.parse_complete_record(
            br#"{"timestamp":"2026-09-29T10:00:03Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-7"}}"#,
        );
        let TranscriptRecordOutcome::Usage(usage) = outcome else {
            panic!("expected completed usage");
        };
        assert_eq!(
            usage.provider_native_session_id.as_deref(),
            Some("codex-session")
        );
        assert_eq!(usage.provider_native_turn_id.as_deref(), Some("turn-7"));
        assert_eq!(usage.model.as_deref(), Some("gpt-5"));
        assert_eq!(usage.tokens.input, 21);
        assert_eq!(usage.tokens.output, 8);
        assert_eq!(usage.tokens.cached_input, 5);
        assert_eq!(usage.tokens.reasoning, 3);
        assert_eq!(usage.occurred_at_ms, Some(1_790_676_003_000));
    }

    #[test]
    fn malformed_records_have_bounded_safe_codes() {
        let mut parser = TranscriptParser::new(TranscriptProvider::Claude);
        assert_eq!(
            parser.parse_complete_record(br#"{"usage": "#),
            TranscriptRecordOutcome::Malformed(TranscriptRecordError {
                code: "invalid_json"
            })
        );
        assert_eq!(
            parser.parse_complete_record(br#"{"usage":{"input_tokens":-1,"output_tokens":2}}"#),
            TranscriptRecordOutcome::Malformed(TranscriptRecordError {
                code: "invalid_token_count"
            })
        );
    }

    #[test]
    fn codex_new_turn_discards_uncompleted_usage_state() {
        let mut parser = TranscriptParser::new(TranscriptProvider::Codex);
        assert_eq!(
            parser.parse_complete_record(
                br#"{"type":"turn_context","payload":{"turn_id":"turn-old","model":"gpt-5"}}"#
            ),
            TranscriptRecordOutcome::Ignored
        );
        assert_eq!(
            parser.parse_complete_record(
                br#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":12,"output_tokens":3}}}}"#
            ),
            TranscriptRecordOutcome::Ignored
        );
        assert_eq!(
            parser.parse_complete_record(
                br#"{"type":"turn_context","payload":{"turn_id":"turn-new","model":"gpt-5"}}"#
            ),
            TranscriptRecordOutcome::Ignored
        );
        assert_eq!(
            parser.parse_complete_record(
                br#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-new"}}"#
            ),
            TranscriptRecordOutcome::Ignored
        );
    }

    #[test]
    fn rfc3339_parser_validates_dates_and_offsets() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_ms("1970-01-01T01:00:00+01:00"), Some(0));
        assert!(parse_rfc3339_ms("2025-02-29T00:00:00Z").is_none());
        assert!(parse_rfc3339_ms("2024-02-29T00:00:00Z").is_some());
    }
}
