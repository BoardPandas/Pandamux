use pandamux_cli::{
    cmd_identify, cmd_mcp, cmd_notify, cmd_ping, cmd_thread_list, cmd_thread_send,
    parse_cli_args, print_help, CliAction,
};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let action = match parse_cli_args(&args) {
        Ok(a) => a,
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };

    let result = match action {
        CliAction::Ping { pipe, json } => cmd_ping(pipe.as_deref(), json).await,

        CliAction::Identify { pipe, json } => cmd_identify(pipe.as_deref(), json).await,

        CliAction::ThreadList {
            pipe,
            status,
            limit,
            json,
        } => cmd_thread_list(pipe.as_deref(), status, limit, json).await,

        CliAction::ThreadSend {
            pipe,
            thread_id,
            text,
            model,
            effort,
            json,
        } => {
            cmd_thread_send(
                pipe.as_deref(),
                &thread_id,
                &text,
                model.as_deref(),
                effort.as_deref(),
                json,
            )
            .await
        }

        CliAction::Notify {
            pipe,
            title,
            body,
            source,
            json,
        } => {
            cmd_notify(
                pipe.as_deref(),
                &title,
                body.as_deref(),
                source.as_deref(),
                json,
            )
            .await
        }

        CliAction::Mcp { pipe } => cmd_mcp(pipe.as_deref()).await,

        CliAction::Help => {
            print_help();
            Ok(())
        }

        CliAction::Version => {
            println!("pandamux v{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
