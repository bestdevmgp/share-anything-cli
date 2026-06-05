use crate::client::ApiClient;
use crate::core::p2p::sender::{SenderEvent, SenderEventFn, SenderOptions};
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
) -> Result<()> {
    let spinner_slot: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));
    let pb_slot: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));

    let sslot = spinner_slot.clone();
    let pslot = pb_slot.clone();

    let on_event: SenderEventFn = Arc::new(move |ev| {
        match ev {
            SenderEvent::Created { share_code, files } => {
                println!();
                println!("\x1b[32m✓ Secure transfer ready\x1b[0m");
                println!("  Code     : {}", share_code);
                println!("  Command  : share download {}", share_code);
                if files.len() == 1 {
                    println!(
                        "  File     : {} ({})",
                        files[0].name,
                        crate::format::format_size_u64(files[0].size)
                    );
                } else {
                    println!("  Files    : {} files", files.len());
                    for f in &files {
                        println!(
                            "    - {} ({})",
                            f.name,
                            crate::format::format_size_u64(f.size)
                        );
                    }
                }
                println!();
                let mut slot = sslot.lock().unwrap();
                *slot = Some(create_spinner("Waiting for receiver to connect..."));
            }
            SenderEvent::ReceiverArrived { device_info } => {
                if let Some(s) = sslot.lock().unwrap().take() {
                    s.finish_and_clear();
                }
                let info_str = device_info.as_deref().unwrap_or("Unknown device");
                println!(
                    "  \x1b[36m→\x1b[0m Receiver arrived ({}), waiting for download to start...",
                    info_str
                );
            }
            SenderEvent::PeerMatched { device_info } => {
                if let Some(s) = sslot.lock().unwrap().take() {
                    s.finish_and_clear();
                }
                let info_str = device_info.as_deref().unwrap_or("Unknown device");
                println!("  \x1b[32m✓\x1b[0m Connected to receiver ({})", info_str);
                println!();
            }
            SenderEvent::FileStart { name, size } => {
                let mut slot = pslot.lock().unwrap();
                *slot = Some(create_upload_progress(size, &name));
            }
            SenderEvent::Progress { delta } => {
                if let Some(pb) = pslot.lock().unwrap().as_ref() {
                    pb.inc(delta);
                }
            }
            SenderEvent::FileEnd => {
                if let Some(pb) = pslot.lock().unwrap().take() {
                    finish_progress(&pb);
                }
            }
            SenderEvent::WaitingForNext => {
                {
                    if let Some(s) = sslot.lock().unwrap().take() {
                        s.finish_and_clear();
                    }
                }
                let mut slot = sslot.lock().unwrap();
                *slot = Some(create_spinner("Waiting for next request..."));
            }
            SenderEvent::TransferComplete => {
                println!();
                println!("\x1b[32m✓ Transfer complete!\x1b[0m");
                println!();
            }
            SenderEvent::ReceiverDisconnected => {
                println!("\n\x1b[33m⚠ Receiver disconnected. Waiting for new receiver...\x1b[0m");
                let mut slot = sslot.lock().unwrap();
                *slot = Some(create_spinner("Waiting for receiver to connect..."));
            }
            SenderEvent::RelayDetected => {
                println!("\x1b[33mℹ TURN server relay in use\x1b[0m");
            }
            SenderEvent::Failed(msg) => {
                println!("\x1b[31m✗ {}\x1b[0m", msg);
            }
        }
    });

    let opts = SenderOptions {
        files,
        stdin_data,
        stdin_name: name,
        password,
    };

    crate::core::p2p::sender::run(client, opts, on_event).await?;
    Ok(())
}
