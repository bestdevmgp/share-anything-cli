use crate::client::ApiClient;
use crate::error::{CliError, Result};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

#[derive(Debug, Deserialize)]
struct IceServerResponse {
    urls: Vec<String>,
    username: Option<String>,
    credential: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TurnCredentialsResponse {
    ice_servers: Vec<IceServerResponse>,
}

pub async fn fetch_ice_servers(client: &ApiClient) -> Result<Vec<RTCIceServer>> {
    let resp = client.client.get(client.url("/turn/credentials")).send().await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let creds: TurnCredentialsResponse = r
                .json()
                .await
                .map_err(|e| CliError::P2P(format!("Failed to parse TURN credentials: {}", e)))?;

            let mut servers: Vec<RTCIceServer> = Vec::new();

            for s in creds.ice_servers {
                if s.urls.is_empty() {
                    continue;
                }

                let has_creds = s.username.as_ref().is_some_and(|u| !u.is_empty())
                    && s.credential.as_ref().is_some_and(|c| !c.is_empty());
                let has_turn_url = s.urls.iter().any(|u| u.starts_with("turn:") || u.starts_with("turns:"));

                if has_turn_url && !has_creds {
                    continue;
                }

                if has_creds {
                    servers.push(RTCIceServer {
                        urls: s.urls,
                        username: s.username.unwrap_or_default(),
                        credential: s.credential.unwrap_or_default(),
                        credential_type: webrtc::ice_transport::ice_credential_type::RTCIceCredentialType::Password,
                        ..Default::default()
                    });
                } else {
                    servers.push(RTCIceServer {
                        urls: s.urls,
                        ..Default::default()
                    });
                }
            }

            Ok(servers)
        }
        _ => Ok(vec![RTCIceServer {
            urls: vec!["stun:stun.cloudflare.com:3478".to_string()],
            ..Default::default()
        }]),
    }
}

pub async fn create_peer_connection(
    ice_servers: Vec<RTCIceServer>,
) -> Result<Arc<RTCPeerConnection>> {
    let mut media_engine = MediaEngine::default();
    media_engine.register_default_codecs()?;

    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut media_engine)?;

    let api = APIBuilder::new()
        .with_media_engine(media_engine)
        .with_interceptor_registry(registry)
        .build();

    let config = RTCConfiguration {
        ice_servers,
        ..Default::default()
    };

    let pc = api.new_peer_connection(config).await?;
    Ok(Arc::new(pc))
}

pub async fn create_data_channel(pc: &Arc<RTCPeerConnection>) -> Result<Arc<RTCDataChannel>> {
    use webrtc::data_channel::data_channel_init::RTCDataChannelInit;

    let init = RTCDataChannelInit {
        ordered: Some(true),
        ..Default::default()
    };

    let dc = pc.create_data_channel("file-transfer", Some(init)).await?;
    Ok(dc)
}

pub fn setup_ice_candidate_handler(
    pc: &Arc<RTCPeerConnection>,
    ice_tx: mpsc::UnboundedSender<RTCIceCandidateInit>,
) {
    pc.on_ice_candidate(Box::new(move |candidate| {
        let ice_tx = ice_tx.clone();
        Box::pin(async move {
            if let Some(c) = candidate {
                if let Ok(json) = c.to_json() {
                    let _ = ice_tx.send(json);
                }
            }
        })
    }));
}

pub fn setup_connection_state_handler(
    pc: &Arc<RTCPeerConnection>,
    state_tx: mpsc::UnboundedSender<RTCIceConnectionState>,
) {
    pc.on_ice_connection_state_change(Box::new(move |state| {
        let state_tx = state_tx.clone();
        Box::pin(async move {
            let _ = state_tx.send(state);
        })
    }));
}

pub async fn create_offer(pc: &Arc<RTCPeerConnection>) -> Result<RTCSessionDescription> {
    let offer = pc.create_offer(None).await?;
    pc.set_local_description(offer.clone()).await?;
    Ok(offer)
}

pub async fn create_answer(pc: &Arc<RTCPeerConnection>) -> Result<RTCSessionDescription> {
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer.clone()).await?;
    Ok(answer)
}

pub async fn set_remote_description(pc: &Arc<RTCPeerConnection>, sdp: RTCSessionDescription) -> Result<()> {
    pc.set_remote_description(sdp).await?;
    Ok(())
}

pub async fn add_ice_candidate(pc: &Arc<RTCPeerConnection>, candidate: RTCIceCandidateInit) -> Result<()> {
    pc.add_ice_candidate(candidate).await?;
    Ok(())
}

pub async fn check_relay(pc: &Arc<RTCPeerConnection>) {
    let stats = pc.get_stats().await;
    for (_, stat) in stats.reports.iter() {
        if let webrtc::stats::StatsReportType::LocalCandidate(local) = stat {
            if local.candidate_type == webrtc::ice::candidate::CandidateType::Relay {
                println!("  \x1b[33mℹ\x1b[0m TURN server relay in use");
                return;
            }
        }
    }
}
