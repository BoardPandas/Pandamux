use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use pandamux_core::event::ThreadEventKind;
use pandamux_core::provider_config::{ProviderInstanceConfig, ProviderKind};
use pandamux_core::thread::WorktreeRef;
use pandamux_core::{ProviderInstanceId, UserSettings};
use pandamux_protocol::{RpcRequest, ServerRole};
use pandamux_providers::MockProviderDriver;
use pandamux_server::{DriverRegistry, Router, Store};

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
        drivers,
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
            "requestId": request_id,
            "decision": "approved"
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

    // 4. Verify thread is back to Idle and turn is Interrupted
    let get_req = RpcRequest::new(4, "thread.get", Some(json!({ "threadId": thread_id })));
    let get_res = router.handle_request(get_req).await.expect("get response");
    let res_val = get_res.result.unwrap();
    assert_eq!(res_val["thread"]["status"], "idle");
    assert_eq!(res_val["turns"][0]["status"], "interrupted");
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
