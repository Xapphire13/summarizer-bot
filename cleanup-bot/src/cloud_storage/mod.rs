mod onedrive;
mod proton_drive;

pub use onedrive::{OneDriveClient, TokenStore};
pub use proton_drive::ProtonDriveClient;

use std::path::Path;

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, Utc};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CloudStorageError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Token storage error: {0}")]
    TokenStorage(String),

    #[error("Upload failed: {0}")]
    Upload(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Abstraction over a cloud storage backend used for backing up files.
#[async_trait]
pub trait CloudStorage: Send + Sync {
    /// Upload a local file to cloud storage, organized under a date-based folder layout.
    async fn upload_file(&self, local_path: &Path) -> Result<(), CloudStorageError>;
}

/// Build a relative remote path (`YYYY/MM/DD/filename`) for a local backup file.
/// Extracts the date from the parent directory name (format: YYYY-MM-DD), falling back to today.
pub(crate) fn build_relative_remote_path(local_path: &Path) -> String {
    let file_name = local_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let (year, month, day) = local_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .map(|d| (d.year(), d.month(), d.day()))
        .unwrap_or_else(|| {
            let now = Utc::now();
            (now.year(), now.month(), now.day())
        });

    format!("{year:04}/{month:02}/{day:02}/{file_name}")
}
