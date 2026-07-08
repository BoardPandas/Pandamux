use serde_json::{Value, json};
use std::error::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        print_usage();
        return Ok(());
    };

    match command {
        "ping" => {
            println!("{}", send_v1("ping").await?);
        }
        "identify" => print_json(send_v2("system.identify", json!({})).await?),
        "capabilities" => print_json(send_v2("system.capabilities", json!({})).await?),
        "tree" => print_json(send_v2("system.tree", json!({})).await?),
        "new-workspace" => {
            print_json(send_v2("workspace.create", workspace_create_params(&args[1..])?).await?)
        }
        "list-workspaces" => print_json(send_v2("workspace.list", json!({})).await?),
        "select-workspace" => print_json(send_v2("workspace.select", id_param(&args[1..])?).await?),
        "rename-workspace" => {
            print_json(send_v2("workspace.rename", rename_workspace_params(&args[1..])?).await?)
        }
        "close-workspace" => print_json(send_v2("workspace.close", id_param(&args[1..])?).await?),
        "split" => print_json(send_v2("pane.split", split_params(&args[1..])?).await?),
        "close-pane" => {
            print_json(send_v2("pane.close", id_with_optional_workspace_param(&args[1..])?).await?)
        }
        "focus-pane" => {
            print_json(send_v2("pane.focus", id_with_optional_workspace_param(&args[1..])?).await?)
        }
        "zoom-pane" => print_json(send_v2("pane.zoom", optional_pane_param(&args[1..])?).await?),
        "new-surface" => {
            print_json(send_v2("surface.create", surface_create_params(&args[1..])?).await?)
        }
        "focus-surface" => print_json(
            send_v2(
                "surface.focus",
                id_with_optional_workspace_param(&args[1..])?,
            )
            .await?,
        ),
        "close-surface" => print_json(
            send_v2(
                "surface.close",
                id_with_optional_workspace_param(&args[1..])?,
            )
            .await?,
        ),
        "list-panes" => {
            print_json(send_v2("pane.list", optional_workspace_param(&args[1..])?).await?)
        }
        "list-surfaces" => {
            print_json(send_v2("surface.list", list_surfaces_params(&args[1..])?).await?)
        }
        "send" => print_json(send_v2("surface.send_text", send_text_params(&args[1..])?).await?),
        "send-key" => print_json(send_v2("surface.send_key", send_key_params(&args[1..])?).await?),
        "read-screen" => {
            print_json(send_v2("surface.read_text", read_screen_params(&args[1..])?).await?)
        }
        "trigger-flash" => {
            print_json(send_v2("surface.trigger_flash", optional_surface_param(&args[1..])?).await?)
        }
        "notify" => print_json(send_v2("notification.raise", notify_params(&args[1..])?).await?),
        "list-notifications" => print_json(send_v2("notification.list", json!({})).await?),
        "clear-notifications" => print_json(
            send_v2(
                "notification.clear",
                clear_notifications_params(&args[1..])?,
            )
            .await?,
        ),
        "layout" if args.get(1).map(String::as_str) == Some("grid") => {
            print_json(send_v2("layout.grid", layout_grid_params(&args[2..])?).await?);
        }
        _ => {
            print_usage();
            return Err(format!("unknown command: {command}").into());
        }
    }

    Ok(())
}

fn workspace_create_params(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = serde_json::Map::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--title" => {
                params.insert(
                    "title".to_string(),
                    json!(args.get(index + 1).ok_or("--title requires a value")?),
                );
                index += 2;
            }
            "--shell" => {
                params.insert(
                    "shell".to_string(),
                    json!(args.get(index + 1).ok_or("--shell requires a value")?),
                );
                index += 2;
            }
            unknown => return Err(format!("unknown new-workspace option: {unknown}").into()),
        }
    }

    Ok(Value::Object(params))
}

fn id_param(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let id = args.first().ok_or("missing id")?;
    Ok(json!({ "id": id }))
}

fn rename_workspace_params(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let id = args.first().ok_or("missing workspace id")?;
    let title = args.get(1).ok_or("missing title")?;
    Ok(json!({ "id": id, "title": title }))
}

fn id_with_optional_workspace_param(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let id = args.first().ok_or("missing id")?;
    let mut params = serde_json::Map::new();
    params.insert("id".to_string(), json!(id));
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--workspace" => {
                params.insert(
                    "workspaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--workspace requires a value")?),
                );
                index += 2;
            }
            unknown => return Err(format!("unknown option: {unknown}").into()),
        }
    }
    Ok(Value::Object(params))
}

fn optional_workspace_param(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = serde_json::Map::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--workspace" => {
                params.insert(
                    "workspaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--workspace requires a value")?),
                );
                index += 2;
            }
            unknown => return Err(format!("unknown option: {unknown}").into()),
        }
    }
    Ok(Value::Object(params))
}

