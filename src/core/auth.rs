use crate::config::CliConfig;
use crate::core::error::CoreError;
use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct DeviceSession {
    pub session_id: String,
    pub login_url: String,
    pub expires_in_seconds: u64,
}

#[derive(Deserialize, Clone)]
pub struct DeviceStatus {
    pub status: String,
    pub personal_token: Option<String>,
    pub user_name: Option<String>,
}

fn unauth_client() -> Result<reqwest::Client, CoreError> {
    reqwest::Client::builder()
        .user_agent(format!("share-cli/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(CoreError::from)
}

pub async fn start_device_session(cfg: &CliConfig) -> Result<DeviceSession, CoreError> {
    let client = unauth_client()?;
    let resp = client.post(format!("{}/cli/auth/session", cfg.server_url())).send().await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let message = body["error"].as_str().unwrap_or("Failed to create sign-in session").to_string();
        return Err(CoreError::Api { status, message });
    }
    resp.json::<DeviceSession>().await.map_err(CoreError::from)
}

pub async fn poll_device_status(cfg: &CliConfig, session_id: &str) -> Result<DeviceStatus, CoreError> {
    let client = unauth_client()?;
    let resp = client
        .get(format!("{}/cli/auth/session/{}/status", cfg.server_url(), session_id))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(CoreError::Api { status, message: "Status poll failed".into() });
    }
    resp.json::<DeviceStatus>().await.map_err(CoreError::from)
}

pub async fn verify_token(cfg: &CliConfig, token: &str) -> Result<TokenInfo, CoreError> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "X-Personal-Token",
        reqwest::header::HeaderValue::from_str(token).map_err(|_| CoreError::Other("Invalid token".into()))?,
    );
    headers.insert(
        "User-Agent",
        reqwest::header::HeaderValue::from_str(&format!("share-cli/{}", env!("CARGO_PKG_VERSION"))).unwrap(),
    );
    let client = reqwest::Client::builder().default_headers(headers).build()?;
    let resp = client.get(format!("{}/cli/me", cfg.server_url())).send().await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(CoreError::Api { status, message: "Token verification failed".into() });
    }
    let body: serde_json::Value = resp.json().await?;
    Ok(TokenInfo {
        name: body["name"].as_str().unwrap_or("User").to_string(),
        last_used_at: body["last_used_at"].as_str().map(|s| s.to_string()),
    })
}

#[derive(Clone)]
pub struct TokenInfo {
    pub name: String,
    pub last_used_at: Option<String>,
}
