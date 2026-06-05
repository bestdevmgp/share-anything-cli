use crate::client::ApiClient;
use crate::core::error::CoreError;
use crate::core::shares::{api_error, map_envelope};
use crate::core::ProgressFn;
use bytes::Bytes;
use futures_util::StreamExt;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Semaphore;
use tokio_util::io::ReaderStream;

pub type PhaseFn = Arc<dyn Fn(UploadPhase) + Send + Sync>;

#[derive(Debug, Clone, Copy)]
pub enum UploadPhase {
    InitStarted,
    InitDone,
}

#[derive(Clone, Default)]
pub struct UploadOptions {
    pub password: Option<String>,
    pub expiration: Option<String>,
    pub one_time: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ShareResult {
    pub share_code: String,
    pub files: Vec<String>,
    pub expires_at: String,
}

#[derive(Clone, Debug)]
pub enum FileSource {
    Path(PathBuf),
    /// In-memory bytes (e.g. stdin upload). Held resident because the source isn't seekable.
    Memory(Bytes),
}

pub struct FileEntry {
    pub name: String,
    pub size: u64,
    pub content_type: String,
    pub source: FileSource,
}

pub fn read_files(paths: &[PathBuf]) -> Result<Vec<FileEntry>, CoreError> {
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        if !p.exists() {
            return Err(CoreError::Other(format!("File not found: {}", p.display())));
        }
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
        let size = std::fs::metadata(p)?.len();
        let content_type = mime_guess::from_path(p).first_or_octet_stream().to_string();
        out.push(FileEntry {
            name,
            size,
            content_type,
            source: FileSource::Path(p.clone()),
        });
    }
    Ok(out)
}

// Matches the server's R2 part size; files larger than this are split into multiple PUTs.
const CHUNK_SIZE: i64 = 50 * 1024 * 1024;
const STREAM_CHUNK: usize = 16384;
const MAX_CONCURRENT_PARTS: usize = 4;
// Sweet spot above which syscall savings flatten while TLS records (16 KB) and the
// TCP send window saturate.
const READER_BUFFER_CAPACITY: usize = 256 * 1024;

fn progress_stream(
    data: Bytes,
    on_progress: ProgressFn,
) -> impl futures_util::Stream<Item = std::result::Result<Vec<u8>, std::io::Error>> + Send {
    let data: Vec<u8> = data.to_vec();
    futures_util::stream::unfold((data, 0usize, on_progress), |(data, offset, on_progress)| async move {
        if offset >= data.len() {
            return None;
        }
        let end = std::cmp::min(offset + STREAM_CHUNK, data.len());
        let chunk = data[offset..end].to_vec();
        on_progress(chunk.len() as u64);
        Some((Ok(chunk), (data, end, on_progress)))
    })
}

async fn entry_body(source: FileSource, on_progress: ProgressFn) -> Result<reqwest::Body, CoreError> {
    match source {
        FileSource::Memory(bytes) => {
            let stream = progress_stream(bytes, on_progress);
            Ok(reqwest::Body::wrap_stream(stream))
        }
        FileSource::Path(path) => {
            let file = tokio::fs::File::open(&path).await?;
            let raw = ReaderStream::with_capacity(file, READER_BUFFER_CAPACITY);
            let stream = raw.map(move |res| {
                if let Ok(ref chunk) = res {
                    on_progress(chunk.len() as u64);
                }
                res
            });
            Ok(reqwest::Body::wrap_stream(stream))
        }
    }
}

pub async fn upload_files(
    client: &ApiClient,
    files: Vec<FileEntry>,
    opts: UploadOptions,
    on_progress: ProgressFn,
    on_phase: Option<PhaseFn>,
) -> Result<ShareResult, CoreError> {
    if files.is_empty() {
        return Err(CoreError::Other("No files to upload".into()));
    }
    upload_via_presigned(client, files, opts, on_progress, on_phase).await
}

#[derive(Debug, Deserialize)]
struct MultipartInitResponse {
    upload_session_id: String,
    share_code: String,
    files: Vec<MultipartFileInit>,
    #[allow(dead_code)] // wire-protocol field; client uses local CHUNK_SIZE constant
    chunk_size: i64,
}

