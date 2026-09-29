use crate::store::Store;
use pandamux_core::{
    EnvironmentId, ProjectId, ProviderInstanceId, ScheduleRecord, ScheduleTarget, UserSettings,
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
        ]
    }

    /// Execute an MCP tool.
    pub fn call_tool(&self, params: &McpCallToolParams) -> McpCallToolResult {
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
            unknown => McpCallToolResult {
                content: vec![McpContentItem::Text {
                    text: format!("Unknown tool: {unknown}"),
                }],
                is_error: true,
            },
        }
    }
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
}
