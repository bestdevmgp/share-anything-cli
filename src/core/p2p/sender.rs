use crate::client::ApiClient;
use crate::core::error::CoreError;
use crate::p2p::protocol::{
    decode_ice_candidate, device_info_string, encode_ice_candidate, FileMetadata,
    SignalingMessage, BUFFERED_AMOUNT_HIGH, DC_CHUNK_SIZE, EOF_SIGNAL,
};

const BUFFERED_AMOUNT_LOW: usize = 1024 * 1024;
use crate::p2p::{rtc, signaling::SignalingClient};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, Notify};
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

pub type SenderEventFn = Arc<dyn Fn(SenderEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub enum SenderEvent {
    /// Share session created on the server.
    Created { share_code: String, files: Vec<FileSummary> },
    /// Receiver opened the share page (but transfer hasn't begun yet).
    ReceiverArrived { device_info: Option<String> },
    /// Receiver matched and WebRTC negotiation starting.
    PeerMatched { device_info: Option<String> },
    /// A specific file is starting to transfer.
    FileStart { name: String, size: u64 },
    /// More bytes sent for the current file.
    Progress { delta: u64 },
    /// Current file finished; waiting for the receiver to request the next file.
    FileEnd,
    /// File sent; session is alive but idle — waiting for the next receiver request.
    WaitingForNext,
    /// Receiver explicitly ended the session (Done click forwarded from server).
    TransferComplete,
    /// Receiver disconnected mid-flight.
    ReceiverDisconnected,
    /// ICE selected a TURN relay candidate — bytes are going through a relay, expect slower
    /// throughput.
    RelayDetected,
    /// Fatal transfer failure.
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct FileSummary {
    pub name: String,
    pub size: u64,
}

pub struct SenderOptions {
    pub files: Vec<PathBuf>,
    pub stdin_data: Option<Vec<u8>>,
    pub stdin_name: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Serialize)]
struct P2PFileInfo {
    name: String,
    size: i64,
    #[serde(rename = "type")]
    content_type: String,
}

#[derive(Debug, Serialize)]
struct P2PCreateRequest {
    files: Vec<P2PFileInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
}

#[derive(Debug, Deserialize)]
struct P2PCreateResponse {
    share_code: String,
    #[allow(dead_code)]
    files: Vec<String>,
    #[allow(dead_code)]
    expires_at: String,
}

pub enum PreparedFileSource {
    /// File on disk — opened and read in chunks at send time.
    Path(PathBuf),
    /// In-memory bytes (e.g., stdin upload). Kept resident because the source
    /// is not seekable.
    Memory(Vec<u8>),
}

struct PreparedFile {
    name: String,
    size: u64,
    content_type: String,
    source: PreparedFileSource,
}

pub async fn run(
    client: &ApiClient,
    opts: SenderOptions,
    on_event: SenderEventFn,
) -> Result<(), CoreError> {
    let prepared = prepare_files(opts.files, opts.stdin_data, opts.stdin_name)?;
    if prepared.is_empty() {
        return Err(CoreError::Other("No files to send".into()));
    }

    let file_infos: Vec<P2PFileInfo> = prepared
        .iter()
        .map(|f| P2PFileInfo {
            name: f.name.clone(),
            size: f.size as i64,
            content_type: f.content_type.clone(),
        })
        .collect();

    let resp = client
        .client
        .post(client.url("/cli/p2p/create"))
        .json(&P2PCreateRequest {
            files: file_infos,
            password: opts.password.clone(),
        })
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let msg = body["message"]
            .as_str()
            .unwrap_or("Failed to create P2P session")
            .to_string();
        return Err(CoreError::Api { status, message: msg });
    }

    let session: P2PCreateResponse = resp.json().await?;
    let share_code = session.share_code.clone();

    let file_summaries: Vec<FileSummary> = prepared
        .iter()
        .map(|f| FileSummary { name: f.name.clone(), size: f.size })
        .collect();

    on_event(SenderEvent::Created {
        share_code: share_code.clone(),
        files: file_summaries,
    });

    let mut sig = SignalingClient::connect(&client.base_url)
        .await
        .map_err(|e| CoreError::P2P(e.to_string()))?;

    let ice_servers = rtc::fetch_ice_servers(client)
        .await
        .map_err(|e| CoreError::P2P(e.to_string()))?;

    let peer_id = uuid_simple();

    sig.send(SignalingMessage::UploaderReady {
        share_code: share_code.clone(),
        peer_id: peer_id.clone(),
        device_info: Some(device_info_string()),
    })
    .map_err(|e| CoreError::P2P(e.to_string()))?;

    // Single PC + DC + WS for the entire session. After the first PeerMatched the
    // receiver requests every subsequent file via `SignalingMessage::FileRequest`
    // on the same WS; the uploader streams each file on the same DC. This drops
    // per-file dead time from "full ICE handshake" (5–15s on a TURN relay) to
    // "one signaling RTT".
    let (first_file_name, first_device_info) = loop {
        match sig.recv().await {
            Some(SignalingMessage::DownloaderArrived { device_info, .. }) => {
                on_event(SenderEvent::ReceiverArrived { device_info });
            }
            Some(SignalingMessage::PeerMatched { file_name, device_info, .. }) => {
                break (file_name, device_info);
            }
            Some(SignalingMessage::TransferComplete { .. }) => {
                on_event(SenderEvent::TransferComplete);
                sig.shutdown();
                return Ok(());
            }
            Some(SignalingMessage::Error { message }) => {
                return Err(CoreError::P2P(message));
            }
            None => {
                return Err(CoreError::P2P("Signaling connection closed".into()));
            }
            _ => {}
        }
    };

    on_event(SenderEvent::PeerMatched { device_info: first_device_info });

    let pc = rtc::create_peer_connection(ice_servers.clone())
        .await
        .map_err(|e| CoreError::P2P(e.to_string()))?;
    let dc = rtc::create_data_channel(&pc)
        .await
        .map_err(|e| CoreError::P2P(e.to_string()))?;

    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<RTCIceCandidateInit>();
    let (state_tx, mut state_rx) = mpsc::unbounded_channel::<RTCIceConnectionState>();
    rtc::setup_ice_candidate_handler(&pc, ice_tx);
    rtc::setup_connection_state_handler(&pc, state_tx);

    let offer = rtc::create_offer(&pc)
        .await
        .map_err(|e| CoreError::P2P(e.to_string()))?;
    sig.send(SignalingMessage::Offer {
        share_code: share_code.clone(),
        sdp: offer.sdp,
        peer_id: peer_id.clone(),
    })
    .map_err(|e| CoreError::P2P(e.to_string()))?;

    // Negotiation stall guard: ICE typically completes within seconds (≤15s even
    // on a TURN-only relay). Anything (msg, ICE, state) resets the deadline.
    const NEGOTIATION_STALL_TIMEOUT: std::time::Duration =
        std::time::Duration::from_secs(60);
    let mut deadline = tokio::time::Instant::now() + NEGOTIATION_STALL_TIMEOUT;

    // First-file ICE handshake + first-file send happens here.
    'negotiation: loop {
        tokio::select! {
            Some(msg) = sig.recv() => {
                deadline = tokio::time::Instant::now() + NEGOTIATION_STALL_TIMEOUT;
                match msg {
                    SignalingMessage::Answer { sdp, .. } => {
                        let answer = RTCSessionDescription::answer(sdp)
                            .map_err(|e| CoreError::P2P(e.to_string()))?;
                        rtc::set_remote_description(&pc, answer)
                            .await
                            .map_err(|e| CoreError::P2P(e.to_string()))?;
                    }
                    SignalingMessage::IceCandidate { candidate, .. } => {
                        if let Some((cand, mid, idx)) = decode_ice_candidate(&candidate) {
                            let init = RTCIceCandidateInit {
                                candidate: cand,
                                sdp_mid: mid,
                                sdp_mline_index: idx,
                                ..Default::default()
                            };
                            let _ = rtc::add_ice_candidate(&pc, init).await;
                        }
                    }
                    SignalingMessage::DownloaderArrived { device_info, .. } => {
                        on_event(SenderEvent::ReceiverArrived { device_info });
                    }
                    SignalingMessage::DownloaderOffline { .. } => {
                        // Stale relic from any cleanup race; ICE state will surface a
                        // real drop via Disconnected/Failed.
                    }
                    SignalingMessage::TransferComplete { .. } => {
                        on_event(SenderEvent::TransferComplete);
                        let _ = pc.close().await;
                        sig.shutdown();
                        return Ok(());
                    }
                    SignalingMessage::Error { message } => {
                        return Err(CoreError::P2P(message));
                    }
                    _ => {}
                }
            }
            Some(candidate) = ice_rx.recv() => {
                deadline = tokio::time::Instant::now() + NEGOTIATION_STALL_TIMEOUT;
                let encoded = encode_ice_candidate(
                    &candidate.candidate,
                    &candidate.sdp_mid,
                    &candidate.sdp_mline_index,
                );
                sig.send(SignalingMessage::IceCandidate {
                    share_code: share_code.clone(),
                    candidate: encoded,
                    sdp_mid: candidate.sdp_mid,
                    sdp_m_line_index: candidate.sdp_mline_index,
                    peer_id: peer_id.clone(),
                })
                .map_err(|e| CoreError::P2P(e.to_string()))?;
            }
            Some(state) = state_rx.recv() => {
                deadline = tokio::time::Instant::now() + NEGOTIATION_STALL_TIMEOUT;
                match state {
                    RTCIceConnectionState::Connected => {
                        if rtc::check_relay(&pc).await {
                            on_event(SenderEvent::RelayDetected);
                        }
                        let file_to_send: &PreparedFile = match &first_file_name {
                            Some(name) => prepared
                                .iter()
                                .find(|f| &f.name == name)
                                .unwrap_or(&prepared[0]),
                            None => &prepared[0],
                        };
                        send_single_file(&dc, file_to_send, &on_event).await?;
                        on_event(SenderEvent::WaitingForNext);
                        break 'negotiation;
                    }
                    RTCIceConnectionState::Failed => {
                        return Err(CoreError::P2P("ICE connection failed".into()));
                    }
                    RTCIceConnectionState::Disconnected => {
                        on_event(SenderEvent::ReceiverDisconnected);
                        let _ = pc.close().await;
                        sig.shutdown();
                        return Ok(());
                    }
                    _ => {}
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                return Err(CoreError::P2P(format!(
                    "ICE negotiation stalled (no progress for {}s)",
                    NEGOTIATION_STALL_TIMEOUT.as_secs()
                )));
            }
            else => {
                let _ = pc.close().await;
                sig.shutdown();
                return Ok(());
            }
        }
    }

    // Steady state: PC+DC are open, ICE is connected. Wait for FileRequests and
    // stream each requested file on the SAME DC. No re-handshake per file.
    loop {
        tokio::select! {
            Some(msg) = sig.recv() => {
                match msg {
                    SignalingMessage::FileRequest { file_name, .. } => {
                        let file = prepared
                            .iter()
                            .find(|f| f.name == file_name)
                            .unwrap_or(&prepared[0]);
                        on_event(SenderEvent::FileStart {
                            name: file.name.clone(),
                            size: file.size,
                        });
                        send_single_file(&dc, file, &on_event).await?;
                        on_event(SenderEvent::WaitingForNext);
                    }
                    SignalingMessage::TransferComplete { .. } => {
                        on_event(SenderEvent::TransferComplete);
                        let _ = pc.close().await;
                        sig.shutdown();
                        return Ok(());
                    }
                    SignalingMessage::DownloaderArrived { device_info, .. } => {
                        on_event(SenderEvent::ReceiverArrived { device_info });
                    }
                    SignalingMessage::DownloaderOffline { .. } => {
                        on_event(SenderEvent::ReceiverDisconnected);
                        let _ = pc.close().await;
                        sig.shutdown();
                        return Ok(());
                    }
                    SignalingMessage::IceCandidate { candidate, .. } => {
                        // Trickle candidates can still arrive post-Connected.
                        if let Some((cand, mid, idx)) = decode_ice_candidate(&candidate) {
                            let init = RTCIceCandidateInit {
                                candidate: cand,
                                sdp_mid: mid,
                                sdp_mline_index: idx,
                                ..Default::default()
                            };
                            let _ = rtc::add_ice_candidate(&pc, init).await;
                        }
                    }
                    SignalingMessage::Error { message } => {
                        return Err(CoreError::P2P(message));
                    }
                    _ => {}
                }
            }
            Some(candidate) = ice_rx.recv() => {
                let encoded = encode_ice_candidate(
                    &candidate.candidate,
                    &candidate.sdp_mid,
                    &candidate.sdp_mline_index,
                );
                sig.send(SignalingMessage::IceCandidate {
                    share_code: share_code.clone(),
                    candidate: encoded,
                    sdp_mid: candidate.sdp_mid,
                    sdp_m_line_index: candidate.sdp_mline_index,
                    peer_id: peer_id.clone(),
                })
                .map_err(|e| CoreError::P2P(e.to_string()))?;
            }
            Some(state) = state_rx.recv() => {
                match state {
                    RTCIceConnectionState::Failed => {
                        return Err(CoreError::P2P("ICE connection failed".into()));
                    }
                    RTCIceConnectionState::Disconnected => {
                        on_event(SenderEvent::ReceiverDisconnected);
                        let _ = pc.close().await;
                        sig.shutdown();
                        return Ok(());
                    }
                    _ => {}
                }
            }
            else => {
                let _ = pc.close().await;
                sig.shutdown();
                return Ok(());
            }
        }
    }
}

