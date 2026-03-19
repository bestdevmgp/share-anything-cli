use crate::config::CliConfig;
use crate::error::{CliError, Result};

pub fn run() -> Result<()> {
    let mut config = CliConfig::load();

    if config.api_key.is_none() {
        println!("No API key configured.");
        return Ok(());
    }

    config.api_key = None;
    config
        .save()
        .map_err(|e| CliError::Other(format!("Failed to save config: {}", e)))?;

    println!("\x1b[32m✓ API key removed.\x1b[0m");
    Ok(())
}
