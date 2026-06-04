use crate::client::ApiClient;
use crate::core::error::CoreError;
use crate::core::shares::{api_error, FileInfo};
use crate::core::ProgressFn;
use futures_util::StreamExt;
use serde::Serialize;
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
        output_dir.join(sanitize_file_name(&file_name, code))
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

/// Strip anything that could escape `output_dir` from a server-supplied filename.
/// The uploader controls this name, so treat it as untrusted: drop path separators,
/// reject `..`, refuse absolute paths, and fall back to a code-derived name if the
/// result is empty.
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

/// Verify a password against a share before kicking off the actual download. Returns
/// `Ok(())` on 200 and a typed `CoreError::Api` with status 401 when the password is
/// wrong, so callers can show a precise toast instead of leaking it to the post-download
/// failure path.
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

/// Stream every file in the share into a single store-only ZIP saved next to `output_dir`.
/// The save name is `share-{code}.zip`. Returns the saved path.
///
/// Uses the public `/download/bulk` endpoint (no `/cli/` prefix). That endpoint accepts an
/// explicit `file_ids` list — we pass all of them.
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
