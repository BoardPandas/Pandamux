use serde_json::json;
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use pandamux_core::event::{ThreadEventKind, TurnOutcome};
use pandamux_core::provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind};
use pandamux_core::thread::WorktreeRef;
use pandamux_core::{
    AccessMode, AttemptBoundary, BroadsidePolicy, EnvironmentId, FailoverPool, FailoverPoolId,
    FailoverPoolMember, OrganizationId, OrganizationPolicy, ProjectId, ProviderInstanceId,
    RoutingCapabilityRequirements, SubscriptionKey, ThreadId, TurnAttemptStatus, TurnId,
    TurnStatus, UsageProvider, UsageSubscription, UserSettings,
};
use pandamux_protocol::{
    HubRoutingContext, RpcRequest, ServerRole, ThreadCreateParams, ThreadSendTurnParams,
};
use pandamux_providers::MockProviderDriver;
use pandamux_server::{ClientIdentity, DriverRegistry, RequestContext, Router, Store};

const FAILOVER_PROJECT_ID: &str = "project-failover";
const FAILOVER_ENVIRONMENT_ID: &str = "env-test";
const FAILING_PROVIDER_ID: &str = "mock-primary";
const FALLBACK_PROVIDER_ID: &str = "mock-fallback";

fn mock_capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        supports_streaming: true,
        supports_tool_calls: true,
        supports_sub_agents: true,
        supports_system_prompt: true,
        supports_reasoning: true,
        supports_approvals: true,
        supports_rate_limits: true,
        supports_resume: true,
        supports_images: true,
        max_access_mode: AccessMode::FullAccess,
    }
}

fn failover_provider(instance_id: &str, capacity_failure: Option<&str>) -> ProviderInstanceConfig {
    let mut settings = json!({ "instance": instance_id });
    if let Some(mode) = capacity_failure {
        settings["capacityFailure"] = json!(mode);
    }
    ProviderInstanceConfig {
        id: ProviderInstanceId::new(instance_id),
        provider: ProviderKind::Custom,
        display_name: format!("Mock {instance_id}"),
        profile_dir: format!("profiles/custom/{instance_id}"),
        environment_id: None,
        binary_path: Some(format!("/mock/bin/{instance_id}")),
        concurrency_limit: 2,
        env_overrides: BTreeMap::new(),
        settings,
        enabled: true,
        disabled_reason: None,
    }
}

fn configure_failover_policy(store: &Store, capacity_failure: Option<&str>) {
    let members = [
        (FAILING_PROVIDER_ID, "subkey-primary", 1_000),
        (FALLBACK_PROVIDER_ID, "subkey-fallback", 0),
    ];
    for (provider_id, subscription_key, _) in members {
        store
            .save_usage_subscription_mapping(
                &UsageSubscription {
                    key: SubscriptionKey::new(subscription_key),
                    provider: UsageProvider::Custom,
                    display_name: format!("Subscription {provider_id}"),
                    masked_account_label: None,
                },
                &EnvironmentId::new(FAILOVER_ENVIRONMENT_ID),
                &ProviderInstanceId::new(provider_id),
                100,
            )
            .expect("save failover subscription mapping");
    }

    let pool = FailoverPool {
        id: FailoverPoolId::new("pool-failover"),
        display_name: "Test failover pool".to_string(),
        members: members
            .into_iter()
            .map(
                |(provider_id, subscription_key, user_preference)| FailoverPoolMember {
                    provider_instance_id: ProviderInstanceId::new(provider_id),
                    subscription_key: SubscriptionKey::new(subscription_key),
                    model: "mock-fast".to_string(),
                    declared_capabilities: mock_capabilities(),
                    user_preference,
                },
            )
            .collect(),
        requirements: RoutingCapabilityRequirements {
            requires_instructions: true,
            requires_tool_calls: true,
            requires_approvals: true,
            requires_reasoning: false,
            access_mode: AccessMode::WorkspaceOnly,
        },
        revision: 1,
    };
    let settings = UserSettings {
        providers: vec![
            failover_provider(FAILING_PROVIDER_ID, capacity_failure),
            failover_provider(FALLBACK_PROVIDER_ID, None),
        ],
        organizations: vec![OrganizationPolicy {
            id: OrganizationId::new("org-failover"),
            display_name: "Failover organization".to_string(),
            project_ids: vec![ProjectId::new(FAILOVER_PROJECT_ID)],
            allowed_provider_instances: vec![
                ProviderInstanceId::new(FAILING_PROVIDER_ID),
                ProviderInstanceId::new(FALLBACK_PROVIDER_ID),
            ],
            allowed_subscriptions: vec![
                SubscriptionKey::new("subkey-primary"),
                SubscriptionKey::new("subkey-fallback"),
            ],
            failover_pool_ids: vec![pool.id.clone()],
            default_tier_mapping: None,
            max_concurrency: Some(2),
            daily_budget_micro_usd: None,
            broadside: BroadsidePolicy::default(),
            revision: 1,
        }],
        failover_pools: vec![pool],
        ..UserSettings::default()
    };
    store
        .save_settings_with_organization_policies(&settings, 101)
        .expect("save failover settings and policy");
}

fn init_test_git_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("create temporary git repository");
    let init = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .arg("init")
        .output()
        .expect("run git init");
    assert!(init.status.success());
    std::fs::write(repo.path().join("README.md"), "# Fixture\n").expect("write fixture");
    repo
}

