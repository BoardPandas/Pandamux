use serde_json::{Value, json};
use std::error::Error;
#[cfg(windows)]
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
        "notify" => {
            print_json(send_v2("notification.post", notify_params(&args[1..])?).await?)
        }
        "list-notifications" => {
            print_json(send_v2("notification.list", json!({})).await?)
        }
        "clear-notifications" => {
            print_json(send_v2("notification.clear", clear_notifications_params(&args[1..])?).await?)
        }
        _ => {
            print_usage();
            return Err(format!("unknown command: {command}").into());
        }
    }

    Ok(())
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
        "Usage: pandamux <command>\n\nCommands:\n  ping\n  identify\n  capabilities\n  notify <message> [--body <text>] [--source build|agent|deploy|port|generic]\n  list-notifications\n  clear-notifications [id]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_notify_params() {
        let params = notify_params(&[
            "Build".to_string(),
            "succeeded".to_string(),
            "--body".to_string(),
            "All tests passed".to_string(),
            "--source".to_string(),
            "build".to_string(),
        ])
        .expect("notify params should parse");
        assert_eq!(params["title"], "Build succeeded");
        assert_eq!(params["body"], "All tests passed");
        assert_eq!(params["source"], "build");
    }

    #[test]
    fn parses_clear_notifications_params() {
        let params = clear_notifications_params(&["notif-1".to_string()]).expect("clear");
        assert_eq!(params["id"], "notif-1");

        let empty = clear_notifications_params(&[]).expect("empty clear");
        assert!(empty.get("id").is_none());
    }
}
