use serde_json::json;
use pandamux_protocol::{
    McpCallToolParams, McpCallToolResult, McpContentItem, McpToolDefinition,
};
use crate::store::Store;

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
                description: "Read the current PandaMUX user settings and configuration matrix".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            McpToolDefinition {
                name: "list_schedules".to_string(),
                description: "List all scheduled agent tasks configured on this machine".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
        ]
    }

    /// Execute an MCP tool.
    pub fn call_tool(&self, params: &McpCallToolParams) -> McpCallToolResult {
        match params.name.as_str() {
            "read_settings" => {
                match self.store.get_settings() {
                    Ok(Some(settings)) => McpCallToolResult {
                        content: vec![McpContentItem::Text {
                            text: serde_json::to_string_pretty(&settings).unwrap_or_default(),
                        }],
                        is_error: false,
                    },
                    Ok(None) => McpCallToolResult {
                        content: vec![McpContentItem::Text {
                            text: "No custom settings configured (defaults active)".to_string(),
                        }],
                        is_error: false,
                    },
                    Err(err) => McpCallToolResult {
                        content: vec![McpContentItem::Text {
                            text: format!("Error reading settings: {err}"),
                        }],
                        is_error: true,
                    },
                }
            }
            "list_schedules" => {
                match self.store.list_schedules() {
                    Ok(schedules) => McpCallToolResult {
                        content: vec![McpContentItem::Text {
                            text: serde_json::to_string_pretty(&schedules).unwrap_or_default(),
                        }],
                        is_error: false,
                    },
                    Err(err) => McpCallToolResult {
                        content: vec![McpContentItem::Text {
                            text: format!("Error listing schedules: {err}"),
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
