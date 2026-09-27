use std::time::Instant;

use pandamux_core::{Thread, ThreadId, ThreadStatus};
use pandamux_protocol::{
    IdentifyResult, McpCallToolParams, McpCallToolResult, McpToolDefinition, PingResult,
    ThreadListParams, ThreadSendTurnParams, ThreadSendTurnResult,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::ipc::IpcClient;

/// Runs `pandamux ping` command.
pub async fn cmd_ping(pipe: Option<&str>, json_output: bool) -> Result<(), String> {
    let start = Instant::now();
    let mut client = IpcClient::connect(pipe).await?;
    let res: PingResult = client.call_typed("system.ping", &json!({})).await?;
    let elapsed = start.elapsed();
    let latency_ms = elapsed.as_secs_f64() * 1000.0;

    if json_output {
        println!(
            "{}",
            json!({
                "pong": res.pong,
                "timestampMs": res.timestamp_ms,
                "latencyMs": (latency_ms * 100.0).round() / 100.0
            })
        );
    } else {
        println!(
            "PONG: {:.2}ms latency (server time: {})",
            latency_ms, res.timestamp_ms
        );
    }

    Ok(())
}

/// Runs `pandamux identify` command.
pub async fn cmd_identify(pipe: Option<&str>, json_output: bool) -> Result<(), String> {
    let mut client = IpcClient::connect(pipe).await?;
    let res: IdentifyResult = client.call_typed("system.identify", &json!({})).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&res).unwrap_or_default());
    } else {
        println!("PandaMUX Node Identification");
        println!("----------------------------");
        println!("Server Version:   {}", res.server_version);
        println!("Protocol Version: v{}", res.protocol_version);
        println!("Server Role:      {:?}", res.role);
        println!("Platform:         {}", res.platform);
        println!("Environment ID:   {}", res.environment_id);
    }

    Ok(())
}

/// Runs `pandamux thread list` command.
pub async fn cmd_thread_list(
    pipe: Option<&str>,
    status_filter: Option<ThreadStatus>,
    limit: Option<usize>,
    json_output: bool,
) -> Result<(), String> {
    let mut client = IpcClient::connect(pipe).await?;
    let params = ThreadListParams {
        project_id: None,
        agent_id: None,
        status: status_filter,
        parent_thread_id: None,
    };

    let mut threads: Vec<Thread> = client.call_typed("thread.list", &params).await?;

    if let Some(n) = limit {
        threads.truncate(n);
    }

    if json_output {
        println!("{}", serde_json::to_string_pretty(&threads).unwrap_or_default());
    } else if threads.is_empty() {
        println!("No active threads found.");
    } else {
        println!(
            "{:<36}  {:<12}  {:<20}  {:<18}  {}",
            "THREAD ID", "STATUS", "PROVIDER", "MODEL", "TITLE"
        );
        println!("{}", "-".repeat(105));
        for t in &threads {
            println!(
                "{:<36}  {:<12}  {:<20}  {:<18}  {}",
                t.id.as_str(),
                format!("{:?}", t.status),
                t.provider_instance_id.as_str(),
                t.model,
                t.title
            );
        }
    }

    Ok(())
}

/// Runs `pandamux thread send` command.
pub async fn cmd_thread_send(
    pipe: Option<&str>,
    thread_id: &str,
    text: &str,
    model: Option<&str>,
    effort: Option<&str>,
    json_output: bool,
) -> Result<(), String> {
    let mut client = IpcClient::connect(pipe).await?;
    let params = ThreadSendTurnParams {
        thread_id: ThreadId::from(thread_id),
        text: text.to_string(),
        attachment_ids: vec![],
        model: model.map(String::from),
        effort: effort.map(String::from),
    };

    let res: ThreadSendTurnResult = client.call_typed("thread.send_turn", &params).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&res).unwrap_or_default());
    } else {
        println!(
            "Turn submitted: {} (sequence: {})",
            res.turn_id.as_str(),
            res.seq
        );
        println!("Prompt: \"{text}\"");
    }

    Ok(())
}

/// Runs `pandamux notify` command.
pub async fn cmd_notify(
    pipe: Option<&str>,
    title: &str,
    body: Option<&str>,
    source: Option<&str>,
    json_output: bool,
) -> Result<(), String> {
    let mut client = IpcClient::connect(pipe).await?;
    let params = json!({
        "title": title,
        "body": body.unwrap_or(""),
        "source": source.unwrap_or("generic"),
    });

    let res: Value = client.call("notification.post", Some(params)).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&res).unwrap_or_default());
    } else {
        println!("Notification posted: \"{title}\"");
    }

    Ok(())
}

/// Runs `pandamux mcp` stdio JSON-RPC bridge.
pub async fn cmd_mcp(pipe: Option<&str>) -> Result<(), String> {
    let mut client = IpcClient::connect(pipe).await?;

    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut stdin_lines = BufReader::new(stdin).lines();

    while let Ok(Some(line)) = stdin_lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": "Parse error" }
                });
                let mut out = serde_json::to_string(&err_resp).unwrap_or_default();
                out.push('\n');
                let _ = stdout.write_all(out.as_bytes()).await;
                let _ = stdout.flush().await;
                continue;
            }
        };

        let id = parsed.get("id").cloned().unwrap_or(Value::Null);
        let method = parsed
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let params = parsed.get("params").cloned().unwrap_or(Value::Null);

        let response = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {
                        "name": "pandamux-mcp",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {
                        "tools": {}
                    }
                }
            }),

            "ping" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {}
            }),

            "tools/list" => {
                let tools_res: Result<Vec<McpToolDefinition>, String> =
                    client.call_typed("mcp.list_tools", &json!({})).await;

                match tools_res {
                    Ok(tools) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "tools": tools }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32603, "message": e }
                    }),
                }
            }

            "tools/call" => {
                let call_params = McpCallToolParams {
                    name: params
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string(),
                    arguments: params
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                };

                let call_res: Result<McpCallToolResult, String> =
                    client.call_typed("mcp.call_tool", &call_params).await;

                match call_res {
                    Ok(res) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": res
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32603, "message": e }
                    }),
                }
            }

            // Forward other methods directly to pandamux-server
            other => match client.call(other, Some(params)).await {
                Ok(res) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": res
                }),
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": e }
                }),
            },
        };

        let mut out_str = serde_json::to_string(&response).unwrap_or_default();
        out_str.push('\n');
        let _ = stdout.write_all(out_str.as_bytes()).await;
        let _ = stdout.flush().await;
    }

    Ok(())
}
