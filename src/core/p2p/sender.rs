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
use webrtc::data_channel::data_channel_state::RTCDataChannelState;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

pub type SenderEventFn = Arc<dyn Fn(SenderEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub enum SenderEvent {
    Created { share_code: String, files: Vec<FileSummary> },
    ReceiverArrived { device_info: Option<String> },
    PeerMatched { device_info: Option<String> },
    FileStart { name: String, size: u64 },
    Progress { delta: u64 },
    FileEnd,
    WaitingForNext,
    TransferComplete,
    ReceiverCancelled,
    RelayDetected,
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
    /// Root-relative path (including the leaf name) for folder transfers.
    /// Omitted for files shared at the root so the backend stores no path.
    #[serde(skip_serializing_if = "Option::is_none")]
    relative_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct P2PCreateRequest {
    files: Vec<P2PFileInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    empty_folders: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct P2PCreateResponse {
    share_code: String,
}

pub enum PreparedFileSource {
    Path(PathBuf),
    Memory(Vec<u8>),
}

struct PreparedFile {
    name: String,
    size: u64,
    content_type: String,
    relative_path: Option<String>,
    source: PreparedFileSource,
}

impl PreparedFile {
    /// The key receivers use to request this file: the relative path when set,
    /// otherwise the base name. Mirrors the web client's
    /// `fileKey = relative_path || file_name`.
    fn key(&self) -> &str {
        match &self.relative_path {
            Some(p) if !p.is_empty() => p.as_str(),
            _ => &self.name,
        }
    }
}

/// Resolve a requested file key back to a prepared file. A receiver may request
/// by relative path (web, folder transfers) or by base name (CLI receiver,
/// which is not served `relative_path`), so try both.
fn find_prepared<'a>(prepared: &'a [PreparedFile], requested: &str) -> Option<&'a PreparedFile> {
    prepared
        .iter()
        .find(|f| f.key() == requested)
        .or_else(|| prepared.iter().find(|f| f.name == requested))
}

pub async fn run(
    client: &ApiClient,
    opts: SenderOptions,
    on_event: SenderEventFn,
) -> Result<(), CoreError> {
    let (prepared, empty_folders) = prepare_files(opts.files, opts.stdin_data, opts.stdin_name)?;
    if prepared.is_empty() {
        return Err(CoreError::Other("No files to send".into()));
    }

    let file_infos: Vec<P2PFileInfo> = prepared
        .iter()
        .map(|f| P2PFileInfo {
            name: f.name.clone(),
            size: f.size as i64,
            content_type: f.content_type.clone(),
            relative_path: f.relative_path.clone(),
        })
        .collect();

    let resp = client
        .client
        .post(client.url("/cli/p2p/create"))
        .json(&P2PCreateRequest {
            files: file_infos,
            password: opts.password.clone(),
            empty_folders,
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

    let mut sig = SignalingClient::connect(&client.base_url, client.token.as_deref())
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

    'session: loop {

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
    let dc_signal = rtc::setup_data_channel_close_signal(&dc).await;

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

    const NEGOTIATION_STALL_TIMEOUT: std::time::Duration =
        std::time::Duration::from_secs(60);
    let mut deadline = tokio::time::Instant::now() + NEGOTIATION_STALL_TIMEOUT;

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
                            Some(name) => {
                                find_prepared(&prepared, name).unwrap_or(&prepared[0])
                            }
                            None => &prepared[0],
                        };
                        if send_single_file(&dc, file_to_send, &on_event, &dc_signal).await.is_err() {
                            on_event(SenderEvent::ReceiverCancelled);
                            let _ = pc.close().await;
                            continue 'session;
                        }
                        on_event(SenderEvent::WaitingForNext);
                        break 'negotiation;
                    }
                    RTCIceConnectionState::Failed
                    | RTCIceConnectionState::Closed => {
                        on_event(SenderEvent::ReceiverCancelled);
                        let _ = pc.close().await;
                        continue 'session;
                    }
                    RTCIceConnectionState::Disconnected => {
                    }
                    _ => {}
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                on_event(SenderEvent::ReceiverCancelled);
                let _ = pc.close().await;
                continue 'session;
            }
            _ = dc_signal.closed.notified() => {
                on_event(SenderEvent::ReceiverCancelled);
                let _ = pc.close().await;
                continue 'session;
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                let st = dc.ready_state();
                if st != RTCDataChannelState::Open
                    && st != RTCDataChannelState::Connecting
                {
                    on_event(SenderEvent::ReceiverCancelled);
                    let _ = pc.close().await;
                    continue 'session;
                }
            }
            else => {
                let _ = pc.close().await;
                continue 'session;
            }
        }
    }


    loop {
        tokio::select! {
            Some(msg) = sig.recv() => {
                match msg {
                    SignalingMessage::FileRequest { file_name, .. } => {
                        let file = find_prepared(&prepared, &file_name).unwrap_or(&prepared[0]);
                        on_event(SenderEvent::FileStart {
                            name: file.name.clone(),
                            size: file.size,
                        });
                        if let Err(_e) = send_single_file(&dc, file, &on_event, &dc_signal).await {
                            on_event(SenderEvent::ReceiverCancelled);
                            let _ = pc.close().await;
                            continue 'session;
                        }
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
                        on_event(SenderEvent::ReceiverCancelled);
                        let _ = pc.close().await;
                        continue 'session;
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
                    RTCIceConnectionState::Failed
                    | RTCIceConnectionState::Closed => {
                        on_event(SenderEvent::ReceiverCancelled);
                        let _ = pc.close().await;
                        continue 'session;
                    }
                    RTCIceConnectionState::Disconnected => {
                    }
                    _ => {}
                }
            }
            _ = dc_signal.closed.notified() => {
                on_event(SenderEvent::ReceiverCancelled);
                let _ = pc.close().await;
                continue 'session;
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                let st = dc.ready_state();
                if st != RTCDataChannelState::Open {
                    on_event(SenderEvent::ReceiverCancelled);
                    let _ = pc.close().await;
                    continue 'session;
                }
            }
            else => {
                let _ = pc.close().await;
                continue 'session;
            }
        }
    }

    }
}

