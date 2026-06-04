use crate::client::ApiClient;
use crate::core::error::CoreError;
use crate::p2p::protocol::{
    decode_ice_candidate, device_info_string, encode_ice_candidate, FileMetadata,
    SignalingMessage, EOF_SIGNAL,
};
use crate::p2p::{rtc, signaling::SignalingClient};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

pub type ReceiverEventFn = Arc<dyn Fn(ReceiverEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub enum ReceiverEvent {
    /// Connected to sender via signaling, waiting for WebRTC handshake.
    Connecting,
    /// WebRTC peer matched.
    PeerMatched { device_info: Option<String> },
    /// New file metadata received; transfer about to begin.
    FileStart { name: String, size: u64 },
    /// More bytes received for the current file.
    Progress { delta: u64 },
    /// Current file fully received.
    FileEnd {
        #[allow(dead_code)]
        name: String,
        saved_to: PathBuf,
    },
    /// All files done.
    TransferComplete,
    /// Sender disconnected / cancelled.
    SenderGone(String),
    /// Fatal transfer failure.
    Failed(String),
}

pub struct ReceiverOptions {
    pub share_code: String,
    pub password: Option<String>,
    pub output_dir: PathBuf,
    /// File names in download order. The receiver requests each one sequentially so the user
    /// gets the whole share without needing to pick files like the web client does.
    pub files: Vec<String>,
}

pub async fn run(
    client: &ApiClient,
    opts: ReceiverOptions,
    on_event: ReceiverEventFn,
) -> Result<(), CoreError> {
    let share_code = opts.share_code;
    let output_dir = opts.output_dir;
    let password = opts.password;
    let files = opts.files;

    if files.is_empty() {
        return Err(CoreError::P2P("Share has no files to download".into()));
    }

    if !output_dir.exists() {
        std::fs::create_dir_all(&output_dir)?;
    }

    let ice_servers = rtc::fetch_ice_servers(client)
        .await
        .map_err(|e| CoreError::P2P(e.to_string()))?;

    let peer_id = uuid_simple();

    on_event(ReceiverEvent::Connecting);

    // Signaling emits PeerMatched per file; only surface it to the UI on the first one.
    let mut announced_peer = false;

    // Per-file WebSocket, matching the web client. Each file gets a clean signaling queue —
    // structurally, not via drain/sleep heuristics — so stale ICE candidates from the
    // previous file's tear-down can't lock the next pc's ICE onto a dead endpoint. The
    // server closes the previous WS, sends DownloaderOffline to the uploader (sender drops
    // it as expected), then matches the new WS's DownloaderJoin to the still-registered
    // uploader. Reconnect cost is ~50-200 ms, comfortably under what a drain + safety sleep
    // would have added.
    let total_files = files.len();
    for (idx, file_name) in files.iter().enumerate() {
        let mut sig = SignalingClient::connect(&client.base_url)
            .await
            .map_err(|e| CoreError::P2P(e.to_string()))?;

        let result = download_one_file(
            &mut sig,
            &peer_id,
            &share_code,
            file_name,
            password.clone(),
            &output_dir,
            &on_event,
            &mut announced_peer,
            ice_servers.clone(),
        )
        .await;

        // On the last successful file, tell the sender the whole transfer is done before
        // tearing down this WS — otherwise the sender just sees DownloaderOffline (same as
        // between files) and keeps waiting for the next PeerMatched.
        if result.is_ok() && idx == total_files - 1 {
            let _ = sig.send(SignalingMessage::TransferComplete {
                share_code: share_code.clone(),
            });
            // Give the write task a moment to flush the message over WS before shutdown
            // aborts it. Without this, the send_handle can be killed before the bytes
            // hit the socket.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // Always shut down — `SignalingClient` doesn't abort its background tasks on drop,
        // so propagating an error without calling shutdown would leak the read/write/ping
        // tasks until the server eventually times out the WS.
        sig.shutdown();
        result?;
    }

    on_event(ReceiverEvent::TransferComplete);
    Ok(())
}

/// Download a single file in a fresh WebRTC session. The signaling websocket is reused
/// across files; only the PeerConnection / DataChannel is rebuilt per file.
#[allow(clippy::too_many_arguments)]
async fn download_one_file(
    sig: &mut SignalingClient,
    peer_id: &str,
    share_code: &str,
    file_name: &str,
    password: Option<String>,
    output_dir: &Path,
    on_event: &ReceiverEventFn,
    announced_peer: &mut bool,
    ice_servers: Vec<webrtc::ice_transport::ice_server::RTCIceServer>,
) -> Result<(), CoreError> {
    let pc = rtc::create_peer_connection(ice_servers)
        .await
        .map_err(|e| CoreError::P2P(e.to_string()))?;

    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<RTCIceCandidateInit>();
    let (state_tx, mut state_rx) = mpsc::unbounded_channel::<RTCIceConnectionState>();
    rtc::setup_ice_candidate_handler(&pc, ice_tx);
    rtc::setup_connection_state_handler(&pc, state_tx);

    let (file_tx, mut file_rx) = mpsc::unbounded_channel::<ReceivedFile>();
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<u64>();

    // Per-file mutable state shared between the on_message handler and the EOF finalizer. Both
    // tokio Mutex types because we still need to .await across the .lock(), but the critical
    // section in each branch is tiny (a Vec extension or a take).
    let current_meta: Arc<tokio::sync::Mutex<Option<FileMetadata>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let current_data: Arc<tokio::sync::Mutex<Vec<u8>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    {
        let file_tx = file_tx.clone();
        let progress_tx = progress_tx.clone();
        let on_event_dc = on_event.clone();
        let current_meta = current_meta.clone();
        let current_data = current_data.clone();

        pc.on_data_channel(Box::new(move |dc: Arc<webrtc::data_channel::RTCDataChannel>| {
            // Register on_message SYNCHRONOUSLY in this callback body — registering it inside a
            // returned async block introduces a race where the underlying SCTP layer can start
            // delivering messages before the handler is in place, which manifests as the sender
            // reporting progress while the receiver stays stuck at 0%.
            let meta_clone = current_meta.clone();
            let data_clone = current_data.clone();
            let file_tx_clone = file_tx.clone();
            let progress_tx_clone = progress_tx.clone();
            let on_event_msg = on_event_dc.clone();

            dc.on_message(Box::new(move |msg: webrtc::data_channel::data_channel_message::DataChannelMessage| {
                let meta = meta_clone.clone();
                let data = data_clone.clone();
                let file_tx = file_tx_clone.clone();
                let progress_tx = progress_tx_clone.clone();
                let on_event_msg = on_event_msg.clone();

                Box::pin(async move {
                    if msg.is_string {
                        let text = String::from_utf8_lossy(&msg.data);
                        if text.as_ref() == EOF_SIGNAL {
                            let m = meta.lock().await.take();
                            let mut d = data.lock().await;
                            let file_data = std::mem::take(&mut *d);
                            if let Some(m) = m {
                                let _ = file_tx.send(ReceivedFile {
                                    name: m.file_name,
                                    data: file_data,
                                });
                            }
                        } else if let Ok(fm) = serde_json::from_str::<FileMetadata>(&text) {
                            on_event_msg(ReceiverEvent::FileStart {
                                name: fm.file_name.clone(),
                                size: fm.file_size,
                            });
                            let mut m = meta.lock().await;
                            *m = Some(fm);
                            let mut d = data.lock().await;
                            d.clear();
                        }
                    } else {
                        let chunk_len = msg.data.len() as u64;
                        let mut d = data.lock().await;
                        d.extend_from_slice(&msg.data);
                        let _ = progress_tx.send(chunk_len);
                    }
                })
            }));

            // Empty future: all setup already happened synchronously above.
            Box::pin(async {})
        }));
    }

    // Password is sent on every join so the signaling server re-verifies each request;
    // wrong passwords come back as `SignalingMessage::Error`.
    sig.send(SignalingMessage::DownloaderJoin {
        share_code: share_code.to_string(),
        peer_id: peer_id.to_string(),
        file_name: Some(file_name.to_string()),
        device_info: Some(device_info_string()),
        password: password.clone(),
    })
    .map_err(|e| CoreError::P2P(e.to_string()))?;

    let mut received: Option<ReceivedFile> = None;
    let mut peer_offline = false;

    // Per-file inactivity guard. Any signaling message, ICE update, or chunk resets the
    // deadline. If nothing happens for STALL_TIMEOUT (e.g. the backend lost the peer
    // mapping mid-negotiation, sender wedged, or relay died silently), bail out instead
    // of waiting forever — `Disconnected` alone doesn't always wake the loop.
    const STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
    let mut deadline = tokio::time::Instant::now() + STALL_TIMEOUT;

    'session: loop {
        tokio::select! {
            biased;
            Some(f) = file_rx.recv() => {
                received = Some(f);
                break 'session;
            }
            Some(msg) = sig.recv() => {
                deadline = tokio::time::Instant::now() + STALL_TIMEOUT;
                match msg {
                    SignalingMessage::PeerMatched { device_info, .. } => {
                        if !*announced_peer {
                            on_event(ReceiverEvent::PeerMatched { device_info });
                            *announced_peer = true;
                        }
                    }
                    SignalingMessage::Offer { sdp, .. } => {
                        let offer = RTCSessionDescription::offer(sdp)
                            .map_err(|e| CoreError::P2P(e.to_string()))?;
                        rtc::set_remote_description(&pc, offer)
                            .await
                            .map_err(|e| CoreError::P2P(e.to_string()))?;
                        let answer = rtc::create_answer(&pc)
                            .await
                            .map_err(|e| CoreError::P2P(e.to_string()))?;
                        sig.send(SignalingMessage::Answer {
                            share_code: share_code.to_string(),
                            sdp: answer.sdp,
                            peer_id: peer_id.to_string(),
                        })
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
                    SignalingMessage::UploaderCancelled { .. } => {
                        on_event(ReceiverEvent::SenderGone("Sender cancelled the transfer".into()));
                        return Err(CoreError::P2P("Sender cancelled".into()));
                    }
                    SignalingMessage::UploaderOffline { .. } => {
                        peer_offline = true;
                    }
                    SignalingMessage::Error { message } => {
                        // Password mismatch and other signaling-level rejections land here.
                        // Surface as a hard failure so the UI flips to Failed.
                        on_event(ReceiverEvent::Failed(message.clone()));
                        return Err(CoreError::P2P(message));
                    }
                    _ => {}
                }
            }
            Some(candidate) = ice_rx.recv() => {
                deadline = tokio::time::Instant::now() + STALL_TIMEOUT;
                let encoded = encode_ice_candidate(
                    &candidate.candidate,
                    &candidate.sdp_mid,
                    &candidate.sdp_mline_index,
                );
                sig.send(SignalingMessage::IceCandidate {
                    share_code: share_code.to_string(),
                    candidate: encoded,
                    sdp_mid: candidate.sdp_mid,
                    sdp_m_line_index: candidate.sdp_mline_index,
                    peer_id: peer_id.to_string(),
                })
                .map_err(|e| CoreError::P2P(e.to_string()))?;
            }
            Some(state) = state_rx.recv() => {
                deadline = tokio::time::Instant::now() + STALL_TIMEOUT;
                match state {
                    RTCIceConnectionState::Connected => {
                        let _ = rtc::check_relay(&pc).await;
                    }
                    RTCIceConnectionState::Failed | RTCIceConnectionState::Disconnected => {
                        // ICE state can flip to Failed/Disconnected during the brief window between
                        // the receiver getting the last chunk + EOF and the data-channel handler
                        // forwarding the assembled file. Wait briefly then accept any file that
                        // arrived; only treat it as a real failure if the file never lands.
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        if let Ok(f) = file_rx.try_recv() {
                            received = Some(f);
                            break 'session;
                        }
                        if matches!(state, RTCIceConnectionState::Failed) {
                            return Err(CoreError::P2P("ICE connection failed".into()));
                        }
                        if peer_offline {
                            on_event(ReceiverEvent::SenderGone("Sender disconnected".into()));
                            return Err(CoreError::P2P("Sender disconnected".into()));
                        }
                    }
                    _ => {}
                }
            }
            Some(delta) = progress_rx.recv() => {
                deadline = tokio::time::Instant::now() + STALL_TIMEOUT;
                on_event(ReceiverEvent::Progress { delta });
            }
            _ = tokio::time::sleep_until(deadline) => {
                return Err(CoreError::P2P(format!(
                    "Transfer for '{}' stalled (no signaling or data for {}s)",
                    file_name,
                    STALL_TIMEOUT.as_secs()
                )));
            }
            else => break,
        }
    }

    // Drain remaining progress so the UI reaches 100% before close.
    while let Ok(delta) = progress_rx.try_recv() {
        on_event(ReceiverEvent::Progress { delta });
    }

    let _ = pc.close().await;

    let Some(file) = received else {
        return Err(CoreError::P2P(format!(
            "Did not receive file '{}'",
            file_name
        )));
    };

    let dest = output_dir.join(&file.name);
    std::fs::write(&dest, &file.data)?;
    on_event(ReceiverEvent::FileEnd {
        name: file.name,
        saved_to: dest,
    });
    Ok(())
}

struct ReceivedFile {
    name: String,
    data: Vec<u8>,
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("cli-{:x}", t)
}
