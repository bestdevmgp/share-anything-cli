use crate::client::ApiClient;
use crate::core::error::CoreError;
use crate::core::shares::api_error;
use crate::core::ProgressFn;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Default)]
pub struct DownloadOptions {
    pub password: Option<String>,
    pub file_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CliDownloadUrlResponse {
    download_url: String,
    file_id: String,
    file_name: String,
    file_size: i64,
}

#[derive(Debug, Serialize)]
struct CliDownloadCompleteRequest<'a> {
    file_id: &'a str,
}

pub async fn download_share(
    client: &ApiClient,
    code: &str,
    opts: DownloadOptions,
    output_dir: &Path,
    on_progress: ProgressFn,
) -> Result<PathBuf, CoreError> {
    let mut url = client.url(&format!("/cli/shares/{}/download-url", code));
    let mut params = Vec::new();
    if let Some(ref pw) = opts.password { params.push(format!("password={}", pw)); }
    if let Some(ref fid) = opts.file_id { params.push(format!("file_id={}", fid)); }
    if !params.is_empty() { url = format!("{}?{}", url, params.join("&")); }

    let resp = client.client.post(&url).send().await?;
    if !resp.status().is_success() { return Err(api_error(resp).await); }
    let issued: CliDownloadUrlResponse = resp.json().await?;

    let r2_client = reqwest::Client::builder()
        .user_agent(format!("share-cli/{}", env!("CARGO_PKG_VERSION")))
        .http1_only()
        .build()
        .map_err(CoreError::from)?;

    let output_path = if output_dir.is_dir() {
        output_dir.join(sanitize_file_name(&issued.file_name, code))
    } else {
        output_dir.to_path_buf()
    };

    let total_size = issued.file_size.max(0) as u64;
    let workers = pick_workers(total_size);

    if workers <= 1 || total_size == 0 {
        download_single_stream(&r2_client, &issued.download_url, &output_path, on_progress.clone()).await?;
    } else {
        download_ranges_parallel(
            &r2_client,
            &issued.download_url,
            &output_path,
            total_size,
            workers,
            on_progress.clone(),
        )
        .await?;
    }

    let _ = client
        .client
        .post(client.url(&format!("/cli/shares/{}/download-complete", code)))
        .json(&CliDownloadCompleteRequest { file_id: &issued.file_id })
        .send()
        .await;

    Ok(output_path)
}

async fn download_single_stream(
    r2_client: &reqwest::Client,
    url: &str,
    output_path: &Path,
    on_progress: ProgressFn,
) -> Result<(), CoreError> {
    let resp = r2_client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(CoreError::Api {
            status: resp.status().as_u16(),
            message: format!(
                "Storage rejected the download (HTTP {}). The presigned URL may have expired — retry the command.",
                resp.status()
            ),
        });
    }
    let mut file = tokio::fs::File::create(output_path).await?;
    let mut stream = resp.bytes_stream();
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        on_progress(chunk.len() as u64);
    }
    file.flush().await?;
    Ok(())
}