#[derive(Debug, Deserialize, Clone)]
struct MultipartFileInit {
    #[allow(dead_code)] // server echoes file_name; client tracks names via FileEntry
    file_name: String,
    storage_key: String,
    upload_id: String,
    total_parts: i32,
}

#[derive(Debug, Deserialize)]
struct PresignPartsResponse {
    urls: Vec<PartUrl>,
}

#[derive(Debug, Deserialize)]
struct PartUrl {
    part_number: i32,
    presigned_url: String,
}

async fn upload_via_presigned(
    client: &ApiClient,
    files: Vec<FileEntry>,
    opts: UploadOptions,
    on_progress: ProgressFn,
    on_phase: Option<PhaseFn>,
) -> Result<ShareResult, CoreError> {
    let file_descs: Vec<serde_json::Value> = files
        .iter()
        .map(|f| {
            serde_json::json!({
                "file_name": f.name,
                "file_size": f.size as i64,
                "content_type": f.content_type,
            })
        })
        .collect();

    let mut init_body = serde_json::json!({
        "files": file_descs,
        "chunk_size": CHUNK_SIZE,
    });
    if let Some(pw) = &opts.password {
        init_body["password"] = serde_json::json!(pw);
    }
    if let Some(exp) = &opts.expiration {
        init_body["expiration"] = serde_json::json!(exp);
    }
    if opts.one_time {
        init_body["is_one_time"] = serde_json::json!(true);
    }

    if let Some(ref cb) = on_phase {
        cb(UploadPhase::InitStarted);
    }

    let resp = client
        .client
        .post(client.url("/cli/uploads/multipart"))
        .json(&init_body)
        .send()
        .await?;

    if !resp.status().is_success() {
        if let Some(ref cb) = on_phase {
            cb(UploadPhase::InitDone);
        }
        return Err(api_error(resp).await);
    }

    let init: MultipartInitResponse = resp.json().await?;
    if let Some(ref cb) = on_phase {
        cb(UploadPhase::InitDone);
    }

    if init.files.len() != files.len() {
        return Err(CoreError::Other(format!(
            "Server returned {} file slots for {} files",
            init.files.len(),
            files.len()
        )));
    }

    let mut completed_files: Vec<serde_json::Value> = Vec::with_capacity(files.len());
    let file_init_list = init.files.clone();
    for (entry, file_init) in files.into_iter().zip(file_init_list.iter()) {
        let parts = upload_file_parts(
            client,
            &init.upload_session_id,
            &entry,
            file_init,
            on_progress.clone(),
        )
        .await?;

        // Empty upload_id means the server skipped S3 multipart-init for this file; the
        // complete endpoint expects the sentinel "direct" to know not to finalize it on S3.
        let upload_id_str = if file_init.upload_id.is_empty() {
            "direct".to_string()
        } else {
            file_init.upload_id.clone()
        };

        completed_files.push(serde_json::json!({
            "file_name": entry.name,
            "storage_key": file_init.storage_key,
            "upload_id": upload_id_str,
            "file_size": entry.size as i64,
            "content_type": entry.content_type,
            "parts": parts,
        }));
    }

    let complete_resp = client
        .client
        .post(client.url(&format!(
            "/cli/uploads/multipart/{}/complete",
            init.upload_session_id
        )))
        .json(&serde_json::json!({
            "upload_session_id": init.upload_session_id,
            "share_code": init.share_code,
            "files": completed_files,
        }))
        .send()
        .await?;

    map_envelope::<ShareResult>(complete_resp).await
}

