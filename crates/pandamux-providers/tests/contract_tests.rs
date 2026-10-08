use pandamux_core::{
    AttachmentRecord,
    event::TurnOutcome,
    ids::{ProviderInstanceId, ThreadId},
    provider_config::ProviderKind,
    thread::{AccessMode, TurnInput},
};
use pandamux_providers::{
    ENV_CLAUDE_CONFIG_DIR, ENV_CODEX_HOME, ENV_GEMINI_HOME, apply_profile_environment,
    build_claude_interrupt_request, build_claude_launch_args, build_claude_user_message,
    claude::{ClaudeAuthStatus, ClaudeStreamEventParser, ControlResponse, parse_stream_json_lines},
    claude_disallowed_tools, claude_permission_mode,
    codex::{
        CodexApprovalPolicy, CodexNotificationEvent, CodexSandboxPolicy, JsonRpcResponse,
        build_approval_response, build_thread_start_request, build_turn_start_request,
        build_turn_start_request_with_attachments, codex_session_policies,
        parse_codex_notification, parse_rate_limits_response, parse_rate_limits_value,
    },
    resolve_command_shim, resolve_profile_dir,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CLAUDE_AUTH_JSON: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/claude/auth_status_auth.json");
const CLAUDE_UNAUTH_JSON: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/claude/auth_status_unauth.json");
const CLAUDE_STREAM_NDJSON: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/claude/stream_json_turn.ndjson");
const CLAUDE_DENIAL_NDJSON: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/claude/control_denial_turn.ndjson");

const CODEX_SCHEMA_JSON: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/codex/schema_dump.json");
const CODEX_LIFECYCLE_JSONL: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/codex/turn_lifecycle.jsonl");
const CODEX_RATELIMITS_JSON: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/codex/account_ratelimits.json");

#[test]
fn test_claude_auth_probe_contract() {
    let unauth: ClaudeAuthStatus =
        serde_json::from_str(CLAUDE_UNAUTH_JSON).expect("parse unauth fixture");
    assert!(!unauth.logged_in);
    assert_eq!(unauth.auth_method.as_deref(), Some("none"));

    let auth: ClaudeAuthStatus =
        serde_json::from_str(CLAUDE_AUTH_JSON).expect("parse auth fixture");
    assert!(auth.logged_in);
    assert_eq!(auth.email.as_deref(), Some("developer@boardpandas.com"));
    assert_eq!(auth.subscription_type.as_deref(), Some("max"));
}

#[test]
fn test_claude_stream_json_and_subagents() {
    let parsed = parse_stream_json_lines(CLAUDE_STREAM_NDJSON).expect("parse stream ndjson");
    assert_eq!(parsed.session_id.as_deref(), Some("sess_claude_01"));

    // Control requests
    assert_eq!(parsed.control_requests.len(), 1);
    let perm = &parsed.control_requests[0];
    assert_eq!(perm.tool_name, "Bash");
    assert_eq!(
        perm.command.as_deref(),
        Some("curl -I https://api.anthropic.com")
    );

    // Allow response
    let allow = ControlResponse::allow(&perm.request_id, perm.input.clone());
    let allow_json = serde_json::to_string(&allow).expect("serialize allow");
    assert!(allow_json.contains("\"behavior\":\"allow\""));
    assert!(allow_json.contains("\"updatedInput\""));

    // Subagent extraction
    assert_eq!(parsed.subagents.len(), 1);
    let sub = &parsed.subagents[0];
    assert_eq!(sub.subagent_type, "reviewer");
    assert_eq!(sub.parent_tool_use_id.as_deref(), Some("toolu_01"));
    assert_eq!(sub.model.as_deref(), Some("claude-3-5-haiku-20241022"));

    // Usage extraction
    let usage = parsed.usage.expect("usage exists");
    assert_eq!(usage.input_tokens, 1250);
    assert_eq!(usage.output_tokens, 380);
    assert_eq!(usage.cost_usd, Some(0.0084));
}

#[test]
fn test_claude_control_denial_and_launch_args() {
    let parsed = parse_stream_json_lines(CLAUDE_DENIAL_NDJSON).expect("parse denial ndjson");
    assert_eq!(parsed.control_requests.len(), 1);

    let deny = ControlResponse::deny(
        &parsed.control_requests[0].request_id,
        "Outbound POST blocked",
    );
    let deny_json = serde_json::to_string(&deny).expect("serialize deny");
    assert!(deny_json.contains("\"behavior\":\"deny\""));
    assert!(deny_json.contains("Outbound POST blocked"));

    let args = build_claude_launch_args(
        Some("Injected agent instructions"),
        &["Read".into(), "Edit".into()],
        &["Bash".into()],
        Some("auto"),
        Some("sess-uuid-42"),
    );
    assert!(args.contains(&"--append-system-prompt".into()));
    assert!(args.contains(&"--allowedTools".into()));
    assert!(args.contains(&"--disallowedTools".into()));
    assert!(args.contains(&"--permission-mode".into()));
    assert!(args.contains(&"--session-id".into()));
    assert!(args.contains(&"--include-partial-messages".into()));

    let bypass_args = build_claude_launch_args(None, &[], &[], Some("bypassPermissions"), None);
    assert!(
        bypass_args.contains(&"--allow-dangerously-skip-permissions".to_string()),
        "full access must explicitly enable Claude's bypass mode"
    );
}

#[test]
fn test_claude_access_modes_map_to_supported_permission_modes() {
    assert_eq!(claude_permission_mode(AccessMode::ReadOnly), "plan");
    assert_eq!(claude_permission_mode(AccessMode::Ask), "manual");
    assert_eq!(claude_permission_mode(AccessMode::WorkspaceOnly), "manual");
    assert_eq!(claude_permission_mode(AccessMode::AutoEdit), "acceptEdits");
    assert_eq!(
        claude_permission_mode(AccessMode::FullAccess),
        "bypassPermissions"
    );
}

#[test]
fn test_claude_read_only_disallows_exiting_plan_mode() {
    let configured = vec!["WebFetch".to_string()];
    assert_eq!(
        claude_disallowed_tools(AccessMode::ReadOnly, &configured),
        vec!["WebFetch".to_string(), "ExitPlanMode".to_string()]
    );
    assert_eq!(
        claude_disallowed_tools(AccessMode::Ask, &configured),
        configured
    );
    assert_eq!(
        claude_disallowed_tools(AccessMode::ReadOnly, &["ExitPlanMode".to_string()]),
        vec!["ExitPlanMode".to_string()]
    );
}

#[test]
fn test_claude_stream_input_uses_the_sdk_user_message_schema() {
    let input = TurnInput {
        text: "Describe the attachment".to_string(),
        attachments: vec![AttachmentRecord {
            id: "att-claude-1".to_string(),
            thread_id: ThreadId::from("thread-claude-1"),
            file_name: "diagram.png".to_string(),
            mime_type: "image/png".to_string(),
            size_bytes: 12,
            file_path: "/workspace/diagram.png".to_string(),
            created_at_ms: 1,
        }],
        ..TurnInput::default()
    };

    let message = build_claude_user_message(&input);
    assert_eq!(message["type"], "user");
    assert_eq!(message["message"]["role"], "user");
    assert_eq!(message["message"]["content"][0]["type"], "text");
    let text = message["message"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(text.starts_with("Describe the attachment"));
    assert!(text.contains("diagram.png (image/png, 12 B): /workspace/diagram.png"));
    assert!(message.get("text").is_none());
    assert!(message.get("attachments").is_none());
}

#[test]
fn test_claude_current_stream_output_schema() {
    let assistant = serde_json::json!({
        "type": "assistant",
        "message": {
            "content": [{"type": "text", "text": "R0_CLAUDE_PROTOCOL_OK"}]
        },
        "session_id": "session-current"
    });
    assert_eq!(
        pandamux_providers::claude::parse_stream_json_line(&assistant.to_string()),
        Some(pandamux_providers::ProviderEvent::TextDelta {
            delta: "R0_CLAUDE_PROTOCOL_OK".to_string()
        })
    );

    let failed = serde_json::json!({
        "type": "result",
        "subtype": "error_during_execution",
        "is_error": true,
        "usage": {"input_tokens": 3, "output_tokens": 1}
    });
    assert!(matches!(
        pandamux_providers::claude::parse_stream_json_line(&failed.to_string()),
        Some(pandamux_providers::ProviderEvent::TurnCompleted {
            outcome: TurnOutcome::Failed,
            ..
        })
    ));

    let tool_use = serde_json::json!({
        "type": "assistant",
        "message": {
            "content": [{
                "type": "tool_use",
                "id": "tool-current",
                "name": "Bash",
                "input": {"command": "pwd"}
            }]
        }
    });
    assert!(matches!(
        pandamux_providers::claude::parse_stream_json_line(&tool_use.to_string()),
        Some(pandamux_providers::ProviderEvent::ToolCallStarted { id, name, .. })
            if id == "tool-current" && name == "Bash"
    ));

    let tool_result = serde_json::json!({
        "type": "user",
        "message": {
            "content": [{
                "type": "tool_result",
                "tool_use_id": "tool-current",
                "content": "workspace path",
                "is_error": false
            }]
        }
    });
    assert!(matches!(
        pandamux_providers::claude::parse_stream_json_line(&tool_result.to_string()),
        Some(pandamux_providers::ProviderEvent::ToolCallFinished { id, error: None, .. })
            if id == "tool-current"
    ));
}

#[test]
fn test_claude_stream_accumulates_exact_message_components() {
    let lines = [
        serde_json::json!({
            "type": "system",
            "session_id": "session-component"
        }),
        serde_json::json!({
            "type": "assistant",
            "session_id": "session-component",
            "timestamp": "2026-10-06T10:00:01Z",
            "message": {
                "id": "message-thinking-and-text",
                "model": "claude-actual-one",
                "content": [{"type": "thinking", "thinking": "reasoning"}],
                "usage": {"input_tokens": 10, "output_tokens": 1}
            }
        }),
        serde_json::json!({
            "type": "assistant",
            "session_id": "session-component",
            "timestamp": "2026-10-06T10:00:02Z",
            "message": {
                "id": "message-thinking-and-text",
                "model": "claude-actual-one",
                "content": [{"type": "text", "text": "first answer"}],
                "usage": {"input_tokens": 10, "output_tokens": 3}
            }
        }),
        serde_json::json!({
            "type": "assistant",
            "session_id": "session-component",
            "timestamp": "2026-10-06T10:00:03Z",
            "message": {
                "id": "message-after-tool",
                "model": "claude-actual-two",
                "content": [{"type": "text", "text": "tool result"}],
                "usage": {
                    "input_tokens": 2,
                    "output_tokens": 4,
                    "cache_read_input_tokens": 5,
                    "cache_creation_input_tokens": 6
                }
            }
        }),
        serde_json::json!({
            "type": "result",
            "subtype": "success",
            "session_id": "session-component",
            "uuid": "result-uuid-not-a-message-id",
            "timestamp": "2026-10-06T10:00:04Z",
            "model": "selected-routing-model",
            "total_cost_usd": 0.0123,
            "usage": {"input_tokens": 12, "output_tokens": 7}
        }),
    ];
    let mut parser = ClaudeStreamEventParser::default();
    let mut completed = None;
    for line in lines {
        if let Some(pandamux_providers::ProviderEvent::TurnCompleted { usage, .. }) =
            parser.parse_line(&line.to_string())
        {
            completed = usage;
        }
    }
    let usage = completed.expect("terminal usage");
    assert_eq!(usage.components.len(), 2);
    assert_eq!(usage.input_tokens, 12);
    assert_eq!(usage.output_tokens, 7);
    assert_eq!(usage.provider_reported_micro_usd, Some(12_300));
    assert_eq!(
        usage
            .provenance
            .as_ref()
            .and_then(|value| value.provider_native_turn_id.as_deref()),
        Some("result-uuid-not-a-message-id")
    );

    let first = &usage.components[0];
    assert_eq!(
        first.provider_native_message_id,
        "message-thinking-and-text"
    );
    assert_eq!(first.model.as_deref(), Some("claude-actual-one"));
    assert_eq!(first.output_tokens, 3);
    assert_eq!(first.cache_read_tokens, None);
    assert_eq!(first.cache_write_tokens, None);

    let second = &usage.components[1];
    assert_eq!(second.provider_native_message_id, "message-after-tool");
    assert_eq!(second.model.as_deref(), Some("claude-actual-two"));
    assert_eq!(second.cache_read_tokens, Some(5));
    assert_eq!(second.cache_write_tokens, Some(6));
    assert!(
        usage
            .components
            .iter()
            .all(|component| component.model.as_deref() != Some("selected-routing-model"))
    );
}

#[test]
fn test_claude_partial_message_usage_finalizes_tool_turn_components() {
    let lines = [
        serde_json::json!({
            "type": "system",
            "session_id": "session-partial"
        }),
        serde_json::json!({
            "type": "stream_event",
            "parent_tool_use_id": null,
            "event": {
                "type": "message_start",
                "message": {
                    "id": "message-before-tool",
                    "model": "claude-actual",
                    "usage": {
                        "input_tokens": 2,
                        "output_tokens": 8,
                        "cache_read_input_tokens": 11,
                        "cache_creation_input_tokens": 12
                    }
                }
            }
        }),
        serde_json::json!({
            "type": "assistant",
            "session_id": "session-partial",
            "timestamp": "2026-10-07T04:18:50Z",
            "message": {
                "id": "message-before-tool",
                "model": "claude-actual",
                "usage": {
                    "input_tokens": 2,
                    "output_tokens": 16,
                    "cache_read_input_tokens": 11,
                    "cache_creation_input_tokens": 12
                }
            }
        }),
        serde_json::json!({
            "type": "stream_event",
            "parent_tool_use_id": null,
            "event": {
                "type": "message_delta",
                "usage": {
                    "input_tokens": 2,
                    "output_tokens": 74,
                    "cache_read_input_tokens": 11,
                    "cache_creation_input_tokens": 12
                }
            }
        }),
        serde_json::json!({
            "type": "stream_event",
            "parent_tool_use_id": null,
            "event": {"type": "message_stop"}
        }),
        serde_json::json!({
            "type": "stream_event",
            "parent_tool_use_id": null,
            "event": {
                "type": "message_start",
                "message": {
                    "id": "message-after-tool",
                    "model": "claude-actual",
                    "usage": {
                        "input_tokens": 3,
                        "output_tokens": 6,
                        "cache_read_input_tokens": 21,
                        "cache_creation_input_tokens": 22
                    }
                }
            }
        }),
        serde_json::json!({
            "type": "assistant",
            "session_id": "session-partial",
            "timestamp": "2026-10-07T04:18:55Z",
            "message": {
                "id": "message-after-tool",
                "model": "claude-actual",
                "usage": {
                    "input_tokens": 3,
                    "output_tokens": 6,
                    "cache_read_input_tokens": 21,
                    "cache_creation_input_tokens": 22
                }
            }
        }),
        serde_json::json!({
            "type": "stream_event",
            "parent_tool_use_id": null,
            "event": {
                "type": "message_delta",
                "usage": {
                    "input_tokens": 3,
                    "output_tokens": 272,
                    "cache_read_input_tokens": 21,
                    "cache_creation_input_tokens": 22
                }
            }
        }),
        serde_json::json!({
            "type": "stream_event",
            "parent_tool_use_id": null,
            "event": {"type": "message_stop"}
        }),
        serde_json::json!({
            "type": "result",
            "subtype": "success",
            "session_id": "session-partial",
            "uuid": "result-not-a-message-id",
            "total_cost_usd": 0.123,
            "usage": {
                "input_tokens": 5,
                "output_tokens": 346,
                "cache_read_input_tokens": 32,
                "cache_creation_input_tokens": 34
            }
        }),
    ];
    let mut parser = ClaudeStreamEventParser::default();
    let mut completed = None;
    for line in lines {
        if let Some(pandamux_providers::ProviderEvent::TurnCompleted { usage, .. }) =
            parser.parse_line(&line.to_string())
        {
            completed = usage;
        }
    }

    let usage = completed.expect("terminal usage");
    assert_eq!(usage.output_tokens, 346);
    assert_eq!(usage.components.len(), 2);
    assert_eq!(
        usage.components[0].provider_native_message_id,
        "message-before-tool"
    );
    assert_eq!(usage.components[0].output_tokens, 74);
    assert_eq!(usage.components[0].occurred_at_ms, Some(1_791_346_730_000));
    assert_eq!(
        usage.components[1].provider_native_message_id,
        "message-after-tool"
    );
    assert_eq!(usage.components[1].output_tokens, 272);
    assert_eq!(
        usage
            .components
            .iter()
            .map(|component| component.output_tokens)
            .sum::<u64>(),
        usage.output_tokens
    );
}

#[test]
fn test_claude_current_control_request_and_response_schema() {
    let request = serde_json::json!({
        "type": "control_request",
        "request_id": "request-current",
        "request": {
            "subtype": "can_use_tool",
            "tool_name": "Bash",
            "input": {
                "command": "printf ready > /tmp/marker",
                "description": "Write marker"
            },
            "description": "Write marker"
        }
    });
    let parsed = parse_stream_json_lines(&request.to_string()).expect("parse current request");
    let permission = parsed.control_requests.first().expect("control request");
    assert_eq!(permission.request_id, "request-current");
    assert_eq!(permission.tool_name, "Bash");
    assert_eq!(
        permission.command.as_deref(),
        Some("printf ready > /tmp/marker")
    );
    assert_eq!(permission.input["description"], "Write marker");

    assert!(matches!(
        pandamux_providers::claude::parse_stream_json_line(&request.to_string()),
        Some(pandamux_providers::ProviderEvent::ApprovalRequested {
            request_id,
            command: Some(command),
            ..
        }) if request_id == "request-current" && command == "printf ready > /tmp/marker"
    ));

    let response = ControlResponse::allow(&permission.request_id, permission.input.clone());
    assert_eq!(
        serde_json::to_value(response).expect("serialize current allow"),
        serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": "request-current",
                "response": {
                    "behavior": "allow",
                    "updatedInput": {
                        "command": "printf ready > /tmp/marker",
                        "description": "Write marker"
                    }
                }
            }
        })
    );

    assert_eq!(
        build_claude_interrupt_request("interrupt-current"),
        serde_json::json!({
            "type": "control_request",
            "request_id": "interrupt-current",
            "request": {"subtype": "interrupt"}
        })
    );
}

