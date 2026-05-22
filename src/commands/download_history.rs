use crate::client::ApiClient;
use crate::error::{CliError, Result};
use crate::format::{format_size, pad_display, truncate_display};

pub async fn run(client: &ApiClient) -> Result<()> {
    if !client.is_authenticated() {
        return Err(CliError::Other(
            "Personal token required. Use `share login <token>` first".to_string(),
        ));
    }

    let resp = client
        .client
        .get(client.url("/v1/me/downloads"))
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
                .unwrap_or("Failed to fetch downloads")
                .to_string(),
        });
    }

    let body: serde_json::Value = resp.json().await?;
    let downloads = body["downloads"].as_array();

    if let Some(downloads) = downloads {
        if downloads.is_empty() {
            println!("No downloads found.");
            return Ok(());
        }

        println!();
        println!(
            "{} {} {} {}",
            pad_display("CODE", 10),
            pad_display("FILE", 40),
            pad_display("SIZE", 10),
            pad_display("DOWNLOADED", 20),
        );
        println!("{}", "-".repeat(83));

        for d in downloads {
            let code = d["share_code"].as_str().unwrap_or("-");
            let name = d["file_name"].as_str().unwrap_or("-");
            let size = d["file_size"].as_i64().unwrap_or(0);
            let downloaded_raw = d["downloaded_at"].as_str().unwrap_or("-");
            let downloaded = crate::time::utc_to_local(downloaded_raw);

            let display_name = truncate_display(name, 40);

            println!(
                "{} {} {} {}",
                pad_display(code, 10),
                pad_display(&display_name, 40),
                pad_display(&format_size(size), 10),
                pad_display(&downloaded, 20),
            );
        }
        println!();
    } else {
        println!("No downloads found.");
    }

    Ok(())
}

