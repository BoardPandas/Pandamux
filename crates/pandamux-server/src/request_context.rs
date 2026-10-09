use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use pandamux_core::{EnvironmentId, ThreadId};
use pandamux_protocol::RpcError;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerRole {
    Internal,
    LocalControl,
    McpAdapter,
    ProviderChild,
    NodeProxy,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientIdentity {
    pub description: String,
    pub process_id: Option<u32>,
    pub user_id: Option<u32>,
    pub executable: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapabilityScope {
    Internal,
    Control,
    Mcp,
    ProviderChild {
        thread_id: ThreadId,
        methods: BTreeSet<String>,
        expires_at_ms: u64,
    },
    NodeProxy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestContext {
    pub peer_role: PeerRole,
    pub authenticated_environment_id: EnvironmentId,
    pub client_identity: ClientIdentity,
    pub capability_scope: CapabilityScope,
    pub connection_id: String,
}

impl RequestContext {
    pub fn internal(environment_id: EnvironmentId) -> Self {
        Self {
            peer_role: PeerRole::Internal,
            authenticated_environment_id: environment_id,
            client_identity: ClientIdentity {
                description: "server-internal".to_string(),
                ..ClientIdentity::default()
            },
            capability_scope: CapabilityScope::Internal,
            connection_id: "server-internal".to_string(),
        }
    }

    pub fn node_proxy(environment_id: EnvironmentId, client_identity: ClientIdentity) -> Self {
        let connection_id = format!("node-proxy:{}", environment_id.as_str());
        Self {
            peer_role: PeerRole::NodeProxy,
            authenticated_environment_id: environment_id,
            client_identity,
            capability_scope: CapabilityScope::NodeProxy,
            connection_id,
        }
    }

    pub fn authorize(&self, method: &str, params: &Value) -> Result<(), RpcError> {
        let allowed = match &self.capability_scope {
            CapabilityScope::Internal => true,
            CapabilityScope::Control => {
                !method.starts_with("mcp.")
                    && !matches!(method, "usage.export" | "usage.backfill" | "usage.ack")
            }
            CapabilityScope::Mcp => matches!(
                method,
                "system.hello" | "system.ping" | "mcp.list_tools" | "mcp.call_tool"
            ),
            CapabilityScope::ProviderChild {
                thread_id,
                methods,
                expires_at_ms,
            } => {
                now_ms() < *expires_at_ms
                    && (method == "system.hello"
                        || (methods.contains(method)
                            && params
                                .get("threadId")
                                .or_else(|| params.get("thread_id"))
                                .and_then(Value::as_str)
                                .is_some_and(|requested| requested == thread_id.as_str())))
            }
            CapabilityScope::NodeProxy => {
                !method.starts_with("mcp.")
                    && params
                        .get("environmentId")
                        .or_else(|| params.get("environment_id"))
                        .and_then(Value::as_str)
                        .is_none_or(|requested| {
                            requested == self.authenticated_environment_id.as_str()
                        })
            }
        };
        if allowed {
            Ok(())
        } else {
            Err(RpcError::with_data(
                -32001,
                "Capability does not allow this method",
                serde_json::json!({
                    "reason": "capability_scope_denied",
                    "method": method,
                }),
            ))
        }
    }
}

#[derive(Clone)]
pub struct CapabilityAuthority {
    grants: Arc<RwLock<HashMap<String, RequestContext>>>,
    endpoint: Arc<RwLock<Option<String>>>,
    environment_id: EnvironmentId,
}

impl CapabilityAuthority {
    pub fn new(environment_id: EnvironmentId) -> Self {
        Self {
            grants: Arc::new(RwLock::new(HashMap::new())),
            endpoint: Arc::new(RwLock::new(None)),
            environment_id,
        }
    }

    pub fn configure_local_capabilities(
        &self,
        endpoint: String,
        control_token: String,
        mcp_token: String,
    ) {
        *self.endpoint.write().expect("capability endpoint lock") = Some(endpoint);
        let mut grants = self.grants.write().expect("capability grant lock");
        grants.retain(|_, context| {
            !matches!(
                context.capability_scope,
                CapabilityScope::Control | CapabilityScope::Mcp
            )
        });
        grants.insert(
            control_token,
            RequestContext {
                peer_role: PeerRole::LocalControl,
                authenticated_environment_id: self.environment_id.clone(),
                client_identity: ClientIdentity::default(),
                capability_scope: CapabilityScope::Control,
                connection_id: String::new(),
            },
        );
        grants.insert(
            mcp_token,
            RequestContext {
                peer_role: PeerRole::McpAdapter,
                authenticated_environment_id: self.environment_id.clone(),
                client_identity: ClientIdentity::default(),
                capability_scope: CapabilityScope::Mcp,
                connection_id: String::new(),
            },
        );
    }

    pub fn authenticate(
        &self,
        token: &str,
        client_identity: ClientIdentity,
    ) -> Result<RequestContext, RpcError> {
        let context = self
            .grants
            .read()
            .expect("capability grant lock")
            .get(token)
            .cloned()
            .ok_or_else(|| RpcError::unauthorized("Invalid or expired capability"))?;
        if let CapabilityScope::ProviderChild { expires_at_ms, .. } = &context.capability_scope
            && now_ms() >= *expires_at_ms
        {
            self.revoke(token);
            return Err(RpcError::unauthorized("Invalid or expired capability"));
        }
        Ok(RequestContext {
            client_identity,
            connection_id: uuid::Uuid::new_v4().to_string(),
            ..context
        })
    }

    pub fn issue_provider_child(
        &self,
        thread_id: ThreadId,
        methods: impl IntoIterator<Item = String>,
        ttl_ms: u64,
    ) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        self.grants.write().expect("capability grant lock").insert(
            token.clone(),
            RequestContext {
                peer_role: PeerRole::ProviderChild,
                authenticated_environment_id: self.environment_id.clone(),
                client_identity: ClientIdentity::default(),
                capability_scope: CapabilityScope::ProviderChild {
                    thread_id,
                    methods: methods.into_iter().collect(),
                    expires_at_ms: now_ms().saturating_add(ttl_ms),
                },
                connection_id: String::new(),
            },
        );
        token
    }

    pub fn revoke(&self, token: &str) {
        self.grants
            .write()
            .expect("capability grant lock")
            .remove(token);
    }

    pub fn endpoint(&self) -> Option<String> {
        self.endpoint
            .read()
            .expect("capability endpoint lock")
            .clone()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use pandamux_core::ThreadId;
    use serde_json::json;

    use super::*;

    #[test]
    fn provider_child_scope_is_expiring_thread_bound_and_cannot_be_upgraded_by_json() {
        let authority = CapabilityAuthority::new(EnvironmentId::new("env-local"));
        let thread_id = ThreadId::new("thread-allowed");
        let token = authority.issue_provider_child(
            thread_id,
            ["notification.post".to_string(), "memory.search".to_string()],
            60_000,
        );
        let context = authority
            .authenticate(
                &token,
                ClientIdentity {
                    description: "provider-process".to_string(),
                    ..ClientIdentity::default()
                },
            )
            .expect("provider capability authenticates");

        context
            .authorize("system.hello", &json!({}))
            .expect("provider can complete the endpoint handshake");
        context
            .authorize(
                "notification.post",
                &json!({
                    "threadId": "thread-allowed",
                    "peerRole": "internal",
                    "capabilityScope": "control",
                }),
            )
            .expect("exact allowed method and thread");
        context
            .authorize("memory.search", &json!({ "thread_id": "thread-allowed" }))
            .expect("snake-case thread identity is accepted");

        for (method, params) in [
            ("notification.post", json!({})),
            ("notification.post", json!({ "threadId": "thread-other" })),
            (
                "settings.set",
                json!({ "threadId": "thread-allowed", "peerRole": "internal" }),
            ),
            ("organization.set", json!({ "threadId": "thread-allowed" })),
            ("schedule.create", json!({ "threadId": "thread-allowed" })),
            ("environment.list", json!({ "threadId": "thread-allowed" })),
            ("bundle.push", json!({ "threadId": "thread-allowed" })),
        ] {
            let error = context
                .authorize(method, &params)
                .expect_err("provider child method must be denied");
            assert_eq!(error.code, -32001, "unexpected result for {method}");
        }

        let expired = authority.issue_provider_child(
            ThreadId::new("thread-expired"),
            ["notification.post".to_string()],
            0,
        );
        assert!(
            authority
                .authenticate(&expired, ClientIdentity::default())
                .is_err(),
            "zero-duration provider capability must already be expired"
        );
    }

    #[test]
    fn local_control_and_mcp_tokens_receive_distinct_server_owned_roles() {
        let authority = CapabilityAuthority::new(EnvironmentId::new("env-local"));
        authority.configure_local_capabilities(
            "endpoint".to_string(),
            "control-token".to_string(),
            "mcp-token".to_string(),
        );

        let control = authority
            .authenticate("control-token", ClientIdentity::default())
            .expect("control token");
        assert_eq!(control.peer_role, PeerRole::LocalControl);
        control
            .authorize("settings.get", &Value::Null)
            .expect("control method");
        assert!(control.authorize("mcp.list_tools", &Value::Null).is_err());
        assert!(control.authorize("usage.export", &Value::Null).is_err());
        assert!(control.authorize("usage.backfill", &Value::Null).is_err());
        assert!(control.authorize("usage.ack", &Value::Null).is_err());

        let mcp = authority
            .authenticate("mcp-token", ClientIdentity::default())
            .expect("MCP token");
        assert_eq!(mcp.peer_role, PeerRole::McpAdapter);
        mcp.authorize("mcp.list_tools", &Value::Null)
            .expect("MCP method");
        assert!(mcp.authorize("settings.get", &Value::Null).is_err());
    }

    #[test]
    fn node_proxy_cannot_claim_another_authenticated_environment() {
        let context =
            RequestContext::node_proxy(EnvironmentId::new("env-node"), ClientIdentity::default());
        context
            .authorize("thread.create", &json!({ "environmentId": "env-node" }))
            .expect("matching environment");
        assert!(
            context
                .authorize(
                    "thread.create",
                    &json!({
                        "environmentId": "env-other",
                        "authenticatedEnvironmentId": "env-node",
                        "peerRole": "internal",
                    }),
                )
                .is_err()
        );
        assert!(context.authorize("mcp.list_tools", &json!({})).is_err());
    }
}