#[test]
fn test_codex_schema_and_knobs_contract() {
    let schema_val: serde_json::Value =
        serde_json::from_str(CODEX_SCHEMA_JSON).expect("parse schema");
    let methods = schema_val.get("methods").expect("methods in schema");

    assert!(methods.get("thread/start").is_some());
    assert!(methods.get("thread/resume").is_some());
    assert!(methods.get("turn/interrupt").is_some());
    assert!(methods.get("model/list").is_some());
    assert!(methods.get("account/rateLimits/read").is_some());

    let initialize = &methods["initialize"]["params"]["properties"];
    assert!(initialize.get("clientInfo").is_some());

    let thread_start = &methods["thread/start"]["params"]["properties"];
    assert!(thread_start.get("developerInstructions").is_some());
    assert!(thread_start.get("sandbox").is_some());
    assert!(thread_start.get("approvalPolicy").is_some());

    let thread_req = build_thread_start_request(
        10,
        "C:\\Workspace",
        "o3-mini",
        Some("Developer instructions for reviewer"),
        CodexSandboxPolicy::WorkspaceWrite,
        CodexApprovalPolicy::OnRequest,
    );
    let thread_json = serde_json::to_string(&thread_req).expect("serialize thread/start");
    assert!(thread_json.contains("developerInstructions"));
    assert!(thread_json.contains("workspace-write"));
    assert!(thread_json.contains("on-request"));
    assert!(thread_json.contains("\"sandbox\""));
    assert!(!thread_json.contains("sandboxPolicy"));
    assert!(!thread_json.contains("jsonrpc"));

    let turn_req = build_turn_start_request(11, "th_codex_01", "Inspect crate boundaries");
    let turn_json = serde_json::to_string(&turn_req).expect("serialize turn/start");
    assert!(turn_json.contains("th_codex_01"));
    assert!(turn_json.contains("\"type\":\"text\""));
    assert!(!turn_json.contains("jsonrpc"));

    let image = AttachmentRecord {
        id: "att-1".to_string(),
        thread_id: ThreadId::from("thread-1"),
        file_name: "diagram.png".to_string(),
        mime_type: "image/png".to_string(),
        size_bytes: 12,
        file_path: "/workspace/diagram.png".to_string(),
        created_at_ms: 1,
    };
    let image_req = build_turn_start_request_with_attachments(
        12,
        "th_codex_01",
        "Describe the image",
        &[image],
    );
    assert_eq!(
        image_req.params["input"][1],
        serde_json::json!({
            "type": "localImage",
            "path": "/workspace/diagram.png"
        })
    );
}