fn optional_pane_param(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = serde_json::Map::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--pane" => {
                params.insert(
                    "paneId".to_string(),
                    json!(args.get(index + 1).ok_or("--pane requires a value")?),
                );
                index += 2;
            }
            "--workspace" => {
                params.insert(
                    "workspaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--workspace requires a value")?),
                );
                index += 2;
            }
            value if !value.starts_with("--") && !params.contains_key("id") => {
                params.insert("id".to_string(), json!(value));
                index += 1;
            }
            unknown => return Err(format!("unknown pane option: {unknown}").into()),
        }
    }
    Ok(Value::Object(params))
}

fn split_params(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = serde_json::Map::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--down" => {
                params.insert("direction".to_string(), json!("down"));
                index += 1;
            }
            "--type" => {
                params.insert(
                    "type".to_string(),
                    json!(args.get(index + 1).ok_or("--type requires a value")?),
                );
                index += 2;
            }
            "--pane" => {
                params.insert(
                    "paneId".to_string(),
                    json!(args.get(index + 1).ok_or("--pane requires a value")?),
                );
                index += 2;
            }
            "--surface" => {
                params.insert(
                    "surfaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--surface requires a value")?),
                );
                index += 2;
            }
            "--workspace" => {
                params.insert(
                    "workspaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--workspace requires a value")?),
                );
                index += 2;
            }
            unknown => return Err(format!("unknown split option: {unknown}").into()),
        }
    }

    params
        .entry("direction".to_string())
        .or_insert_with(|| json!("right"));
    params
        .entry("type".to_string())
        .or_insert_with(|| json!("terminal"));

    Ok(Value::Object(params))
}

fn surface_create_params(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = serde_json::Map::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--type" => {
                params.insert(
                    "type".to_string(),
                    json!(args.get(index + 1).ok_or("--type requires a value")?),
                );
                index += 2;
            }
            "--pane" => {
                params.insert(
                    "paneId".to_string(),
                    json!(args.get(index + 1).ok_or("--pane requires a value")?),
                );
                index += 2;
            }
            "--workspace" => {
                params.insert(
                    "workspaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--workspace requires a value")?),
                );
                index += 2;
            }
            unknown => return Err(format!("unknown new-surface option: {unknown}").into()),
        }
    }

    params
        .entry("type".to_string())
        .or_insert_with(|| json!("terminal"));

    Ok(Value::Object(params))
}

fn list_surfaces_params(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = serde_json::Map::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--workspace" => {
                params.insert(
                    "workspaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--workspace requires a value")?),
                );
                index += 2;
            }
            "--pane" => {
                params.insert(
                    "paneId".to_string(),
                    json!(args.get(index + 1).ok_or("--pane requires a value")?),
                );
                index += 2;
            }
            unknown => return Err(format!("unknown list-surfaces option: {unknown}").into()),
        }
    }

    Ok(Value::Object(params))
}

fn optional_surface_param(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = serde_json::Map::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--surface" => {
                params.insert(
                    "surfaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--surface requires a value")?),
                );
                index += 2;
            }
            "--workspace" => {
                params.insert(
                    "workspaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--workspace requires a value")?),
                );
                index += 2;
            }
            value if !value.starts_with("--") && !params.contains_key("surfaceId") => {
                params.insert("surfaceId".to_string(), json!(value));
                index += 1;
            }
            unknown => return Err(format!("unknown surface option: {unknown}").into()),
        }
    }
    Ok(Value::Object(params))
}

fn send_text_params(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = optional_surface_param(&[])?;
    let Value::Object(ref mut map) = params else {
        unreachable!("params should be an object");
    };

    let mut text_parts = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--surface" => {
                map.insert(
                    "surfaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--surface requires a value")?),
                );
                index += 2;
            }
            "--workspace" => {
                map.insert(
                    "workspaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--workspace requires a value")?),
                );
                index += 2;
            }
            value => {
                text_parts.push(value.to_string());
                index += 1;
            }
        }
    }

    map.insert("text".to_string(), json!(text_parts.join(" ")));
    Ok(params)
}

fn send_key_params(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = serde_json::Map::new();
    let mut key = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--surface" => {
                params.insert(
                    "surfaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--surface requires a value")?),
                );
                index += 2;
            }
            "--workspace" => {
                params.insert(
                    "workspaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--workspace requires a value")?),
                );
                index += 2;
            }
            "--ctrl" => {
                params.insert("ctrl".to_string(), json!(true));
                index += 1;
            }
            "--shift" => {
                params.insert("shift".to_string(), json!(true));
                index += 1;
            }
            "--alt" => {
                params.insert("alt".to_string(), json!(true));
                index += 1;
            }
            value if !value.starts_with("--") && key.is_none() => {
                key = Some(value.to_string());
                index += 1;
            }
            unknown => return Err(format!("unknown send-key option: {unknown}").into()),
        }
    }

    params.insert(
        "key".to_string(),
        json!(key.ok_or("send-key requires a key")?),
    );
    Ok(Value::Object(params))
}

