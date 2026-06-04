use crate::client::ApiClient;
use crate::core::upload::{read_files, upload_files, FileEntry, FileSource, PhaseFn, ShareResult, UploadOptions, UploadPhase};
use crate::core::ProgressFn;
use crate::error::Result;
use crate::progress::{create_spinner, create_upload_progress, finish_progress};
use indicatif::ProgressBar;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub async fn run(
    client: &ApiClient,
    files: Vec<PathBuf>,
    stdin_data: Option<Vec<u8>>,
    name: Option<String>,
    password: Option<String>,
    expiration: Option<String>,
    one_time: bool,
) -> Result<()> {
    let entries: Vec<FileEntry> = if let Some(data) = stdin_data {
        let n = name.unwrap_or_else(|| "stdin.txt".to_string());
        let size = data.len() as u64;
        vec![FileEntry {
            name: n,
            size,
            content_type: "application/octet-stream".into(),
            source: FileSource::Memory(bytes::Bytes::from(data)),
        }]
    } else {
        read_files(&files)?
    };

    let total: u64 = entries.iter().map(|e| e.size).sum();
    let display = if entries.len() == 1 {
        entries[0].name.clone()
    } else {
        format!("{} files", entries.len())
    };

    // pb is created on InitDone; the lazy branch in on_progress is a safety net in case a
    // chunk somehow arrives first.
    let pb_slot: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));
    let spinner_slot: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));

    let pb_for_progress = pb_slot.clone();
    let display_clone = display.clone();
    let on_progress: ProgressFn = Arc::new(move |n: u64| {
        let mut slot = pb_for_progress.lock().unwrap();
        if slot.is_none() {
            *slot = Some(create_upload_progress(total, &display_clone));
        }
        slot.as_ref().unwrap().inc(n);
    });

    let pb_for_phase = pb_slot.clone();
    let spinner_for_phase = spinner_slot.clone();
    let display_for_phase = display.clone();
    let on_phase: PhaseFn = Arc::new(move |phase: UploadPhase| {
        match phase {
            UploadPhase::InitStarted => {
                let mut s = spinner_for_phase.lock().unwrap();
                *s = Some(create_spinner("Initializing multipart upload..."));
            }
            UploadPhase::InitDone => {
                if let Some(s) = spinner_for_phase.lock().unwrap().take() {
                    s.finish_and_clear();
                }
                let mut p = pb_for_phase.lock().unwrap();
                if p.is_none() {
                    *p = Some(create_upload_progress(total, &display_for_phase));
                }
            }
        }
    });

    let opts = UploadOptions { password, expiration, one_time };
    let result = upload_files(client, entries, opts, on_progress, Some(on_phase)).await;

    if let Some(pb) = pb_slot.lock().unwrap().take() { finish_progress(&pb); }
    if let Some(s) = spinner_slot.lock().unwrap().take() { s.finish_and_clear(); }

    let result = result?;
    print_upload_result(&result);
    Ok(())
}

pub async fn run_secure(
    client: &ApiClient,
    files: Vec<PathBuf>,
    stdin_data: Option<Vec<u8>>,
    name: Option<String>,
    password: Option<String>,
) -> Result<()> {
    crate::p2p::sender::run(client, files, stdin_data, name, password).await
}

fn print_upload_result(result: &ShareResult) {
    println!();
    println!("\x1b[32m✓ Upload complete!\x1b[0m");
    println!("  Share code : {}", result.share_code);
    println!("  Command    : share download {}", result.share_code);
    println!("  Expires    : {}", crate::time::utc_to_local(&result.expires_at));
    if result.files.len() > 1 {
        println!("  Files:");
        for f in &result.files {
            println!("    - {}", f);
        }
    }
    println!();
}
