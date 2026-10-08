use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pandamux_core::{EnvironmentId, ThreadEvent, ThreadEventKind, ThreadId, event::TurnOutcome};
use pandamux_protocol::{
    PROTOCOL_VERSION, PROXY_PROTOCOL_VERSION, ProxyMessage, RpcRequest, ServerRole,
};
use pandamux_server::{
    EnvironmentTransport, ProxyEnvironmentTransport, Server, ServerConfig, serve_proxy_connection,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

fn test_server(environment_id: &str) -> Arc<Server> {
    Arc::new(
        Server::new(ServerConfig {
            role: ServerRole::Node,
            environment_id: environment_id.to_string(),
            runtime_dir: std::env::temp_dir().join("pandamux-proxy-test-runtime"),
            db_path: None,
        })
        .expect("create test server"),
    )
}

async fn read_message(reader: &mut BufReader<tokio::io::ReadHalf<DuplexStream>>) -> ProxyMessage {
    let mut line = String::new();
    reader.read_line(&mut line).await.expect("read frame");
    serde_json::from_str(line.trim_end()).expect("parse frame")
}

async fn write_message(writer: &mut tokio::io::WriteHalf<DuplexStream>, message: &ProxyMessage) {
    let mut bytes = serde_json::to_vec(message).expect("serialize frame");
    bytes.push(b'\n');
    writer.write_all(&bytes).await.expect("write frame");
}

async fn accept_hello(
    stream: DuplexStream,
    environment_id: EnvironmentId,
    proxy_version: u32,
) -> (
    BufReader<tokio::io::ReadHalf<DuplexStream>>,
    tokio::io::WriteHalf<DuplexStream>,
) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let hello = read_message(&mut reader).await;
    assert!(matches!(hello, ProxyMessage::Hello { .. }));
    write_message(
        &mut writer,
        &ProxyMessage::HelloAck {
            proxy_protocol_version: proxy_version,
            rpc_protocol_version: PROTOCOL_VERSION,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            environment_id,
            current_event_seq: 0,
        },
    )
    .await;
    (reader, writer)
}

#[tokio::test]
async fn production_proxy_routes_requests_relays_events_and_joins_cleanup() {
    let environment_id = EnvironmentId::from("env-remote");
    let server = test_server(environment_id.as_str());
    let (hub_stream, node_stream) = tokio::io::duplex(64 * 1024);
    let node = tokio::spawn(serve_proxy_connection(server.clone(), node_stream));

    let transport = server
        .register_proxy_stream(environment_id.clone(), hub_stream)
        .await
        .expect("register production proxy transport");

    let response = server
        .router()
        .environment_router()
        .forward_request(
            &environment_id,
            &RpcRequest::new(7, "system.identify", None),
        )
        .await
        .expect("route system.identify through environment router");
    let result = response.result.expect("identify result");
    assert_eq!(result["role"], "node");
    assert_eq!(result["environmentId"], environment_id.as_str());

    let thread_id = ThreadId::from("thread-proxy-1");
    let event = ThreadEvent {
        thread_id: thread_id.clone(),
        seq: 1,
        at_ms: 123,
        kind: ThreadEventKind::TurnCompleted {
            outcome: TurnOutcome::Success,
        },
    };
    server.store().append_event(&event).expect("store event");
    let mut events = transport.subscribe_events();
    server
        .router()
        .thread_manager()
        .broadcaster()
        .send(pandamux_protocol::EventEnvelope {
            subscription_id: String::new(),
            environment_id: environment_id.clone(),
            thread_id: Some(thread_id),
            run_id: None,
            seq: 1,
            at_ms: 123,
            kind: event.kind,
        })
        .expect("broadcast event");

    let relayed = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("event relay timeout")
        .expect("event relay closed");
    assert_eq!(relayed.environment_id, environment_id);
    assert_eq!(transport.last_event_seq(), 1);

    server.disconnect_remote_environment(&environment_id).await;
    assert_eq!(
        transport.state(),
        pandamux_server::EnvironmentTransportState::Closed
    );
    tokio::time::timeout(Duration::from_secs(1), node)
        .await
        .expect("node proxy cleanup timeout")
        .expect("join node proxy task")
        .expect("node proxy result");
}

