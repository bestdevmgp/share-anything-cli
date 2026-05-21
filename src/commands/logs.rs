use crate::client::ApiClient;
use crate::error::{CliError, Result};

pub async fn run(client: &ApiClient, code: String) -> Result<()> {
    if !client.is_authenticated() {
        return Err(CliError::Other(
            "Personal token required. Use `share login <token>` first.".to_string(),
        ));
    }

    let resp = client
        .client
        .get(client.url(&format!("/v1/me/uploads/{}/downloads", code)))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        return Err(CliError::Api {
            status,
            message: body["error"]["message"]
                .as_str()
                .or_else(|| body["message"].as_str())
                .unwrap_or("Failed to fetch download logs")
                .to_string(),
        });
    }

    let body: serde_json::Value = resp.json().await?;
    let downloads = body["downloads"].as_array();

    if let Some(downloads) = downloads {
        if downloads.is_empty() {
            println!("No downloads yet for \x1b[1m{}\x1b[0m.", code);
            return Ok(());
        }

        println!();
        println!(
            "{:<20} {:<18} {:<16} {:<12} {:<20}",
            "DOWNLOADER", "FILE", "IP", "PLATFORM", "AT"
        );
        println!("{}", "-".repeat(90));

        for d in downloads {
            let downloader = d["downloader_name"].as_str().unwrap_or("Anonymous");
            let file = d["file_name"].as_str().unwrap_or("-");
            let ip = d["ip_address"].as_str().unwrap_or("-");
            let platform = d["device_platform"].as_str().unwrap_or("-");
            let at_raw = d["downloaded_at"].as_str().unwrap_or("-");
            let at = crate::time::utc_to_local(at_raw);

            let trunc_downloader = if downloader.len() > 18 {
                format!("{}...", &downloader[..15])
            } else {
                downloader.to_string()
            };
            let trunc_file = if file.len() > 16 {
                format!("{}...", &file[..13])
            } else {
                file.to_string()
            };
            let trunc_platform = if platform.len() > 10 {
                format!("{}...", &platform[..7])
            } else {
                platform.to_string()
            };

            println!(
                "{:<20} {:<18} {:<16} {:<12} {:<20}",
                trunc_downloader, trunc_file, ip, trunc_platform, at
            );
        }
        println!();
    } else {
        println!("No downloads yet.");
    }

    Ok(())
}
