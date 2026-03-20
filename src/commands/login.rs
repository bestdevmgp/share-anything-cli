use crate::client::ApiClient;
use crate::config::CliConfig;
use crate::error::{CliError, Result};

pub async fn run(token: String) -> Result<()> {
    if !token.starts_with("sa_") {
        return Err(CliError::Other(
            "Invalid token format. Tokens should start with 'sa_'".to_string(),
        ));
    }

    // Save token first
    let mut config = CliConfig::load();
    config.token = Some(token);
    config
        .save()
        .map_err(|e| CliError::Other(format!("Failed to save config: {}", e)))?;

    // Verify token against server
    let config = CliConfig::load();
    let api_client = ApiClient::new(&config)?;

    let resp = api_client
        .client
        .get(api_client.url("/cli/me"))
        .send()
        .await?;

    if resp.status().is_success() {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let name = body["name"].as_str().unwrap_or("User");
        let last_used = body["last_used_at"].as_str();

        println!("\x1b[32m✓ Welcome, {}!\x1b[0m", name);
        if let Some(last) = last_used {
            println!("  Last login: {}", last);
        }
    } else {
        // Token saved but verification failed — warn user
        println!("\x1b[33m⚠ Token saved, but verification failed. Please check if the token is valid.\x1b[0m");
    }

    Ok(())
}
