use crate::client::ApiClient;
use crate::error::{CliError, Result};
use crate::progress::create_download_progress;
use futures_util::StreamExt;
use serde::Deserialize;
use std::path::PathBuf;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct FileInfoResponse {
    share_code: String,
    files: Vec<FileDetail>,
    has_password: bool,
}

#[derive(Debug, Deserialize)]
struct FileDetail {
    file_name: String,
    file_size: i64,
}

pub async fn run(
    client: &ApiClient,
    code: String,
    password: Option<String>,
    output: Option<PathBuf>,
    file_id: Option<String>,
) -> Result<()> {
    let info_resp = client
        .client
        .get(client.url(&format!("/cli/download/{}/info", code)))
        .send()
        .await?;

    if !info_resp.status().is_success() {
        let status = info_resp.status().as_u16();
        let body: serde_json::Value = info_resp.json().await.unwrap_or_default();
        return Err(CliError::Api {
            status,
            message: body["message"]
                .as_str()
                .unwrap_or("File not found")
                .to_string(),
        });
    }

    let info: FileInfoResponse = info_resp.json().await?;

    if info.has_password && password.is_none() {
        return Err(CliError::Other(
            "This file requires a password. Use --password <password>".to_string(),
        ));
    }

    let mut url = client.url(&format!("/cli/download/{}", code));
    let mut params = Vec::new();
    if let Some(ref pw) = password {
        params.push(format!("password={}", pw));
    }
    if let Some(ref fid) = file_id {
        params.push(format!("file_id={}", fid));
    }
    if !params.is_empty() {
        url = format!("{}?{}", url, params.join("&"));
    }

    let resp = client.client.get(&url).send().await?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        return Err(CliError::Api {
            status,
            message: body["message"]
                .as_str()
                .unwrap_or("Download failed")
                .to_string(),
        });
    }

    let file_name = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            // Try filename*=UTF-8''... first
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

    let content_length = resp.content_length().unwrap_or(0);
    let target_file = if !info.files.is_empty() {
        info.files[0].file_size as u64
    } else {
        content_length
    };

    let pb = create_download_progress(target_file, &file_name);

    let output_path = if let Some(dir) = output {
        if dir.is_dir() {
            dir.join(&file_name)
        } else {
            dir
        }
    } else {
        PathBuf::from(&file_name)
    };

    let mut file = tokio::fs::File::create(&output_path).await?;
    let mut stream = resp.bytes_stream();

    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| CliError::Http(e))?;
        file.write_all(&chunk).await?;
        pb.inc(chunk.len() as u64);
    }

    file.flush().await?;
    pb.finish_and_clear();

    println!();
    println!("\x1b[32m✓ Download complete!\x1b[0m");
    let display_path = if output_path.is_relative() && !output_path.starts_with(".") {
        format!("./{}", output_path.display())
    } else {
        output_path.display().to_string()
    };
    println!("  Saved to: {}", display_path);
    println!();

    Ok(())
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
