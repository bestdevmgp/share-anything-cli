use crate::client::ApiClient;
use crate::core::download::{download_share, DownloadOptions};
use crate::core::shares::get_share_info;
use crate::core::ProgressFn;
use crate::error::{CliError, Result};
use crate::progress::create_download_progress;
use std::path::PathBuf;
use std::sync::Arc;

pub async fn run(
    client: &ApiClient,
    code: String,
    password: Option<String>,
    output: Option<PathBuf>,
    file_id: Option<String>,
) -> Result<()> {
    let info = get_share_info(client, &code).await?;

    if info.has_password && password.is_none() {
        return Err(CliError::Other(
            "This file requires a password. Use --password <password>".to_string(),
        ));
    }

    if info.transfer_type.as_deref() == Some("p2p") {
        return crate::p2p::receiver::run(client, code, password, output).await;
    }

    let target_total = if !info.files.is_empty() { info.files[0].file_size as u64 } else { 0 };
    let target_name = if !info.files.is_empty() { info.files[0].file_name.clone() } else { format!("download_{}", code) };

    let pb = create_download_progress(target_total, &target_name);
    let pb_cb = pb.clone();
    let on_progress: ProgressFn = Arc::new(move |n: u64| pb_cb.inc(n));

    let output_dir = output.unwrap_or_else(|| PathBuf::from("."));
    let result = download_share(
        client,
        &code,
        &info,
        DownloadOptions { password, file_id },
        &output_dir,
        on_progress,
    )
    .await;
    pb.finish_and_clear();
    let saved = result?;

    println!();
    println!("\x1b[32m✓ Download complete!\x1b[0m");
    let display_path = if saved.is_relative() && !saved.starts_with(".") {
        format!("./{}", saved.display())
    } else {
        saved.display().to_string()
    };
    println!("  Saved to: {}", display_path);
    println!();
    Ok(())
}
