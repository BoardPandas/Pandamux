use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use pandamux_core::UserSettings;
use pandamux_protocol::{
    HelloParams, HelloResult, IdentifyResult, McpCallToolParams, PROTOCOL_VERSION, PingResult,
    RpcError, RpcRequest, RpcResponse, ServerCapabilities, ServerRole, ThreadCancelTurnParams,
    ThreadCreateParams, ThreadGetParams, ThreadListParams, ThreadRespondApprovalParams,
    ThreadResumeParams, ThreadSendTurnParams,
};

use crate::driver_registry::DriverRegistry;
use crate::mcp_server::McpServer;
use crate::store::Store;
use crate::thread_manager::ThreadManager;

pub struct Router {
    store: Store,
    mcp: McpServer,
    thread_manager: ThreadManager,
    notifications: Arc<tokio::sync::Mutex<Vec<pandamux_core::notification::NotificationInfo>>>,
    role: ServerRole,
    environment_id: String,
    server_version: String,
}

impl Router {
    pub fn new(store: Store, role: ServerRole, environment_id: String) -> Self {
        let drivers = Arc::new(DriverRegistry::default_local());
        Self::with_drivers(store, role, environment_id, drivers)
    }

    pub fn with_drivers(
        store: Store,
        role: ServerRole,
        environment_id: String,
        drivers: Arc<DriverRegistry>,
    ) -> Self {
        let mcp = McpServer::new(store.clone());
        let thread_manager = ThreadManager::new(store.clone(), drivers, environment_id.clone());
        Self {
            store,
            mcp,
            thread_manager,
            notifications: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            role,
            environment_id,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn thread_manager(&self) -> &ThreadManager {
        &self.thread_manager
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Handle a single parsed JSON-RPC request and return an optional response.
    pub async fn handle_request(&self, req: RpcRequest) -> Option<RpcResponse> {
        let is_notification = req.is_notification();
        let id = req.id;
        let method = req.method;
        let params = req.params.unwrap_or(Value::Null);

        let result = match method.as_str() {
            "system.ping" => self.handle_ping(),
            "system.hello" => self.handle_hello(params),
            "system.identify" => self.handle_identify(),
            "system.capabilities" => self.handle_capabilities(),
            "thread.create" => self.handle_thread_create(params),
            "thread.get" => self.handle_thread_get(params),
            "thread.list" => self.handle_thread_list(params),
            "thread.send_turn" | "thread.sendTurn" => self.handle_thread_send_turn(params).await,
            "thread.cancel_turn" | "thread.interrupt" => {
                self.handle_thread_cancel_turn(params).await
            }
            "thread.respond_approval" | "thread.respondApproval" => {
                self.handle_thread_respond_approval(params).await
            }
            "thread.resume" => self.handle_thread_resume(params),
            "settings.get" => self.handle_settings_get(),
            "settings.set" => self.handle_settings_set(params),
            "mcp.list_tools" => self.handle_mcp_list_tools(),
            "mcp.call_tool" => self.handle_mcp_call_tool(params),
            "notification.post" => self.handle_notification_post(params).await,
            "notification.list" => self.handle_notification_list().await,
            "notification.clear" => self.handle_notification_clear(params).await,
            unknown => Err(RpcError::method_not_found(unknown)),
        };

        if is_notification {
            return None;
        }

        Some(match result {
            Ok(val) => RpcResponse::success(id.unwrap_or(1.into()), val),
            Err(err) => RpcResponse::error(id, err),
        })
    }

    fn handle_ping(&self) -> Result<Value, RpcError> {
        let ping = PingResult {
            pong: true,
            timestamp_ms: Self::now_ms(),
        };
        serde_json::to_value(ping).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_hello(&self, params: Value) -> Result<Value, RpcError> {
        let hello: HelloParams = serde_json::from_value(params)
            .map_err(|e| RpcError::invalid_params(format!("Invalid hello params: {e}")))?;

        if hello.protocol_version != PROTOCOL_VERSION {
            return Err(RpcError::invalid_params(format!(
                "Protocol version mismatch: client requested {}, server supports {}",
                hello.protocol_version, PROTOCOL_VERSION
            )));
        }

        let result = HelloResult {
            server_version: self.server_version.clone(),
            protocol_version: PROTOCOL_VERSION,
            role: self.role,
            capabilities: ServerCapabilities {
                streaming: true,
                attachments: true,
                worktrees: true,
                mcp: true,
                schedules: true,
            },
        };

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_identify(&self) -> Result<Value, RpcError> {
        let result = IdentifyResult {
            server_version: self.server_version.clone(),
            protocol_version: PROTOCOL_VERSION,
            role: self.role,
            platform: std::env::consts::OS.to_string(),
            environment_id: self.environment_id.clone(),
        };
        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_capabilities(&self) -> Result<Value, RpcError> {
        let caps = ServerCapabilities {
            streaming: true,
            attachments: true,
            worktrees: true,
            mcp: true,
            schedules: true,
        };
        serde_json::to_value(caps).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_thread_create(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadCreateParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let thread = self.thread_manager.create_thread(p)?;
        serde_json::to_value(&thread).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_thread_get(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadGetParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self.thread_manager.get_thread(p)?;
        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_thread_list(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadListParams = if params.is_null() {
            Default::default()
        } else {
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?
        };
        let threads = self.thread_manager.list_threads(p)?;
        serde_json::to_value(threads).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_thread_send_turn(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadSendTurnParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self.thread_manager.send_turn(p).await?;
        serde_json::to_value(&result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_thread_cancel_turn(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadCancelTurnParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        self.thread_manager.interrupt_turn(p).await?;
        Ok(json!({ "interrupted": true }))
    }

    async fn handle_thread_respond_approval(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadRespondApprovalParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        self.thread_manager.respond_approval(p).await?;
        Ok(json!({ "acknowledged": true }))
    }

    fn handle_thread_resume(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadResumeParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self.thread_manager.resume_thread(p)?;
        serde_json::to_value(&result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_settings_get(&self) -> Result<Value, RpcError> {
        let settings = self
            .store
            .get_settings()
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .unwrap_or_default();
        serde_json::to_value(settings).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_settings_set(&self, params: Value) -> Result<Value, RpcError> {
        let settings: UserSettings =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        self.store
            .save_settings(&settings)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;
        Ok(json!({ "success": true }))
    }

    fn handle_mcp_list_tools(&self) -> Result<Value, RpcError> {
        let tools = self.mcp.list_tools();
        serde_json::to_value(tools).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_mcp_call_tool(&self, params: Value) -> Result<Value, RpcError> {
        let call_params: McpCallToolParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self.mcp.call_tool(&call_params);
        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_notification_post(&self, params: Value) -> Result<Value, RpcError> {
        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if title.is_empty() {
            return Err(RpcError::invalid_params("Notification title is required"));
        }
        let body = params
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let source_str = params
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("generic");
        let source = match source_str {
            "build" => pandamux_core::notification::NotificationSource::Build,
            "agent" => pandamux_core::notification::NotificationSource::Agent,
            "deploy" => pandamux_core::notification::NotificationSource::Deploy,
            "port" => pandamux_core::notification::NotificationSource::Port,
            _ => pandamux_core::notification::NotificationSource::Generic,
        };

        let id = format!("notif-{}", uuid::Uuid::new_v4().simple());
        let info = pandamux_core::notification::NotificationInfo {
            id: id.clone(),
            workspace_id: None,
            surface_id: None,
            title,
            body,
            source,
            timestamp_ms: Self::now_ms(),
            read: false,
        };

        let mut lock = self.notifications.lock().await;
        lock.push(info);

        Ok(json!({ "id": id, "posted": true }))
    }

    async fn handle_notification_list(&self) -> Result<Value, RpcError> {
        let lock = self.notifications.lock().await;
        serde_json::to_value(&*lock).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_notification_clear(&self, params: Value) -> Result<Value, RpcError> {
        let mut lock = self.notifications.lock().await;
        if let Some(id) = params.get("id").and_then(|v| v.as_str()) {
            lock.retain(|n| n.id != id);
        } else {
            lock.clear();
        }
        Ok(json!({ "cleared": true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn router_handles_ping_and_capabilities() {
        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Hub, "env-test".to_string());

        let ping_req = RpcRequest::new(1, "system.ping", None);
        let res = router.handle_request(ping_req).await.expect("response");
        assert!(res.is_success());
        assert_eq!(res.result.unwrap()["pong"], true);

        let caps_req = RpcRequest::new(2, "system.capabilities", None);
        let res = router.handle_request(caps_req).await.expect("response");
        assert!(res.is_success());
        assert_eq!(res.result.unwrap()["streaming"], true);
    }

    #[tokio::test]
    async fn router_handles_mcp_discovery_and_call() {
        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Hub, "env-test".to_string());

        let list_req = RpcRequest::new(1, "mcp.list_tools", None);
        let res = router.handle_request(list_req).await.expect("response");
        assert!(res.is_success());
        let tools = res.result.unwrap();
        assert!(tools.as_array().unwrap().len() >= 2);

        let call_req = RpcRequest::new(
            2,
            "mcp.call_tool",
            Some(json!({ "name": "read_settings", "arguments": {} })),
        );
        let res = router.handle_request(call_req).await.expect("response");
        assert!(res.is_success());
    }

    #[tokio::test]
    async fn router_handles_thread_lifecycle_and_turn_streaming() {
        let store = Store::in_memory().unwrap();
        let drivers = Arc::new(DriverRegistry::default_local());
        let router = Router::with_drivers(store, ServerRole::Hub, "env-test".to_string(), drivers);

        // 1. Create thread
        let create_req = RpcRequest::new(
            1,
            "thread.create",
            Some(json!({
                "title": "Test Chat",
                "model": "mock-fast"
            })),
        );
        let res = router
            .handle_request(create_req)
            .await
            .expect("create response");
        assert!(res.is_success());
        let thread_val = res.result.unwrap();
        let thread_id = thread_val["id"].as_str().unwrap().to_string();

        // 2. Subscribe to events
        let mut event_rx = router.thread_manager().subscribe();

        // 3. Send turn
        let send_req = RpcRequest::new(
            2,
            "thread.send_turn",
            Some(json!({
                "threadId": thread_id,
                "text": "Hello PandaMUX"
            })),
        );
        let send_res = router
            .handle_request(send_req)
            .await
            .expect("send response");
        assert!(send_res.is_success());

        // 4. Receive streamed events
        let mut got_turn_requested = false;
        let mut got_assistant_text = false;
        let mut got_turn_settled = false;

        for _ in 0..10 {
            if let Ok(Ok(env)) =
                tokio::time::timeout(std::time::Duration::from_millis(500), event_rx.recv()).await
            {
                match env.kind {
                    pandamux_core::event::ThreadEventKind::TurnRequested { .. } => {
                        got_turn_requested = true;
                    }
                    pandamux_core::event::ThreadEventKind::AssistantText { ref text, .. } => {
                        if text.contains("Hello PandaMUX") {
                            got_assistant_text = true;
                        }
                    }
                    pandamux_core::event::ThreadEventKind::TurnSettled { .. } => {
                        got_turn_settled = true;
                        break;
                    }
                    _ => {}
                }
            }
        }

        assert!(got_turn_requested, "Should receive TurnRequested event");
        assert!(got_assistant_text, "Should receive AssistantText event");
        assert!(got_turn_settled, "Should receive TurnSettled event");

        // 5. Get thread and verify turn history
        let get_req = RpcRequest::new(3, "thread.get", Some(json!({ "threadId": thread_id })));
        let get_res = router.handle_request(get_req).await.expect("get response");
        assert!(get_res.is_success());
        let get_val = get_res.result.unwrap();
        assert_eq!(get_val["thread"]["id"], thread_id);
        assert_eq!(get_val["turns"].as_array().unwrap().len(), 1);
    }
}
