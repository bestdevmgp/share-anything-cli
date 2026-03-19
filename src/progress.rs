use indicatif::{ProgressBar, ProgressStyle};

pub fn create_upload_progress(total_size: u64, file_name: &str) -> ProgressBar {
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::with_template(
            "{msg}\n{wide_bar:.cyan/blue} {percent}% | {bytes}/{total_bytes} | {bytes_per_sec} | {eta}"
        )
        .unwrap()
        .progress_chars("██░"),
    );
    pb.set_message(format!("Uploading: {}", file_name));
    pb
}

pub fn create_download_progress(total_size: u64, file_name: &str) -> ProgressBar {
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::with_template(
            "{msg}\n{wide_bar:.green/white} {percent}% | {bytes}/{total_bytes} | {bytes_per_sec} | {eta}"
        )
        .unwrap()
        .progress_chars("██░"),
    );
    pb.set_message(format!("Downloading: {}", file_name));
    pb
}

pub fn create_spinner(message: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.green} {msg}")
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}
