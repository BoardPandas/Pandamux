use std::fmt;
use pandamux_protocol::RpcRequest;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum TransportError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Disconnected,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "Transport I/O error: {err}"),
            Self::Json(err) => write!(f, "Transport JSON error: {err}"),
            Self::Disconnected => write!(f, "Transport disconnected"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<std::io::Error> for TransportError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for TransportError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

/// In-memory mock transport for testing and reducer verification without OS sockets.
pub struct MockTransport {
    incoming_rx: mpsc::UnboundedReceiver<String>,
    outgoing_tx: mpsc::UnboundedSender<String>,
}

impl MockTransport {
    pub fn pair() -> (Self, MockTransportPeer) {
        let (in_tx, in_rx) = mpsc::unbounded_channel();
        let (out_tx, out_rx) = mpsc::unbounded_channel();

        let transport = Self {
            incoming_rx: in_rx,
            outgoing_tx: out_tx,
        };
        let peer = MockTransportPeer {
            incoming_rx: out_rx,
            outgoing_tx: in_tx,
        };
        (transport, peer)
    }

    pub async fn send_request(&mut self, req: &RpcRequest) -> Result<(), TransportError> {
        let line = serde_json::to_string(req)?;
        self.outgoing_tx
            .send(line)
            .map_err(|_| TransportError::Disconnected)
    }

    pub async fn recv_line(&mut self) -> Result<Option<String>, TransportError> {
        Ok(self.incoming_rx.recv().await)
    }
}

/// Simulated server peer for the mock transport.
pub struct MockTransportPeer {
    incoming_rx: mpsc::UnboundedReceiver<String>,
    outgoing_tx: mpsc::UnboundedSender<String>,
}

impl MockTransportPeer {
    pub async fn recv_line(&mut self) -> Option<String> {
        self.incoming_rx.recv().await
    }

    pub fn send_line(&self, line: impl Into<String>) -> Result<(), TransportError> {
        self.outgoing_tx
            .send(line.into())
            .map_err(|_| TransportError::Disconnected)
    }
}