async fn wait_for_turn_status(store: &Store, turn_id: &TurnId, expected: TurnStatus) {
    for _ in 0..100 {
        if store
            .get_turn(turn_id)
            .expect("load turn while waiting")
            .is_some_and(|turn| turn.status == expected)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let actual = store
        .get_turn(turn_id)
        .expect("load timed out turn")
        .map(|turn| turn.status);
    panic!("turn did not reach {expected:?}; actual status was {actual:?}");
}

async fn create_failover_thread(router: &Router, cwd: &str, request_id: i64) -> String {
    let response = router
        .handle_request(RpcRequest::new(
            request_id,
            "thread.create",
            Some(json!({
                "projectId": FAILOVER_PROJECT_ID,
                "environmentId": FAILOVER_ENVIRONMENT_ID,
                "providerInstanceId": FAILING_PROVIDER_ID,
                "model": "mock-fast",
                "cwd": cwd
            })),
        ))
        .await
        .expect("thread create response");
    assert!(response.is_success(), "thread create failed: {response:?}");
    response.result.expect("thread create result")["id"]
        .as_str()
        .expect("thread id")
        .to_string()
}

fn configure_mock_instance(store: &Store, instance_id: &str, limit: u32) {
    let settings = UserSettings {
        providers: vec![ProviderInstanceConfig {
            id: ProviderInstanceId::new(instance_id),
            provider: ProviderKind::Custom,
            display_name: "Mock Provider".to_string(),
            profile_dir: format!("profiles/custom/{instance_id}"),
            environment_id: None,
            binary_path: Some(format!("/mock/bin/{instance_id}")),
            concurrency_limit: limit,
            env_overrides: BTreeMap::new(),
            settings: json!({ "instance": instance_id }),
            enabled: true,
            disabled_reason: None,
        }],
        ..UserSettings::default()
    };
    store.save_settings(&settings).expect("save mock settings");
}

#[tokio::test]
async fn test_thread_approval_flow() {
    let store = Store::in_memory().expect("open store");
    configure_mock_instance(&store, "mock-default", 2);
    let drivers = Arc::new(DriverRegistry::new());
    let mock = Arc::new(MockProviderDriver::new(
        ProviderKind::Custom,
        "Mock Provider",
    ));
    drivers.register(ProviderKind::Custom, mock);

    let router = Router::with_drivers(
        store.clone(),
        ServerRole::Hub,
        "env-test".to_string(),
        drivers.clone(),
    );

    // 1. Create thread
    let create_req = RpcRequest::new(
        1,
        "thread.create",
        Some(json!({
            "title": "Approval Test",
            "providerInstanceId": "mock-default",
            "model": "mock-fast"
        })),
    );
    let create_res = router
        .handle_request(create_req)
        .await
        .expect("create response");
    assert!(create_res.is_success());
    let thread_id = create_res.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut event_rx = router.thread_manager().subscribe();

    // 2. Send turn prompting approval request
    let send_req = RpcRequest::new(
        2,
        "thread.send_turn",
        Some(json!({
            "threadId": thread_id,
            "text": "request_approval: rm -rf /cache"
        })),
    );
    let send_res = router
        .handle_request(send_req)
        .await
        .expect("send response");
    assert!(send_res.is_success());

    // 3. Await ApprovalRequested event
    let mut got_approval_req = false;
    let mut request_id = String::new();

    for _ in 0..10 {
        if let Ok(Ok(env)) = timeout(Duration::from_millis(500), event_rx.recv()).await
            && let ThreadEventKind::ApprovalRequested {
                request_id: ref req_id,
                ..
            } = env.kind
        {
            got_approval_req = true;
            request_id = req_id.clone();
            break;
        }
    }
    assert!(
        got_approval_req,
        "Should have received ApprovalRequested event"
    );

    // Verify thread is now awaiting approval
    let get_req = RpcRequest::new(3, "thread.get", Some(json!({ "threadId": thread_id })));
    let get_res = router.handle_request(get_req).await.expect("get response");
    assert_eq!(
        get_res.result.unwrap()["thread"]["status"],
        "awaiting_approval"
    );

    // 4. Respond to approval
    let resp_req = RpcRequest::new(
        4,
        "thread.respond_approval",
        Some(json!({
            "threadId": thread_id,
            "requestId": request_id.clone(),
            "decision": "denied"
        })),
    );
    let resp_res = router
        .handle_request(resp_req)
        .await
        .expect("respond response");
    assert!(resp_res.is_success());

    // 5. Await turn settlement
    let mut turn_settled = false;
    for _ in 0..10 {
        if let Ok(Ok(env)) = timeout(Duration::from_millis(500), event_rx.recv()).await
            && let ThreadEventKind::TurnSettled { .. } = env.kind
        {
            turn_settled = true;
            break;
        }
    }
    assert!(
        turn_settled,
        "Should have received TurnSettled event after approval"
    );

    // Verify thread status settled to Idle
    let final_get = router
        .handle_request(RpcRequest::new(
            5,
            "thread.get",
            Some(json!({ "threadId": thread_id })),
        ))
        .await
        .expect("get response");
    assert_eq!(final_get.result.unwrap()["thread"]["status"], "idle");

    let persisted = store
        .get_events(&ThreadId::from(thread_id.as_str()), None)
        .expect("load approval lifecycle events");
    assert!(
        persisted
            .windows(2)
            .all(|events| events[1].seq == events[0].seq + 1),
        "approval continuation must not collide with the resolved event sequence"
    );
    let resolved_index = persisted
        .iter()
        .position(|event| matches!(event.kind, ThreadEventKind::ApprovalResolved { .. }))
        .expect("persist approval resolution");
    let continuation_index = persisted
        .iter()
        .position(|event| matches!(event.kind, ThreadEventKind::AssistantText { .. }))
        .expect("persist provider continuation after approval");
    assert!(resolved_index < continuation_index);

    // A later turn may reuse the same provider request id. Its durable delivery
    // identity remains separate because the turn id is part of the key.
    let second_send = router
        .handle_request(RpcRequest::new(
            6,
            "thread.send_turn",
            Some(json!({
                "threadId": thread_id,
                "text": "request_approval: echo second"
            })),
        ))
        .await
        .expect("second send response");
    assert!(second_send.is_success());
    let mut second_request_id = None;
    for _ in 0..10 {
        if let Ok(Ok(env)) = timeout(Duration::from_millis(500), event_rx.recv()).await
            && let ThreadEventKind::ApprovalRequested { request_id, .. } = env.kind
        {
            second_request_id = Some(request_id);
            break;
        }
    }
    let second_request_id = second_request_id.expect("second approval request");
    assert_eq!(second_request_id, request_id);
    let second_approval = router
        .handle_request(RpcRequest::new(
            7,
            "thread.respond_approval",
            Some(json!({
                "threadId": thread_id,
                "requestId": second_request_id,
                "decision": "approved"
            })),
        ))
        .await
        .expect("second approval response");
    assert!(second_approval.is_success());
    let mut second_turn_settled = false;
    for _ in 0..10 {
        if let Ok(Ok(env)) = timeout(Duration::from_millis(500), event_rx.recv()).await
            && let ThreadEventKind::TurnSettled { .. } = env.kind
        {
            second_turn_settled = true;
            break;
        }
    }
    assert!(second_turn_settled, "second approval turn must settle");

    router.thread_manager().shutdown_all().await;
    let restarted = Router::with_drivers(
        store.clone(),
        ServerRole::Hub,
        "env-test".to_string(),
        drivers,
    );
    let duplicate = restarted
        .handle_request(RpcRequest::new(
            8,
            "thread.respondApproval",
            Some(json!({
                "threadId": thread_id,
                "requestId": request_id,
                "decision": "denied"
            })),
        ))
        .await
        .expect("duplicate approval response");
    assert!(
        duplicate.is_success(),
        "delivered approval must be idempotent after manager restart: {duplicate:?}"
    );
    let persisted = store
        .get_events(&ThreadId::from(thread_id.as_str()), None)
        .expect("reload approval events");
    assert_eq!(
        persisted
            .iter()
            .filter(|event| matches!(event.kind, ThreadEventKind::ApprovalResolved { .. }))
            .count(),
        2
    );
}

#[tokio::test]
async fn test_thread_interrupt_flow() {
    let store = Store::in_memory().expect("open store");
    configure_mock_instance(&store, "mock-default", 2);
    let drivers = Arc::new(DriverRegistry::new());
    let mock = Arc::new(MockProviderDriver::new(
        ProviderKind::Custom,
        "Mock Provider",
    ));
    drivers.register(ProviderKind::Custom, mock);

    let router = Router::with_drivers(
        store.clone(),
        ServerRole::Hub,
        "env-test".to_string(),
        drivers,
    );

    // 1. Create thread
    let create_req = RpcRequest::new(
        1,
        "thread.create",
        Some(json!({
            "title": "Interrupt Test",
            "providerInstanceId": "mock-default",
            "model": "mock-fast"
        })),
    );
    let create_res = router
        .handle_request(create_req)
        .await
        .expect("create response");
    let thread_id = create_res.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut event_rx = router.thread_manager().subscribe();

    // 2. Send turn prompting approval request (stalling until answered)
    let send_req = RpcRequest::new(
        2,
        "thread.send_turn",
        Some(json!({
            "threadId": thread_id,
            "text": "request_approval: deploy_production"
        })),
    );
    router
        .handle_request(send_req)
        .await
        .expect("send response");

    // Wait for approval request event
    for _ in 0..10 {
        if let Ok(Ok(env)) = timeout(Duration::from_millis(500), event_rx.recv()).await
            && let ThreadEventKind::ApprovalRequested { .. } = env.kind
        {
            break;
        }
    }

    // 3. Interrupt the turn
    let cancel_req = RpcRequest::new(
        3,
        "thread.cancel_turn",
        Some(json!({ "threadId": thread_id })),
    );
    let cancel_res = router
        .handle_request(cancel_req)
        .await
        .expect("cancel response");
    assert!(cancel_res.is_success());

    tokio::time::sleep(Duration::from_millis(100)).await;

    // 4. Verify thread is back to Idle and turn is Interrupted
    let get_req = RpcRequest::new(4, "thread.get", Some(json!({ "threadId": thread_id })));
    let get_res = router.handle_request(get_req).await.expect("get response");
    let res_val = get_res.result.unwrap();
    assert_eq!(res_val["thread"]["status"], "idle");
    assert_eq!(res_val["turns"][0]["status"], "interrupted");

    let persisted = store
        .get_events(&ThreadId::from(thread_id.as_str()), None)
        .expect("load cancellation lifecycle events");
    assert!(
        persisted
            .windows(2)
            .all(|events| events[1].seq == events[0].seq + 1),
        "cancellation events must remain contiguous"
    );
    assert!(persisted.iter().any(|event| matches!(
        event.kind,
        ThreadEventKind::TurnCompleted {
            outcome: TurnOutcome::Interrupted
        }
    )));
    assert!(matches!(
        persisted.last().map(|event| &event.kind),
        Some(ThreadEventKind::TurnSettled { .. })
    ));
    assert!(!persisted.iter().any(|event| matches!(
        &event.kind,
        ThreadEventKind::Error { class, .. } if class == "provider_process"
    )));
}

#[tokio::test]
async fn provider_stream_close_emits_failed_terminal_sequence() {
    let store = Store::in_memory().expect("open store");
    configure_mock_instance(&store, "mock-default", 2);
    let drivers = Arc::new(DriverRegistry::new());
    drivers.register(
        ProviderKind::Custom,
        Arc::new(MockProviderDriver::new(
            ProviderKind::Custom,
            "Mock Provider",
        )),
    );
    let router = Router::with_drivers(
        store.clone(),
        ServerRole::Hub,
        "env-test".to_string(),
        drivers,
    );
    let mut events = router.thread_manager().subscribe();

    let created = router
        .handle_request(RpcRequest::new(
            1,
            "thread.create",
            Some(json!({
                "title": "Stream Close Test",
                "providerInstanceId": "mock-default",
                "model": "mock-fast"
            })),
        ))
        .await
        .expect("create response");
    let thread_id = created.result.expect("thread result")["id"]
        .as_str()
        .expect("thread id")
        .to_string();

    let sent = router
        .handle_request(RpcRequest::new(
            2,
            "thread.send_turn",
            Some(json!({
                "threadId": thread_id,
                "text": "close_stream"
            })),
        ))
        .await
        .expect("send response");
    assert!(sent.is_success());

    let mut settled = false;
    for _ in 0..10 {
        if let Ok(Ok(event)) = timeout(Duration::from_millis(500), events.recv()).await
            && matches!(event.kind, ThreadEventKind::TurnSettled { .. })
        {
            settled = true;
            break;
        }
    }
    assert!(settled, "stream closure must settle the failed turn");

    let persisted = store
        .get_events(&ThreadId::from(thread_id.as_str()), None)
        .expect("load failed lifecycle events");
    assert!(
        persisted
            .windows(2)
            .all(|events| events[1].seq == events[0].seq + 1),
        "failed terminal events must remain contiguous"
    );
    assert!(persisted.iter().any(|event| matches!(
        event.kind,
        ThreadEventKind::TurnCompleted {
            outcome: TurnOutcome::Failed
        }
    )));
    assert!(matches!(
        persisted.last().map(|event| &event.kind),
        Some(ThreadEventKind::TurnSettled { .. })
    ));

    let final_get = router
        .handle_request(RpcRequest::new(
            3,
            "thread.get",
            Some(json!({ "threadId": thread_id })),
        ))
        .await
        .expect("get response");
    let result = final_get.result.expect("get result");
    assert_eq!(result["thread"]["status"], "errored");
    assert_eq!(result["turns"][0]["status"], "failed");
}

#[tokio::test]
async fn test_thread_resumption_across_restart() {
    let store = Store::in_memory().expect("open store");
    configure_mock_instance(&store, "mock-default", 2);

    // Run first turn with Driver 1 issuing resume token
    let drivers1 = Arc::new(DriverRegistry::new());
    let mock1 = Arc::new(
        MockProviderDriver::new(ProviderKind::Custom, "Mock Provider 1")
            .with_resume_token("res-tok-abc-123"),
    );
    drivers1.register(ProviderKind::Custom, mock1);

    let router1 = Router::with_drivers(
        store.clone(),
        ServerRole::Hub,
        "env-test".to_string(),
        drivers1,
    );

    let create_req = RpcRequest::new(
        1,
        "thread.create",
        Some(json!({
            "title": "Resume Test",
            "providerInstanceId": "mock-default",
            "model": "mock-fast"
        })),
    );
    let create_res = router1
        .handle_request(create_req)
        .await
        .expect("create response");
    let thread_id = create_res.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut event_rx1 = router1.thread_manager().subscribe();

    let send1 = RpcRequest::new(
        2,
        "thread.send_turn",
        Some(json!({
            "threadId": thread_id,
            "text": "First turn query"
        })),
    );
    router1
        .handle_request(send1)
        .await
        .expect("send 1 response");

    // Await first turn settle
    for _ in 0..10 {
        if let Ok(Ok(env)) = timeout(Duration::from_millis(500), event_rx1.recv()).await
            && let ThreadEventKind::TurnSettled { .. } = env.kind
        {
            break;
        }
    }

    // Verify resume token stored in SQLite
    let resume_req = RpcRequest::new(3, "thread.resume", Some(json!({ "threadId": thread_id })));
    let resume_res = router1
        .handle_request(resume_req)
        .await
        .expect("resume response");
    assert_eq!(
        resume_res.result.unwrap()["resumeToken"].as_str().unwrap(),
        "res-tok-abc-123"
    );

    // Simulated Server Restart: create new DriverRegistry and new Router against same Store
    let drivers2 = Arc::new(DriverRegistry::new());
    let mock2 = Arc::new(
        MockProviderDriver::new(ProviderKind::Custom, "Mock Provider 2")
            .with_resume_token("res-tok-def-456"),
    );
    drivers2.register(ProviderKind::Custom, mock2.clone());

    let router2 = Router::with_drivers(
        store.clone(),
        ServerRole::Hub,
        "env-test".to_string(),
        drivers2,
    );
    let mut event_rx2 = router2.thread_manager().subscribe();

    // Send second turn after restart
    let send2 = RpcRequest::new(
        4,
        "thread.send_turn",
        Some(json!({
            "threadId": thread_id,
            "text": "Second turn continuation"
        })),
    );
    let send2_res = router2
        .handle_request(send2)
        .await
        .expect("send 2 response");
    assert!(send2_res.is_success());

    // Await second turn settle
    for _ in 0..10 {
        if let Ok(Ok(env)) = timeout(Duration::from_millis(500), event_rx2.recv()).await
            && let ThreadEventKind::TurnSettled { .. } = env.kind
        {
            break;
        }
    }

    // Verify driver 2 received previous resumption token
    let tokens = mock2.received_resume_tokens();
    assert_eq!(tokens, vec!["res-tok-abc-123".to_string()]);
}

#[tokio::test]
async fn exact_provider_instance_flows_through_health_login_process_turn_and_audit() {
    let store = Store::in_memory().expect("open store");
    let temp_dir = tempfile::tempdir().expect("temp data directory");
    let instance = |id: &str, account: &str| ProviderInstanceConfig {
        id: ProviderInstanceId::new(id),
        provider: ProviderKind::Antigravity,
        display_name: format!("Antigravity {account}"),
        profile_dir: format!("profiles/antigravity/{account}"),
        environment_id: None,
        binary_path: Some(format!("/mock/bin/{account}")),
        concurrency_limit: 1,
        env_overrides: BTreeMap::from([("ACCOUNT".to_string(), account.to_string())]),
        settings: json!({ "account": account }),
        enabled: true,
        disabled_reason: None,
    };
    store
        .save_settings(&UserSettings {
            providers: vec![
                instance("agy-work", "work"),
                instance("agy-personal", "personal"),
            ],
            ..UserSettings::default()
        })
        .expect("save provider instances");

    let drivers = Arc::new(DriverRegistry::new());
    let mock = Arc::new(MockProviderDriver::new(
        ProviderKind::Antigravity,
        "Mock Antigravity",
    ));
    drivers.register(ProviderKind::Antigravity, mock.clone());
    let router = Router::with_drivers(
        store.clone(),
        ServerRole::Hub,
        "env-test".to_string(),
        drivers,
    )
    .with_data_dir(temp_dir.path().to_path_buf());
    let mut events = router.thread_manager().subscribe();

    let mut observed_turns = Vec::new();
    for (request_id, instance_id) in [(1, "agy-work"), (10, "agy-personal")] {
        let created = router
            .handle_request(RpcRequest::new(
                request_id,
                "thread.create",
                Some(json!({
                    "title": instance_id,
                    "providerInstanceId": instance_id,
                    "model": "mock-fast"
                })),
            ))
            .await
            .expect("create response");
        assert!(created.is_success());
        let thread_id = created.result.expect("thread result")["id"]
            .as_str()
            .expect("thread id")
            .to_string();
        let sent = router
            .handle_request(RpcRequest::new(
                request_id + 1,
                "thread.send_turn",
                Some(json!({
                    "threadId": thread_id,
                    "text": format!("hello from {instance_id}")
                })),
            ))
            .await
            .expect("send response");
        assert!(sent.is_success());
        let turn_id: pandamux_core::TurnId =
            serde_json::from_value(sent.result.expect("turn result")["turnId"].clone())
                .expect("typed turn id");
        observed_turns.push((instance_id.to_string(), turn_id));

        for _ in 0..10 {
            if let Ok(Ok(event)) = timeout(Duration::from_millis(500), events.recv()).await
                && matches!(event.kind, ThreadEventKind::TurnSettled { .. })
            {
                break;
            }
        }
    }

    let received = mock.received_configs();
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].id.as_str(), "agy-work");
    assert_eq!(received[0].settings["account"], "work");
    assert_eq!(received[0].env_overrides["ACCOUNT"], "work");
    assert_eq!(received[0].binary_path.as_deref(), Some("/mock/bin/work"));
    assert_eq!(
        received[0].profile_dir,
        temp_dir
            .path()
            .join("profiles/antigravity/work")
            .to_string_lossy()
    );
    assert_eq!(received[1].id.as_str(), "agy-personal");
    assert_eq!(received[1].settings["account"], "personal");
    assert_eq!(received[1].env_overrides["ACCOUNT"], "personal");
    assert_eq!(
        received[1].profile_dir,
        temp_dir
            .path()
            .join("profiles/antigravity/personal")
            .to_string_lossy()
    );

    let health = router
        .handle_request(RpcRequest::new(
            30,
            "provider.health",
            Some(json!({
                "instanceId": "agy-personal",
                "environmentId": "env-test"
            })),
        ))
        .await
        .expect("health response");
    assert_eq!(
        health.result.expect("health result")["reports"][0]["instanceId"],
        "agy-personal"
    );
    let login = router
        .handle_request(RpcRequest::new(
            31,
            "antigravity.oauth_start",
            Some(json!({
                "providerInstanceId": "agy-personal",
                "environmentId": "env-test"
            })),
        ))
        .await
        .expect("login response");
    let login_error = login.error.expect("uninstalled login must fail");
    assert_eq!(
        login_error.data.expect("typed login error")["providerInstanceId"],
        "agy-personal"
    );

    for (instance_id, turn_id) in observed_turns {
        let turn = store
            .get_turn(&turn_id)
            .expect("get turn")
            .expect("stored turn");
        assert_eq!(turn.environment_id.as_str(), "env-test");
        assert_eq!(turn.provider_instance_id.as_str(), instance_id);
        let audit = store
            .get_provider_execution_audit(&turn_id)
            .expect("get audit")
            .expect("stored audit");
        assert_eq!(audit.environment_id.as_str(), "env-test");
        assert_eq!(audit.provider_instance_id.as_str(), instance_id);
        assert_eq!(audit.provider_kind, ProviderKind::Antigravity);
        assert!(audit.capabilities.supports_streaming);
    }
}

