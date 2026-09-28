use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pandamux_core::EnvironmentId;
use pandamux_protocol::{
    PROTOCOL_VERSION, PROXY_PROTOCOL_VERSION, ProxyMessage, RpcError, RpcId, RpcResponse,
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::{AbortHandle, JoinSet};

use crate::{ClientIdentity, RequestContext, Server};

const MAX_PROXY_FRAME_BYTES: usize = 1024 * 1024;
const OUTBOUND_QUEUE_CAPACITY: usize = 256;
const MAX_IN_FLIGHT_REQUESTS: usize = 32;
const REPLAY_PAGE_SIZE: usize = 128;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

pub async fn serve_proxy_connection<S>(server: Arc<Server>, stream: S) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let Some(hello) = read_proxy_message(&mut reader).await? else {
        return Ok(());
    };

    let (
        proxy_protocol_version,
        rpc_protocol_version,
        expected_environment_id,
        resume_after_event_seq,
    ) = match hello {
        ProxyMessage::Hello {
            proxy_protocol_version,
            rpc_protocol_version,
            environment_id,
            resume_after_event_seq,
            ..
        } => (
            proxy_protocol_version,
            rpc_protocol_version,
            environment_id,
            resume_after_event_seq,
        ),
        _ => {
            write_proxy_message(
                &mut write_half,
                &ProxyMessage::Error {
                    code: "proxy_hello_required".to_string(),
                    message: "the first proxy frame must be hello".to_string(),
                },
            )
            .await?;
            return Ok(());
        }
    };

    if proxy_protocol_version != PROXY_PROTOCOL_VERSION {
        write_proxy_message(
            &mut write_half,
            &ProxyMessage::Error {
                code: "proxy_protocol_mismatch".to_string(),
                message: format!(
                    "node supports proxy protocol {}, Hub requested {}",
                    PROXY_PROTOCOL_VERSION, proxy_protocol_version
                ),
            },
        )
        .await?;
        return Ok(());
    }
    if rpc_protocol_version != PROTOCOL_VERSION {
        write_proxy_message(
            &mut write_half,
            &ProxyMessage::Error {
                code: "rpc_protocol_mismatch".to_string(),
                message: format!(
                    "node supports RPC protocol {}, Hub requested {}",
                    PROTOCOL_VERSION, rpc_protocol_version
                ),
            },
        )
        .await?;
        return Ok(());
    }

    let environment_id = EnvironmentId::from(server.environment_id());
    if expected_environment_id != environment_id {
        write_proxy_message(
            &mut write_half,
            &ProxyMessage::Error {
                code: "proxy_environment_mismatch".to_string(),
                message: format!(
                    "proxy requested environment '{}', node owns '{}'",
                    expected_environment_id.as_str(),
                    environment_id.as_str()
                ),
            },
        )
        .await?;
        return Ok(());
    }
    let request_context = Arc::new(RequestContext::node_proxy(
        environment_id.clone(),
        ClientIdentity {
            description: "ssh-proxy".to_string(),
            ..ClientIdentity::default()
        },
    ));

    let mut live_events = server.router().thread_manager().subscribe();
    let replay_through = server
        .store()
        .latest_event_row_id()
        .map_err(store_io_error)?;
    let (outbound, mut outbound_rx) = mpsc::channel::<ProxyMessage>(OUTBOUND_QUEUE_CAPACITY);
    let writer = tokio::spawn(async move {
        while let Some(message) = outbound_rx.recv().await {
            let goodbye = matches!(message, ProxyMessage::Goodbye { .. });
            write_proxy_message(&mut write_half, &message).await?;
            if goodbye {
                break;
            }
        }
        write_half.shutdown().await
    });

    outbound
        .send(ProxyMessage::HelloAck {
            proxy_protocol_version: PROXY_PROTOCOL_VERSION,
            rpc_protocol_version: PROTOCOL_VERSION,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            environment_id: environment_id.clone(),
            current_event_seq: replay_through,
        })
        .await
        .map_err(queue_closed)?;

    let mut replay_cursor = resume_after_event_seq;
    while replay_cursor < replay_through {
        let page = server
            .store()
            .get_event_envelopes_after(
                &environment_id,
                replay_cursor,
                replay_through,
                REPLAY_PAGE_SIZE,
            )
            .map_err(store_io_error)?;
        if page.is_empty() {
            break;
        }
        for (event_seq, envelope) in page {
            outbound
                .send(ProxyMessage::Event {
                    event_seq,
                    envelope,
                })
                .await
                .map_err(queue_closed)?;
            replay_cursor = event_seq;
        }
    }

    let (event_shutdown_tx, mut event_shutdown_rx) = watch::channel(false);
    let event_outbound = outbound.clone();
    let event_store = server.store().clone();
    let event_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = event_shutdown_rx.changed() => {
                    if changed.is_err() || *event_shutdown_rx.borrow() {
                        break;
                    }
                }
                event = live_events.recv() => {
                    match event {
                        Ok(envelope) => {
                            let Some(thread_id) = envelope.thread_id.as_ref() else {
                                continue;
                            };
                            let event_seq = match event_store.event_row_id(thread_id, envelope.seq) {
                                Ok(Some(value)) => value,
                                Ok(None) => continue,
                                Err(error) => {
                                    let _ = event_outbound.send(ProxyMessage::Error {
                                        code: "proxy_event_store_failed".to_string(),
                                        message: error.to_string(),
                                    }).await;
                                    break;
                                }
                            };
                            if event_seq <= replay_through {
                                continue;
                            }
                            if event_outbound.send(ProxyMessage::Event {
                                event_seq,
                                envelope,
                            }).await.is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(count)) => {
                            let _ = event_outbound.send(ProxyMessage::Error {
                                code: "proxy_event_lagged".to_string(),
                                message: format!("node proxy event relay lagged by {count} event(s)"),
                            }).await;
                            break;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    });

    let mut requests = JoinSet::<RpcId>::new();
    let mut active: HashMap<RpcId, AbortHandle> = HashMap::new();
    let notification_counter = AtomicU64::new(1);
    let mut close_reason = "proxy input reached EOF".to_string();

    loop {
        tokio::select! {
            completed = requests.join_next(), if !requests.is_empty() => {
                if let Some(Ok(id)) = completed {
                    active.remove(&id);
                }
            }
            message = read_proxy_message(&mut reader) => {
                match message? {
                    Some(ProxyMessage::Request { request }) => {
                        if requests.len() >= MAX_IN_FLIGHT_REQUESTS {
                            if let Some(id) = request.id {
                                outbound.send(ProxyMessage::Response {
                                    response: RpcResponse::error(
                                        Some(id),
                                        RpcError::with_data(
                                            -32011,
                                            "node proxy request capacity reached",
                                            serde_json::json!({
                                                "maxInFlight": MAX_IN_FLIGHT_REQUESTS,
                                            }),
                                        ),
                                    ),
                                }).await.map_err(queue_closed)?;
                            }
                            continue;
                        }

                        let request_id = request.id.clone().unwrap_or_else(|| {
                            RpcId::String(format!(
                                "notification-{}",
                                notification_counter.fetch_add(1, Ordering::SeqCst)
                            ))
                        });
                        if active.contains_key(&request_id) {
                            outbound.send(ProxyMessage::Response {
                                response: RpcResponse::error(
                                    Some(request_id),
                                    RpcError::conflict("duplicate in-flight proxy request id"),
                                ),
                            }).await.map_err(queue_closed)?;
                            continue;
                        }

                        let request_id_for_task = request_id.clone();
                        let request_outbound = outbound.clone();
                        let request_server = server.clone();
                        let request_context = request_context.clone();
                        let abort_handle = requests.spawn(async move {
                            if let Some(response) = request_server
                                .router()
                                .handle_request_with_context(&request_context, request)
                                .await
                            {
                                let _ = request_outbound
                                    .send(ProxyMessage::Response { response })
                                    .await;
                            }
                            request_id_for_task
                        });
                        active.insert(request_id, abort_handle);
                    }
                    Some(ProxyMessage::Cancel { id }) => {
                        if let Some(handle) = active.remove(&id) {
                            handle.abort();
                        }
                    }
                    Some(ProxyMessage::Goodbye { reason }) => {
                        close_reason = reason;
                        break;
                    }
                    Some(ProxyMessage::Hello { .. }) => {
                        outbound.send(ProxyMessage::Error {
                            code: "proxy_duplicate_hello".to_string(),
                            message: "proxy hello is valid only as the first frame".to_string(),
                        }).await.map_err(queue_closed)?;
                        break;
                    }
                    Some(ProxyMessage::HelloAck { .. })
                    | Some(ProxyMessage::Response { .. })
                    | Some(ProxyMessage::Event { .. })
                    | Some(ProxyMessage::Error { .. }) => {
                        outbound.send(ProxyMessage::Error {
                            code: "proxy_direction_violation".to_string(),
                            message: "Hub sent a node-only proxy frame".to_string(),
                        }).await.map_err(queue_closed)?;
                        break;
                    }
                    None => break,
                }
            }
        }
    }

    requests.abort_all();
    while requests.join_next().await.is_some() {}
    let _ = event_shutdown_tx.send(true);
    join_or_abort(event_task).await;
    let _ = outbound
        .send(ProxyMessage::Goodbye {
            reason: close_reason,
        })
        .await;
    drop(outbound);
    join_writer(writer).await
}

pub(crate) async fn read_proxy_message<R>(reader: &mut R) -> io::Result<Option<ProxyMessage>>
where
    R: AsyncBufRead + Unpin,
{
    let mut frame = Vec::new();
    let read = reader.read_until(b'\n', &mut frame).await?;
    if read == 0 {
        return Ok(None);
    }
    if frame.len() > MAX_PROXY_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("proxy frame exceeds {MAX_PROXY_FRAME_BYTES} bytes"),
        ));
    }
    while matches!(frame.last(), Some(b'\n' | b'\r')) {
        frame.pop();
    }
    let message = serde_json::from_slice(&frame)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(message))
}

pub(crate) async fn write_proxy_message<W>(writer: &mut W, message: &ProxyMessage) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut frame = serde_json::to_vec(message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if frame.len() + 1 > MAX_PROXY_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("proxy frame exceeds {MAX_PROXY_FRAME_BYTES} bytes"),
        ));
    }
    frame.push(b'\n');
    writer.write_all(&frame).await?;
    writer.flush().await
}

async fn join_or_abort(mut task: tokio::task::JoinHandle<()>) {
    if tokio::time::timeout(CLEANUP_TIMEOUT, &mut task)
        .await
        .is_err()
    {
        task.abort();
        let _ = task.await;
    }
}

async fn join_writer(mut task: tokio::task::JoinHandle<io::Result<()>>) -> io::Result<()> {
    match tokio::time::timeout(CLEANUP_TIMEOUT, &mut task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(io::Error::other(format!(
            "proxy writer task failed: {error}"
        ))),
        Err(_) => {
            task.abort();
            let _ = task.await;
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "proxy writer cleanup timed out",
            ))
        }
    }
}

fn store_io_error(error: crate::StoreError) -> io::Error {
    io::Error::other(error)
}

fn queue_closed(_: mpsc::error::SendError<ProxyMessage>) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "proxy output queue closed")
}