async fn upload_file_parts(
    client: &ApiClient,
    upload_session_id: &str,
    entry: &FileEntry,
    file_init: &MultipartFileInit,
    on_progress: ProgressFn,
) -> Result<Vec<serde_json::Value>, CoreError> {
    let file_size = entry.size as i64;

    if file_init.total_parts <= 1 {
        let presign_resp = client
            .client
            .post(client.url(&format!(
                "/cli/uploads/multipart/{}/parts",
                upload_session_id
            )))
            .json(&serde_json::json!({
                "upload_session_id": upload_session_id,
                "storage_key": file_init.storage_key,
                "upload_id": file_init.upload_id,
                "part_numbers": [1],
            }))
            .send()
            .await?;

        if !presign_resp.status().is_success() {
            return Err(api_error(presign_resp).await);
        }
        let presign: PresignPartsResponse = presign_resp.json().await?;
        let url = presign
            .urls
            .first()
            .ok_or_else(|| CoreError::Other("Server returned no presigned URL".into()))?
            .presigned_url
            .clone();

        let body = entry_body(entry.source.clone(), on_progress.clone()).await?;

        let resp = client
            .client
            .put(&url)
            .header("content-type", entry.content_type.as_str())
            .header("content-length", file_size.to_string())
            .body(body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(CoreError::Api {
                status,
                message: format!("Storage PUT failed: {}", body),
            });
        }

        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| CoreError::Other("Missing ETag on PUT response".into()))?
            .trim_matches('"')
            .to_string();

        return Ok(vec![serde_json::json!({
            "part_number": 1,
            "etag": etag,
        })]);
    }

    let chunk_size = CHUNK_SIZE as u64;
    let total_parts = file_init.total_parts;
    let part_numbers: Vec<i32> = (1..=total_parts).collect();

    let presign_resp = client
        .client
        .post(client.url(&format!(
            "/cli/uploads/multipart/{}/parts",
            upload_session_id
        )))
        .json(&serde_json::json!({
            "upload_session_id": upload_session_id,
            "storage_key": file_init.storage_key,
            "upload_id": file_init.upload_id,
            "part_numbers": part_numbers,
        }))
        .send()
        .await?;
    if !presign_resp.status().is_success() {
        return Err(api_error(presign_resp).await);
    }
    let presign: PresignPartsResponse = presign_resp.json().await?;

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_PARTS));
    let mut tasks: tokio::task::JoinSet<Result<(i32, String), CoreError>> =
        tokio::task::JoinSet::new();

    for url_entry in presign.urls {
        let part_number = url_entry.part_number;
        let presigned_url = url_entry.presigned_url;
        let start = (part_number as u64 - 1) * chunk_size;
        let part_size = std::cmp::min(chunk_size, file_size as u64 - start);
        let source_clone = entry.source.clone();

        let client_for_task = client.client.clone();
        let on_progress_for_task = on_progress.clone();
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;

        tasks.spawn(async move {
            let _permit = permit; // hold semaphore until task ends
            let body = match source_clone {
                FileSource::Path(path) => {
                    let mut f = tokio::fs::File::open(&path).await?;
                    f.seek(std::io::SeekFrom::Start(start)).await?;
                    let limited = f.take(part_size);
                    let raw = ReaderStream::with_capacity(limited, READER_BUFFER_CAPACITY);
                    let stream = raw.map(move |res| {
                        if let Ok(ref chunk) = res {
                            on_progress_for_task(chunk.len() as u64);
                        }
                        res
                    });
                    reqwest::Body::wrap_stream(stream)
                }
                FileSource::Memory(bytes) => {
                    let slice = bytes.slice(start as usize..(start + part_size) as usize);
                    let stream = progress_stream(slice, on_progress_for_task);
                    reqwest::Body::wrap_stream(stream)
                }
            };
            let resp = client_for_task
                .put(&presigned_url)
                .header("content-length", part_size.to_string())
                .body(body)
                .send()
                .await
                .map_err(CoreError::from)?;

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                return Err(CoreError::Api {
                    status,
                    message: format!("Storage PUT failed on part {}: {}", part_number, body),
                });
            }

            let etag = resp
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    CoreError::Other(format!("Missing ETag on part {}", part_number))
                })?
                .trim_matches('"')
                .to_string();

            Ok((part_number, etag))
        });
    }

    let mut parallel_parts: Vec<(i32, String)> = Vec::with_capacity(total_parts as usize);
    while let Some(res) = tasks.join_next().await {
        match res {
            Ok(Ok((pn, etag))) => parallel_parts.push((pn, etag)),
            Ok(Err(e)) => {
                tasks.shutdown().await;
                return Err(e);
            }
            Err(e) => {
                tasks.shutdown().await;
                return Err(CoreError::Other(e.to_string()));
            }
        }
    }
    parallel_parts.sort_by_key(|(pn, _)| *pn);
    Ok(parallel_parts
        .into_iter()
        .map(|(pn, etag)| {
            serde_json::json!({
                "part_number": pn,
                "etag": etag,
            })
        })
        .collect())
}