#[tokio::test]
async fn provider_capacity_is_held_for_process_lifetime_and_released_on_cancel() {
    let store = Store::in_memory().expect("open store");
    configure_mock_instance(&store, "mock-one", 1);
    let drivers = Arc::new(DriverRegistry::new());
    drivers.register(
        ProviderKind::Custom,
        Arc::new(MockProviderDriver::new(
            ProviderKind::Custom,
            "Mock Provider",
        )),
    );
    let router = Router::with_drivers(store, ServerRole::Hub, "env-test".to_string(), drivers);
    let mut events = router.thread_manager().subscribe();

    let mut thread_ids = Vec::new();
    for request_id in [1, 2] {
        let response = router
            .handle_request(RpcRequest::new(
                request_id,
                "thread.create",
                Some(json!({
                    "providerInstanceId": "mock-one",
                    "model": "mock-fast"
                })),
            ))
            .await
            .expect("create response");
        thread_ids.push(
            response.result.expect("thread result")["id"]
                .as_str()
                .expect("thread id")
                .to_string(),
        );
    }

    let first = router
        .handle_request(RpcRequest::new(
            3,
            "thread.send_turn",
            Some(json!({
                "threadId": thread_ids[0],
                "text": "request_approval: hold capacity"
            })),
        ))
        .await
        .expect("first send response");
    assert!(first.is_success());
    for _ in 0..10 {
        if let Ok(Ok(event)) = timeout(Duration::from_millis(500), events.recv()).await
            && matches!(event.kind, ThreadEventKind::ApprovalRequested { .. })
        {
            break;
        }
    }

    let rejected = router
        .handle_request(RpcRequest::new(
            4,
            "thread.send_turn",
            Some(json!({
                "threadId": thread_ids[1],
                "text": "must wait"
            })),
        ))
        .await
        .expect("capacity response");
    assert_eq!(
        rejected
            .error
            .expect("capacity error")
            .data
            .expect("capacity data")["reason"],
        "provider_capacity_reached"
    );

    let cancelled = router
        .handle_request(RpcRequest::new(
            5,
            "thread.cancel_turn",
            Some(json!({ "threadId": thread_ids[0] })),
        ))
        .await
        .expect("cancel response");
    assert!(cancelled.is_success());
    assert_eq!(
        router.provider_resolver().active_processes(
            &ProviderInstanceId::new("mock-one"),
            &pandamux_core::EnvironmentId::new("env-test")
        ),
        0
    );

    let accepted = router
        .handle_request(RpcRequest::new(
            6,
            "thread.send_turn",
            Some(json!({
                "threadId": thread_ids[1],
                "text": "capacity released"
            })),
        ))
        .await
        .expect("second send response");
    assert!(accepted.is_success());
}

