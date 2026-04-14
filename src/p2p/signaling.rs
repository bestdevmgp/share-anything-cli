use crate::error::{CliError, Result};
use crate::p2p::protocol::{SignalingMessage, WS_PING_INTERVAL_SECS};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

pub struct SignalingClient {
    pub sender: mpsc::UnboundedSender<SignalingMessage>,
    pub receiver: mpsc::UnboundedReceiver<SignalingMessage>,
    read_handle: JoinHandle<()>,
    write_handle: JoinHandle<()>,
    ping_handle: JoinHandle<()>,
}

impl SignalingClient {
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

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<SignalingMessage>();
        let (in_tx, in_rx) = mpsc::unbounded_channel::<SignalingMessage>();
        let (ping_tx, mut ping_rx) = mpsc::unbounded_channel::<()>();

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

        let read_handle = tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_stream_rx.next().await {
                match msg {
                    Message::Text(text) => {
                        if let Ok(sig_msg) = serde_json::from_str::<SignalingMessage>(&text) {
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

        let ping_handle = tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(WS_PING_INTERVAL_SECS));
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

    pub fn send(&self, msg: SignalingMessage) -> Result<()> {
        self.sender
            .send(msg)
            .map_err(|e| CliError::WebSocket(format!("Send failed: {}", e)))
    }

    pub async fn recv(&mut self) -> Option<SignalingMessage> {
        self.receiver.recv().await
    }

    pub fn shutdown(self) {
        self.read_handle.abort();
        self.write_handle.abort();
        self.ping_handle.abort();
    }
}
