use crate::client::ApiClient;
use crate::core::error::CoreError;
use crate::core::shares::{api_error, FileInfo};
use crate::core::ProgressFn;
use futures_util::StreamExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Default)]
pub struct DownloadOptions {
    pub password: Option<String>,
    pub file_id: Option<String>,
}

pub async fn download_share(
    client: &ApiClient,
    code: &str,
    info: &FileInfo,
    opts: DownloadOptions,
    output_dir: &Path,
    on_progress: ProgressFn,
) -> Result<PathBuf, CoreError> {
    let mut url = client.url(&format!("/cli/shares/{}/download", code));
    let mut params = Vec::new();
    if let Some(ref pw) = opts.password { params.push(format!("password={}", pw)); }
    if let Some(ref fid) = opts.file_id { params.push(format!("file_id={}", fid)); }
    if !params.is_empty() { url = format!("{}?{}", url, params.join("&")); }

    let resp = client.client.get(&url).send().await?;
    if !resp.status().is_success() { return Err(api_error(resp).await); }

    let file_name = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            if let Some(start) = v.find("filename*=UTF-8''") {
                let encoded = &v[start + 17..];
                let encoded = encoded.split(';').next().unwrap_or(encoded).trim();
                percent_decode(encoded)
            } else if let Some(start) = v.find("filename=") {
                let name = &v[start + 9..];
                let name = name.split(';').next().unwrap_or(name).trim();
                Some(name.trim_matches('"').to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            if !info.files.is_empty() {
                info.files[0].file_name.clone()
            } else {
                format!("download_{}", code)
            }
        });

    let output_path = if output_dir.is_dir() {
        output_dir.join(&file_name)
    } else {
        output_dir.to_path_buf()
    };

    let mut file = tokio::fs::File::create(&output_path).await?;
    let mut stream = resp.bytes_stream();
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        on_progress(chunk.len() as u64);
    }
    file.flush().await?;
    Ok(output_path)
}

fn percent_decode(input: &str) -> Option<String> {
    let mut result = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                result.push(byte);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(result).ok()
}