#[tokio::test]
async fn graceful_shutdown_drains_the_remote_goodbye_before_returning() {
    let environment_id = EnvironmentId::from("env-drain-goodbye");
    let (hub_stream, node_stream) = tokio::io::duplex(16 * 1024);
    let expected_environment_id = environment_id.clone();
    let (goodbye_tx, goodbye_rx) = tokio::sync::oneshot::channel();
    let fake_node = tokio::spawn(async move {
        let (mut reader, mut writer) =
            accept_hello(node_stream, expected_environment_id, PROXY_PROTOCOL_VERSION).await;
        loop {
            match read_message(&mut reader).await {
                ProxyMessage::Keepalive { sequence } => {
                    write_message(&mut writer, &ProxyMessage::KeepaliveAck { sequence }).await;
                }
                ProxyMessage::Goodbye { .. } => break,
                other => panic!("expected keepalive or goodbye, got {other:?}"),
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        write_message(
            &mut writer,
            &ProxyMessage::Goodbye {
                reason: "node drained".to_string(),
            },
        )
        .await;
        goodbye_tx.send(()).expect("report remote goodbye");
    });

    let transport =
        ProxyEnvironmentTransport::connect(hub_stream, environment_id, Arc::new(AtomicU64::new(0)))
            .await
            .expect("connect fake node");
    transport.shutdown().await;
    goodbye_rx
        .await
        .expect("shutdown returned before remote goodbye");
    fake_node.await.expect("join fake node");
}

#[tokio::test]
async fn dropped_request_sends_cancel_frame() {
    let environment_id = EnvironmentId::from("env-cancel");
    let (hub_stream, node_stream) = tokio::io::duplex(16 * 1024);
    let expected_environment_id = environment_id.clone();
    let fake_node = tokio::spawn(async move {
        let (mut reader, _writer) =
            accept_hello(node_stream, expected_environment_id, PROXY_PROTOCOL_VERSION).await;
        let request_id = match read_message(&mut reader).await {
            ProxyMessage::Request { request } => request.id.expect("request id"),
            other => panic!("expected request, got {other:?}"),
        };
        match read_message(&mut reader).await {
            ProxyMessage::Cancel { id } => assert_eq!(id, request_id),
            other => panic!("expected cancellation, got {other:?}"),
        }
    });

    let transport =
        ProxyEnvironmentTransport::connect(hub_stream, environment_id, Arc::new(AtomicU64::new(0)))
            .await
            .expect("connect fake node");
    let request = RpcRequest::new(11, "thread.send_turn", None);
    let timed_out = tokio::time::timeout(
        Duration::from_millis(25),
        transport.forward_request(&request),
    )
    .await;
    assert!(timed_out.is_err(), "request should still be pending");
    tokio::time::timeout(Duration::from_secs(1), fake_node)
        .await
        .expect("cancel frame timeout")
        .expect("join fake node");
    transport.shutdown().await;
}

#[tokio::test]
async fn mismatched_proxy_protocol_is_rejected() {
    let environment_id = EnvironmentId::from("env-mismatch");
    let (hub_stream, node_stream) = tokio::io::duplex(8 * 1024);
    let expected_environment_id = environment_id.clone();
    let fake_node = tokio::spawn(async move {
        let _ = accept_hello(
            node_stream,
            expected_environment_id,
            PROXY_PROTOCOL_VERSION + 1,
        )
        .await;
    });

    let error =
        ProxyEnvironmentTransport::connect(hub_stream, environment_id, Arc::new(AtomicU64::new(0)))
            .await
            .expect_err("protocol mismatch must fail");
    assert!(error.message.contains("proxy protocol"));
    fake_node.await.expect("join fake node");
}

#[tokio::test]
async fn reconnect_hello_resumes_after_last_durable_event() {
    let environment_id = EnvironmentId::from("env-reconnect");
    let cursor = Arc::new(AtomicU64::new(0));
    let (hub_stream, node_stream) = tokio::io::duplex(16 * 1024);
    let expected_environment_id = environment_id.clone();
    let first_node = tokio::spawn(async move {
        let (mut reader, mut writer) = accept_hello(
            node_stream,
            expected_environment_id.clone(),
            PROXY_PROTOCOL_VERSION,
        )
        .await;
        write_message(
            &mut writer,
            &ProxyMessage::Event {
                event_seq: 9,
                envelope: pandamux_protocol::EventEnvelope {
                    subscription_id: "proxy".to_string(),
                    environment_id: expected_environment_id,
                    thread_id: Some(ThreadId::from("thread-reconnect")),
                    run_id: None,
                    seq: 2,
                    at_ms: 200,
                    kind: ThreadEventKind::TurnCompleted {
                        outcome: TurnOutcome::Success,
                    },
                },
            },
        )
        .await;
        let _ = read_message(&mut reader).await;
    });
    let first =
        ProxyEnvironmentTransport::connect(hub_stream, environment_id.clone(), cursor.clone())
            .await
            .expect("first connection");
    let mut events = first.subscribe_events();
    tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("first event timeout")
        .expect("first event");
    assert_eq!(cursor.load(Ordering::SeqCst), 9);
    first.shutdown().await;
    first_node.await.expect("join first node");

    let (hub_stream, node_stream) = tokio::io::duplex(8 * 1024);
    let expected_environment_id = environment_id.clone();
    let second_node = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(node_stream);
        let mut reader = BufReader::new(reader);
        match read_message(&mut reader).await {
            ProxyMessage::Hello {
                resume_after_event_seq,
                ..
            } => assert_eq!(resume_after_event_seq, 9),
            other => panic!("expected hello, got {other:?}"),
        }
        write_message(
            &mut writer,
            &ProxyMessage::HelloAck {
                proxy_protocol_version: PROXY_PROTOCOL_VERSION,
                rpc_protocol_version: PROTOCOL_VERSION,
                server_version: env!("CARGO_PKG_VERSION").to_string(),
                environment_id: expected_environment_id,
                current_event_seq: 9,
            },
        )
        .await;
        let _ = read_message(&mut reader).await;
    });
    let second = ProxyEnvironmentTransport::connect(hub_stream, environment_id, cursor)
        .await
        .expect("reconnected transport");
    second.shutdown().await;
    second_node.await.expect("join second node");
}

