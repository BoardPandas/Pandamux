use std::sync::Arc;
use std::time::Duration;

use pandamux_cli::IpcClient;
use pandamux_protocol::{
    IdentifyResult, McpToolDefinition, PingResult, RpcRequest, ServerRole, ThreadCreateParams,
};
use pandamux_server::{DriverRegistry, Router, Store};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test]
async fn test_cli_ipc_contract_ping_and_identify() {
    let sock_path = std::env::temp_dir().join(format!(
        "test_pandamux_{}.sock",
        uuid::Uuid::new_v4().simple()
    ));
    let sock_str = sock_path.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&sock_path);

    let store = Store::in_memory().expect("in-memory store");
    let drivers = Arc::new(DriverRegistry::default_local());
    let router = Arc::new(Router::with_drivers(
        store,
        ServerRole::Hub,
        "env-test".to_string(),
        drivers,
    ));

    // Spawn server IPC listener
    #[cfg(unix)]
    let listener = tokio::net::UnixListener::bind(&sock_path).expect("bind socket");

    let router_clone = router.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        while let Ok((stream, _)) = listener.accept().await {
            let router = router_clone.clone();
            tokio::spawn(async move {
                let (reader, mut writer) = tokio::io::split(stream);
                let mut lines = BufReader::new(reader).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(req) = serde_json::from_str::<RpcRequest>(&line)
                        && let Some(resp) = router.handle_request(req).await
                    {
                        let mut out = serde_json::to_string(&resp).unwrap_or_default();
                        out.push('\n');
                        let _ = writer.write_all(out.as_bytes()).await;
                    }
                }
            });
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // 1. Test ping
    let mut client = IpcClient::connect(Some(&sock_str))
        .await
        .expect("connect to test socket");

    let ping: PingResult = client
        .call_typed("system.ping", &json!({}))
        .await
        .expect("ping call");
    assert!(ping.pong);
    assert!(ping.timestamp_ms > 0);

    // 2. Test identify
    let id: IdentifyResult = client
        .call_typed("system.identify", &json!({}))
        .await
        .expect("identify call");
    assert_eq!(id.protocol_version, 3);
    assert_eq!(id.role, ServerRole::Hub);
    assert_eq!(id.environment_id, "env-test");

    // 3. Test notification.post
    let post_res: serde_json::Value = client
        .call(
            "notification.post",
            Some(json!({
                "title": "Build Complete",
                "body": "All tests passed successfully",
                "source": "build"
            })),
        )
        .await
        .expect("notification.post");
    assert_eq!(post_res["posted"], true);
    assert!(post_res["id"].as_str().is_some());

    // 4. Test notification.list
    let list_res: serde_json::Value = client
        .call("notification.list", None)
        .await
        .expect("notification.list");
    let notifs = list_res.as_array().expect("array of notifications");
    assert_eq!(notifs.len(), 1);
    assert_eq!(notifs[0]["title"], "Build Complete");
    assert_eq!(notifs[0]["body"], "All tests passed successfully");

    // 5. Test thread.create and thread.list
    let create_params = ThreadCreateParams {
        project_id: None,
        environment_id: None,
        title: Some("CLI Test Thread".to_string()),
        provider_instance_id: None,
        model: Some("claude-3-7-sonnet".to_string()),
        effort: None,
        access_mode: None,
        agent_id: None,
        cwd: None,
        worktree: None,
    };
    let created_thread: pandamux_core::Thread = client
        .call_typed("thread.create", &create_params)
        .await
        .expect("thread.create");
    assert_eq!(created_thread.title, "CLI Test Thread");

    let threads: Vec<pandamux_core::Thread> = client
        .call_typed("thread.list", &json!({}))
        .await
        .expect("thread.list");
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].id, created_thread.id);

    // 6. Test mcp.list_tools
    let tools: Vec<McpToolDefinition> = client
        .call_typed("mcp.list_tools", &json!({}))
        .await
        .expect("mcp.list_tools");
    assert!(!tools.is_empty());
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(tool_names.contains(&"read_settings"));
}