/// Send exactly one file over an already-open DataChannel.
async fn send_single_file(
    dc: &Arc<RTCDataChannel>,
    file: &PreparedFile,
    on_event: &SenderEventFn,
) -> Result<(), CoreError> {
    let start = std::time::Instant::now();
    loop {
        if dc.ready_state() == webrtc::data_channel::data_channel_state::RTCDataChannelState::Open {
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(30) {
            return Err(CoreError::P2P("DataChannel open timeout".into()));
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let metadata = FileMetadata::new(
        file.name.clone(),
        file.size,
        file.content_type.clone(),
    );
    let meta_json = serde_json::to_string(&metadata)
        .map_err(|e| CoreError::P2P(format!("Failed to serialize metadata: {}", e)))?;
    dc.send_text(meta_json)
        .await
        .map_err(|e| CoreError::P2P(e.to_string()))?;

    on_event(SenderEvent::FileStart {
        name: file.name.clone(),
        size: file.size,
    });

    // Track bytes that have left the local DC buffer (≈ bytes on the wire) instead of bytes
    // pushed in, so the bar doesn't run ahead of the receiver over a TURN relay.
    let total_bytes = file.size;
    let mut total_pushed: u64 = 0;
    let mut last_on_wire: u64 = 0;

    let report_on_wire = |total_pushed: u64, buffered: u64, last_on_wire: &mut u64| {
        let on_wire = total_pushed.saturating_sub(buffered);
        if on_wire > *last_on_wire {
            let delta = on_wire - *last_on_wire;
            on_event(SenderEvent::Progress { delta });
            *last_on_wire = on_wire;
        }
    };

    // Event-driven backpressure: wake the send loop when the DC buffer drains below
    // BUFFERED_AMOUNT_LOW instead of busy-polling.
    let drain_notify = Arc::new(Notify::new());
    dc.set_buffered_amount_low_threshold(BUFFERED_AMOUNT_LOW).await;
    let notify_clone = drain_notify.clone();
    dc.on_buffered_amount_low(Box::new(move || {
        let notify_clone = notify_clone.clone();
        Box::pin(async move { notify_clone.notify_one(); })
    })).await;

    match &file.source {
        PreparedFileSource::Memory(data) => {
            let mut offset = 0;
            while offset < data.len() {
                while dc.buffered_amount().await > BUFFERED_AMOUNT_HIGH {
                    let buffered = dc.buffered_amount().await as u64;
                    report_on_wire(total_pushed, buffered, &mut last_on_wire);
                    drain_notify.notified().await;
                }

                let end = std::cmp::min(offset + DC_CHUNK_SIZE, data.len());
                let chunk = &data[offset..end];
                dc.send(&bytes::Bytes::copy_from_slice(chunk))
                    .await
                    .map_err(|e| CoreError::P2P(e.to_string()))?;
                total_pushed += (end - offset) as u64;
                offset = end;

                let buffered = dc.buffered_amount().await as u64;
                report_on_wire(total_pushed, buffered, &mut last_on_wire);
            }
        }
        PreparedFileSource::Path(path) => {
            let mut f = tokio::fs::File::open(path)
                .await
                .map_err(|e| CoreError::Other(format!("Failed to open file for streaming: {}", e)))?;
            let mut buf = vec![0u8; DC_CHUNK_SIZE];
            loop {
                let n = f.read(&mut buf)
                    .await
                    .map_err(|e| CoreError::Other(format!("Read error during streaming: {}", e)))?;
                if n == 0 {
                    break;
                }

                while dc.buffered_amount().await > BUFFERED_AMOUNT_HIGH {
                    let buffered = dc.buffered_amount().await as u64;
                    report_on_wire(total_pushed, buffered, &mut last_on_wire);
                    drain_notify.notified().await;
                }

                dc.send(&bytes::Bytes::copy_from_slice(&buf[..n]))
                    .await
                    .map_err(|e| CoreError::P2P(e.to_string()))?;
                total_pushed += n as u64;

                let buffered = dc.buffered_amount().await as u64;
                report_on_wire(total_pushed, buffered, &mut last_on_wire);
            }
        }
    }

    dc.send_text(EOF_SIGNAL.to_string())
        .await
        .map_err(|e| CoreError::P2P(e.to_string()))?;

    // Drain DC buffer so progress reaches 100% before FileEnd is emitted.
    //
    // `on_buffered_amount_low` only fires when the buffer *crosses down* through
    // BUFFERED_AMOUNT_LOW. For small files that never went above the threshold the callback
    // never fires, so a pure `notified().await` would hang. We mix it with a short polling
    // tick to make progress every 100 ms regardless, and treat anything under 1 KiB as
    // "drained enough" — the receiver typically already has the bytes by then.
    const DRAIN_FLOOR: u64 = 1024;
    let drain_start = std::time::Instant::now();
    loop {
        let buffered = dc.buffered_amount().await as u64;
        report_on_wire(total_pushed, buffered, &mut last_on_wire);
        if buffered <= DRAIN_FLOOR {
            break;
        }
        if drain_start.elapsed() > std::time::Duration::from_secs(30) {
            break;
        }
        tokio::select! {
            _ = drain_notify.notified() => {}
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
        }
    }
    if last_on_wire < total_bytes {
        on_event(SenderEvent::Progress { delta: total_bytes - last_on_wire });
    }

    on_event(SenderEvent::FileEnd);

    Ok(())
}

fn prepare_files(
    files: Vec<PathBuf>,
    stdin_data: Option<Vec<u8>>,
    name: Option<String>,
) -> Result<Vec<PreparedFile>, CoreError> {
    let mut prepared = Vec::new();

    if let Some(data) = stdin_data {
        let file_name = name.unwrap_or_else(|| "stdin.txt".to_string());
        let size = data.len() as u64;
        prepared.push(PreparedFile {
            name: file_name,
            size,
            content_type: "application/octet-stream".to_string(),
            source: PreparedFileSource::Memory(data),
        });
    } else {
        for path in files {
            if !path.exists() {
                return Err(CoreError::Other(format!("File not found: {}", path.display())));
            }
            let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let size = std::fs::metadata(&path)
                .map_err(|e| CoreError::Other(format!("Failed to stat file {}: {}", path.display(), e)))?
                .len();
            let content_type = mime_guess::from_path(&path).first_or_octet_stream().to_string();
            prepared.push(PreparedFile {
                name: file_name,
                size,
                content_type,
                source: PreparedFileSource::Path(path),
            });
        }
    }

    Ok(prepared)
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("cli-{:x}", t)
}