fn read_screen_params(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = serde_json::Map::new();
    params.insert("lines".to_string(), json!(50));
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--lines" => {
                let value = args
                    .get(index + 1)
                    .ok_or("--lines requires a value")?
                    .parse::<usize>()?;
                params.insert("lines".to_string(), json!(value));
                index += 2;
            }
            "--surface" => {
                params.insert(
                    "surfaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--surface requires a value")?),
                );
                index += 2;
            }
            "--workspace" => {
                params.insert(
                    "workspaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--workspace requires a value")?),
                );
                index += 2;
            }
            unknown => return Err(format!("unknown read-screen option: {unknown}").into()),
        }
    }
    Ok(Value::Object(params))
}

fn layout_grid_params(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = serde_json::Map::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--count" => {
                let value = args
                    .get(index + 1)
                    .ok_or("--count requires a value")?
                    .parse::<usize>()?;
                params.insert("count".to_string(), json!(value));
                index += 2;
            }
            "--type" => {
                params.insert(
                    "type".to_string(),
                    json!(args.get(index + 1).ok_or("--type requires a value")?),
                );
                index += 2;
            }
            "--anchor-pane" => {
                params.insert(
                    "anchorPaneId".to_string(),
                    json!(
                        args.get(index + 1)
                            .ok_or("--anchor-pane requires a value")?
                    ),
                );
                index += 2;
            }
            "--anchor-surface" => {
                params.insert(
                    "anchorSurfaceId".to_string(),
                    json!(
                        args.get(index + 1)
                            .ok_or("--anchor-surface requires a value")?
                    ),
                );
                index += 2;
            }
            "--workspace" => {
                params.insert(
                    "workspaceId".to_string(),
                    json!(args.get(index + 1).ok_or("--workspace requires a value")?),
                );
                index += 2;
            }
            unknown => return Err(format!("unknown layout grid option: {unknown}").into()),
        }
    }

    if !params.contains_key("count") {
        return Err("layout grid requires --count <N>".into());
    }
    params
        .entry("type".to_string())
        .or_insert_with(|| json!("terminal"));

    Ok(Value::Object(params))
}

fn notify_params(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = serde_json::Map::new();
    let mut text_parts = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--body" => {
                params.insert(
                    "body".to_string(),
                    json!(args.get(index + 1).ok_or("--body requires a value")?),
                );
                index += 2;
            }
            "--source" => {
                params.insert(
                    "source".to_string(),
                    json!(args.get(index + 1).ok_or("--source requires a value")?),
                );
                index += 2;
            }
            value => {
                text_parts.push(value.to_string());
                index += 1;
            }
        }
    }

    if text_parts.is_empty() {
        return Err("notify requires a title/message".into());
    }
    params.insert("title".to_string(), json!(text_parts.join(" ")));
    Ok(Value::Object(params))
}

fn clear_notifications_params(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut params = serde_json::Map::new();
    if let Some(id) = args.first() {
        params.insert("id".to_string(), json!(id));
    }
    Ok(Value::Object(params))
}

async fn send_v1(message: &str) -> Result<String, Box<dyn Error>> {
    let reply = send_line(&(message.to_string() + "\n")).await?;
    Ok(reply.trim().to_string())
}

async fn send_v2(method: &str, params: Value) -> Result<Value, Box<dyn Error>> {
    let request = json!({
        "method": method,
        "params": params,
        "id": 1,
        "token": read_pipe_token(),
    });
    let reply = send_line(&(serde_json::to_string(&request)? + "\n")).await?;
    let response: Value = serde_json::from_str(reply.trim())?;
    if let Some(error) = response.get("error") {
        return Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("pipe request failed")
            .to_string()
            .into());
    }
    Ok(response.get("result").cloned().unwrap_or(Value::Null))
}

#[cfg(windows)]
async fn send_line(message: &str) -> Result<String, Box<dyn Error>> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let pipe_name =
        std::env::var("PANDAMUX_PIPE").unwrap_or_else(|_| r"\\.\pipe\pandamux".to_string());
    let mut client = ClientOptions::new().open(pipe_name)?;
    client.write_all(message.as_bytes()).await?;
    let mut reader = BufReader::new(client);
    let mut reply = String::new();
    reader.read_line(&mut reply).await?;
    Ok(reply)
}

#[cfg(not(windows))]
async fn send_line(_message: &str) -> Result<String, Box<dyn Error>> {
    Err("named pipes are only implemented on Windows".into())
}