#[test]
fn test_codex_access_modes_preserve_the_pandamux_ceiling() {
    use pandamux_core::thread::AccessMode;

    assert_eq!(
        codex_session_policies(AccessMode::ReadOnly),
        (CodexSandboxPolicy::ReadOnly, CodexApprovalPolicy::Never)
    );
    assert_eq!(
        codex_session_policies(AccessMode::Ask),
        (
            CodexSandboxPolicy::WorkspaceWrite,
            CodexApprovalPolicy::Untrusted
        )
    );
    assert_eq!(
        codex_session_policies(AccessMode::WorkspaceOnly),
        (
            CodexSandboxPolicy::WorkspaceWrite,
            CodexApprovalPolicy::OnRequest
        )
    );
    assert_eq!(
        codex_session_policies(AccessMode::AutoEdit),
        (
            CodexSandboxPolicy::WorkspaceWrite,
            CodexApprovalPolicy::Never
        )
    );
    assert_eq!(
        codex_session_policies(AccessMode::FullAccess),
        (
            CodexSandboxPolicy::DangerFullAccess,
            CodexApprovalPolicy::Never
        )
    );
}

#[test]
fn test_codex_lifecycle_and_ratelimits() {
    let mut outcome = None;
    let mut approval_seen = false;

    for line in CODEX_LIFECYCLE_JSONL.lines() {
        if line.contains("\"item/commandExecution/requestApproval\"") {
            if let Ok(CodexNotificationEvent::ApprovalRequest {
                request_id,
                command,
                ..
            }) = parse_codex_notification(line)
            {
                approval_seen = true;
                assert_eq!(request_id, "approval-01");
                let appr_resp = build_approval_response(true);
                assert_eq!(appr_resp["decision"], "accept");
                assert_eq!(command, "cargo check --workspace");
            }
        } else if line.contains("\"turn/completed\"")
            && let Ok(CodexNotificationEvent::TurnCompleted {
                outcome: turn_outcome,
            }) = parse_codex_notification(line)
        {
            outcome = Some(turn_outcome);
        }
    }

    assert!(approval_seen);
    assert_eq!(outcome, Some(TurnOutcome::Success));

    let limits_raw: serde_json::Value =
        serde_json::from_str(CODEX_RATELIMITS_JSON).expect("parse limits fixture");
    let limits_resp = JsonRpcResponse {
        jsonrpc: None,
        id: 5,
        result: Some(limits_raw),
        error: None,
    };
    let parsed_limits = parse_rate_limits_response(&limits_resp).expect("parse rate limits");
    assert_eq!(parsed_limits.len(), 2);
    assert_eq!(parsed_limits[0].limit_name, "Codex primary");
    assert_eq!(parsed_limits[0].used_millionths, Some(340_000));
    assert_eq!(parsed_limits[0].remaining_millionths, Some(660_000));
    assert_eq!(parsed_limits[0].resets_at_ms, Some(1_790_395_200_000));
    assert_eq!(parsed_limits[1].limit_name, "Codex secondary");
    assert_eq!(parsed_limits[1].used_millionths, Some(180_000));
    assert_eq!(parsed_limits[1].remaining_millionths, Some(820_000));
}

