use crate::dag::{
    HARD_MAX_PLAN_EDGES, HARD_MAX_PLAN_TASKS, HARD_MAX_TASK_ATTEMPTS, PROPOSED_PLAN_SCHEMA_VERSION,
    ProposedPlanV1,
};
use crate::diagnostics::SafeText;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_MODEL_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_MODEL_JSON_DEPTH: usize = 32;
const MAX_MODEL_CONTAINER_ITEMS: usize = 256;
const MAX_MODEL_SCALAR_BYTES: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelParseErrorCode {
    ModelOutputTooLarge,
    ModelInvalidJson,
    ModelNotObject,
    ModelMultipleObjects,
    ModelSchemaVersionMissing,
    ModelSchemaVersionInvalid,
    ModelSchemaVersionUnsupported,
    ModelUnknownField,
    ModelBoundsExceeded,
    ModelParseBudgetExhausted,
    ModelTransportFailure,
    ModelTimeout,
}

impl ModelParseErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ModelOutputTooLarge => "model_output_too_large",
            Self::ModelInvalidJson => "model_invalid_json",
            Self::ModelNotObject => "model_not_object",
            Self::ModelMultipleObjects => "model_multiple_objects",
            Self::ModelSchemaVersionMissing => "model_schema_version_missing",
            Self::ModelSchemaVersionInvalid => "model_schema_version_invalid",
            Self::ModelSchemaVersionUnsupported => "model_schema_version_unsupported",
            Self::ModelUnknownField => "model_unknown_field",
            Self::ModelBoundsExceeded => "model_bounds_exceeded",
            Self::ModelParseBudgetExhausted => "model_parse_budget_exhausted",
            Self::ModelTransportFailure => "model_transport_failure",
            Self::ModelTimeout => "model_timeout",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelParseError {
    pub code: ModelParseErrorCode,
    pub explanation: SafeText,
    pub corrective_action: SafeText,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelParseState {
    parse_failures: u8,
}

impl ModelParseState {
    pub const fn new() -> Self {
        Self { parse_failures: 0 }
    }

    pub const fn parse_failures(self) -> u8 {
        self.parse_failures
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelOutputAttempt<'a> {
    Content(&'a str),
    Timeout,
    TransportFailure,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelParseOutcome {
    Accepted(ProposedPlanV1),
    RetryAllowed {
        state: ModelParseState,
        failure: ModelParseError,
    },
    Rejected(ModelParseError),
}

pub fn parse_model_attempt(
    state: ModelParseState,
    attempt: ModelOutputAttempt<'_>,
) -> ModelParseOutcome {
    let raw = match attempt {
        ModelOutputAttempt::Content(raw) => raw,
        ModelOutputAttempt::Timeout => {
            return ModelParseOutcome::Rejected(model_error(
                ModelParseErrorCode::ModelTimeout,
                "The model request timed out before a proposal was available",
                "Retry through explicit operator or policy control; parse retry does not cover timeouts",
            ));
        }
        ModelOutputAttempt::TransportFailure => {
            return ModelParseOutcome::Rejected(model_error(
                ModelParseErrorCode::ModelTransportFailure,
                "The model transport failed before a proposal was available",
                "Restore the transport and retry through explicit operator or policy control",
            ));
        }
    };

    match parse_content(raw) {
        Ok(proposal) => ModelParseOutcome::Accepted(proposal),
        Err(failure) if state.parse_failures == 0 => ModelParseOutcome::RetryAllowed {
            state: ModelParseState { parse_failures: 1 },
            failure,
        },
        Err(_) => ModelParseOutcome::Rejected(model_error(
            ModelParseErrorCode::ModelParseBudgetExhausted,
            "The model proposal remained invalid after one parse-only retry",
            "Require a corrected proposal or explicit approval; do not dispatch this output",
        )),
    }
}

fn parse_content(raw: &str) -> Result<ProposedPlanV1, ModelParseError> {
    if raw.len() > MAX_MODEL_OUTPUT_BYTES {
        return Err(model_error(
            ModelParseErrorCode::ModelOutputTooLarge,
            "The model output exceeds the 1 MiB parse limit",
            "Return one smaller JSON proposal object",
        ));
    }
    let trimmed = raw.trim();
    let (candidate, value) = match serde_json::from_str::<Value>(trimmed) {
        Ok(value) => (trimmed, value),
        Err(_) => {
            let extracted = extract_one_balanced_object(trimmed)?;
            let value = serde_json::from_str::<Value>(extracted).map_err(|_| {
                model_error(
                    ModelParseErrorCode::ModelInvalidJson,
                    "The balanced model proposal is not valid JSON",
                    "Return one syntactically valid JSON object",
                )
            })?;
            (extracted, value)
        }
    };
    let Some(object) = value.as_object() else {
        return Err(model_error(
            ModelParseErrorCode::ModelNotObject,
            "The model proposal must be one JSON object",
            "Return an object rather than an array, scalar, or null",
        ));
    };
    match object.get("schemaVersion") {
        None => {
            return Err(model_error(
                ModelParseErrorCode::ModelSchemaVersionMissing,
                "The model proposal is missing schemaVersion",
                "Set schemaVersion to integer 1",
            ));
        }
        Some(Value::Number(number))
            if number.as_u64() == Some(u64::from(PROPOSED_PLAN_SCHEMA_VERSION)) => {}
        Some(Value::Number(number)) if number.as_u64().is_some_and(|version| version > 1) => {
            return Err(model_error(
                ModelParseErrorCode::ModelSchemaVersionUnsupported,
                "The model proposal uses a future schema version",
                "Return schemaVersion 1",
            ));
        }
        Some(_) => {
            return Err(model_error(
                ModelParseErrorCode::ModelSchemaVersionInvalid,
                "The model proposal schemaVersion must be integer 1",
                "Set schemaVersion to integer 1 without quotes",
            ));
        }
    }
    validate_json_bounds(&value, 0)?;
    // Decode from the original candidate rather than the intermediate Value so
    // serde can reject duplicate struct fields instead of silently retaining
    // only the last occurrence.
    let proposal = serde_json::from_str::<ProposedPlanV1>(candidate).map_err(|error| {
        let code = if error.to_string().contains("unknown field") {
            ModelParseErrorCode::ModelUnknownField
        } else {
            ModelParseErrorCode::ModelInvalidJson
        };
        model_error(
            code,
            "The model proposal does not match the strict plan schema",
            "Remove unknown fields and correct every typed plan value",
        )
    })?;
    if proposal.tasks.is_empty()
        || proposal.tasks.len() > HARD_MAX_PLAN_TASKS
        || proposal.edges.len() > HARD_MAX_PLAN_EDGES
        || proposal
            .tasks
            .iter()
            .any(|task| task.max_attempts == 0 || task.max_attempts > HARD_MAX_TASK_ATTEMPTS)
    {
        return Err(model_error(
            ModelParseErrorCode::ModelBoundsExceeded,
            "The model proposal exceeds plan task, edge, or attempt bounds",
            "Return 1 to 64 tasks, at most 256 edges, and 1 to 8 attempts per task",
        ));
    }
    Ok(proposal)
}

fn extract_one_balanced_object(raw: &str) -> Result<&str, ModelParseError> {
    let mut start = None;
    let mut end = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in raw.bytes().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' if depth > 0 => in_string = true,
            b'{' => {
                if depth == 0 {
                    if end.is_some() {
                        return Err(model_error(
                            ModelParseErrorCode::ModelMultipleObjects,
                            "The model output contains more than one JSON object",
                            "Return exactly one proposal object",
                        ));
                    }
                    start = Some(index);
                }
                depth = depth.saturating_add(1);
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    end = Some(index + 1);
                }
            }
            _ => {}
        }
    }
    if depth != 0 || in_string {
        return Err(model_error(
            ModelParseErrorCode::ModelInvalidJson,
            "The model output contains an unterminated JSON object or string",
            "Return one complete JSON proposal object",
        ));
    }
    let (Some(start), Some(end)) = (start, end) else {
        return Err(model_error(
            ModelParseErrorCode::ModelInvalidJson,
            "The model output contains no balanced JSON object",
            "Return one JSON proposal object",
        ));
    };
    Ok(&raw[start..end])
}

fn validate_json_bounds(value: &Value, depth: usize) -> Result<(), ModelParseError> {
    if depth > MAX_MODEL_JSON_DEPTH {
        return Err(bounds_error("The model JSON exceeds 32 levels of nesting"));
    }
    match value {
        Value::Object(object) => {
            if object.len() > MAX_MODEL_CONTAINER_ITEMS {
                return Err(bounds_error(
                    "A model JSON object contains more than 256 fields",
                ));
            }
            for (key, value) in object {
                if key.len() > MAX_MODEL_SCALAR_BYTES {
                    return Err(bounds_error("A model JSON key exceeds 32 KiB"));
                }
                validate_json_bounds(value, depth + 1)?;
            }
        }
        Value::Array(values) => {
            if values.len() > MAX_MODEL_CONTAINER_ITEMS {
                return Err(bounds_error(
                    "A model JSON array contains more than 256 items",
                ));
            }
            for value in values {
                validate_json_bounds(value, depth + 1)?;
            }
        }
        Value::String(value) if value.len() > MAX_MODEL_SCALAR_BYTES => {
            return Err(bounds_error("A model JSON string exceeds 32 KiB"));
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

fn bounds_error(explanation: &str) -> ModelParseError {
    model_error(
        ModelParseErrorCode::ModelBoundsExceeded,
        explanation,
        "Reduce the proposal to the published model-output bounds",
    )
}

fn model_error(
    code: ModelParseErrorCode,
    explanation: &str,
    corrective_action: &str,
) -> ModelParseError {
    ModelParseError {
        code,
        explanation: SafeText::new(explanation),
        corrective_action: SafeText::new(corrective_action),
    }
}
