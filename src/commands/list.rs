use crate::client::ApiClient;
use crate::error::{CliError, Result};

pub async fn run(client: &ApiClient) -> Result<()> {
    if !client.is_authenticated() {
        return Err(CliError::Other(
            "API key required. Use `sany login <api-key>` first.".to_string(),
        ));
    }

    let resp = client
        .client
        .get(client.url("/cli/user/uploads"))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        return Err(CliError::Api {
            status,
            message: body["message"]
                .as_str()
                .unwrap_or("Failed to fetch uploads")
                .to_string(),
        });
    }

    let body: serde_json::Value = resp.json().await?;
    let uploads = body["uploads"].as_array();

    if let Some(uploads) = uploads {
        if uploads.is_empty() {
            println!("No uploads found.");
            return Ok(());
        }

        println!();
        println!("{:<10} {:<30} {:<12} {:<20}", "CODE", "FILE", "SIZE", "EXPIRES");
        println!("{}", "-".repeat(72));

        for upload in uploads {
            let code = upload["share_code"].as_str().unwrap_or("-");
            let name = upload["file_name"].as_str().unwrap_or("-");
            let size = upload["file_size"].as_i64().unwrap_or(0);
            let expires = upload["expires_at"].as_str().unwrap_or("-");

            let display_name = if name.len() > 28 {
                format!("{}...", &name[..25])
            } else {
                name.to_string()
            };

            println!("{:<10} {:<30} {:<12} {:<20}", code, display_name, format_size(size), expires);
        }
        println!();
    } else {
        println!("No uploads found.");
    }

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
