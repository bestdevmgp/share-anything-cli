use crate::client::ApiClient;
use crate::core::shares::delete_share;
use crate::error::Result;

pub async fn run(client: &ApiClient, code: String) -> Result<()> {
    delete_share(client, &code).await?;
    println!("\x1b[32m✓\x1b[0m Share \x1b[1m{}\x1b[0m has been deleted.", code);
    Ok(())
}