#[tokio::test]
async fn abrupt_proxy_loss_surfaces_stale_without_local_fallback() {
    let environment_id = EnvironmentId::from("env-abrupt-loss");
    let server = test_server("env-local");
    let (hub_stream, node_stream) = tokio::io::duplex(8 * 1024);
    let (drop_tx, drop_rx) = tokio::sync::oneshot::channel();
    let expected_environment_id = environment_id.clone();
    let fake_node = tokio::spawn(async move {
        let (_reader, _writer) =
            accept_hello(node_stream, expected_environment_id, PROXY_PROTOCOL_VERSION).await;
        let _ = drop_rx.await;
    });

    let transport = server
        .register_proxy_stream(environment_id.clone(), hub_stream)
        .await
        .expect("register proxy before abrupt loss");
    drop_tx.send(()).expect("release fake node");
    fake_node.await.expect("join abruptly closed fake node");

    tokio::time::timeout(Duration::from_secs(1), async {
        while transport.state() == pandamux_server::EnvironmentTransportState::Connected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("transport should become stale");

    let error = server
        .router()
        .environment_router()
        .forward_request(
            &environment_id,
            &RpcRequest::new(21, "system.identify", None),
        )
        .await
        .expect_err("stale remote request must not run locally");
    assert!(error.message.contains("stale"), "{error:?}");

    tokio::time::timeout(
        Duration::from_secs(1),
        server.disconnect_remote_environment(&environment_id),
    )
    .await
    .expect("stale transport cleanup should join");
}

#[cfg(unix)]
#[tokio::test]
async fn proxy_subcommand_bridges_stdio_to_the_node_daemon() {
    use std::process::Stdio;
    use tokio::process::Command;

    let temp = tempfile::tempdir().expect("create daemon temp dir");
    let runtime_dir = temp.path().join("run");
    let binary = env!("CARGO_BIN_EXE_pandamux-server");
    let mut daemon = Command::new(binary)
        .arg("daemon")
        .arg("--runtime-dir")
        .arg(&runtime_dir)
        .arg("--environment-id")
        .arg("env-process-proxy")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn node daemon");

    tokio::time::timeout(Duration::from_secs(3), async {
        while !tokio::fs::try_exists(runtime_dir.join("server.json"))
            .await
            .expect("check node runtime record")
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("node runtime record timeout");

    let mut proxy = Command::new(binary)
        .arg("proxy")
        .arg("--runtime-dir")
        .arg(&runtime_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn proxy bridge");
    let proxy_stdout = proxy.stdout.take().expect("proxy stdout");
    let proxy_stdin = proxy.stdin.take().expect("proxy stdin");
    let proxy_stdio = tokio::io::join(proxy_stdout, proxy_stdin);
    let transport = ProxyEnvironmentTransport::connect(
        proxy_stdio,
        EnvironmentId::from("env-process-proxy"),
        Arc::new(AtomicU64::new(0)),
    )
    .await
    .expect("connect through proxy subcommand");

    let response = transport
        .forward_request(&RpcRequest::new(31, "system.identify", None))
        .await
        .expect("identify through process proxy");
    assert_eq!(
        response.result.expect("identify result")["environmentId"],
        "env-process-proxy"
    );

    transport.shutdown().await;
    let proxy_status = tokio::time::timeout(Duration::from_secs(2), proxy.wait())
        .await
        .expect("proxy process cleanup timeout")
        .expect("wait for proxy process");
    assert!(proxy_status.success(), "proxy status was {proxy_status}");

    daemon.start_kill().expect("stop node daemon");
    tokio::time::timeout(Duration::from_secs(2), daemon.wait())
        .await
        .expect("node daemon cleanup timeout")
        .expect("wait for node daemon");
}
