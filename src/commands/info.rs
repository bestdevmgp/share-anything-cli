use crate::client::ApiClient;
use crate::core::shares::get_share_info;
use crate::error::Result;

pub async fn run(client: &ApiClient, code: String) -> Result<()> {
    let info = get_share_info(client, &code).await?;
    println!();
    println!("Share code  : {}", info.share_code);
    if info.transfer_type.as_deref() == Some("p2p") {
        println!("Transfer    : Secure (P2P)");
    }
    println!("Password    : {}", if info.has_password { "Yes" } else { "No" });
    println!("One-time    : {}", if info.is_one_time { "Yes" } else { "No" });
    println!("Expires at  : {}", crate::time::utc_to_local(&info.expires_at));
    println!("Files ({}):", info.files.len());
    for f in &info.files {
        println!("  - {} ({})", f.file_name, crate::format::format_size(f.file_size));
    }
    println!();
    Ok(())
}
