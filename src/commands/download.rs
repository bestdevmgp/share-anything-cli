use crate::client::ApiClient;
use crate::core::download::{download_bulk_zip, download_share, DownloadOptions};
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
    zip: bool,
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

    let output_dir = output.unwrap_or_else(|| PathBuf::from("."));

    // Branch: single file (or explicit --file-id) → single download; multi-file → loop or ZIP.
    let multi = info.files.len() > 1 && file_id.is_none();

    if multi && zip {
        let total: u64 = info.files.iter().map(|f| f.file_size.max(0) as u64).sum();
        let pb = create_download_progress(total, &format!("share-{}.zip", code));
        let pb_cb = pb.clone();
        let on_progress: ProgressFn = Arc::new(move |n: u64| pb_cb.inc(n));
        let file_ids: Vec<String> = info.files.iter().map(|f| f.id.clone()).collect();
        let saved =
            download_bulk_zip(client, &code, &file_ids, password.as_deref(), &output_dir, on_progress)
                .await?;
        pb.finish_and_clear();
        println!();
        println!("\x1b[32m✓ Download complete!\x1b[0m");
        println!("  Saved to: {}", display_path(&saved));
        println!();
        return Ok(());
    }

    if multi {
        let mut saved_paths: Vec<PathBuf> = Vec::with_capacity(info.files.len());
        for (i, f) in info.files.iter().enumerate() {
            let pb = create_download_progress(
                f.file_size.max(0) as u64,
                &format!("({}/{}) {}", i + 1, info.files.len(), f.file_name),
            );
            let pb_cb = pb.clone();
            let on_progress: ProgressFn = Arc::new(move |n: u64| pb_cb.inc(n));
            let opts = DownloadOptions {
                password: password.clone(),
                file_id: Some(f.id.clone()),
            };
            let saved =
                download_share(client, &code, &info, opts, &output_dir, on_progress).await?;
            pb.finish_and_clear();
            saved_paths.push(saved);
        }
        println!();
        println!(
            "\x1b[32m✓ Download complete! Saved {} files.\x1b[0m",
            saved_paths.len()
        );
        for p in &saved_paths {
            println!("  {}", display_path(p));
        }
        println!();
        return Ok(());
    }

    // Single-file path (or explicit --file-id pinpointing one file of a multi-file share).
    let target_total = info.files.first().map(|f| f.file_size as u64).unwrap_or(0);
    let target_name = info
        .files
        .first()
        .map(|f| f.file_name.clone())
        .unwrap_or_else(|| format!("download_{}", code));
    let pb = create_download_progress(target_total, &target_name);
    let pb_cb = pb.clone();
    let on_progress: ProgressFn = Arc::new(move |n: u64| pb_cb.inc(n));
    let saved = download_share(
        client,
        &code,
        &info,
        DownloadOptions { password, file_id },
        &output_dir,
        on_progress,
    )
    .await?;
    pb.finish_and_clear();
    println!();
    println!("\x1b[32m✓ Download complete!\x1b[0m");
    println!("  Saved to: {}", display_path(&saved));
    println!();
    Ok(())
}

fn display_path(p: &std::path::Path) -> String {
    if p.is_relative() && !p.starts_with(".") {
        format!("./{}", p.display())
    } else {
        p.display().to_string()
    }
}