#[test]
fn test_codex_rate_limit_fraction_conversion_is_exact_and_bounded() {
    let limits = parse_rate_limits_value(&serde_json::json!({
        "limitName": "Codex",
        "primary": {
            "usedPercent": "14.825",
            "windowDurationMins": 300,
            "resetsAt": "1790395200"
        }
    }))
    .expect("parse exact decimal percentage");
    assert_eq!(limits[0].used_millionths, Some(148_250));
    assert_eq!(limits[0].remaining_millionths, Some(851_750));
    assert_eq!(limits[0].resets_at_ms, Some(1_790_395_200_000));

    assert!(
        parse_rate_limits_value(&serde_json::json!({
            "primary": { "usedPercent": 100.0001 }
        }))
        .is_err()
    );
}

#[test]
fn test_codex_backend_permission_controls_hard_limit() {
    let allowed = JsonRpcResponse {
        jsonrpc: None,
        id: 5,
        result: Some(serde_json::json!({
            "ordinaryUsageAllowed": true,
            "rateLimits": {
                "limitId": "codex",
                "primary": {
                    "usedPercent": 100,
                    "windowDurationMins": 10080,
                    "resetsAt": 1791948542
                },
                "credits": {
                    "hasCredits": true,
                    "unlimited": false,
                    "balance": "100"
                },
                "spendControlReached": null,
                "rateLimitReachedType": null
            }
        })),
        error: None,
    };
    let allowed_limits = parse_rate_limits_response(&allowed).expect("parse allowed limits");
    assert_eq!(allowed_limits.len(), 1);
    assert_eq!(allowed_limits[0].used_millionths, Some(1_000_000));
    assert_eq!(allowed_limits[0].remaining_millionths, Some(0));
    assert!(!allowed_limits[0].hard_limit);

    let blocked = JsonRpcResponse {
        result: Some(serde_json::json!({
            "ordinaryUsageAllowed": false,
            "rateLimits": {
                "limitId": "codex",
                "primary": {
                    "usedPercent": 42,
                    "windowDurationMins": 10080,
                    "resetsAt": 1791948542
                }
            }
        })),
        ..allowed
    };
    let blocked_limits = parse_rate_limits_response(&blocked).expect("parse blocked limits");
    assert_eq!(blocked_limits[0].used_millionths, Some(1_000_000));
    assert_eq!(blocked_limits[0].remaining_millionths, Some(0));
    assert!(blocked_limits[0].hard_limit);
}

