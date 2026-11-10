use crate::mutation_service::{MutationProposalRequest, MutationService};
use crate::request_context::RequestContext;
use crate::store::Store;
use pandamux_core::{
    AgentId, ConfigurationRevision, EnvironmentId, MutationImpact, MutationTarget, ProjectId,
    ProviderInstanceId, ScheduleRecord, ScheduleTarget, UserSettings,
};
use pandamux_protocol::{McpCallToolParams, McpCallToolResult, McpContentItem, McpToolDefinition};
use serde_json::json;

/// Local MCP Server endpoint handling tool discovery and execution.
pub struct McpServer {
    store: Store,
}

impl McpServer {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// List all tools exposed by PandaMUX for read-only settings and schedule inspection.
    pub fn list_tools(&self) -> Vec<McpToolDefinition> {
        vec![
            McpToolDefinition {
                name: "read_settings".to_string(),
                description: "Read a redacted summary of PandaMUX settings".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            McpToolDefinition {
                name: "list_schedules".to_string(),
                description: "List redacted schedule metadata configured on this machine"
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            McpToolDefinition {
                name: "validate_thread_proposal".to_string(),
                description: "Validate a proposed project/provider route without creating a thread"
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "projectId": { "type": "string", "minLength": 1 },
                        "environmentId": { "type": "string", "minLength": 1 },
                        "providerInstanceId": { "type": "string", "minLength": 1 }
                    },
                    "required": ["projectId", "environmentId", "providerInstanceId"],
                    "additionalProperties": false
                }),
            },
            McpToolDefinition {
                name: "propose_agent_access".to_string(),
                description: "Propose a reviewed trusted-agent access change without applying it"
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "requestId": { "type": "string", "minLength": 1, "maxLength": 256 },
                        "agentId": { "type": "string", "minLength": 1 },
                        "disabled": { "type": "boolean" },
                        "fullAccess": { "type": "boolean" },
                        "expectedRevision": { "type": "integer", "minimum": 1 }
                    },
                    "required": [
                        "requestId", "agentId", "disabled", "fullAccess", "expectedRevision"
                    ],
                    "additionalProperties": false
                }),
            },
        ]
    }

    /// Execute an MCP tool.
    pub fn call_tool(&self, params: &McpCallToolParams) -> McpCallToolResult {
        self.call_tool_impl(None, None, params)
    }

    pub fn call_tool_with_context(
        &self,
        context: &RequestContext,
        mutations: &MutationService,
        params: &McpCallToolParams,
    ) -> McpCallToolResult {
        self.call_tool_impl(Some(context), Some(mutations), params)
    }

    fn call_tool_impl(
        &self,
        context: Option<&RequestContext>,
        mutations: Option<&MutationService>,
        params: &McpCallToolParams,
    ) -> McpCallToolResult {
        match params.name.as_str() {
            "read_settings" => match self.store.get_settings() {
                Ok(settings) => McpCallToolResult {
                    content: vec![McpContentItem::Text {
                        text: serde_json::to_string_pretty(&settings_projection(
                            &settings.unwrap_or_default(),
                        ))
                        .unwrap_or_default(),
                    }],
                    is_error: false,
                },
                Err(err) => McpCallToolResult {
                    content: vec![McpContentItem::Text {
                        text: format!("Error reading settings: {err}"),
                    }],
                    is_error: true,
                },
            },
            "list_schedules" => match self.store.list_schedules() {
                Ok(schedules) => McpCallToolResult {
                    content: vec![McpContentItem::Text {
                        text: serde_json::to_string_pretty(
                            &schedules
                                .iter()
                                .map(schedule_projection)
                                .collect::<Vec<_>>(),
                        )
                        .unwrap_or_default(),
                    }],
                    is_error: false,
                },
                Err(err) => McpCallToolResult {
                    content: vec![McpContentItem::Text {
                        text: format!("Error listing schedules: {err}"),
                    }],
                    is_error: true,
                },
            },
            "validate_thread_proposal" => {
                let project_id = params
                    .arguments
                    .get("projectId")
                    .and_then(|value| value.as_str());
                let environment_id = params
                    .arguments
                    .get("environmentId")
                    .and_then(|value| value.as_str());
                let provider_instance_id = params
                    .arguments
                    .get("providerInstanceId")
                    .and_then(|value| value.as_str());
                let Some((project_id, environment_id, provider_instance_id)) = project_id
                    .zip(environment_id)
                    .zip(provider_instance_id)
                    .map(|((project, environment), provider)| (project, environment, provider))
                else {
                    return McpCallToolResult {
                        content: vec![McpContentItem::Text {
                            text:
                                "Proposal requires projectId, environmentId, and providerInstanceId"
                                    .to_string(),
                        }],
                        is_error: true,
                    };
                };
                match self.store.authorize_provider_for_project(
                    &ProjectId::new(project_id),
                    &EnvironmentId::new(environment_id),
                    &ProviderInstanceId::new(provider_instance_id),
                ) {
                    Ok((policy, subscription_key)) => {
                        let provider = self
                            .store
                            .get_settings()
                            .ok()
                            .flatten()
                            .unwrap_or_default()
                            .providers
                            .into_iter()
                            .find(|provider| provider.id.as_str() == provider_instance_id);
                        let Some(provider) = provider.filter(|provider| {
                            provider.enabled
                                && provider
                                    .environment_id
                                    .as_ref()
                                    .is_none_or(|configured| configured.as_str() == environment_id)
                        }) else {
                            return McpCallToolResult {
                                content: vec![McpContentItem::Text {
                                    text: serde_json::to_string_pretty(&json!({
                                        "allowed": false,
                                        "reason": "Named provider is disabled or unavailable in the requested environment",
                                        "note": "Rejected before routing or scoring"
                                    }))
                                    .unwrap_or_default(),
                                }],
                                is_error: true,
                            };
                        };
                        let subscription_name = self
                            .store
                            .usage_subscription_by_key(&subscription_key)
                            .ok()
                            .flatten()
                            .map(|subscription| subscription.display_name)
                            .unwrap_or_else(|| "Named subscription unavailable".to_string());
                        McpCallToolResult {
                            content: vec![McpContentItem::Text {
                                text: serde_json::to_string_pretty(&json!({
                                    "allowed": true,
                                    "organization": policy.display_name,
                                    "providerInstanceName": provider.display_name,
                                    "subscriptionName": subscription_name,
                                    "note": "Validation does not create a thread or grant approval"
                                }))
                                .unwrap_or_default(),
                            }],
                            is_error: false,
                        }
                    }
                    Err(error) => McpCallToolResult {
                        content: vec![McpContentItem::Text {
                            text: serde_json::to_string_pretty(&json!({
                                "allowed": false,
                                "reason": error.to_string(),
                                "note": "Rejected before routing or scoring"
                            }))
                            .unwrap_or_default(),
                        }],
                        is_error: true,
                    },
                }
            }
            "propose_agent_access" => {
                let Some((context, mutations)) = context.zip(mutations) else {
                    return tool_error(
                        "Agent access proposals require an authenticated MCP request context",
                    );
                };
                let request = match serde_json::from_value::<AgentAccessProposalInput>(
                    params.arguments.clone(),
                ) {
                    Ok(request) => request,
                    Err(error) => {
                        return tool_error(format!(
                            "Agent access proposal fields are required and cannot be null: {error}"
                        ));
                    }
                };
                let record = match self.store.get_or_initialize_settings(now_ms()) {
                    Ok(record) => record,
                    Err(_) => return tool_error("Trusted settings are unavailable"),
                };
                if record.revision != request.expected_revision {
                    return tool_error("Trusted settings changed before this proposal was created");
                }
                let mut next = record.settings.clone();
                if request.disabled {
                    next.trusted_configuration
                        .disabled_agent_ids
                        .insert(request.agent_id.clone());
                } else {
                    next.trusted_configuration
                        .disabled_agent_ids
                        .remove(&request.agent_id);
                }
                if request.full_access {
                    next.trusted_configuration
                        .full_access_agent_ids
                        .insert(request.agent_id.clone());
                } else {
                    next.trusted_configuration
                        .full_access_agent_ids
                        .remove(&request.agent_id);
                }
                let mut impact = MutationImpact::default();
                impact.agent_ids.insert(request.agent_id.clone());
                impact.access_summaries.insert(format!(
                    "Set agent {} disabled state to {}",
                    request.agent_id, request.disabled
                ));
                impact.full_access_summaries.insert(format!(
                    "Set agent {} FullAccess state to {}",
                    request.agent_id, request.full_access
                ));
                let proposal = mutations.propose(
                    context,
                    MutationProposalRequest {
                        request_id: request.request_id,
                        tool_name: "mcp.propose_agent_access".to_string(),
                        target: MutationTarget::Settings,
                        expected_revision: request.expected_revision,
                        old_json: match serde_json::to_value(
                            pandamux_core::TrustedSettingsMutationState {
                                trusted_configuration: record
                                    .settings
                                    .trusted_configuration
                                    .clone(),
                            },
                        ) {
                            Ok(value) => value,
                            Err(_) => return tool_error("Trusted settings could not be projected"),
                        },
                        new_json: match serde_json::to_value(
                            pandamux_core::TrustedSettingsMutationState {
                                trusted_configuration: next.trusted_configuration,
                            },
                        ) {
                            Ok(value) => value,
                            Err(_) => return tool_error("Trusted settings could not be projected"),
                        },
                        impact,
                        created_at_ms: now_ms(),
                        ttl_ms: None,
                    },
                );
                match proposal {
                    Ok(outcome) => McpCallToolResult {
                        content: vec![McpContentItem::Text {
                            text: serde_json::to_string_pretty(&json!({
                                "proposalId": outcome.proposal.id,
                                "created": outcome.created,
                                "resolution": outcome.proposal.resolution,
                                "note": "No settings were changed. Authenticated local control must approve once."
                            }))
                            .unwrap_or_default(),
                        }],
                        is_error: false,
                    },
                    Err(error) => tool_error(format!(
                        "Agent access proposal was not persisted: {}",
                        error.safe_message
                    )),
                }
            }
            unknown => McpCallToolResult {
                content: vec![McpContentItem::Text {
                    text: format!("Unknown tool: {unknown}"),
                }],
                is_error: true,
            },
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentAccessProposalInput {
    request_id: String,
    agent_id: AgentId,
    disabled: bool,
    full_access: bool,
    expected_revision: ConfigurationRevision,
}

fn tool_error(message: impl Into<String>) -> McpCallToolResult {
    McpCallToolResult {
        content: vec![McpContentItem::Text {
            text: message.into(),
        }],
        is_error: true,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn settings_projection(settings: &UserSettings) -> serde_json::Value {
    json!({
        "version": settings.version,
        "ui": settings.ui,
        "terminal": settings.terminal,
        "keyboard": settings.keyboard,
        "advanced": settings.advanced,
        "providerInstances": settings.providers.iter().map(|provider| json!({
            "id": provider.id,
            "provider": provider.provider,
            "displayName": provider.display_name,
            "enabled": provider.enabled,
            "concurrencyLimit": provider.concurrency_limit,
        })).collect::<Vec<_>>(),
        "environments": settings.environments.iter().map(|environment| json!({
            "id": environment.id,
            "displayName": environment.display_name,
            "status": environment.status,
            "platform": environment.platform,
            "serverVersion": environment.server_version,
            "schedulesCount": environment.schedules_count,
        })).collect::<Vec<_>>(),
        "organizationCount": settings.organizations.len(),
    })
}

fn schedule_projection(schedule: &ScheduleRecord) -> serde_json::Value {
    let target_kind = match &schedule.target {
        ScheduleTarget::Orchestrator { .. } => "orchestrator",
        ScheduleTarget::Agent { .. } => "agent",
        ScheduleTarget::Session { .. } => "session",
    };
    json!({
        "id": schedule.id,
        "name": schedule.name,
        "environment": {
            "id": schedule.environment.id,
            "displayName": schedule.environment.display_name,
        },
        "trigger": schedule.trigger,
        "targetKind": target_kind,
        "approval": schedule.approval,
        "overlap": schedule.overlap,
        "catchUp": schedule.catch_up,
        "budget": schedule.budget,
        "enabled": schedule.enabled,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::request_context::{CapabilityScope, ClientIdentity, PeerRole};
    use crate::{DataPaths, PersonalRepoService};
    use pandamux_core::{
        Environment, EnvironmentId, EnvironmentStatus, ProviderEnvironmentOverride,
        ProviderInstanceConfig, ProviderInstanceId, ProviderKind, ScheduleApprovalPolicy,
        ScheduleBudget, ScheduleCatchUpPolicy, ScheduleEnvironmentRef, ScheduleId,
        ScheduleOverlapPolicy, ScheduleTarget, ScheduleThreadMode, ScheduleTrigger,
    };

    use super::*;

    #[test]
    fn mcp_projections_exclude_sensitive_settings_and_schedule_fields() {
        let store = Store::in_memory().expect("store");
        let mut settings = UserSettings::default();
        settings.providers = vec![ProviderInstanceConfig {
            id: ProviderInstanceId::new("provider-safe-id"),
            provider: ProviderKind::Claude,
            display_name: "Claude Work".to_string(),
            profile_dir: "/secret/profile/sentinel".to_string(),
            environment_id: None,
            binary_path: Some("/secret/binary/sentinel".to_string()),
            concurrency_limit: 3,
            env_overrides: BTreeMap::from([(
                "SECRET_ENV".to_string(),
                "secret-env-sentinel".to_string(),
            )]),
            settings: json!({ "apiToken": "arbitrary-json-sentinel" }),
            enabled: true,
            disabled_reason: None,
        }];
        settings.environments = vec![Environment {
            id: EnvironmentId::new("env-local"),
            kind: pandamux_core::EnvironmentKind::Local,
            display_name: "Local".to_string(),
            status: EnvironmentStatus::Ready,
            platform: None,
            server_version: None,
            schedules_count: Some(1),
            last_error: Some("environment-error-sentinel".to_string()),
            execution_limit: pandamux_core::DEFAULT_ENVIRONMENT_EXECUTION_LIMIT,
            provider_overrides: BTreeMap::from([(
                "provider-safe-id".to_string(),
                ProviderEnvironmentOverride {
                    binary_path: Some("override-path-sentinel".to_string()),
                    ..ProviderEnvironmentOverride::default()
                },
            )]),
        }];
        store.save_settings(&settings).expect("save settings");
        store
            .save_schedule(&ScheduleRecord {
                id: ScheduleId::new("schedule-safe-id"),
                name: "Nightly Review".to_string(),
                project_id: Some(pandamux_core::ProjectId::new("project-safe-id")),
                environment: ScheduleEnvironmentRef {
                    id: EnvironmentId::new("env-local"),
                    display_name: "Local".to_string(),
                },
                trigger: ScheduleTrigger {
                    cron: "0 0 * * *".to_string(),
                    timezone: "UTC".to_string(),
                },
                target: ScheduleTarget::Session {
                    project_path: "/secret/repo/sentinel".to_string(),
                    prompt: "schedule-prompt-sentinel".to_string(),
                    provider: Some("provider-safe-id".to_string()),
                    model: Some("model".to_string()),
                    thread_mode: ScheduleThreadMode::NewThreadPerRun,
                },
                approval: ScheduleApprovalPolicy::ApproveFirst,
                overlap: ScheduleOverlapPolicy::Skip,
                catch_up: ScheduleCatchUpPolicy::Skip,
                budget: ScheduleBudget::default(),
                notify: vec!["notification-target-sentinel".to_string()],
                full_access_opt_in: true,
                enabled: true,
            })
            .expect("save schedule");

        let server = McpServer::new(store);
        for tool in ["read_settings", "list_schedules"] {
            let result = server.call_tool(&McpCallToolParams {
                name: tool.to_string(),
                arguments: json!({}),
            });
            assert!(!result.is_error);
            let McpContentItem::Text { text } = &result.content[0] else {
                panic!("expected text projection")
            };
            for secret in [
                "/secret/profile/sentinel",
                "/secret/binary/sentinel",
                "secret-env-sentinel",
                "arbitrary-json-sentinel",
                "override-path-sentinel",
                "environment-error-sentinel",
                "/secret/repo/sentinel",
                "schedule-prompt-sentinel",
                "notification-target-sentinel",
                "fullAccessOptIn",
            ] {
                assert!(!text.contains(secret), "projection leaked {secret}");
            }
        }
    }

    #[test]
    fn mcp_proposals_validate_policy_without_implicit_approval() {
        let store = Store::in_memory().expect("store");
        store
            .save_usage_subscription_mapping(
                &pandamux_core::UsageSubscription {
                    key: pandamux_core::SubscriptionKey::new("subkey-work"),
                    provider: pandamux_core::UsageProvider::Codex,
                    display_name: "Work Codex".to_string(),
                    masked_account_label: None,
                },
                &EnvironmentId::new("env-local"),
                &ProviderInstanceId::new("codex-default"),
                1000,
            )
            .expect("save mapping");
        store
            .save_organization_policy(
                &pandamux_core::OrganizationPolicy {
                    id: pandamux_core::OrganizationId::new("org-work"),
                    display_name: "Work".to_string(),
                    project_ids: vec![ProjectId::new("project-work")],
                    allowed_provider_instances: vec![ProviderInstanceId::new("codex-default")],
                    allowed_subscriptions: vec![pandamux_core::SubscriptionKey::new("subkey-work")],
                    failover_pool_ids: Vec::new(),
                    default_tier_mapping: None,
                    max_concurrency: None,
                    daily_budget_micro_usd: None,
                    broadside: Default::default(),
                    revision: 1,
                },
                1000,
            )
            .expect("save policy");
        let server = McpServer::new(store);

        let allowed = server.call_tool(&McpCallToolParams {
            name: "validate_thread_proposal".to_string(),
            arguments: json!({
                "projectId": "project-work",
                "environmentId": "env-local",
                "providerInstanceId": "codex-default"
            }),
        });
        assert!(!allowed.is_error);
        let McpContentItem::Text { text } = &allowed.content[0] else {
            panic!("expected validation text")
        };
        assert!(text.contains("does not create a thread or grant approval"));
        assert!(text.contains("Work Codex"));
        assert!(!text.contains("codex-default"));

        let denied = server.call_tool(&McpCallToolParams {
            name: "validate_thread_proposal".to_string(),
            arguments: json!({
                "projectId": "project-work",
                "environmentId": "env-local",
                "providerInstanceId": "codex-personal"
            }),
        });
        assert!(denied.is_error);
        let McpContentItem::Text { text } = &denied.content[0] else {
            panic!("expected rejection text")
        };
        assert!(text.contains("Rejected before routing or scoring"));
        assert!(!text.contains("codex-personal"));
    }

    #[test]
    fn agent_access_tool_requires_non_null_fields_and_uses_server_context() {
        let temp = tempfile::tempdir().expect("temporary MCP mutation fixture");
        let paths = DataPaths::from_root(temp.path().join("app")).expect("data paths");
        paths.ensure_private_roots().expect("private roots");
        let store = Store::in_memory().expect("store");
        let settings = store
            .get_or_initialize_settings(1_000)
            .expect("settings state");
        let personal = PersonalRepoService::new(paths, store.clone());
        let mutations = MutationService::new(store.clone(), personal.clone());
        let server = McpServer::new(store.clone());
        let context = RequestContext {
            peer_role: PeerRole::McpAdapter,
            authenticated_environment_id: EnvironmentId::new("env-authenticated"),
            client_identity: ClientIdentity {
                description: "Authenticated MCP test".to_string(),
                ..ClientIdentity::default()
            },
            capability_scope: CapabilityScope::Mcp,
            connection_id: "mcp-context-owned".to_string(),
        };

        let null_field = server.call_tool_with_context(
            &context,
            &mutations,
            &McpCallToolParams {
                name: "propose_agent_access".to_string(),
                arguments: json!({
                    "requestId": "mcp-null",
                    "agentId": "agent-null",
                    "disabled": null,
                    "fullAccess": true,
                    "expectedRevision": settings.revision,
                }),
            },
        );
        assert!(null_field.is_error);

        let forged_context = server.call_tool_with_context(
            &context,
            &mutations,
            &McpCallToolParams {
                name: "propose_agent_access".to_string(),
                arguments: json!({
                    "requestId": "mcp-forged",
                    "agentId": "agent-forged",
                    "disabled": false,
                    "fullAccess": true,
                    "expectedRevision": settings.revision,
                    "peerRole": "local_control",
                    "environmentId": "env-forged",
                }),
            },
        );
        assert!(forged_context.is_error);

        let without_context = server.call_tool(&McpCallToolParams {
            name: "propose_agent_access".to_string(),
            arguments: json!({
                "requestId": "mcp-no-context",
                "agentId": "agent-no-context",
                "disabled": false,
                "fullAccess": true,
                "expectedRevision": settings.revision,
            }),
        });
        assert!(without_context.is_error);

        let proposed = server.call_tool_with_context(
            &context,
            &mutations,
            &McpCallToolParams {
                name: "propose_agent_access".to_string(),
                arguments: json!({
                    "requestId": "mcp-valid",
                    "agentId": "agent-valid",
                    "disabled": false,
                    "fullAccess": true,
                    "expectedRevision": settings.revision,
                }),
            },
        );
        assert!(!proposed.is_error);
        let McpContentItem::Text { text } = &proposed.content[0] else {
            panic!("expected proposal text")
        };
        let result: serde_json::Value = serde_json::from_str(text).expect("proposal result JSON");
        assert_eq!(result["created"], true);
        assert!(text.contains("No settings were changed"));
        let proposal_id = pandamux_core::MutationProposalId::new(
            result["proposalId"].as_str().expect("proposal identity"),
        );
        let proposal = store
            .get_mutation_proposal(&proposal_id)
            .expect("proposal query")
            .expect("stored proposal");
        assert_eq!(proposal.caller.peer_role, "mcp_adapter");
        assert_eq!(proposal.environment_id.as_str(), "env-authenticated");
        assert_eq!(
            proposal.resolution,
            pandamux_core::MutationResolution::Pending
        );
        let unchanged = store
            .get_or_initialize_settings(1_100)
            .expect("unchanged settings");
        assert_eq!(unchanged.revision, settings.revision);
        assert!(
            unchanged
                .settings
                .trusted_configuration
                .full_access_agent_ids
                .is_empty()
        );
        assert_eq!(
            store
                .list_mutation_proposals(true, None, 100)
                .expect("proposal list")
                .0
                .len(),
            1
        );
        personal.shutdown();
    }
}