async fn send_single_file(
    dc: &Arc<RTCDataChannel>,
    file: &PreparedFile,
    on_event: &SenderEventFn,
    dc_signal: &rtc::DcCloseSignal,
) -> Result<(), CoreError> {

    macro_rules! send_with_timeout {
        ($fut:expr) => {{
            match tokio::time::timeout(std::time::Duration::from_secs(3), $fut).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    return Err(CoreError::P2P(e.to_string()));
                }
                Err(_) => {
                    return Err(CoreError::P2P(
                        "Receiver disconnected mid-transfer".into(),
                    ));
                }
            }
        }};
    }

    let start = std::time::Instant::now();
    loop {
        if dc_signal.is_closed() {
            return Err(CoreError::P2P("Receiver disconnected before transfer started".into()));
        }
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
    send_with_timeout!(dc.send_text(meta_json));

    on_event(SenderEvent::FileStart {
        name: file.name.clone(),
        size: file.size,
    });

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

    let drain_notify = Arc::new(Notify::new());
    dc.set_buffered_amount_low_threshold(BUFFERED_AMOUNT_LOW).await;
    let notify_clone = drain_notify.clone();
    dc.on_buffered_amount_low(Box::new(move || {
        let notify_clone = notify_clone.clone();
        Box::pin(async move { notify_clone.notify_one(); })
    })).await;

    macro_rules! bail_if_remote_closed {
        () => {{
            if dc.ready_state() != RTCDataChannelState::Open || dc_signal.is_closed() {
                return Err(CoreError::P2P("Receiver disconnected mid-transfer".into()));
            }
        }};
    }

    macro_rules! wait_for_drain_or_close {
        () => {{
            while dc.buffered_amount().await > BUFFERED_AMOUNT_HIGH {
                bail_if_remote_closed!();
                let buffered = dc.buffered_amount().await as u64;
                report_on_wire(total_pushed, buffered, &mut last_on_wire);
                tokio::select! {
                    _ = drain_notify.notified() => {}
                    _ = dc_signal.closed.notified() => {
                        return Err(CoreError::P2P("Receiver disconnected mid-transfer".into()));
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
                }
            }
        }};
    }

    match &file.source {
        PreparedFileSource::Memory(data) => {
            let mut offset = 0;
            while offset < data.len() {
                wait_for_drain_or_close!();
                bail_if_remote_closed!();

                let end = std::cmp::min(offset + DC_CHUNK_SIZE, data.len());
                let chunk = &data[offset..end];
                send_with_timeout!(dc.send(&bytes::Bytes::copy_from_slice(chunk)));
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

                wait_for_drain_or_close!();
                bail_if_remote_closed!();

                send_with_timeout!(dc.send(&bytes::Bytes::copy_from_slice(&buf[..n])));
                total_pushed += n as u64;

                let buffered = dc.buffered_amount().await as u64;
                report_on_wire(total_pushed, buffered, &mut last_on_wire);
            }
        }
    }

    send_with_timeout!(dc.send_text(EOF_SIGNAL.to_string()));

    const DRAIN_FLOOR: u64 = 1024;
    let drain_start = std::time::Instant::now();
    loop {
        if dc.ready_state() != RTCDataChannelState::Open || dc_signal.is_closed() {
            return Err(CoreError::P2P("Receiver disconnected before transfer finished".into()));
        }
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
            _ = dc_signal.closed.notified() => {
                return Err(CoreError::P2P("Receiver disconnected before transfer finished".into()));
            }
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
) -> Result<(Vec<PreparedFile>, Vec<String>), CoreError> {
    let mut prepared = Vec::new();
    let mut empty_folders = Vec::new();

    if let Some(data) = stdin_data {
        let file_name = name.unwrap_or_else(|| "stdin.txt".to_string());
        let size = data.len() as u64;
        prepared.push(PreparedFile {
            name: file_name,
            size,
            content_type: "application/octet-stream".to_string(),
            relative_path: None,
            source: PreparedFileSource::Memory(data),
        });
    } else {
        // Recurse into directory arguments, computing each file's root-relative
        // path so the folder structure is preserved on the backend.
        let collected = crate::core::files::collect_files(&files)?;
        empty_folders = collected.empty_folders;
        for c in collected.files {
            let file_name = c.path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let size = std::fs::metadata(&c.path)
                .map_err(|e| CoreError::Other(format!("Failed to stat file {}: {}", c.path.display(), e)))?
                .len();
            let content_type = mime_guess::from_path(&c.path).first_or_octet_stream().to_string();
            prepared.push(PreparedFile {
                name: file_name,
                size,
                content_type,
                relative_path: c.relative_path,
                source: PreparedFileSource::Path(c.path),
            });
        }
    }

    Ok((prepared, empty_folders))
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("cli-{:x}", t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared(name: &str, relative_path: Option<&str>) -> PreparedFile {
        PreparedFile {
            name: name.to_string(),
            size: 1,
            content_type: "application/octet-stream".to_string(),
            relative_path: relative_path.map(|s| s.to_string()),
            source: PreparedFileSource::Memory(Vec::new()),
        }
    }

    #[test]
    fn p2p_file_info_omits_relative_path_when_none() {
        let info = P2PFileInfo {
            name: "a.txt".to_string(),
            size: 3,
            content_type: "text/plain".to_string(),
            relative_path: None,
        };
        let v: serde_json::Value = serde_json::to_value(&info).unwrap();
        assert!(v.get("relative_path").is_none());
        assert_eq!(v["name"], "a.txt");
        assert_eq!(v["type"], "text/plain");
    }

    #[test]
    fn p2p_file_info_includes_relative_path_when_set() {
        let info = P2PFileInfo {
            name: "report.pdf".to_string(),
            size: 3,
            content_type: "application/pdf".to_string(),
            relative_path: Some("docs/2024/report.pdf".to_string()),
        };
        let v: serde_json::Value = serde_json::to_value(&info).unwrap();
        assert_eq!(v["relative_path"], "docs/2024/report.pdf");
    }

    #[test]
    fn p2p_create_request_omits_empty_folders_when_empty() {
        let req = P2PCreateRequest {
            files: vec![],
            password: None,
            empty_folders: vec![],
        };
        let v: serde_json::Value = serde_json::to_value(&req).unwrap();
        assert!(v.get("empty_folders").is_none());
    }

    #[test]
    fn p2p_create_request_includes_empty_folders_when_set() {
        let req = P2PCreateRequest {
            files: vec![],
            password: None,
            empty_folders: vec!["project/logs".to_string()],
        };
        let v: serde_json::Value = serde_json::to_value(&req).unwrap();
        assert_eq!(v["empty_folders"], serde_json::json!(["project/logs"]));
    }

    #[test]
    fn key_prefers_relative_path() {
        assert_eq!(prepared("a.txt", Some("docs/a.txt")).key(), "docs/a.txt");
        assert_eq!(prepared("a.txt", None).key(), "a.txt");
        // Empty relative path falls back to the base name.
        assert_eq!(prepared("a.txt", Some("")).key(), "a.txt");
    }

    #[test]
    fn find_prepared_matches_relative_path_then_name() {
        let files = vec![
            prepared("report.pdf", Some("docs/2024/report.pdf")),
            prepared("report.pdf", Some("docs/2023/report.pdf")),
            prepared("flat.txt", None),
        ];

        // Web/folder receiver requests by relative path.
        assert_eq!(
            find_prepared(&files, "docs/2023/report.pdf").unwrap().key(),
            "docs/2023/report.pdf"
        );
        // CLI receiver requests by base name (no relative path served).
        assert_eq!(find_prepared(&files, "flat.txt").unwrap().name, "flat.txt");
        // Unknown key does not match.
        assert!(find_prepared(&files, "nope.txt").is_none());
    }
}