#[test]
fn test_codex_snapshot_requires_explicit_block_for_hard_limit() {
    let observed = parse_rate_limits_value(&serde_json::json!({
        "limitId": "codex",
        "primary": {
            "usedPercent": 100,
            "windowDurationMins": 10080,
            "resetsAt": 1791948542
        },
        "spendControlReached": null,
        "rateLimitReachedType": null
    }))
    .expect("parse serviceable observation");
    assert!(!observed[0].hard_limit);

    let blocked = parse_rate_limits_value(&serde_json::json!({
        "limitId": "codex",
        "primary": {
            "usedPercent": 100,
            "windowDurationMins": 10080,
            "resetsAt": 1791948542
        },
        "spendControlReached": false,
        "rateLimitReachedType": "rate_limit_reached"
    }))
    .expect("parse blocked observation");
    assert!(blocked[0].hard_limit);
}

#[test]
fn test_multi_instance_profile_isolation() {
    let base = Path::new("/var/pandamux");
    let instance_1 = ProviderInstanceId::from("inst-1");
    let instance_2 = ProviderInstanceId::from("inst-2");

    let p1 = resolve_profile_dir(base, ProviderKind::Claude, &instance_1);
    let p2 = resolve_profile_dir(base, ProviderKind::Claude, &instance_2);
    assert_ne!(p1, p2);
    assert_eq!(p1, PathBuf::from("/var/pandamux/profiles/claude/inst-1"));

    // Check env injection
    let mut cmd = tokio::process::Command::new("echo");
    let mut overrides = BTreeMap::new();
    overrides.insert("CUSTOM_VAR".to_string(), "custom_val".to_string());

    apply_profile_environment(&mut cmd, ProviderKind::Claude, &p1, &overrides);

    let codex_dir = resolve_profile_dir(base, ProviderKind::Codex, &instance_1);
    let mut codex_cmd = tokio::process::Command::new("echo");
    apply_profile_environment(
        &mut codex_cmd,
        ProviderKind::Codex,
        &codex_dir,
        &BTreeMap::new(),
    );

    let gemini_dir = resolve_profile_dir(base, ProviderKind::Antigravity, &instance_1);
    let mut gemini_cmd = tokio::process::Command::new("echo");
    apply_profile_environment(
        &mut gemini_cmd,
        ProviderKind::Antigravity,
        &gemini_dir,
        &BTreeMap::new(),
    );

    assert_eq!(ENV_CLAUDE_CONFIG_DIR, "CLAUDE_CONFIG_DIR");
    assert_eq!(ENV_CODEX_HOME, "CODEX_HOME");
    assert_eq!(ENV_GEMINI_HOME, "GEMINI_HOME");
}

