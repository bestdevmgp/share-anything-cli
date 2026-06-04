use crate::client::ApiClient;
use crate::core::error::CoreError;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct UploadItem {
    pub share_code: String,
    pub file_name: String,
    pub file_size: i64,
    pub expires_at: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DownloadItem {
    pub share_code: String,
    pub file_name: String,
    pub file_size: i64,
    pub downloaded_at: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FileDetail {
    /// ULID for this file row in the share. Empty when the server is older than the
    /// multi-file download patch — callers must fall back to single-file behaviour
    /// (server picks `files[0]` when `file_id` is omitted) if any id is empty.
    #[serde(default)]
    pub id: String,
    pub file_name: String,
    pub file_size: i64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FileInfo {
    pub share_code: String,
    pub files: Vec<FileDetail>,
    pub has_password: bool,
    #[serde(default)]
    pub is_one_time: bool,
    #[serde(default)]
    pub transfer_type: Option<String>,
    pub expires_at: String,
}

pub async fn list_my_uploads(client: &ApiClient) -> Result<Vec<UploadItem>, CoreError> {
    if !client.is_authenticated() {
        return Err(CoreError::Unauthenticated);
    }
    let resp = client.client.get(client.url("/cli/me/uploads")).send().await?;
    map_envelope::<UploadsWrapper>(resp).await.map(|w| w.uploads)
}

pub async fn list_my_downloads(client: &ApiClient) -> Result<Vec<DownloadItem>, CoreError> {
    if !client.is_authenticated() {
        return Err(CoreError::Unauthenticated);
    }
    let resp = client.client.get(client.url("/cli/me/downloads")).send().await?;
    map_envelope::<DownloadsWrapper>(resp).await.map(|w| w.downloads)
}

pub async fn get_share_info(client: &ApiClient, code: &str) -> Result<FileInfo, CoreError> {
    let resp = client.client.get(client.url(&format!("/cli/download/{}/info", code))).send().await?;
    match map_envelope::<FileInfo>(resp).await {
        Ok(v) => Ok(v),
        Err(CoreError::Api { status, message }) if message == "Unknown error" => {
            Err(CoreError::Api { status, message: "File not found".into() })
        }
        Err(e) => Err(e),
    }
}

pub async fn delete_share(client: &ApiClient, code: &str) -> Result<(), CoreError> {
    if !client.is_authenticated() {
        return Err(CoreError::Unauthenticated);
    }
    let resp = client.client.delete(client.url(&format!("/cli/me/uploads/{}", code))).send().await?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    Ok(())
}

/// Result of a bulk delete: number of shares deleted, and any individual errors collected.
pub struct DeleteAllOutcome {
    pub deleted: usize,
    pub failures: Vec<(String, String)>,
}

pub async fn delete_all_shares(client: &ApiClient) -> Result<DeleteAllOutcome, CoreError> {
    if !client.is_authenticated() {
        return Err(CoreError::Unauthenticated);
    }
    let uploads = list_my_uploads(client).await?;
    let mut deleted = 0;
    let mut failures: Vec<(String, String)> = Vec::new();
    for u in &uploads {
        match delete_share(client, &u.share_code).await {
            Ok(()) => deleted += 1,
            Err(e) => failures.push((u.share_code.clone(), e.to_string())),
        }
    }
    Ok(DeleteAllOutcome { deleted, failures })
}

#[derive(Deserialize)]
struct UploadsWrapper { uploads: Vec<UploadItem> }

#[derive(Deserialize)]
struct DownloadsWrapper { downloads: Vec<DownloadItem> }

pub(crate) async fn map_envelope<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T, CoreError> {
    if resp.status().is_success() {
        Ok(resp.json::<T>().await?)
    } else {
        Err(api_error(resp).await)
    }
}

pub(crate) async fn api_error(resp: reqwest::Response) -> CoreError {
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    let message = body["error"]["message"].as_str()
        .or_else(|| body["message"].as_str())
        .unwrap_or("Unknown error")
        .to_string();
    CoreError::Api { status, message }
}
