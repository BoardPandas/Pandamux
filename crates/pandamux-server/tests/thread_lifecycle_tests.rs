use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use pandamux_core::event::ThreadEventKind;
use pandamux_core::provider_config::ProviderKind;
use pandamux_core::thread::WorktreeRef;
use pandamux_protocol::{RpcRequest, ServerRole};
use pandamux_providers::MockProviderDriver;
use pandamux_server::{DriverRegistry, Router, Store};

#[tokio::test]
async fn test_thread_approval_flow() {
    let store = Store::in_memory().expect("open store");
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