#[test]
fn test_shim_resolution() {
    #[cfg(windows)]
    let target = "cmd";
    #[cfg(not(windows))]
    let target = "sh";

    let resolved = resolve_command_shim(target).expect("resolve system shell");
    assert!(resolved.program.exists());
    assert!(resolved.is_direct_executable);
}

const AGY_INIT_JSON: &str =
    include_str!("../../../spikes/phase0-antigravity/fixtures/initialize_response.json");
const AGY_MANIFEST_JSON: &str =
    include_str!("../../../spikes/phase0-antigravity/fixtures/managed_bundle_manifest.json");
const AGY_OAUTH_JSON: &str =
    include_str!("../../../spikes/phase0-antigravity/fixtures/oauth_auth_flow.json");
const AGY_STREAM_NDJSON: &str =
    include_str!("../../../spikes/phase0-antigravity/fixtures/session_turn_stream.ndjson");

#[test]
fn test_antigravity_initialize_contract() {
    let init_val: serde_json::Value =
        serde_json::from_str(AGY_INIT_JSON).expect("parse init fixture");
    let result_obj = init_val.get("result").expect("result in init fixture");
    let res: pandamux_providers::acp::AcpInitializeResult =
        serde_json::from_value(result_obj.clone()).expect("deserialize AcpInitializeResult");

    assert_eq!(res.protocol_version, 1);
    assert_eq!(res.agent_info.name, "antigravity-acp");
    assert!(res.capabilities.load_session);
    assert!(res.capabilities.session_capabilities.resume);
    assert!(res.capabilities.auth.logout);
    assert!(
        res.auth_methods
            .iter()
            .any(|method| method.id() == "oauth-personal")
    );

    pandamux_providers::acp::validate_antigravity_initialize(&res)
        .expect("validate initialize succeeds");
}