#[tokio::test]
async fn missing_provider_instance_never_falls_back_to_custom() {
    let store = Store::in_memory().expect("open store");
    configure_mock_instance(&store, "mock-known", 1);
    let drivers = Arc::new(DriverRegistry::new());
    drivers.register(
        ProviderKind::Custom,
        Arc::new(MockProviderDriver::new(
            ProviderKind::Custom,
            "Mock Provider",
        )),
    );
    let router = Router::with_drivers(store, ServerRole::Hub, "env-test".to_string(), drivers);
    let response = router
        .handle_request(RpcRequest::new(
            1,
            "thread.create",
            Some(json!({ "providerInstanceId": "unknown-instance" })),
        ))
        .await
        .expect("create response");
    assert_eq!(
        response
            .error
            .expect("missing instance error")
            .data
            .expect("missing instance data")["reason"],
        "provider_instance_missing"
    );
}

#[tokio::test]
async fn capacity_before_output_retries_once_on_an_allowed_pool_member() {
    let store = Store::in_memory().expect("open store");
    configure_failover_policy(&store, Some("before"));
    let drivers = Arc::new(DriverRegistry::new());
    let mock = Arc::new(MockProviderDriver::new(
        ProviderKind::Custom,
        "Mock Provider",
    ));
    drivers.register(ProviderKind::Custom, mock.clone());
    let router = Router::with_drivers(
        store.clone(),
        ServerRole::Hub,
        FAILOVER_ENVIRONMENT_ID.to_string(),
        drivers,
    );
    let repo = init_test_git_repo();
    let thread_id =
        create_failover_thread(&router, repo.path().to_str().expect("repository path"), 100).await;

    let response = router
        .handle_request(RpcRequest::new(
            101,
            "thread.send_turn",
            Some(json!({ "threadId": thread_id, "text": "safe retry fixture" })),
        ))
        .await
        .expect("send response");
    assert!(response.is_success(), "turn send failed: {response:?}");
    let result = response.result.expect("send result");
    assert_eq!(
        result["routing"]["selectedProviderInstanceId"],
        FAILING_PROVIDER_ID
    );
    let turn_id = TurnId::new(result["turnId"].as_str().expect("turn id"));
    wait_for_turn_status(&store, &turn_id, TurnStatus::Completed).await;

    let attempts = store
        .list_turn_attempts(&turn_id)
        .expect("load retry attempts");
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].ordinal, 1);
    assert_eq!(
        attempts[0].provider_instance_id.as_str(),
        FAILING_PROVIDER_ID
    );
    assert_eq!(attempts[0].boundary, AttemptBoundary::SafeToRetry);
    assert_eq!(
        attempts[0].status,
        TurnAttemptStatus::InterruptedForCapacity
    );
    assert!(attempts[0].capacity_evidence.is_some());
    assert!(
        attempts[0]
            .execution
            .as_ref()
            .expect("primary execution evidence")
            .profile_dir
            .ends_with(&format!("profiles/custom/{FAILING_PROVIDER_ID}"))
    );
    assert_eq!(attempts[1].ordinal, 2);
    assert_eq!(
        attempts[1].provider_instance_id.as_str(),
        FALLBACK_PROVIDER_ID
    );
    assert_eq!(attempts[1].status, TurnAttemptStatus::Completed);
    assert!(
        attempts[1]
            .execution
            .as_ref()
            .expect("fallback execution evidence")
            .profile_dir
            .ends_with(&format!("profiles/custom/{FALLBACK_PROVIDER_ID}"))
    );
    let completed_turn = store
        .get_turn(&turn_id)
        .expect("load completed turn")
        .expect("completed turn exists");
    assert_eq!(
        completed_turn.provider_instance_id.as_str(),
        FALLBACK_PROVIDER_ID
    );
    assert_eq!(
        store
            .reconcile_active_turns()
            .expect("reconcile completed retry"),
        0
    );
    assert_eq!(
        store
            .get_turn_attempt(&attempts[0].id)
            .expect("reload exhausted attempt")
            .expect("exhausted attempt exists")
            .status,
        TurnAttemptStatus::InterruptedForCapacity
    );
    assert_eq!(mock.session_start_count(), 2);
    assert!(
        store
            .list_continuations(&ThreadId::new(&thread_id))
            .expect("list continuations")
            .is_empty()
    );
    let events = store
        .get_events(&ThreadId::new(&thread_id), None)
        .expect("load events");
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            ThreadEventKind::Notice { message, .. }
                if message.contains("Retrying once") && message.contains(FALLBACK_PROVIDER_ID)
        )
    }));
}

