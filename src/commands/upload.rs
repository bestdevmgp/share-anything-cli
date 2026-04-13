use crate::client::ApiClient;
use crate::error::{CliError, Result};
use crate::progress::{create_upload_progress, create_spinner, update_progress, finish_progress};
use indicatif::ProgressBar;
use reqwest::multipart;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct UploadResponse {
    share_code: String,
    files: Vec<String>,
    curl_command: String,
    expires_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct MultipartInitResponse {
    upload_session_id: String,
    share_code: String,
    files: Vec<MultipartFileInit>,
    chunk_size: i64,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct MultipartFileInit {
    file_name: String,
    storage_key: String,
    upload_id: String,
    total_parts: i32,
}

#[derive(Debug, Deserialize)]
struct PresignPartsResponse {
    urls: Vec<PartUrl>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct PartUrl {
    part_number: i32,
    presigned_url: String,
}

const MULTIPART_THRESHOLD: u64 = 100 * 1024 * 1024; // 100MB
const CHUNK_SIZE: i64 = 50 * 1024 * 1024; // 50MB
const STREAM_CHUNK: usize = 16384; // 16KB

fn progress_stream(
    data: Vec<u8>,
    pb: ProgressBar,
) -> impl futures_util::Stream<Item = std::result::Result<Vec<u8>, std::io::Error>> + Send {
    futures_util::stream::unfold((data, 0usize, pb), |(data, offset, pb)| async move {
        if offset >= data.len() {
            return None;
        }
        let end = std::cmp::min(offset + STREAM_CHUNK, data.len());
        let chunk = data[offset..end].to_vec();
        update_progress(&pb, chunk.len() as u64);
        Some((Ok(chunk), (data, end, pb)))
    })
}

pub async fn run(
    client: &ApiClient,
    files: Vec<PathBuf>,
    stdin_data: Option<Vec<u8>>,
    name: Option<String>,
    password: Option<String>,
    expiration: Option<String>,
    one_time: bool,
) -> Result<()> {
    let mut file_entries: Vec<(String, Vec<u8>, String)> = Vec::new();

    if let Some(data) = stdin_data {
        let file_name = name.unwrap_or_else(|| "stdin.txt".to_string());
        file_entries.push((file_name, data, "application/octet-stream".to_string()));
    } else {
        for path in &files {
            if !path.exists() {
                return Err(CliError::Other(format!("File not found: {}", path.display())));
            }
            let file_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let data = std::fs::read(path)?;
            let content_type = mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string();
            file_entries.push((file_name, data, content_type));
        }
    }

    if file_entries.is_empty() {
        return Err(CliError::Other("No files to upload".to_string()));
    }

    let total_size: u64 = file_entries.iter().map(|(_, d, _)| d.len() as u64).sum();

    if total_size >= MULTIPART_THRESHOLD && file_entries.len() == 1 {
        upload_multipart(client, &file_entries[0], password, expiration, one_time).await
    } else {
        upload_direct(client, file_entries, total_size, password, expiration, one_time).await
    }
}

async fn upload_direct(
    client: &ApiClient,
    files: Vec<(String, Vec<u8>, String)>,
    total_size: u64,
    password: Option<String>,
    expiration: Option<String>,
    one_time: bool,
) -> Result<()> {
    let display_name = if files.len() == 1 {
        files[0].0.clone()
    } else {
        format!("{} files", files.len())
    };

    let pb = create_upload_progress(total_size, &display_name);

    let mut form = multipart::Form::new();

    for (name, data, content_type) in files {
        let len = data.len() as u64;
        let stream = progress_stream(data, pb.clone());
        let body = reqwest::Body::wrap_stream(stream);
        let part = multipart::Part::stream_with_length(body, len)
            .file_name(name)
            .mime_str(&content_type)
            .unwrap();
        form = form.part("file", part);
    }

    if let Some(pw) = password {
        form = form.text("password", pw);
    }
    if let Some(exp) = expiration {
        form = form.text("expiration", exp);
    }
    if one_time {
        form = form.text("is_one_time", "true");
    }

    let resp = client
        .client
        .post(client.url("/cli/upload"))
        .multipart(form)
        .send()
        .await?;

    finish_progress(&pb);

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let msg = body["message"]
            .as_str()
            .unwrap_or("Unknown error")
            .to_string();
        return Err(CliError::Api {
            status,
            message: msg,
        });
    }

    let result: UploadResponse = resp.json().await?;
    print_upload_result(&result);
    Ok(())
}

async fn upload_multipart(
    client: &ApiClient,
    file: &(String, Vec<u8>, String),
    password: Option<String>,
    expiration: Option<String>,
    one_time: bool,
) -> Result<()> {
    let (file_name, data, content_type) = file;
    let file_size = data.len() as i64;

    let spinner = create_spinner("Initializing multipart upload...");

    let mut init_body = serde_json::json!({
        "files": [{
            "file_name": file_name,
            "file_size": file_size,
            "content_type": content_type,
        }],
        "chunk_size": CHUNK_SIZE,
    });

    if let Some(pw) = &password {
        init_body["password"] = serde_json::json!(pw);
    }
    if let Some(exp) = &expiration {
        init_body["expiration"] = serde_json::json!(exp);
    }
    if one_time {
        init_body["is_one_time"] = serde_json::json!(true);
    }

    let resp = client
        .client
        .post(client.url("/cli/upload/multipart/init"))
        .json(&init_body)
        .send()
        .await?;

    spinner.finish_and_clear();

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        return Err(CliError::Api {
            status,
            message: body["message"].as_str().unwrap_or("Init failed").to_string(),
        });
    }

    let init: MultipartInitResponse = resp.json().await?;
    let file_init = &init.files[0];

    let pb = create_upload_progress(file_size as u64, file_name);

    let mut completed_parts: Vec<serde_json::Value> = Vec::new();

    if file_init.total_parts <= 1 {
        let presign_resp = client
            .client
            .post(client.url("/cli/upload/multipart/presign-parts"))
            .json(&serde_json::json!({
                "upload_session_id": init.upload_session_id,
                "storage_key": file_init.storage_key,
                "upload_id": file_init.upload_id,
                "part_numbers": [1],
            }))
            .send()
            .await?;

        let presign: PresignPartsResponse = presign_resp.json().await?;
        let url = &presign.urls[0].presigned_url;

        let len = data.len();
        let stream = progress_stream(data.clone(), pb.clone());
        let body = reqwest::Body::wrap_stream(stream);

        let resp = client
            .client
            .put(url)
            .header("content-type", content_type.as_str())
            .header("content-length", len.to_string())
            .body(body)
            .send()
            .await?;

        if let Some(etag) = resp.headers().get("etag") {
            completed_parts.push(serde_json::json!({
                "part_number": 1,
                "etag": etag.to_str().unwrap_or("").trim_matches('"'),
            }));
        }
    } else {
        let chunk_size = CHUNK_SIZE as usize;
        let total_parts = file_init.total_parts;

        for part_num in 1..=total_parts {
            let start = (part_num as usize - 1) * chunk_size;
            let end = std::cmp::min(start + chunk_size, data.len());
            let chunk_data = data[start..end].to_vec();
            let chunk_len = chunk_data.len();

            let presign_resp = client
                .client
                .post(client.url("/cli/upload/multipart/presign-parts"))
                .json(&serde_json::json!({
                    "upload_session_id": init.upload_session_id,
                    "storage_key": file_init.storage_key,
                    "upload_id": file_init.upload_id,
                    "part_numbers": [part_num],
                }))
                .send()
                .await?;

            let presign: PresignPartsResponse = presign_resp.json().await?;
            let url = &presign.urls[0].presigned_url;

            let stream = progress_stream(chunk_data, pb.clone());
            let body = reqwest::Body::wrap_stream(stream);

            let resp = client
                .client
                .put(url)
                .header("content-length", chunk_len.to_string())
                .body(body)
                .send()
                .await?;

            if let Some(etag) = resp.headers().get("etag") {
                completed_parts.push(serde_json::json!({
                    "part_number": part_num,
                    "etag": etag.to_str().unwrap_or("").trim_matches('"'),
                }));
            }
        }
    }

    finish_progress(&pb);

    let complete_resp = client
        .client
        .post(client.url("/cli/upload/multipart/complete"))
        .json(&serde_json::json!({
            "upload_session_id": init.upload_session_id,
            "share_code": init.share_code,
            "files": [{
                "file_name": file_name,
                "storage_key": file_init.storage_key,
                "upload_id": if file_init.upload_id.is_empty() { "direct" } else { &file_init.upload_id },
                "file_size": file_size,
                "content_type": content_type,
                "parts": completed_parts,
            }],
        }))
        .send()
        .await?;

    if !complete_resp.status().is_success() {
        let status = complete_resp.status().as_u16();
        let body: serde_json::Value = complete_resp.json().await.unwrap_or_default();
        return Err(CliError::Api {
            status,
            message: body["message"]
                .as_str()
                .unwrap_or("Complete failed")
                .to_string(),
        });
    }

    let result: UploadResponse = complete_resp.json().await?;
    print_upload_result(&result);
    Ok(())
}

pub async fn run_secure(
    _client: &ApiClient,
    _files: Vec<PathBuf>,
    _stdin_data: Option<Vec<u8>>,
    _name: Option<String>,
    _password: Option<String>,
) -> Result<()> {
    Err(crate::error::CliError::Other("Secure transfer not yet implemented".into()))
}

fn print_upload_result(result: &UploadResponse) {
    println!();
    println!("\x1b[32m✓ Upload complete!\x1b[0m");
    println!("  Share code : {}", result.share_code);
    println!("  Command    : share download {}", result.share_code);
    println!("  curl       : {}", result.curl_command);
    println!("  Expires    : {}", crate::time::utc_to_local(&result.expires_at));

    if result.files.len() > 1 {
        println!("  Files:");
        for f in &result.files {
            println!("    - {}", f);
        }
    }
    println!();
}