#[test]
fn test_antigravity_session_new_and_client_capabilities() {
    use pandamux_core::thread::AccessMode;
    use pandamux_providers::acp::{AcpMode, build_session_new_request, map_access_mode_to_acp};

    assert_eq!(
        map_access_mode_to_acp(AccessMode::FullAccess),
        AcpMode::Yolo
    );
    assert_eq!(
        map_access_mode_to_acp(AccessMode::AutoEdit),
        AcpMode::AutoEdit
    );
    assert_eq!(
        map_access_mode_to_acp(AccessMode::ReadOnly),
        AcpMode::Default
    );

    let session_req = build_session_new_request(101, AcpMode::Yolo);
    assert_eq!(session_req["method"], "session/new");
    assert_eq!(session_req["params"]["mode"], "yolo");

    let client_caps = &session_req["params"]["clientCapabilities"];
    assert_eq!(client_caps["terminal"], false);
    assert_eq!(client_caps["fs"]["readTextFile"], true);
    assert_eq!(client_caps["fs"]["writeTextFile"], true);
}

#[test]
fn test_antigravity_oauth_flow_contract() {
    use pandamux_providers::antigravity::{
        extract_auth_url_from_output, validate_callback_query, validate_google_oauth_url,
    };

    let oauth_fixture: serde_json::Value =
        serde_json::from_str(AGY_OAUTH_JSON).expect("parse oauth fixture");

    let stdout_marker = oauth_fixture["stdoutMarker"]
        .as_str()
        .expect("stdoutMarker");
    let extracted_url =
        extract_auth_url_from_output(stdout_marker).expect("extract auth url from marker");

    let validated = validate_google_oauth_url(&extracted_url).expect("validate google oauth url");
    assert_eq!(validated.redirect_port, 8085);
    assert_eq!(validated.state, "sec_state_9876");
    assert!(validated.client_id.contains("apps.googleusercontent.com"));

    let callback_query = oauth_fixture["simulatedCallbackQuery"]
        .as_str()
        .expect("simulatedCallbackQuery");
    let code =
        validate_callback_query(callback_query, "sec_state_9876").expect("validate callback query");
    assert_eq!(code, "4/0AQ_TEST_TOKEN");

    // Mismatched state must be rejected
    assert!(validate_callback_query(callback_query, "wrong_state").is_err());
}

#[test]
fn test_antigravity_managed_bundle_manifest_validation() {
    use pandamux_providers::antigravity::{
        ManagedBundleManifest, pinned_bundle_manifest, validate_pinned_manifest,
    };

    let manifest: ManagedBundleManifest =
        serde_json::from_str(AGY_MANIFEST_JSON).expect("parse manifest fixture");
    assert_eq!(manifest.version, "agy_acp_server_1.1.1");
    assert_eq!(manifest.executable.name, "agy_acp_server.exe");
    assert_eq!(manifest.executable.size, 430_801_616);
    assert_eq!(manifest.harness.name, "localharness_external.exe");
    assert_eq!(manifest.harness.size, 130_971_800);
    assert_eq!(
        manifest,
        pinned_bundle_manifest("windows", "x86_64").expect("Windows x64 pin")
    );
    validate_pinned_manifest(&manifest).expect("official pin is accepted");

    // Any caller-controlled release metadata must be rejected.
    let mut bad_manifest = manifest.clone();
    bad_manifest.executable.name = "../evil.exe".to_string();
    assert!(validate_pinned_manifest(&bad_manifest).is_err());
}

