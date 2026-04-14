use serde::{Deserialize, Serialize};

pub const DC_CHUNK_SIZE: usize = 65536;
pub const BUFFERED_AMOUNT_HIGH: usize = 1_048_576;
pub const EOF_SIGNAL: &str = "__EOF__";
pub const WS_PING_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalingMessage {
    UploaderReady {
        share_code: String,
        peer_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        device_info: Option<String>,
    },
    DownloaderJoin {
        share_code: String,
        peer_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        device_info: Option<String>,
    },
    PeerMatched {
        peer_id: String,
        role: PeerRole,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        device_info: Option<String>,
    },
    Offer {
        share_code: String,
        sdp: String,
        peer_id: String,
    },
    Answer {
        share_code: String,
        sdp: String,
        peer_id: String,
    },
    IceCandidate {
        share_code: String,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u16>,
        peer_id: String,
    },
    Error {
        message: String,
    },
    TransferComplete {
        share_code: String,
    },
    UploaderOffline {
        share_code: String,
    },
    DownloaderOffline {
        share_code: String,
    },
    DownloaderArrived {
        share_code: String,
        peer_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        device_info: Option<String>,
    },
    UploaderCancelled {
        share_code: String,
    },
    Ping {},
    Pong {},
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerRole {
    Uploader,
    Downloader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub file_name: String,
    pub file_size: u64,
    pub file_type: String,
}

impl FileMetadata {
    pub fn new(file_name: String, file_size: u64, file_type: String) -> Self {
        Self {
            msg_type: "file_metadata".to_string(),
            file_name,
            file_size,
            file_type,
        }
    }
}

pub fn encode_ice_candidate(
    candidate: &str,
    sdp_mid: &Option<String>,
    sdp_mline_index: &Option<u16>,
) -> String {
    serde_json::json!({
        "candidate": candidate,
        "sdpMid": sdp_mid,
        "sdpMLineIndex": sdp_mline_index,
    })
    .to_string()
}

pub fn decode_ice_candidate(json_str: &str) -> Option<(String, Option<String>, Option<u16>)> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
        if let Some(candidate) = v.get("candidate").and_then(|v| v.as_str()) {
            let sdp_mid = v.get("sdpMid").and_then(|v| v.as_str()).map(|s| s.to_string());
            let sdp_mline_index = v
                .get("sdpMLineIndex")
                .and_then(|v| v.as_u64())
                .map(|n| n as u16);
            return Some((candidate.to_string(), sdp_mid, sdp_mline_index));
        }
    }
    if json_str.contains("candidate:") || json_str.starts_with("a=") {
        return Some((json_str.to_string(), None, None));
    }
    None
}

pub fn device_info_string() -> String {
    let info = os_info::get();
    format!("{} {} (CLI)", info.os_type(), info.version())
}