#[tokio::test]
async fn node_proxy_routes_with_hub_policy_without_a_local_organization_copy() {
    let hub_store = Store::in_memory().expect("open Hub store");
    configure_failover_policy(&hub_store, Some("before"));
    let hub_settings = hub_store
        .get_settings()
        .expect("load Hub settings")
        .expect("Hub settings exist");
    let hub_routing_context = HubRoutingContext {
        organization: hub_settings.organizations[0].clone(),
        failover_pools: hub_settings.failover_pools.clone(),
    };

    let node_store = Store::in_memory().expect("open node store");
    for (provider_id, subscription_key) in [
        (FAILING_PROVIDER_ID, "subkey-primary"),
        (FALLBACK_PROVIDER_ID, "subkey-fallback"),
    ] {
        node_store
            .save_usage_subscription_mapping(
                &UsageSubscription {
                    key: SubscriptionKey::new(subscription_key),
                    provider: UsageProvider::Custom,
                    display_name: format!("Subscription {provider_id}"),
                    masked_account_label: None,
                },
                &EnvironmentId::new(FAILOVER_ENVIRONMENT_ID),
                &ProviderInstanceId::new(provider_id),
                100,
            )
            .expect("save node subscription mapping");
    }
    node_store
        .save_settings(&UserSettings {
            providers: hub_settings.providers,
            ..UserSettings::default()
        })
        .expect("save node provider settings");
    assert!(
        node_store
            .organization_policy_for_project(&ProjectId::new(FAILOVER_PROJECT_ID))
            .is_err(),
        "the node fixture must not contain a local organization policy"
    );

    let drivers = Arc::new(DriverRegistry::new());
    let mock = Arc::new(MockProviderDriver::new(
        ProviderKind::Custom,
        "Mock Provider",
    ));
    drivers.register(ProviderKind::Custom, mock.clone());
    let router = Router::with_drivers(
        node_store.clone(),
        ServerRole::Node,
        FAILOVER_ENVIRONMENT_ID.to_string(),
        drivers,
    );
    let repo = init_test_git_repo();
    let thread = router
        .thread_manager()
        .create_thread_from_hub(ThreadCreateParams {
            project_id: Some(ProjectId::new(FAILOVER_PROJECT_ID)),
            environment_id: Some(EnvironmentId::new(FAILOVER_ENVIRONMENT_ID)),
            title: Some("Remote failover".to_string()),
            provider_instance_id: Some(ProviderInstanceId::new(FAILING_PROVIDER_ID)),
            model: Some("mock-fast".to_string()),
            effort: None,
            access_mode: Some(AccessMode::WorkspaceOnly),
            agent_id: None,
            cwd: Some(repo.path().to_string_lossy().into_owned()),
            worktree: None,
        })
        .expect("create Hub-authorized node thread");
    let context = RequestContext::node_proxy(
        EnvironmentId::new(FAILOVER_ENVIRONMENT_ID),
        ClientIdentity::default(),
    );
    let missing_context = router
        .handle_request_with_context(
            &context,
            RpcRequest::new(
                148,
                "thread.send_turn",
                Some(
                    serde_json::to_value(ThreadSendTurnParams {
                        thread_id: thread.id.clone(),
                        text: "missing Hub policy".to_string(),
                        attachment_ids: Vec::new(),
                        model: None,
                        effort: None,
                        hub_routing_context: None,
                    })
                    .expect("encode missing-context turn"),
                ),
            ),
        )
        .await
        .expect("missing-context response");
    assert_eq!(
        missing_context
            .error
            .expect("missing Hub context must fail")
            .data
            .expect("missing Hub context error data")["reason"],
        "organization_policy_denied"
    );
    let response = router
        .handle_request_with_context(
            &context,
            RpcRequest::new(
                149,
                "thread.send_turn",
                Some(
                    serde_json::to_value(ThreadSendTurnParams {
                        thread_id: thread.id,
                        text: "remote safe retry".to_string(),
                        attachment_ids: Vec::new(),
                        model: None,
                        effort: None,
                        hub_routing_context: Some(hub_routing_context),
                    })
                    .expect("encode node turn"),
                ),
            ),
        )
        .await
        .expect("node turn response");
    assert!(response.is_success(), "node turn failed: {response:?}");
    let turn_id = TurnId::new(
        response.result.expect("node turn result")["turnId"]
            .as_str()
            .expect("node turn id"),
    );
    wait_for_turn_status(&node_store, &turn_id, TurnStatus::Completed).await;
    let attempts = node_store
        .list_turn_attempts(&turn_id)
        .expect("load node attempts");
    assert_eq!(attempts.len(), 2);
    assert_eq!(
        attempts[0].provider_instance_id.as_str(),
        FAILING_PROVIDER_ID
    );
    assert_eq!(
        attempts[1].provider_instance_id.as_str(),
        FALLBACK_PROVIDER_ID
    );
    assert_eq!(mock.session_start_count(), 2);
}

