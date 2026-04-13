#![allow(dead_code)]

use crate::client::ApiClient;
use crate::error::{CliError, Result};
use crate::p2p::protocol::{device_info_string, FileMetadata, SignalingMessage, EOF_SIGNAL};
use crate::p2p::{rtc, signaling::SignalingClient};
use crate::progress::{create_download_progress, create_spinner, finish_progress};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

/// A file received over the DataChannel.
struct ReceivedFile {
    name: String,
    data: Vec<u8>,
}

/// Run the P2P receiver (downloader) flow.
pub async fn run(client: &ApiClient, share_code: String, output: Option<PathBuf>) -> Result<()> {
    println!();
    println!("  Transfer : Secure (P2P, end-to-end)");
    println!("  Code     : {}", share_code);
    println!();

    // 1. Connect signaling, fetch ICE, create peer connection
    let mut sig = SignalingClient::connect(&client.base_url).await?;
    let ice_servers = rtc::fetch_ice_servers(client).await?;
    let pc = rtc::create_peer_connection(ice_servers).await?;

    // 2. Setup ICE/state handlers
    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<RTCIceCandidateInit>();
    let (state_tx, mut state_rx) = mpsc::unbounded_channel::<RTCIceConnectionState>();
    rtc::setup_ice_candidate_handler(&pc, ice_tx);
    rtc::setup_connection_state_handler(&pc, state_tx);

    // 3. Channel for completed files from DataChannel handler
    let (file_tx, mut file_rx) = mpsc::unbounded_channel::<ReceivedFile>();

    // 4. Register on_data_channel handler
    {
        let file_tx = file_tx.clone();
        pc.on_data_channel(Box::new(move |dc: Arc<webrtc::data_channel::RTCDataChannel>| {
            let file_tx = file_tx.clone();
            Box::pin(async move {
                // State for accumulating the current file
                let current_meta: Arc<tokio::sync::Mutex<Option<FileMetadata>>> =
                    Arc::new(tokio::sync::Mutex::new(None));
                let current_data: Arc<tokio::sync::Mutex<Vec<u8>>> =
                    Arc::new(tokio::sync::Mutex::new(Vec::new()));
                let progress: Arc<tokio::sync::Mutex<Option<indicatif::ProgressBar>>> =
                    Arc::new(tokio::sync::Mutex::new(None));

                let meta_clone = current_meta.clone();
                let data_clone = current_data.clone();
                let progress_clone = progress.clone();
                let file_tx_clone = file_tx.clone();

                dc.on_message(Box::new(move |msg: webrtc::data_channel::data_channel_message::DataChannelMessage| {
                    let meta = meta_clone.clone();
                    let data = data_clone.clone();
                    let pb_holder = progress_clone.clone();
                    let file_tx = file_tx_clone.clone();

                    Box::pin(async move {
                        if msg.is_string {
                            let text = String::from_utf8_lossy(&msg.data);

                            if text.as_ref() == EOF_SIGNAL {
                                // File complete
                                let m = meta.lock().await.take();
                                let mut d = data.lock().await;
                                let file_data = std::mem::take(&mut *d);

                                // Finish progress bar
                                let mut pb = pb_holder.lock().await;
                                if let Some(ref p) = *pb {
                                    finish_progress(p);
                                }
                                *pb = None;

                                if let Some(m) = m {
                                    let _ = file_tx.send(ReceivedFile {
                                        name: m.file_name,
                                        data: file_data,
                                    });
                                }
                            } else {
                                // Try to parse as FileMetadata
                                if let Ok(fm) = serde_json::from_str::<FileMetadata>(&text) {
                                    // Create progress bar for this file
                                    let new_pb = create_download_progress(fm.file_size, &fm.file_name);
                                    let mut pb = pb_holder.lock().await;
                                    *pb = Some(new_pb);

                                    let mut m = meta.lock().await;
                                    *m = Some(fm);

                                    let mut d = data.lock().await;
                                    d.clear();
                                }
                            }
                        } else {
                            // Binary chunk
                            let chunk_len = msg.data.len() as u64;
                            let mut d = data.lock().await;
                            d.extend_from_slice(&msg.data);

                            let pb = pb_holder.lock().await;
                            if let Some(ref p) = *pb {
                                p.inc(chunk_len);
                            }
                        }
                    })
                }));
            })
        }));
    }

    let peer_id = uuid_simple();

    // 5. Send DownloaderJoin
    sig.send(SignalingMessage::DownloaderJoin {
        share_code: share_code.clone(),
        peer_id: peer_id.clone(),
        file_name: None,
        device_info: Some(device_info_string()),
    })?;

    let spinner = create_spinner("Connecting to sender...");
    let mut received_files: Vec<ReceivedFile> = Vec::new();
    let mut transfer_done = false;

    // 6. Event loop
    loop {
        tokio::select! {
            Some(msg) = sig.recv() => {
                match msg {
                    SignalingMessage::PeerMatched { device_info, .. } => {
                        spinner.finish_and_clear();
                        let info_str = device_info.as_deref().unwrap_or("Unknown device");
                        println!("  Connected: {}", info_str);
                    }
                    SignalingMessage::Offer { sdp, .. } => {
                        let offer = RTCSessionDescription::offer(sdp)?;
                        rtc::set_remote_description(&pc, offer).await?;

                        let answer = rtc::create_answer(&pc).await?;
                        sig.send(SignalingMessage::Answer {
                            share_code: share_code.clone(),
                            sdp: answer.sdp,
                            peer_id: peer_id.clone(),
                        })?;
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
                    SignalingMessage::UploaderCancelled { .. } => {
                        spinner.finish_and_clear();
                        println!("\x1b[33m⚠ Sender cancelled the transfer\x1b[0m");
                        break;
                    }
                    SignalingMessage::UploaderOffline { .. } => {
                        spinner.finish_and_clear();
                        println!("\x1b[33m⚠ Sender disconnected\x1b[0m");
                        break;
                    }
                    SignalingMessage::TransferComplete { .. } => {
                        transfer_done = true;
                        // Give a moment for any remaining file data to arrive
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        // Drain any remaining files
                        while let Ok(f) = file_rx.try_recv() {
                            received_files.push(f);
                        }
                        break;
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
                    RTCIceConnectionState::Failed => {
                        spinner.finish_and_clear();
                        return Err(CliError::P2P("ICE connection failed".into()));
                    }
                    RTCIceConnectionState::Disconnected => {
                        if !transfer_done {
                            // May still get files in the buffer, wait briefly
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            while let Ok(f) = file_rx.try_recv() {
                                received_files.push(f);
                            }
                            if received_files.is_empty() {
                                spinner.finish_and_clear();
                                println!("\x1b[33m⚠ Connection lost\x1b[0m");
                            }
                            break;
                        }
                    }
                    _ => {}
                }
            }
            Some(file) = file_rx.recv() => {
                received_files.push(file);
            }
            else => break,
        }
    }

    let _ = transfer_done; // suppress unused_assignments warning

    // 7. Save received files to disk
    let output_dir = output.unwrap_or_else(|| PathBuf::from("."));
    if !output_dir.exists() {
        std::fs::create_dir_all(&output_dir)?;
    }

    for file in &received_files {
        let dest = output_dir.join(&file.name);
        std::fs::write(&dest, &file.data)?;
        println!(
            "  Saved: {} ({})",
            dest.display(),
            format_size(file.data.len() as u64)
        );
    }

    // 8. Cleanup
    let _ = pc.close().await;
    sig.shutdown();

    if !received_files.is_empty() {
        println!();
        println!("\x1b[32m✓ Download complete!\x1b[0m");
        println!();
    }

    Ok(())
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
