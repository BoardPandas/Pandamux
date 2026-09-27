pub mod commands;
pub mod ipc;

pub use commands::*;
pub use ipc::*;
use pandamux_core::ThreadStatus;

/// Enumeration of parsed CLI actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliAction {
    Ping {
        pipe: Option<String>,
        json: bool,
    },
    Identify {
        pipe: Option<String>,
        json: bool,
    },
    ThreadList {
        pipe: Option<String>,
        status: Option<ThreadStatus>,
        limit: Option<usize>,
        json: bool,
    },
    ThreadSend {
        pipe: Option<String>,
        thread_id: String,
        text: String,
        model: Option<String>,
        effort: Option<String>,
        json: bool,
    },
    Notify {
        pipe: Option<String>,
        title: String,
        body: Option<String>,
        source: Option<String>,
        json: bool,
    },
    Mcp {
        pipe: Option<String>,
    },
    Help,
    Version,
}

/// Parses command-line arguments into a structured `CliAction`.
pub fn parse_cli_args(args: &[String]) -> Result<CliAction, String> {
    if args.is_empty() {
        return Ok(CliAction::Help);
    }

    let mut pipe = None;
    let mut json = false;
    let mut positional = Vec::new();
    let mut index = 0;

    // First extract global flags: --pipe, --json, -h, --help, -V, --version
    while index < args.len() {
        match args[index].as_str() {
            "--pipe" => {
                let val = args
                    .get(index + 1)
                    .ok_or_else(|| "--pipe requires a path argument".to_string())?;
                pipe = Some(val.clone());
                index += 2;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            "-h" | "--help" | "help" => return Ok(CliAction::Help),
            "-V" | "--version" | "version" => return Ok(CliAction::Version),
            _ => {
                positional.push(args[index].clone());
                index += 1;
            }
        }
    }

    if positional.is_empty() {
        return Ok(CliAction::Help);
    }

    match positional[0].as_str() {
        "ping" => Ok(CliAction::Ping { pipe, json }),

        "identify" | "id" => Ok(CliAction::Identify { pipe, json }),

        "thread" => {
            let sub = positional.get(1).map(String::as_str).unwrap_or("");
            match sub {
                "list" | "ls" => {
                    let mut status = None;
                    let mut limit = None;
                    let mut i = 2;
                    while i < positional.len() {
                        match positional[i].as_str() {
                            "--status" => {
                                let val = positional
                                    .get(i + 1)
                                    .ok_or_else(|| "--status requires a status value".to_string())?;
                                status = Some(parse_status(val)?);
                                i += 2;
                            }
                            "--limit" => {
                                let val = positional
                                    .get(i + 1)
                                    .ok_or_else(|| "--limit requires a number".to_string())?;
                                limit = val.parse::<usize>().ok();
                                i += 2;
                            }
                            _ => i += 1,
                        }
                    }
                    Ok(CliAction::ThreadList {
                        pipe,
                        status,
                        limit,
                        json,
                    })
                }

                "send" => {
                    let thread_id = positional
                        .get(2)
                        .ok_or_else(|| "Usage: pandamux thread send <thread_id> <prompt>".to_string())?
                        .clone();

                    let mut text_parts = Vec::new();
                    let mut model = None;
                    let mut effort = None;
                    let mut i = 3;
                    while i < positional.len() {
                        match positional[i].as_str() {
                            "--model" => {
                                model = positional.get(i + 1).cloned();
                                i += 2;
                            }
                            "--effort" => {
                                effort = positional.get(i + 1).cloned();
                                i += 2;
                            }
                            _ => {
                                text_parts.push(positional[i].clone());
                                i += 1;
                            }
                        }
                    }

                    if text_parts.is_empty() {
                        return Err("Thread send requires a prompt text".to_string());
                    }

                    Ok(CliAction::ThreadSend {
                        pipe,
                        thread_id,
                        text: text_parts.join(" "),
                        model,
                        effort,
                        json,
                    })
                }

                _ => Err(format!(
                    "Unknown thread subcommand: '{}'. Supported: list, send",
                    sub
                )),
            }
        }

        "notify" => {
            let mut text_parts = Vec::new();
            let mut body = None;
            let mut source = None;
            let mut i = 1;
            while i < positional.len() {
                match positional[i].as_str() {
                    "--body" => {
                        body = positional.get(i + 1).cloned();
                        i += 2;
                    }
                    "--source" => {
                        source = positional.get(i + 1).cloned();
                        i += 2;
                    }
                    _ => {
                        text_parts.push(positional[i].clone());
                        i += 1;
                    }
                }
            }

            if text_parts.is_empty() {
                return Err("Usage: pandamux notify <title> [--body <body>] [--source <build|agent|deploy|port|generic>]".to_string());
            }

            Ok(CliAction::Notify {
                pipe,
                title: text_parts.join(" "),
                body,
                source,
                json,
            })
        }

        "mcp" => Ok(CliAction::Mcp { pipe }),

        unknown => Err(format!("Unknown command: '{unknown}'. Run 'pandamux --help' for usage.")),
    }
}

fn parse_status(val: &str) -> Result<ThreadStatus, String> {
    match val.to_lowercase().as_str() {
        "idle" => Ok(ThreadStatus::Idle),
        "working" => Ok(ThreadStatus::Working),
        "awaiting_approval" | "awaitingapproval" | "approval" => {
            Ok(ThreadStatus::AwaitingApproval)
        }
        "errored" | "error" => Ok(ThreadStatus::Errored),
        "paused" => Ok(ThreadStatus::Paused),
        "archived" => Ok(ThreadStatus::Archived),
        _ => Err(format!("Invalid thread status: '{val}'")),
    }
}

pub fn print_help() {
    println!("PandaMUX CLI (v{}) - Native Terminal Multiplexer & AI Agent CLI", env!("CARGO_PKG_VERSION"));
    println!();
    println!("USAGE:");
    println!("  pandamux <COMMAND> [OPTIONS]");
    println!();
    println!("COMMANDS:");
    println!("  ping                       Check connectivity and latency to PandaMUX server");
    println!("  identify                   Inspect node identification, platform, and capabilities");
    println!("  thread list                List active threads and turns");
    println!("  thread send <ID> <PROMPT>  Send a prompt to a thread with streaming output");
    println!("  notify <TITLE>             Send a desktop notification to PandaMUX clients");
    println!("  mcp                        Run interactive Model Context Protocol stdio bridge");
    println!();
    println!("OPTIONS:");
    println!("  --pipe <PATH>              Specify explicit IPC pipe or Unix domain socket");
    println!("  --json                     Output response as raw JSON");
    println!("  -h, --help                 Display this help message");
    println!("  -V, --version              Display version information");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ping_and_flags() {
        let args = vec!["ping".to_string(), "--json".to_string()];
        let action = parse_cli_args(&args).expect("parse ping");
        assert_eq!(
            action,
            CliAction::Ping {
                pipe: None,
                json: true
            }
        );

        let args_pipe = vec![
            "--pipe".to_string(),
            "/tmp/custom.sock".to_string(),
            "ping".to_string(),
        ];
        let action_pipe = parse_cli_args(&args_pipe).expect("parse ping with pipe");
        assert_eq!(
            action_pipe,
            CliAction::Ping {
                pipe: Some("/tmp/custom.sock".to_string()),
                json: false
            }
        );
    }

    #[test]
    fn test_parse_identify() {
        let args = vec!["identify".to_string()];
        let action = parse_cli_args(&args).expect("parse identify");
        assert_eq!(
            action,
            CliAction::Identify {
                pipe: None,
                json: false
            }
        );
    }

    #[test]
    fn test_parse_thread_list_and_send() {
        let list_args = vec![
            "thread".to_string(),
            "list".to_string(),
            "--status".to_string(),
            "idle".to_string(),
            "--limit".to_string(),
            "10".to_string(),
        ];
        let action_list = parse_cli_args(&list_args).expect("parse thread list");
        assert_eq!(
            action_list,
            CliAction::ThreadList {
                pipe: None,
                status: Some(ThreadStatus::Idle),
                limit: Some(10),
                json: false
            }
        );

        let send_args = vec![
            "thread".to_string(),
            "send".to_string(),
            "thread-123".to_string(),
            "Explain".to_string(),
            "the".to_string(),
            "code".to_string(),
            "--model".to_string(),
            "claude-3-7-sonnet".to_string(),
            "--effort".to_string(),
            "high".to_string(),
        ];
        let action_send = parse_cli_args(&send_args).expect("parse thread send");
        assert_eq!(
            action_send,
            CliAction::ThreadSend {
                pipe: None,
                thread_id: "thread-123".to_string(),
                text: "Explain the code".to_string(),
                model: Some("claude-3-7-sonnet".to_string()),
                effort: Some("high".to_string()),
                json: false
            }
        );
    }

    #[test]
    fn test_parse_notify() {
        let args = vec![
            "notify".to_string(),
            "Build".to_string(),
            "Finished".to_string(),
            "--body".to_string(),
            "All 42 tests passed".to_string(),
            "--source".to_string(),
            "build".to_string(),
        ];
        let action = parse_cli_args(&args).expect("parse notify");
        assert_eq!(
            action,
            CliAction::Notify {
                pipe: None,
                title: "Build Finished".to_string(),
                body: Some("All 42 tests passed".to_string()),
                source: Some("build".to_string()),
                json: false
            }
        );
    }

    #[test]
    fn test_parse_mcp() {
        let args = vec!["mcp".to_string()];
        let action = parse_cli_args(&args).expect("parse mcp");
        assert_eq!(action, CliAction::Mcp { pipe: None });
    }
}