#[test]
fn test_antigravity_spawn_environment_and_credential_sanitization() {
    use pandamux_providers::antigravity::{build_antigravity_env, validate_sanitized_environment};
    use std::collections::HashMap;

    let base = Path::new("/var/pandamux");
    let harness = Path::new("/var/pandamux/tools/localharness_external");
    let env_config = build_antigravity_env("inst-test", base, harness, "pandamux-browser-helper");

    assert_eq!(
        env_config.gemini_home,
        PathBuf::from("/var/pandamux/profiles/antigravity/inst-test")
    );
    assert_eq!(
        env_config.scratch_dir,
        PathBuf::from("/var/pandamux/scratch/antigravity/inst-test")
    );
    assert_eq!(
        env_config
            .vars
            .get("AGY_ACP_FORCE_FILE_STORAGE")
            .map(|s| s.as_str()),
        Some("1")
    );
    assert_eq!(
        env_config.vars.get("PYTHONUNBUFFERED").map(|s| s.as_str()),
        Some("1")
    );
    assert_eq!(
        env_config.vars.get("BROWSER").map(|s| s.as_str()),
        Some("pandamux-browser-helper")
    );

    // Sanitized environment check
    assert!(validate_sanitized_environment(&env_config.vars));

    let mut tainted_vars = HashMap::new();
    tainted_vars.insert("GEMINI_API_KEY".to_string(), "secret-123".to_string());
    assert!(!validate_sanitized_environment(&tainted_vars));

    let mut tainted_oauth = HashMap::new();
    tainted_oauth.insert("AGY_ACP_TOKEN".to_string(), "tok-456".to_string());
    assert!(!validate_sanitized_environment(&tainted_oauth));
}

#[test]
fn test_antigravity_seven_reliability_rules() {
    use pandamux_providers::acp::{AcpPermissionRequest, is_interaction_prompt};
    use pandamux_providers::antigravity::{
        AntigravityConcurrencyLimiter, probe_antigravity_health_offline, sweep_orphan_temp_dirs,
    };

    // Rule 1: Zero-spawn offline health check
    let non_existent = Path::new("/non/existent/agy_acp_server");
    let gemini_home = Path::new("/var/pandamux/profiles/antigravity/inst-1");
    let health = probe_antigravity_health_offline(non_existent, gemini_home, None);
    assert!(matches!(
        health,
        pandamux_providers::ProviderHealth::Unavailable { .. }
    ));

    // Rule 2: Temp directory sweeper
    let scratch = std::env::temp_dir().join("pandamux_test_scratch");
    let _ = std::fs::create_dir_all(&scratch);
    let orphan1 = scratch.join("_MEI12345");
    let orphan2 = scratch.join("agy_tmp_67890");
    let keep_file = scratch.join("valid_file.txt");
    let _ = std::fs::create_dir_all(&orphan1);
    let _ = std::fs::create_dir_all(&orphan2);
    let _ = std::fs::write(&keep_file, b"keep");

    let swept = sweep_orphan_temp_dirs(&scratch).expect("sweep orphan dirs");
    assert_eq!(swept, 2);
    assert!(!orphan1.exists());
    assert!(!orphan2.exists());
    assert!(keep_file.exists());
    let _ = std::fs::remove_dir_all(&scratch);

    // Rule 3: Concurrency limiter (max 2 active processes)
    let limiter = Arc::new(AntigravityConcurrencyLimiter::new(2));
    let guard1 = limiter.try_acquire().expect("acquire 1");
    let guard2 = limiter.try_acquire().expect("acquire 2");
    assert!(
        limiter.try_acquire().is_err(),
        "Third concurrent acquire must fail"
    );
    drop(guard1);
    let guard3 = limiter.try_acquire().expect("acquire 3 after drop");
    drop(guard2);
    drop(guard3);

    // Rule 6: Interaction prompt detection
    assert!(is_interaction_prompt("interaction_confirm_deployment"));
    assert!(!is_interaction_prompt("fs/writeTextFile"));

    // Rule 7: Permission request security warning extraction
    let perm_json = r#"{
        "sessionId": "sess_agy_4242",
        "permissionType": "file_write",
        "resource": "src/main.rs",
        "options": ["allow_once", "allow_always", "reject_once"],
        "_meta": {
            "agy.security.warning": "Prompt injection risk detected in source comment"
        }
    }"#;
    let perm_req: AcpPermissionRequest = serde_json::from_str(perm_json).expect("parse permission");
    assert_eq!(
        perm_req.security_warning().as_deref(),
        Some("Prompt injection risk detected in source comment")
    );
}

#[test]
fn test_antigravity_stream_parsing() {
    use pandamux_providers::acp::parse_acp_line;

    let mut event_count = 0;
    for line in AGY_STREAM_NDJSON.lines() {
        if let Some(event) = parse_acp_line(line) {
            event_count += 1;
            match event {
                pandamux_providers::ProviderEvent::TextDelta { delta } => {
                    assert!(delta.contains("Analyzing codebase"));
                }
                pandamux_providers::ProviderEvent::TurnCompleted { usage, .. } => {
                    assert_eq!(usage.unwrap().input_tokens, 2450);
                }
                _ => {}
            }
        }
    }
    assert!(event_count >= 1);
}
