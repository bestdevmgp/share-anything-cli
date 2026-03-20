use crate::config::CliConfig;
use crate::error::{CliError, Result};

pub fn run() -> Result<()> {
    let mut config = CliConfig::load();

    if config.token.is_none() {
        println!("No personal token configured.");
        return Ok(());
    }

    config.token = None;
    config
        .save()
        .map_err(|e| CliError::Other(format!("Failed to save config: {}", e)))?;

    println!("\x1b[32m✓ Personal token removed.\x1b[0m");
    Ok(())
}