async fn download_ranges_parallel(
    r2_client: &reqwest::Client,
    url: &str,
    output_path: &Path,
    total_size: u64,
    workers: u64,
    on_progress: ProgressFn,
) -> Result<(), CoreError> {
    {
        let f = tokio::fs::File::create(output_path).await?;
        f.set_len(total_size).await?;
    }

    let chunk_size = (total_size + workers - 1) / workers;
    let mut handles: Vec<tokio::task::JoinHandle<Result<(), CoreError>>> =
        Vec::with_capacity(workers as usize);

    for i in 0..workers {
        let start = i * chunk_size;
        if start >= total_size {
            break;
        }
        let end = (start + chunk_size).min(total_size) - 1;
        let url = url.to_string();
        let path = output_path.to_path_buf();
        let client = r2_client.clone();
        let on_progress = on_progress.clone();
        let handle = tokio::spawn(async move {
            let resp = client
                .get(&url)
                .header(reqwest::header::RANGE, format!("bytes={}-{}", start, end))
                .send()
                .await?;
            let status = resp.status();
            if status != reqwest::StatusCode::PARTIAL_CONTENT
                && status != reqwest::StatusCode::OK
            {
                return Err(CoreError::Api {
                    status: status.as_u16(),
                    message: format!(
                        "Storage rejected the range request (HTTP {}). The presigned URL may have expired — retry the command.",
                        status
                    ),
                });
            }
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .await?;
            use tokio::io::{AsyncSeekExt, AsyncWriteExt};
            file.seek(std::io::SeekFrom::Start(start)).await?;
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                file.write_all(&chunk).await?;
                on_progress(chunk.len() as u64);
            }
            file.flush().await?;
            Ok(())
        });
        handles.push(handle);
    }

    let mut first_err: Option<CoreError> = None;
    for h in handles {
        let r = match h.await {
            Ok(inner) => inner,
            Err(e) => Err(CoreError::Other(format!("download worker panicked: {}", e))),
        };
        if let Err(e) = r {
            if first_err.is_none() {
                first_err = Some(e);
            }
        }
    }
    if let Some(e) = first_err {
        return Err(e);
    }
    Ok(())
}

fn pick_workers(total_size: u64) -> u64 {
    const MEDIUM_MIN: u64 = 50 * 1024 * 1024;
    const LARGE_MIN: u64 = 200 * 1024 * 1024;
    if total_size < MEDIUM_MIN {
        1
    } else if total_size < LARGE_MIN {
        4
    } else {
        8
    }
}

fn sanitize_file_name(raw: &str, code: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let trimmed = last.trim();
    let safe = matches!(trimmed, "" | "." | "..");
    if safe {
        format!("download_{}", code)
    } else {
        trimmed.to_string()
    }
}

#[derive(Serialize)]
struct VerifyPasswordRequest<'a> {
    code: &'a str,
    password: &'a str,
}

pub async fn verify_password(
    client: &ApiClient,
    code: &str,
    password: &str,
) -> Result<(), CoreError> {
    let body = VerifyPasswordRequest { code, password };
    let resp = client
        .client
        .post(client.url("/file/verify-password"))
        .json(&body)
        .send()
        .await?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(api_error(resp).await)
    }
}

#[derive(Serialize)]
struct BulkDownloadRequest<'a> {
    code: &'a str,
    file_ids: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<&'a str>,
}

pub async fn download_bulk_zip(
    client: &ApiClient,
    code: &str,
    file_ids: &[String],
    password: Option<&str>,
    output_dir: &Path,
    on_progress: ProgressFn,
) -> Result<PathBuf, CoreError> {
    let body = BulkDownloadRequest { code, file_ids, password };
    let resp = client
        .client
        .post(client.url("/download/bulk"))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }

    let output_path = if output_dir.is_dir() {
        output_dir.join(format!("share-{}.zip", code))
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

#[cfg(test)]
mod sanitize_tests {
    use super::sanitize_file_name;

    #[test]
    fn keeps_normal_names() {
        assert_eq!(sanitize_file_name("report.pdf", "987085"), "report.pdf");
        assert_eq!(sanitize_file_name("안녕.txt", "987085"), "안녕.txt");
    }

    #[test]
    fn strips_path_components() {
        assert_eq!(sanitize_file_name("../etc/passwd", "987085"), "passwd");
        assert_eq!(sanitize_file_name("a/b/c/d.bin", "987085"), "d.bin");
        assert_eq!(sanitize_file_name(r"C:\Windows\notepad.exe", "987085"), "notepad.exe");
    }

    #[test]
    fn rejects_dot_and_dotdot() {
        assert_eq!(sanitize_file_name("..", "987085"), "download_987085");
        assert_eq!(sanitize_file_name(".", "987085"), "download_987085");
        assert_eq!(sanitize_file_name("foo/..", "987085"), "download_987085");
    }

    #[test]
    fn falls_back_when_empty() {
        assert_eq!(sanitize_file_name("", "987085"), "download_987085");
        assert_eq!(sanitize_file_name("   ", "987085"), "download_987085");
        assert_eq!(sanitize_file_name("/", "987085"), "download_987085");
    }
}

