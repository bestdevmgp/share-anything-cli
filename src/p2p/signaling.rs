use crate::error::{CliError, Result};
use crate::p2p::protocol::{SignalingMessage, WS_PING_INTERVAL_SECS};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

/// WebSocket signaling client for P2P transfer.
///
/// Spawns background tasks for reading, writing, and keepalive pings.
pub struct SignalingClient {
    /// Send signaling messages to the WebSocket.
    pub sender: mpsc::UnboundedSender<SignalingMessage>,
    /// Receive signaling messages from the WebSocket (Pong filtered out).
    pub receiver: mpsc::UnboundedReceiver<SignalingMessage>,
    read_handle: JoinHandle<()>,
    write_handle: JoinHandle<()>,
    ping_handle: JoinHandle<()>,
}

impl SignalingClient {
    /// Connect to the signaling WebSocket server.
    ///
    /// Converts the API base URL from http(s) to ws(s) and connects to `/ws/signaling`.
    pub async fn connect(api_base_url: &str) -> Result<Self> {
        let ws_url = api_base_url
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        let ws_url = format!("{}/ws/signaling", ws_url);

        let (ws_stream, _) =
            tokio_tungstenite::connect_async(&ws_url)
                .await
                .map_err(|e| CliError::WebSocket(format!("Failed to connect: {}", e)))?;

        let (mut ws_sink, mut ws_stream_rx) = ws_stream.split();

        // Channel: outgoing signaling messages -> WS write task
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<SignalingMessage>();

        // Channel: WS read task -> incoming signaling messages
        let (in_tx, in_rx) = mpsc::unbounded_channel::<SignalingMessage>();

        // Channel for ping task to send pings through the write task
        let (ping_tx, mut ping_rx) = mpsc::unbounded_channel::<()>();

        // Write task: sends outgoing signaling messages and pings
        let write_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(msg) = out_rx.recv() => {
                        let json = match serde_json::to_string(&msg) {
                            Ok(j) => j,
                            Err(_) => continue,
                        };
                        if ws_sink.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Some(()) = ping_rx.recv() => {
                        let ping_msg = serde_json::json!({"type": "ping"}).to_string();
                        if ws_sink.send(Message::Text(ping_msg)).await.is_err() {
                            break;
                        }
                    }
                    else => break,
                }
            }
        });

        // Read task: reads from WS and forwards to in_tx (filters out Pong)
        let read_handle = tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_stream_rx.next().await {
                match msg {
                    Message::Text(text) => {
                        if let Ok(sig_msg) = serde_json::from_str::<SignalingMessage>(&text) {
                            // Filter out Pong messages
                            if matches!(sig_msg, SignalingMessage::Pong {}) {
                                continue;
                            }
                            if in_tx.send(sig_msg).is_err() {
                                break;
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        });

        // Ping keepalive task
        let ping_handle = tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(WS_PING_INTERVAL_SECS));
            // Skip the first immediate tick
            interval.tick().await;
            loop {
                interval.tick().await;
                if ping_tx.send(()).is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            sender: out_tx,
            receiver: in_rx,
            read_handle,
            write_handle,
            ping_handle,
        })
    }

    /// Send a signaling message.
    pub fn send(&self, msg: SignalingMessage) -> Result<()> {
        self.sender
            .send(msg)
            .map_err(|e| CliError::WebSocket(format!("Send failed: {}", e)))
    }

    /// Receive the next signaling message (Pong messages are already filtered).
    pub async fn recv(&mut self) -> Option<SignalingMessage> {
        self.receiver.recv().await
    }

    /// Shut down all background tasks.
    pub fn shutdown(self) {
        self.read_handle.abort();
        self.write_handle.abort();
        self.ping_handle.abort();
    }
}
