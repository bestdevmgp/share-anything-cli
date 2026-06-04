use crate::config::CliConfig;
use crate::core::auth::{poll_device_status, start_device_session, verify_token};
use crate::error::{CliError, Result};
use indicatif::{ProgressBar, ProgressStyle};
use qrcode::{EcLevel, QrCode};
use std::time::{Duration, Instant};

pub async fn run(token: Option<String>, config: &CliConfig) -> Result<()> {
    if token.is_none() {
        if let Some(ref existing) = config.token {
            if !existing.is_empty() {
                println!("\x1b[33mYou are already signed in. To sign in with a different account, run \x1b[0m\x1b[1mshare logout\x1b[0m\x1b[33m first.\x1b[0m");
                return Ok(());
            }
        }
    }

    match token {
        Some(token) => run_token_login(token, config).await,
        None => run_device_login(config).await,
    }
}

async fn run_token_login(token: String, _config: &CliConfig) -> Result<()> {
    if !token.starts_with("sat_") {
        return Err(CliError::Other(
            "Invalid token format. Tokens should start with 'sat_'".to_string(),
        ));
    }

    let mut cfg = CliConfig::load();
    cfg.token = Some(token.clone());
    cfg.save()
        .map_err(|e| CliError::Other(format!("Failed to save config: {}", e)))?;

    let config = CliConfig::load();

    match verify_token(&config, &token).await {
        Ok(info) => {
            let mut cfg = CliConfig::load();
            cfg.user_name = Some(info.name.clone());
            let _ = cfg.save();
            println!("\x1b[32m✓ Welcome, {}!\x1b[0m", info.name);
            if let Some(last) = info.last_used_at {
                println!("  Last sign-in: {}", last);
            }
        }
        Err(_) => {
            println!("\x1b[33m⚠ Token saved, but verification failed. Please check if the token is valid.\x1b[0m");
        }
    }

    Ok(())
}

async fn run_device_login(config: &CliConfig) -> Result<()> {
    let session = start_device_session(config).await?;

    match open::that(&session.login_url) {
        Ok(_) => println!("\x1b[32mBrowser opened. Please complete the sign-in.\x1b[0m"),
        Err(_) => println!("\x1b[33mCould not open the browser.\x1b[0m"),
    }

    println!();
    print_qr_code(&session.login_url);
    println!();
    println!(
        "  If you can't use a browser, scan the QR code or visit the link below to sign in:"
    );
    println!("  \x1b[36m{}\x1b[0m", session.login_url);
    println!();

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );
    spinner.set_message("Waiting for sign-in...");

    let start = Instant::now();
    let timeout = Duration::from_secs(session.expires_in_seconds);

    loop {
        if start.elapsed() > timeout {
            spinner.finish_and_clear();
            return Err(CliError::Other(
                "Session expired. Please try again".to_string(),
            ));
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
        spinner.tick();

        let status = match poll_device_status(config, &session.session_id).await {
            Ok(s) => s,
            Err(_) => continue, // Network or server error, retry
        };

        match status.status.as_str() {
            "pending" => continue,
            "completed" => {
                spinner.finish_and_clear();

                let personal_token = status.personal_token.ok_or_else(|| {
                    CliError::Other("Server did not return a token".to_string())
                })?;

                let mut cfg = CliConfig::load();
                cfg.token = Some(personal_token);
                cfg.user_name = status.user_name.clone();
                cfg.save()
                    .map_err(|e| CliError::Other(format!("Failed to save config: {}", e)))?;

                let user_name = status.user_name.as_deref().unwrap_or("User");
                println!("\x1b[32m✓ Signed in! Welcome, {}\x1b[0m", user_name);
                return Ok(());
            }
            "expired" => {
                spinner.finish_and_clear();
                return Err(CliError::Other(
                    "Session expired. Please try again".to_string(),
                ));
            }
            _ => continue,
        }
    }
}

fn print_qr_code(url: &str) {
    let code = match QrCode::with_error_correction_level(url.as_bytes(), EcLevel::L) {
        Ok(c) => c,
        Err(_) => {
            println!("  \x1b[33m(Failed to generate QR code)\x1b[0m");
            return;
        }
    };

    let w = code.width();
    let data = code.to_colors();
    let is_dark = |x: usize, y: usize| -> bool {
        x >= 1 && y >= 1 && x <= w && y <= w
            && data[(y - 1) * w + (x - 1)] == qrcode::Color::Dark
    };

    let total = w + 2;
    let mut buf = String::with_capacity(total * (total / 2 + 1) * 4);

    for y in (0..total).step_by(2) {
        buf.push_str("  ");
        for x in 0..total {
            buf.push(match (is_dark(x, y), is_dark(x, y + 1)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        buf.push('\n');
    }

    print!("{}", buf);
}
