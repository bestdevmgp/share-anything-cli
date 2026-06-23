//! Shared file collection for uploads and P2P transfers.
//!
//! When a directory is passed as an argument we recurse into it and compute
//! each file's root-relative path (including the leaf file name) so that the
//! folder structure can be preserved end to end. The relative path is
//! normalized to match the backend's `sanitize_relative_path`: backslashes
//! become forward slashes, any leading slash is stripped, and `.`/`..`
//! segments are removed.

use crate::core::error::CoreError;
use std::path::{Path, PathBuf};

/// A file collected for upload or transfer.
pub struct CollectedFile {
    /// Path to the file on disk.
    pub path: PathBuf,
    /// Root-relative path INCLUDING the leaf file name (e.g.
    /// `docs/2024/report.pdf`), normalized to forward slashes with no leading
    /// slash or `.`/`..` segments. `None` for files passed directly as
    /// arguments (i.e. not discovered inside a directory).
    pub relative_path: Option<String>,
}

pub struct Collected {
    pub files: Vec<CollectedFile>,
    pub empty_folders: Vec<String>,
}

/// Walk the given input paths. Plain files are collected as-is with no relative
/// path. Directories are recursed; every contained file gets a relative path
/// rooted at the directory argument's own name, so the top-level folder is
/// preserved (e.g. arg `./docs` → `docs/2024/report.pdf`).
pub fn collect_files(inputs: &[PathBuf]) -> Result<Collected, CoreError> {
    let mut files = Vec::new();
    let mut empty_folders = Vec::new();
    for input in inputs {
        if !input.exists() {
            return Err(CoreError::Other(format!(
                "File not found: {}",
                input.display()
            )));
        }
        if input.is_dir() {
            // Use the directory's own name as the root prefix so the top folder
            // survives in the relative path. Falls back to an empty prefix for
            // odd inputs like `.` or `..` where there is no file name.
            let prefix = input
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            collect_dir(input, &prefix, &mut files, &mut empty_folders)?;
        } else {
            files.push(CollectedFile {
                path: input.clone(),
                relative_path: None,
            });
        }
    }
    Ok(Collected { files, empty_folders })
}

fn collect_dir(
    dir: &Path,
    prefix: &str,
    files: &mut Vec<CollectedFile>,
    empty_folders: &mut Vec<String>,
) -> Result<bool, CoreError> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| CoreError::Other(format!("Failed to read directory {}: {}", dir.display(), e)))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    // Deterministic order keeps the backend sort order (and receiver download
    // order) stable across runs.
    entries.sort();

    let mut any_file = false;
    for path in entries {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{}/{}", prefix, name)
        };
        // `is_dir` / `is_file` follow symlinks; anything that is neither (e.g. a
        // broken symlink) is skipped.
        if path.is_dir() {
            if collect_dir(&path, &rel, files, empty_folders)? {
                any_file = true;
            }
        } else if path.is_file() {
            files.push(CollectedFile {
                path,
                relative_path: normalize_relative_path(&rel),
            });
            any_file = true;
        }
    }

    if !any_file {
        if let Some(p) = normalize_relative_path(prefix) {
            empty_folders.push(p);
        }
    }

    Ok(any_file)
}

/// Normalize a relative path to match the backend's `sanitize_relative_path`.
/// Returns `None` when the path is empty after sanitization or contains a `..`
/// segment (which the backend rejects entirely).
pub fn normalize_relative_path(raw: &str) -> Option<String> {
    if raw.contains('\0') {
        return None;
    }
    let normalized = raw.replace('\\', "/");
    let trimmed = normalized.trim_start_matches('/');

    let mut segments: Vec<&str> = Vec::new();
    for segment in trimmed.split('/') {
        match segment {
            "" | "." => continue,
            ".." => return None,
            other => segments.push(other),
        }
    }

    let result = segments.join("/");
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_basic() {
        assert_eq!(
            normalize_relative_path("docs/2024/report.pdf").as_deref(),
            Some("docs/2024/report.pdf")
        );
    }

    #[test]
    fn normalize_backslashes_to_slashes() {
        assert_eq!(
            normalize_relative_path(r"docs\2024\report.pdf").as_deref(),
            Some("docs/2024/report.pdf")
        );
    }

    #[test]
    fn normalize_strips_leading_slash() {
        assert_eq!(
            normalize_relative_path("/docs/a.txt").as_deref(),
            Some("docs/a.txt")
        );
    }

    #[test]
    fn normalize_drops_dot_segments() {
        assert_eq!(
            normalize_relative_path("docs/./a.txt").as_deref(),
            Some("docs/a.txt")
        );
    }

    #[test]
    fn normalize_rejects_dotdot() {
        assert_eq!(normalize_relative_path("../etc/passwd"), None);
        assert_eq!(normalize_relative_path("docs/../a.txt"), None);
    }

    #[test]
    fn normalize_empty_is_none() {
        assert_eq!(normalize_relative_path(""), None);
        assert_eq!(normalize_relative_path("/"), None);
        assert_eq!(normalize_relative_path("."), None);
    }

    /// Build a unique temp directory for filesystem tests.
    fn unique_tmp(tag: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("share-cli-test-{}-{}", tag, nanos));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn collect_plain_file_has_no_relative_path() {
        let root = unique_tmp("plain");
        let file = root.join("hello.txt");
        std::fs::write(&file, b"hi").unwrap();

        let collected = collect_files(&[file.clone()]).unwrap();
        assert_eq!(collected.files.len(), 1);
        assert_eq!(collected.files[0].relative_path, None);
        assert_eq!(collected.files[0].path, file);
        assert!(collected.empty_folders.is_empty());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn collect_directory_preserves_structure_including_top_folder() {
        let base = unique_tmp("dir");
        let proj = base.join("project");
        std::fs::create_dir_all(proj.join("docs/2024")).unwrap();
        std::fs::create_dir_all(proj.join("src")).unwrap();
        std::fs::write(proj.join("README.md"), b"r").unwrap();
        std::fs::write(proj.join("docs/2024/report.pdf"), b"p").unwrap();
        std::fs::write(proj.join("src/main.rs"), b"m").unwrap();

        let collected = collect_files(&[proj.clone()]).unwrap();
        let mut rels: Vec<String> = collected
            .files
            .iter()
            .map(|c| c.relative_path.clone().unwrap())
            .collect();
        rels.sort();

        assert_eq!(
            rels,
            vec![
                "project/README.md".to_string(),
                "project/docs/2024/report.pdf".to_string(),
                "project/src/main.rs".to_string(),
            ]
        );
        assert!(collected.empty_folders.is_empty());

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn collect_records_empty_folders_at_every_level() {
        let base = unique_tmp("empty");
        let proj = base.join("project");
        std::fs::create_dir_all(proj.join("logs")).unwrap();
        std::fs::create_dir_all(proj.join("data/cache")).unwrap();
        std::fs::write(proj.join("README.md"), b"r").unwrap();

        let collected = collect_files(&[proj.clone()]).unwrap();

        let rels: Vec<String> = collected
            .files
            .iter()
            .map(|c| c.relative_path.clone().unwrap())
            .collect();
        assert_eq!(rels, vec!["project/README.md".to_string()]);

        let mut empties = collected.empty_folders.clone();
        empties.sort();
        assert_eq!(
            empties,
            vec![
                "project/data".to_string(),
                "project/data/cache".to_string(),
                "project/logs".to_string(),
            ]
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn collect_missing_path_errors() {
        let missing = std::env::temp_dir().join("share-cli-test-does-not-exist-zzz");
        assert!(collect_files(&[missing]).is_err());
    }
}
