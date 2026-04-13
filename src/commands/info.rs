use crate::client::ApiClient;
use crate::error::{CliError, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct FileInfoResponse {
    share_code: String,
    files: Vec<FileDetail>,
    has_password: bool,
    is_one_time: bool,
    #[serde(default)]
    transfer_type: Option<String>,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct FileDetail {
    file_name: String,
    file_size: i64,
}

pub async fn run(client: &ApiClient, code: String) -> Result<()> {
    let resp = client
        .client
        .get(client.url(&format!("/cli/download/{}/info", code)))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        return Err(CliError::Api {
            status,
            message: body["message"]
                .as_str()
                .unwrap_or("File not found")
                .to_string(),
        });
    }

    let info: FileInfoResponse = resp.json().await?;

    println!();
    println!("Share code  : {}", info.share_code);
    if info.transfer_type.as_deref() == Some("p2p") {
        println!("Transfer    : Secure (P2P)");
    }
    println!("Password    : {}", if info.has_password { "Yes" } else { "No" });
    println!("One-time    : {}", if info.is_one_time { "Yes" } else { "No" });
    println!("Expires at  : {}", crate::time::utc_to_local(&info.expires_at));
    println!("Files ({}):", info.files.len());
    for f in &info.files {
        println!("  - {} ({})", f.file_name, format_size(f.file_size));
    }
    println!();

    Ok(())
}

fn format_size(bytes: i64) -> String {
    const KB: i64 = 1024;
    const MB: i64 = 1024 * KB;
    const GB: i64 = 1024 * MB;

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
