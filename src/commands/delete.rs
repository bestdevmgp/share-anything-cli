use crate::client::ApiClient;
use crate::error::{CliError, Result};

pub async fn run(client: &ApiClient, code: String) -> Result<()> {
    if !client.is_authenticated() {
        return Err(CliError::Other(
            "Personal token required. Use `share login <token>` first".to_string(),
        ));
    }

    let resp = client
        .client
        .delete(client.url(&format!("/cli/me/uploads/{}", code)))
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
                .unwrap_or("Failed to delete share")
                .to_string(),
        });
    }

    println!("\x1b[32m✓\x1b[0m Share \x1b[1m{}\x1b[0m has been deleted.", code);
    Ok(())
}
