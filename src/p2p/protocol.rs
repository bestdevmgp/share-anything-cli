use serde::{Deserialize, Serialize};

/// Data channel chunk size (64KB)
pub const DC_CHUNK_SIZE: usize = 65536;
/// Pause sending when buffered amount exceeds this (1MB)
pub const BUFFERED_AMOUNT_HIGH: usize = 1_048_576;
/// Resume sending when buffered amount drops below this (256KB)
#[allow(dead_code)]
pub const BUFFERED_AMOUNT_LOW: usize = 262_144;
/// Sent after all file chunks to indicate end of file
pub const EOF_SIGNAL: &str = "__EOF__";
/// WebSocket ping interval in seconds
pub const WS_PING_INTERVAL_SECS: u64 = 30;

/// Signaling messages matching the backend's `SignalingMessage` enum exactly.
/// Uses `#[serde(tag = "type", rename_all = "snake_case")]` for wire format.
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
    UploaderInfo {
        share_code: String,
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

/// File metadata sent over the DataChannel before binary chunks.
/// Uses camelCase field names to match the web frontend protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    #[serde(rename = "type")]
    pub msg_type: String, // always "file_metadata"
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

/// Returns a device info string like "CLI, macOS 14.0"
pub fn device_info_string() -> String {
    let info = os_info::get();
    format!("CLI, {} {}", info.os_type(), info.version())
}
