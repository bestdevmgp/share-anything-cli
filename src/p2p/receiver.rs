use crate::client::ApiClient;
use crate::core::p2p::receiver::{ReceiverEvent, ReceiverEventFn, ReceiverOptions};
use crate::error::Result;
use crate::progress::{create_download_progress, create_spinner, finish_progress};
use indicatif::ProgressBar;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub async fn run(
    client: &ApiClient,
    share_code: String,
    password: Option<String>,
    output: Option<PathBuf>,
) -> Result<()> {
    println!();

    let spinner_slot: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));
    let pb_slot: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));

    let sslot = spinner_slot.clone();
    let pslot = pb_slot.clone();

    let on_event: ReceiverEventFn = Arc::new(move |ev| {
        match ev {
            ReceiverEvent::Connecting => {
                let mut slot = sslot.lock().unwrap();
                *slot = Some(create_spinner("Connecting to sender..."));
            }
            ReceiverEvent::PeerMatched { device_info } => {
                if let Some(s) = sslot.lock().unwrap().take() {
                    s.finish_and_clear();
                }
                let info_str = device_info.as_deref().unwrap_or("Unknown device");
                println!("  \x1b[32m✓\x1b[0m Connected to sender ({})", info_str);
                println!();
            }
            ReceiverEvent::FileStart { name, size } => {
                let mut slot = pslot.lock().unwrap();
                *slot = Some(create_download_progress(size, &name));
            }
            ReceiverEvent::Progress { delta } => {
                if let Some(pb) = pslot.lock().unwrap().as_ref() {
                    pb.inc(delta);
                }
            }
            ReceiverEvent::FileEnd { name: _, saved_to } => {
                if let Some(pb) = pslot.lock().unwrap().take() {
                    finish_progress(&pb);
                }
                let size = std::fs::metadata(&saved_to).map(|m| m.len()).unwrap_or(0);
                println!(
                    "  Saved: {} ({})",
                    saved_to.display(),
                    crate::format::format_size_u64(size)
                );
            }
            ReceiverEvent::TransferComplete => {
                println!();
                println!("\x1b[32m✓ Download complete!\x1b[0m");
                println!();
            }
            ReceiverEvent::SenderGone(reason) => {
                if let Some(s) = sslot.lock().unwrap().take() {
                    s.finish_and_clear();
                }
                if reason == "Sender cancelled the transfer" {
                    println!("\x1b[33m⚠ Sender cancelled the transfer\x1b[0m");
                } else if reason == "Sender disconnected" {
                    println!("\x1b[33m⚠ Sender disconnected\x1b[0m");
                } else {
                    println!("\x1b[33m⚠ {}\x1b[0m", reason);
                }
            }
            ReceiverEvent::Failed(msg) => {
                if let Some(s) = sslot.lock().unwrap().take() {
                    s.finish_and_clear();
                }
                println!("\x1b[31m✗ {}\x1b[0m", msg);
            }
        }
    });

    let output_dir = output.unwrap_or_else(|| PathBuf::from("."));

    let info = crate::core::shares::get_share_info(client, &share_code).await?;
    let files: Vec<String> = info.files.iter().map(|f| f.file_name.clone()).collect();

    let opts = ReceiverOptions {
        share_code,
        password,
        output_dir,
        files,
    };

    crate::core::p2p::receiver::run(client, opts, on_event).await?;
    Ok(())
}
