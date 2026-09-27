use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};
use pandamux_core::{
    EnvironmentId, Thread, ThreadId, ThreadStatus, ThreadWorkspace, UserSettings,
};
use pandamux_protocol::{
    HelloParams, HelloResult, IdentifyResult, McpCallToolParams, PingResult,
    PROTOCOL_VERSION, RpcError, RpcRequest, RpcResponse, ServerCapabilities, ServerRole,
    ThreadCreateParams, ThreadGetParams, ThreadGetResult,
};
use crate::mcp_server::McpServer;
use crate::store::Store;

pub struct Router {
    store: Store,
    mcp: McpServer,
    role: ServerRole,
    environment_id: String,
    server_version: String,
}

impl Router {
    pub fn new(store: Store, role: ServerRole, environment_id: String) -> Self {
        let mcp = McpServer::new(store.clone());
        Self {
            store,
            mcp,
            role,
            environment_id,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Handle a single parsed JSON-RPC request and return an optional response.
    pub fn handle_request(&self, req: RpcRequest) -> Option<RpcResponse> {
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
            "settings.get" => self.handle_settings_get(),
            "settings.set" => self.handle_settings_set(params),
            "mcp.list_tools" => self.handle_mcp_list_tools(),
            "mcp.call_tool" => self.handle_mcp_call_tool(params),
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
        Ok(serde_json::to_value(ping).map_err(|e| RpcError::internal_error(e.to_string()))?)
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

        Ok(serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))?)
    }

    fn handle_identify(&self) -> Result<Value, RpcError> {
        let result = IdentifyResult {
            server_version: self.server_version.clone(),
            protocol_version: PROTOCOL_VERSION,
            role: self.role,
            platform: std::env::consts::OS.to_string(),
            environment_id: self.environment_id.clone(),
        };
        Ok(serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))?)
    }

    fn handle_capabilities(&self) -> Result<Value, RpcError> {
        let caps = ServerCapabilities {
            streaming: true,
            attachments: true,
            worktrees: true,
            mcp: true,
            schedules: true,
        };
        Ok(serde_json::to_value(caps).map_err(|e| RpcError::internal_error(e.to_string()))?)
    }

    fn handle_thread_create(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadCreateParams = serde_json::from_value(params)
            .map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let now = Self::now_ms();
        let thread = Thread {
            id: ThreadId::generate(),
            project_id: p.project_id,
            environment_id: p.environment_id.unwrap_or_else(|| EnvironmentId::from("env-local")),
            parent_thread_id: None,
            title: p.title.unwrap_or_else(|| "New Thread".to_string()),
            provider_instance_id: p
                .provider_instance_id
                .unwrap_or_else(|| pandamux_core::ProviderInstanceId::from("prov-default")),
            model: p.model.unwrap_or_else(|| "default".to_string()),
            effort: p.effort,
            access_mode: p.access_mode.unwrap_or_default(),
            workspace: ThreadWorkspace {
                cwd: std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                worktree: None,
            },
            status: ThreadStatus::Idle,
            agent: None,
            origin: Default::default(),
            created_at_ms: now,
            updated_at_ms: now,
        };

        self.store
            .save_thread(&thread)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        Ok(serde_json::to_value(&thread).map_err(|e| RpcError::internal_error(e.to_string()))?)
    }

    fn handle_thread_get(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadGetParams = serde_json::from_value(params)
            .map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let thread = self
            .store
            .get_thread(&p.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread not found: {}", p.thread_id)))?;

        let result = ThreadGetResult {
            thread,
            turns: vec![],
        };
        Ok(serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))?)
    }

    fn handle_thread_list(&self, _params: Value) -> Result<Value, RpcError> {
        let threads = self
            .store
            .list_threads()
            .map_err(|e| RpcError::internal_error(e.to_string()))?;
        Ok(serde_json::to_value(threads).map_err(|e| RpcError::internal_error(e.to_string()))?)
    }

    fn handle_settings_get(&self) -> Result<Value, RpcError> {
        let settings = self
            .store
            .get_settings()
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .unwrap_or_default();
        Ok(serde_json::to_value(settings).map_err(|e| RpcError::internal_error(e.to_string()))?)
    }

    fn handle_settings_set(&self, params: Value) -> Result<Value, RpcError> {
        let settings: UserSettings = serde_json::from_value(params)
            .map_err(|e| RpcError::invalid_params(e.to_string()))?;
        self.store
            .save_settings(&settings)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;
        Ok(json!({ "success": true }))
    }

    fn handle_mcp_list_tools(&self) -> Result<Value, RpcError> {
        let tools = self.mcp.list_tools();
        Ok(serde_json::to_value(tools).map_err(|e| RpcError::internal_error(e.to_string()))?)
    }

    fn handle_mcp_call_tool(&self, params: Value) -> Result<Value, RpcError> {
        let call_params: McpCallToolParams = serde_json::from_value(params)
            .map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self.mcp.call_tool(&call_params);
        Ok(serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_handles_ping_and_capabilities() {
        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Hub, "env-test".to_string());

        let ping_req = RpcRequest::new(1, "system.ping", None);
        let res = router.handle_request(ping_req).expect("response");
        assert!(res.is_success());
        assert_eq!(res.result.unwrap()["pong"], true);

        let caps_req = RpcRequest::new(2, "system.capabilities", None);
        let res = router.handle_request(caps_req).expect("response");
        assert!(res.is_success());
        assert_eq!(res.result.unwrap()["streaming"], true);
    }

    #[test]
    fn router_handles_mcp_discovery_and_call() {
        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Hub, "env-test".to_string());

        let list_req = RpcRequest::new(1, "mcp.list_tools", None);
        let res = router.handle_request(list_req).expect("response");
        assert!(res.is_success());
        let tools = res.result.unwrap();
        assert!(tools.as_array().unwrap().len() >= 2);

        let call_req = RpcRequest::new(
            2,
            "mcp.call_tool",
            Some(json!({ "name": "read_settings", "arguments": {} })),
        );
        let res = router.handle_request(call_req).expect("response");
        assert!(res.is_success());
    }
}
