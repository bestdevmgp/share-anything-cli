use crate::client::ApiClient;
use crate::core::shares::list_my_uploads;
use crate::error::Result;
use crate::format::{format_size, pad_display, truncate_display};

pub async fn run(client: &ApiClient) -> Result<()> {
    let uploads = list_my_uploads(client).await?;

    if uploads.is_empty() {
        println!("No uploads found.");
        return Ok(());
    }

    println!();
    println!(
        "{} {} {} {}",
        pad_display("CODE", 10),
        pad_display("FILE", 40),
        pad_display("SIZE", 10),
        pad_display("EXPIRES", 20),
    );
    println!("{}", "-".repeat(83));

    for u in &uploads {
        let display_name = truncate_display(&u.file_name, 40);
        println!(
            "{} {} {} {}",
            pad_display(&u.share_code, 10),
            pad_display(&display_name, 40),
            pad_display(&format_size(u.file_size), 10),
            pad_display(&crate::time::utc_to_local(&u.expires_at), 20),
        );
    }
    println!();
    Ok(())
}