#[tokio::test]
async fn automatic_failover_switch_preserves_an_explicit_capacity_interruption() {
    let store = Store::in_memory().expect("open store");
    configure_failover_policy(&store, Some("before"));
    let mut settings = store
        .get_settings()
        .expect("load settings")
        .expect("settings exist");
    settings.advanced.automatic_failover_enabled = false;
    store
        .save_settings(&settings)
        .expect("disable automatic failover");
    let drivers = Arc::new(DriverRegistry::new());
    let mock = Arc::new(MockProviderDriver::new(
        ProviderKind::Custom,
        "Mock Provider",
    ));
    drivers.register(ProviderKind::Custom, mock.clone());
    let router = Router::with_drivers(
        store.clone(),
        ServerRole::Hub,
        FAILOVER_ENVIRONMENT_ID.to_string(),
        drivers,
    );
    let repo = init_test_git_repo();
    let thread_id =
        create_failover_thread(&router, repo.path().to_str().expect("repository path"), 150).await;
    let response = router
        .handle_request(RpcRequest::new(
            151,
            "thread.send_turn",
            Some(json!({ "threadId": thread_id, "text": "manual failover only" })),
        ))
        .await
        .expect("send response");
    let result = response.result.expect("send result");
    let turn_id = TurnId::new(result["turnId"].as_str().expect("turn id"));
    wait_for_turn_status(&store, &turn_id, TurnStatus::InterruptedForCapacity).await;

    let attempts = store.list_turn_attempts(&turn_id).expect("load attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].boundary, AttemptBoundary::SafeToRetry);
    assert_eq!(
        attempts[0].status,
        TurnAttemptStatus::InterruptedForCapacity
    );
    assert_eq!(mock.session_start_count(), 1);
    assert_eq!(
        store
            .list_continuations(&ThreadId::new(&thread_id))
            .expect("load explicit continuation")
            .len(),
        1
    );
}

#[tokio::test]
async fn capacity_after_output_or_action_never_replays_the_turn() {
    for (case_index, (mode, expected_boundary)) in [
        ("after_text", AttemptBoundary::VisibleOutputBegun),
        ("after_file", AttemptBoundary::ToolSideEffectBegun),
        ("after_command", AttemptBoundary::ToolSideEffectBegun),
        ("after_network", AttemptBoundary::ToolSideEffectBegun),
        ("after_approval", AttemptBoundary::ExternallyCommitted),
    ]
    .into_iter()
    .enumerate()
    {
        let store = Store::in_memory().expect("open store");
        configure_failover_policy(&store, Some(mode));
        let drivers = Arc::new(DriverRegistry::new());
        let mock = Arc::new(MockProviderDriver::new(
            ProviderKind::Custom,
            "Mock Provider",
        ));
        drivers.register(ProviderKind::Custom, mock.clone());
        let router = Router::with_drivers(
            store.clone(),
            ServerRole::Hub,
            FAILOVER_ENVIRONMENT_ID.to_string(),
            drivers,
        );
        let repo = init_test_git_repo();
        let thread_id = create_failover_thread(
            &router,
            repo.path().to_str().expect("repository path"),
            200 + case_index as i64 * 10,
        )
        .await;
        let response = router
            .handle_request(RpcRequest::new(
                201 + case_index as i64 * 10,
                "thread.send_turn",
                Some(json!({ "threadId": thread_id, "text": mode })),
            ))
            .await
            .expect("send response");
        assert!(
            response.is_success(),
            "turn send failed for {mode}: {response:?}"
        );
        let result = response.result.expect("send result");
        let turn_id = TurnId::new(result["turnId"].as_str().expect("turn id"));
        wait_for_turn_status(&store, &turn_id, TurnStatus::InterruptedForCapacity).await;

        let attempts = store
            .list_turn_attempts(&turn_id)
            .expect("load interrupted attempt");
        assert_eq!(attempts.len(), 1, "{mode} must not be replayed");
        assert_eq!(attempts[0].boundary, expected_boundary, "mode {mode}");
        assert_eq!(
            attempts[0].status,
            TurnAttemptStatus::InterruptedForCapacity,
            "mode {mode}"
        );
        assert_eq!(
            attempts[0]
                .capacity_evidence
                .as_ref()
                .expect("capacity evidence")
                .provider_instance_id
                .as_str(),
            FAILING_PROVIDER_ID
        );
        if mode == "after_text" {
            assert_eq!(attempts[0].partial_output, "partial mock output");
        } else if mode != "after_approval" {
            let ledger = store.list_tool_ledger(&turn_id).expect("load tool ledger");
            assert_eq!(ledger.len(), 1, "mode {mode}");
            assert_eq!(ledger[0].status, pandamux_core::ToolLedgerStatus::Started);
        }
        let continuations = store
            .list_continuations(&ThreadId::new(&thread_id))
            .expect("load continuation");
        assert_eq!(continuations.len(), 1, "mode {mode}");
        assert!(continuations[0].checkpoint.is_some(), "mode {mode}");
        assert_eq!(mock.session_start_count(), 1, "mode {mode}");
        let events = store
            .get_events(&ThreadId::new(&thread_id), None)
            .expect("load events");
        assert!(
            events
                .iter()
                .any(|event| { matches!(event.kind, ThreadEventKind::CapacityInterrupted { .. }) })
        );
        router.thread_manager().shutdown_all().await;
    }
}

#[tokio::test]
async fn audited_capacity_continuation_is_fresh_and_idempotent_after_manager_restart() {
    let store = Store::in_memory().expect("open store");
    configure_failover_policy(&store, Some("after_text"));
    let drivers = Arc::new(DriverRegistry::new());
    let mock = Arc::new(MockProviderDriver::new(
        ProviderKind::Custom,
        "Mock Provider",
    ));
    drivers.register(ProviderKind::Custom, mock.clone());
    let router = Arc::new(Router::with_drivers(
        store.clone(),
        ServerRole::Hub,
        FAILOVER_ENVIRONMENT_ID.to_string(),
        drivers.clone(),
    ));
    let repo = init_test_git_repo();
    let thread_id =
        create_failover_thread(&router, repo.path().to_str().expect("repository path"), 300).await;
    let response = router
        .handle_request(RpcRequest::new(
            301,
            "thread.send_turn",
            Some(json!({ "threadId": thread_id, "text": "continue safely" })),
        ))
        .await
        .expect("send response");
    let source_turn_id = TurnId::new(
        response.result.expect("source turn result")["turnId"]
            .as_str()
            .expect("source turn id"),
    );
    wait_for_turn_status(&store, &source_turn_id, TurnStatus::InterruptedForCapacity).await;
    store
        .save_resume_token(&ThreadId::new(&thread_id), "source-native-token")
        .expect("save source resume token");
    let continuation = store
        .list_continuations(&ThreadId::new(&thread_id))
        .expect("load continuation")
        .into_iter()
        .next()
        .expect("offered continuation");

    let first_request = RpcRequest::new(
        302,
        "thread.continue_capacity",
        Some(json!({
            "continuationId": continuation.id,
            "threadId": thread_id,
        })),
    );
    let concurrent_request = RpcRequest::new(
        303,
        "thread.continueCapacity",
        Some(json!({
            "continuationId": continuation.id,
            "threadId": thread_id,
        })),
    );
    let (first, concurrent) = tokio::join!(
        router.handle_request(first_request),
        router.handle_request(concurrent_request)
    );
    let first = first.expect("continue response");
    let concurrent = concurrent.expect("concurrent continue response");
    assert!(first.is_success(), "continuation failed: {first:?}");
    assert!(
        concurrent.is_success(),
        "concurrent duplicate continuation failed: {concurrent:?}"
    );
    let first_result = first.result.expect("continuation result");
    let concurrent_result = concurrent.result.expect("concurrent continuation result");
    assert_eq!(concurrent_result["turnId"], first_result["turnId"]);
    assert_eq!(
        first_result["routing"]["selectedProviderInstanceId"],
        FALLBACK_PROVIDER_ID
    );
    let target_turn_id = TurnId::new(
        first_result["turnId"]
            .as_str()
            .expect("continuation target turn id"),
    );
    wait_for_turn_status(&store, &target_turn_id, TurnStatus::Completed).await;
    assert!(
        mock.received_resume_tokens().is_empty(),
        "a provider-account handoff must not reuse the source resume token"
    );
    assert_eq!(
        store
            .get_resume_token(&ThreadId::new(&thread_id))
            .expect("load cleared source resume token"),
        None,
        "a provider-account handoff must retire the source resume token"
    );
    assert_eq!(mock.session_start_count(), 2);
    router.thread_manager().shutdown_all().await;

    let restarted = Router::with_drivers(
        store.clone(),
        ServerRole::Hub,
        FAILOVER_ENVIRONMENT_ID.to_string(),
        drivers,
    );
    let duplicate = restarted
        .handle_request(RpcRequest::new(
            304,
            "thread.continueCapacity",
            Some(json!({
                "continuationId": continuation.id,
                "threadId": thread_id,
            })),
        ))
        .await
        .expect("duplicate continuation response");
    assert!(
        duplicate.is_success(),
        "duplicate continuation failed: {duplicate:?}"
    );
    assert_eq!(
        duplicate.result.expect("duplicate result")["turnId"],
        target_turn_id.as_str()
    );
    assert_eq!(mock.session_start_count(), 2);
    assert_eq!(
        store
            .get_continuation(&continuation.id)
            .expect("load completed continuation")
            .expect("completed continuation")
            .state,
        pandamux_core::ContinuationState::Completed
    );
}

#[tokio::test]
async fn organization_filter_blocks_the_only_healthy_pool_member_before_scoring() {
    let store = Store::in_memory().expect("open store");
    configure_failover_policy(&store, None);
    let mut settings = store
        .get_settings()
        .expect("load settings")
        .expect("settings exist");
    settings
        .providers
        .iter_mut()
        .find(|provider| provider.id.as_str() == FAILING_PROVIDER_ID)
        .expect("primary provider")
        .settings["runtimeHealth"] = json!("unavailable");
    store
        .save_settings(&settings)
        .expect("save provider health fixture");
    let mut policy = store
        .organization_policy_for_project(&ProjectId::new(FAILOVER_PROJECT_ID))
        .expect("load organization policy");
    policy.allowed_provider_instances = vec![ProviderInstanceId::new(FAILING_PROVIDER_ID)];
    policy.allowed_subscriptions = vec![SubscriptionKey::new("subkey-primary")];
    policy.revision += 1;
    store
        .save_organization_policy(&policy, 102)
        .expect("save adversarial organization policy");

    let drivers = Arc::new(DriverRegistry::new());
    drivers.register(
        ProviderKind::Custom,
        Arc::new(MockProviderDriver::new(
            ProviderKind::Custom,
            "Mock Provider",
        )),
    );
    let router = Router::with_drivers(
        store,
        ServerRole::Hub,
        FAILOVER_ENVIRONMENT_ID.to_string(),
        drivers,
    );
    let repo = init_test_git_repo();
    let thread_id =
        create_failover_thread(&router, repo.path().to_str().expect("repository path"), 400).await;
    let response = router
        .handle_request(RpcRequest::new(
            401,
            "thread.send_turn",
            Some(json!({ "threadId": thread_id, "text": "must remain in organization" })),
        ))
        .await
        .expect("route response");
    let error = response.error.expect("routing must fail closed");
    assert_eq!(
        error.data.as_ref().expect("routing error data")["reason"],
        "no_eligible_failover_route"
    );
    let evaluations = error.data.expect("routing evaluations")["routing"]["evaluations"]
        .as_array()
        .expect("evaluation array")
        .clone();
    let fallback = evaluations
        .iter()
        .find(|evaluation| evaluation["providerInstanceId"] == FALLBACK_PROVIDER_ID)
        .expect("fallback evaluation");
    assert_eq!(fallback["eligible"], false);
    assert!(
        fallback["rejectionCodes"]
            .as_array()
            .expect("rejection codes")
            .iter()
            .any(|code| code == "organization_policy_denied")
    );
    assert!(fallback["score"].is_null());
}

#[tokio::test]
async fn test_thread_optional_worktree() {
    let store = Store::in_memory().expect("open store");
    let router = Router::new(store, ServerRole::Hub, "env-test".to_string());

    let worktree_ref = WorktreeRef {
        path: "/custom/worktree/feat-1".to_string(),
        branch: "feat-1".to_string(),
        base_ref: "master".to_string(),
    };

    let create_req = RpcRequest::new(
        1,
        "thread.create",
        Some(json!({
            "title": "Worktree Thread",
            "cwd": "/base/repo",
            "worktree": worktree_ref,
        })),
    );
    let create_res = router
        .handle_request(create_req)
        .await
        .expect("create response");
    assert!(create_res.is_success());

    let thread_val = create_res.result.unwrap();
    let thread_id = thread_val["id"].as_str().unwrap().to_string();

    let get_req = RpcRequest::new(2, "thread.get", Some(json!({ "threadId": thread_id })));
    let get_res = router.handle_request(get_req).await.expect("get response");
    let thread = get_res.result.unwrap()["thread"].clone();

    assert_eq!(thread["workspace"]["cwd"], "/base/repo");
    assert_eq!(
        thread["workspace"]["worktree"]["path"],
        "/custom/worktree/feat-1"
    );
    assert_eq!(thread["workspace"]["worktree"]["branch"], "feat-1");
    assert_eq!(thread["workspace"]["worktree"]["baseRef"], "master");
}
