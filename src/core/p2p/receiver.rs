use crate::client::ApiClient;
use crate::core::error::CoreError;
use crate::p2p::protocol::{
    decode_ice_candidate, device_info_string, encode_ice_candidate, FileMetadata,
    SignalingMessage, EOF_SIGNAL,
};
use crate::p2p::{rtc, signaling::SignalingClient};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

pub type ReceiverEventFn = Arc<dyn Fn(ReceiverEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub enum ReceiverEvent {
    Connecting,
    PeerMatched { device_info: Option<String> },
    FileStart { name: String, size: u64 },
    Progress { delta: u64 },
    FileEnd {
        #[allow(dead_code)]
        name: String,
        saved_to: PathBuf,
    },
    TransferComplete,
    SenderGone(String),
    Failed(String),
}

pub struct ReceiverOptions {
    pub share_code: String,
    pub password: Option<String>,
    pub output_dir: PathBuf,
    pub files: Vec<String>,
}

pub async fn run(
    client: &ApiClient,
    opts: ReceiverOptions,
    on_event: ReceiverEventFn,
) -> Result<(), CoreError> {
    let ReceiverOptions {
        share_code,
        password,
        output_dir,
        files,
    } = opts;

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

    let mut sig = SignalingClient::connect(&client.base_url)
        .await
        .map_err(|e| CoreError::P2P(e.to_string()))?;

    let pc = rtc::create_peer_connection(ice_servers)
        .await
        .map_err(|e| CoreError::P2P(e.to_string()))?;

    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<RTCIceCandidateInit>();
    let (state_tx, mut state_rx) = mpsc::unbounded_channel::<RTCIceConnectionState>();
    rtc::setup_ice_candidate_handler(&pc, ice_tx);
    rtc::setup_connection_state_handler(&pc, state_tx);

    let (file_tx, mut file_rx) = mpsc::unbounded_channel::<ReceivedFile>();
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<u64>();

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

            Box::pin(async {})
        }));
    }

    sig.send(SignalingMessage::DownloaderJoin {
        share_code: share_code.clone(),
        peer_id: peer_id.clone(),
        file_name: Some(files[0].clone()),
        device_info: Some(device_info_string()),
        password: password.clone(),
    })
    .map_err(|e| CoreError::P2P(e.to_string()))?;

    const STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
    let mut deadline = tokio::time::Instant::now() + STALL_TIMEOUT;

    let mut announced_peer = false;
    let mut peer_offline = false;
    let mut file_idx: usize = 0;
    let mut saved_files: Vec<PathBuf> = Vec::with_capacity(files.len());

    macro_rules! handle_received {
        ($f:expr) => {{
            let f = $f;
            let dest = output_dir.join(&f.name);
            std::fs::write(&dest, &f.data)?;
            on_event(ReceiverEvent::FileEnd {
                name: f.name.clone(),
                saved_to: dest.clone(),
            });
            saved_files.push(dest);
            file_idx += 1;

            if file_idx >= files.len() {
                let _ = sig.send(SignalingMessage::TransferComplete {
                    share_code: share_code.clone(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                break Ok(());
            }

            sig.send(SignalingMessage::FileRequest {
                share_code: share_code.clone(),
                file_name: files[file_idx].clone(),
            })
            .map_err(|e| CoreError::P2P(e.to_string()))?;
        }};
    }

    let result: Result<(), CoreError> = loop {
        tokio::select! {
            biased;
            Some(f) = file_rx.recv() => {
                deadline = tokio::time::Instant::now() + STALL_TIMEOUT;
                handle_received!(f);
            }
            Some(msg) = sig.recv() => {
                deadline = tokio::time::Instant::now() + STALL_TIMEOUT;
                match msg {
                    SignalingMessage::PeerMatched { device_info, .. } => {
                        if !announced_peer {
                            on_event(ReceiverEvent::PeerMatched { device_info });
                            announced_peer = true;
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
                            share_code: share_code.clone(),
                            sdp: answer.sdp,
                            peer_id: peer_id.clone(),
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
                        break Err(CoreError::P2P("Sender cancelled".into()));
                    }
                    SignalingMessage::UploaderOffline { .. } => {
                        peer_offline = true;
                    }
                    SignalingMessage::Error { message } => {
                        on_event(ReceiverEvent::Failed(message.clone()));
                        break Err(CoreError::P2P(message));
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
                    share_code: share_code.clone(),
                    candidate: encoded,
                    sdp_mid: candidate.sdp_mid,
                    sdp_m_line_index: candidate.sdp_mline_index,
                    peer_id: peer_id.clone(),
                })
                .map_err(|e| CoreError::P2P(e.to_string()))?;
            }
            Some(state) = state_rx.recv() => {
                deadline = tokio::time::Instant::now() + STALL_TIMEOUT;
                match state {
                    RTCIceConnectionState::Connected => {
                        let _ = rtc::check_relay(&pc).await;
                    }
                    RTCIceConnectionState::Failed
                    | RTCIceConnectionState::Disconnected
                    | RTCIceConnectionState::Closed => {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        if let Ok(f) = file_rx.try_recv() {
                            handle_received!(f);
                        } else {
                            let msg = if matches!(state, RTCIceConnectionState::Failed) {
                                "Sender connection failed".to_string()
                            } else if peer_offline {
                                "Sender disconnected".to_string()
                            } else {
                                "Sender cancelled the transfer".to_string()
                            };
                            on_event(ReceiverEvent::SenderGone(msg.clone()));
                            break Err(CoreError::P2P(msg));
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
                let stalled_file = files.get(file_idx).cloned().unwrap_or_default();
                let msg = if stalled_file.is_empty() {
                    "Connection lost. The sender may be offline.".to_string()
                } else {
                    format!("Connection lost while receiving '{}'.", stalled_file)
                };
                on_event(ReceiverEvent::SenderGone(msg.clone()));
                break Err(CoreError::P2P(msg));
            }
            else => break Ok(()),
        }
    };

    let _ = pc.close().await;
    sig.shutdown();
    result?;
    on_event(ReceiverEvent::TransferComplete);
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
