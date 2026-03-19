use crate::config::CliConfig;
use crate::error::{CliError, Result};

pub fn run(api_key: String) -> Result<()> {
    if !api_key.starts_with("sa_") {
        return Err(CliError::Other(
            "Invalid API key format. Keys should start with 'sa_'".to_string(),
        ));
    }

    let mut config = CliConfig::load();
    config.api_key = Some(api_key);
    config
        .save()
        .map_err(|e| CliError::Other(format!("Failed to save config: {}", e)))?;

    println!("\x1b[32m✓ API key saved successfully!\x1b[0m");
    println!("  Config: {}", CliConfig::config_path().display());
    Ok(())
}
