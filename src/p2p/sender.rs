use crate::client::ApiClient;
use crate::error::{CliError, Result};
use crate::p2p::protocol::{
    device_info_string, FileMetadata, SignalingMessage, BUFFERED_AMOUNT_HIGH,
    DC_CHUNK_SIZE, EOF_SIGNAL,
};
use crate::p2p::{rtc, signaling::SignalingClient};
use crate::progress::{create_spinner, create_upload_progress, finish_progress};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

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
    expires_at: String,
}

/// A prepared file ready for P2P transfer.
struct PreparedFile {
    name: String,
    data: Vec<u8>,
    content_type: String,
}

/// Run the P2P sender (uploader) flow.
pub async fn run(
    client: &ApiClient,
    files: Vec<PathBuf>,
    stdin_data: Option<Vec<u8>>,
    name: Option<String>,
    password: Option<String>,
) -> Result<()> {
    // 1. Prepare file data
    let prepared = prepare_files(files, stdin_data, name)?;
    if prepared.is_empty() {
        return Err(CliError::Other("No files to send".into()));
    }

    // 2. Create P2P session on the server
    let file_infos: Vec<P2PFileInfo> = prepared
        .iter()
        .map(|f| P2PFileInfo {
            name: f.name.clone(),
            size: f.data.len() as i64,
            content_type: f.content_type.clone(),
        })
        .collect();

    let resp = client
        .client
        .post(client.url("/cli/p2p/create"))
        .json(&P2PCreateRequest {
            files: file_infos,
            password: password.clone(),
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
        return Err(CliError::Api {
            status,
            message: msg,
        });
    }

    let session: P2PCreateResponse = resp.json().await?;
    let share_code = session.share_code.clone();

    // 3. Print share code + instructions
    println!();
    println!("\x1b[32m✓ Secure transfer ready!\x1b[0m");
    println!("  Code     : {}", share_code);
    println!("  Command  : share download {}", share_code);
    println!("  Expires  : {}", crate::time::utc_to_local(&session.expires_at));
    if prepared.len() == 1 {
        println!(
            "  File     : {} ({})",
            prepared[0].name,
            format_size(prepared[0].data.len() as u64)
        );
    } else {
        println!("  Files    : {} files", prepared.len());
        for f in &prepared {
            println!("    - {} ({})", f.name, format_size(f.data.len() as u64));
        }
    }
    println!();

    // 4. Connect signaling, fetch ICE, create peer connection + data channel
    let mut sig = SignalingClient::connect(&client.base_url).await?;
    let ice_servers = rtc::fetch_ice_servers(client).await?;
    let pc = rtc::create_peer_connection(ice_servers).await?;
    let dc = rtc::create_data_channel(&pc).await?;

    // 5. Setup ICE/state handlers
    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<RTCIceCandidateInit>();
    let (state_tx, mut state_rx) = mpsc::unbounded_channel::<RTCIceConnectionState>();
    rtc::setup_ice_candidate_handler(&pc, ice_tx);
    rtc::setup_connection_state_handler(&pc, state_tx);

    let peer_id = uuid_simple();

    // 6. Send UploaderReady
    sig.send(SignalingMessage::UploaderReady {
        share_code: share_code.clone(),
        peer_id: peer_id.clone(),
        device_info: Some(device_info_string()),
    })?;

    let spinner = create_spinner("Waiting for receiver to connect...");

    // 7. Event loop
    let mut transfer_done = false;
    let mut peer_matched = false;
    loop {
        tokio::select! {
            Some(msg) = sig.recv() => {
                match msg {
                    SignalingMessage::PeerMatched { device_info, .. }
                    | SignalingMessage::DownloaderArrived { device_info, .. } => {
                        spinner.finish_and_clear();
                        let info_str = device_info.as_deref().unwrap_or("Unknown device");
                        println!("  \x1b[32m✓\x1b[0m Connected to receiver ({})", info_str);
                        println!();
                        peer_matched = true;

                        // Create and send SDP offer
                        let offer = rtc::create_offer(&pc).await?;
                        sig.send(SignalingMessage::Offer {
                            share_code: share_code.clone(),
                            sdp: offer.sdp,
                            peer_id: peer_id.clone(),
                        })?;
                    }
                    SignalingMessage::Answer { sdp, .. } => {
                        let answer = RTCSessionDescription::answer(sdp)?;
                        rtc::set_remote_description(&pc, answer).await?;
                    }
                    SignalingMessage::IceCandidate {
                        candidate,
                        sdp_mid,
                        sdp_m_line_index,
                        ..
                    } => {
                        let init = RTCIceCandidateInit {
                            candidate,
                            sdp_mid,
                            sdp_mline_index: sdp_m_line_index,
                            ..Default::default()
                        };
                        rtc::add_ice_candidate(&pc, init).await?;
                    }
                    SignalingMessage::DownloaderOffline { .. } => {
                        if peer_matched {
                            println!("\n\x1b[33m⚠ Receiver disconnected. Waiting for new receiver...\x1b[0m");
                            peer_matched = false;
                        }
                    }
                    SignalingMessage::Error { message } => {
                        spinner.finish_and_clear();
                        return Err(CliError::P2P(message));
                    }
                    _ => {}
                }
            }
            Some(candidate) = ice_rx.recv() => {
                sig.send(SignalingMessage::IceCandidate {
                    share_code: share_code.clone(),
                    candidate: candidate.candidate,
                    sdp_mid: candidate.sdp_mid,
                    sdp_m_line_index: candidate.sdp_mline_index,
                    peer_id: peer_id.clone(),
                })?;
            }
            Some(state) = state_rx.recv() => {
                match state {
                    RTCIceConnectionState::Connected => {
                        spinner.finish_and_clear();
                        // Send files over the data channel
                        send_files(&dc, &prepared).await?;

                        // Signal transfer complete
                        sig.send(SignalingMessage::TransferComplete {
                            share_code: share_code.clone(),
                        })?;

                        transfer_done = true;
                        break;
                    }
                    RTCIceConnectionState::Failed => {
                        spinner.finish_and_clear();
                        return Err(CliError::P2P("ICE connection failed".into()));
                    }
                    RTCIceConnectionState::Disconnected => {
                        if !transfer_done {
                            spinner.finish_and_clear();
                            println!("\x1b[33m⚠ Connection lost\x1b[0m");
                            break;
                        }
                    }
                    _ => {}
                }
            }
            else => break,
        }
    }

    // 9. Cleanup
    let _ = pc.close().await;
    sig.shutdown();

    if transfer_done {
        println!();
        println!("\x1b[32m✓ Transfer complete!\x1b[0m");
        println!();
    }

    Ok(())
}

/// Send all files over the DataChannel with backpressure management.
async fn send_files(dc: &Arc<RTCDataChannel>, files: &[PreparedFile]) -> Result<()> {
    // Wait for DataChannel to be open (up to 30s)
    let start = std::time::Instant::now();
    loop {
        if dc.ready_state() == webrtc::data_channel::data_channel_state::RTCDataChannelState::Open {
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(30) {
            return Err(CliError::P2P("DataChannel open timeout".into()));
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    for file in files {
        // Send file metadata
        let metadata = FileMetadata::new(
            file.name.clone(),
            file.data.len() as u64,
            file.content_type.clone(),
        );
        let meta_json = serde_json::to_string(&metadata)
            .map_err(|e| CliError::P2P(format!("Failed to serialize metadata: {}", e)))?;
        dc.send_text(meta_json).await?;

        // Send chunks with progress bar
        let pb = create_upload_progress(file.data.len() as u64, &file.name);
        let mut offset = 0;

        while offset < file.data.len() {
            // Backpressure: wait if too much is buffered
            while dc.buffered_amount().await > BUFFERED_AMOUNT_HIGH {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }

            let end = std::cmp::min(offset + DC_CHUNK_SIZE, file.data.len());
            let chunk = &file.data[offset..end];
            dc.send(&bytes::Bytes::copy_from_slice(chunk)).await?;
            let sent = (end - offset) as u64;
            pb.inc(sent);
            offset = end;
        }

        // Send EOF signal
        dc.send_text(EOF_SIGNAL.to_string()).await?;
        finish_progress(&pb);
    }

    Ok(())
}

fn prepare_files(
    files: Vec<PathBuf>,
    stdin_data: Option<Vec<u8>>,
    name: Option<String>,
) -> Result<Vec<PreparedFile>> {
    let mut prepared = Vec::new();

    if let Some(data) = stdin_data {
        let file_name = name.unwrap_or_else(|| "stdin.txt".to_string());
        prepared.push(PreparedFile {
            name: file_name,
            data,
            content_type: "application/octet-stream".to_string(),
        });
    } else {
        for path in &files {
            if !path.exists() {
                return Err(CliError::Other(format!("File not found: {}", path.display())));
            }
            let file_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let data = std::fs::read(path)?;
            let content_type = mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string();
            prepared.push(PreparedFile {
                name: file_name,
                data,
                content_type,
            });
        }
    }

    Ok(prepared)
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("cli-{:x}", t)
}