fn read_pipe_token() -> String {
    std::env::var("PANDAMUX_PIPE_TOKEN")
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

fn print_json(value: Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("json values should serialize")
    );
}

fn print_usage() {
    println!(
        "Usage: pandamux <command>\n\nCommands:\n  ping\n  identify\n  capabilities\n  tree\n  new-workspace [--title <title>] [--shell <shell>]\n  list-workspaces\n  select-workspace <id>\n  rename-workspace <id> <title>\n  close-workspace <id>\n  split [--down] [--type terminal|markdown|diff] [--pane <id>] [--surface <id>] [--workspace <id>]\n  close-pane <id> [--workspace <id>]\n  focus-pane <id> [--workspace <id>]\n  zoom-pane [id] [--workspace <id>]\n  new-surface [--type terminal|markdown|diff] [--pane <id>] [--workspace <id>]\n  focus-surface <id> [--workspace <id>]\n  close-surface <id> [--workspace <id>]\n  list-panes [--workspace <id>]\n  list-surfaces [--workspace <id>] [--pane <id>]\n  send <text> [--surface <id>] [--workspace <id>]\n  send-key <key> [--ctrl] [--shift] [--alt] [--surface <id>] [--workspace <id>]\n  read-screen [--lines <N>] [--surface <id>] [--workspace <id>]\n  trigger-flash [surfaceId]\n  notify <message> [--body <text>] [--source build|agent|deploy|port|generic]\n  list-notifications\n  clear-notifications [id]\n  layout grid --count <N> [--type terminal|markdown|diff] [--anchor-pane <id>] [--anchor-surface <id>] [--workspace <id>]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_layout_grid_params() {
        let params = layout_grid_params(&[
            "--count".to_string(),
            "3".to_string(),
            "--anchor-pane".to_string(),
            "pane-1".to_string(),
        ])
        .expect("params should parse");

        assert_eq!(params["count"], 3);
        assert_eq!(params["type"], "terminal");
        assert_eq!(params["anchorPaneId"], "pane-1");
    }

    #[test]
    fn parses_workspace_create_params() {
        let params = workspace_create_params(&[
            "--title".to_string(),
            "Agents".to_string(),
            "--shell".to_string(),
            "pwsh".to_string(),
        ])
        .expect("params should parse");

        assert_eq!(params["title"], "Agents");
        assert_eq!(params["shell"], "pwsh");
    }

    #[test]
    fn parses_list_surface_params() {
        let params = list_surfaces_params(&[
            "--workspace".to_string(),
            "ws-1".to_string(),
            "--pane".to_string(),
            "pane-1".to_string(),
        ])
        .expect("params should parse");

        assert_eq!(params["workspaceId"], "ws-1");
        assert_eq!(params["paneId"], "pane-1");
    }

    #[test]
    fn parses_split_params() {
        let params = split_params(&[
            "--down".to_string(),
            "--pane".to_string(),
            "pane-1".to_string(),
            "--workspace".to_string(),
            "ws-1".to_string(),
        ])
        .expect("params should parse");

        assert_eq!(params["direction"], "down");
        assert_eq!(params["type"], "terminal");
        assert_eq!(params["paneId"], "pane-1");
        assert_eq!(params["workspaceId"], "ws-1");
    }

    #[test]
    fn parses_surface_create_params() {
        let params = surface_create_params(&[
            "--type".to_string(),
            "markdown".to_string(),
            "--pane".to_string(),
            "pane-1".to_string(),
        ])
        .expect("params should parse");

        assert_eq!(params["type"], "markdown");
        assert_eq!(params["paneId"], "pane-1");
    }

    #[test]
    fn parses_terminal_io_params() {
        let send = send_text_params(&[
            "hello".to_string(),
            "world".to_string(),
            "--surface".to_string(),
            "surf-1".to_string(),
        ])
        .expect("send params should parse");
        assert_eq!(send["text"], "hello world");
        assert_eq!(send["surfaceId"], "surf-1");

        let key = send_key_params(&["enter".to_string(), "--ctrl".to_string()])
            .expect("key params should parse");
        assert_eq!(key["key"], "enter");
        assert_eq!(key["ctrl"], true);

        let read = read_screen_params(&["--lines".to_string(), "12".to_string()])
            .expect("read params should parse");
        assert_eq!(read["lines"], 12);
    }

    #[test]
    fn parses_optional_pane_params() {
        let params = optional_pane_param(&[
            "pane-1".to_string(),
            "--workspace".to_string(),
            "ws-1".to_string(),
        ])
        .expect("pane params should parse");

        assert_eq!(params["id"], "pane-1");
        assert_eq!(params["workspaceId"], "ws-1");
    }
}
