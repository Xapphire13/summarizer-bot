use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{debug, info};

use super::auth::TokenStore;
use crate::cloud_storage::{CloudStorage, CloudStorageError, build_relative_remote_path};

const GRAPH_API: &str = "https://graph.microsoft.com/v1.0";
const SIMPLE_UPLOAD_LIMIT: u64 = 4 * 1024 * 1024; // 4MB
const CHUNK_SIZE: usize = 10 * 1024 * 1024; // 10MB chunks for resumable upload

#[derive(Deserialize)]
struct UploadSession {
    #[serde(rename = "uploadUrl")]
    upload_url: String,
}

pub struct OneDriveClient {
    http: Client,
    token_store: Arc<Mutex<TokenStore>>,
    upload_folder: String,
}

impl OneDriveClient {
    pub fn new(token_store: Arc<Mutex<TokenStore>>, upload_folder: String) -> Self {
        Self {
            http: Client::new(),
            token_store,
            upload_folder,
        }
    }

    /// Build the remote path with date-based organization.
    fn build_remote_path(&self, local_path: &Path) -> String {
        format!(
            "{}/{}",
            self.upload_folder.trim_end_matches('/'),
            build_relative_remote_path(local_path),
        )
    }

    /// Simple upload for files < 4MB.
    async fn simple_upload(
        &self,
        local_path: &Path,
        remote_path: &str,
    ) -> Result<(), CloudStorageError> {
        let token = self.token_store.lock().await.get_valid_token().await?;
        let content = tokio::fs::read(local_path).await?;

        let url = format!("{GRAPH_API}/me/drive/root:{remote_path}:/content");

        let resp = self
            .http
            .put(&url)
            .bearer_auth(&token)
            .header("Content-Type", "application/octet-stream")
            .body(content)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudStorageError::Upload(format!(
                "Upload failed with status {status}: {body}"
            )));
        }

        debug!("Simple upload completed for {remote_path}");
        Ok(())
    }

    /// Resumable upload for files >= 4MB.
    async fn resumable_upload(
        &self,
        local_path: &Path,
        remote_path: &str,
        _file_size: u64,
    ) -> Result<(), CloudStorageError> {
        let token = self.token_store.lock().await.get_valid_token().await?;

        // Create upload session
        let url = format!("{GRAPH_API}/me/drive/root:{remote_path}:/createUploadSession");
        let body = serde_json::json!({
            "item": {
                "@microsoft.graph.conflictBehavior": "replace"
            }
        });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudStorageError::Upload(format!(
                "Failed to create upload session: {status}: {body}"
            )));
        }

        let session: UploadSession = resp.json().await?;
        debug!("Created upload session for {remote_path}");

        // Read file and upload in chunks
        let content = tokio::fs::read(local_path).await?;
        let total_size = content.len();

        for (chunk_num, chunk) in content.chunks(CHUNK_SIZE).enumerate() {
            let start = chunk_num * CHUNK_SIZE;
            let end = start + chunk.len() - 1;
            let content_range = format!("bytes {start}-{end}/{total_size}");

            debug!("Uploading chunk {}: {content_range}", chunk_num + 1);

            let resp = self
                .http
                .put(&session.upload_url)
                .header("Content-Range", &content_range)
                .body(chunk.to_vec())
                .send()
                .await?;

            if !resp.status().is_success() && resp.status().as_u16() != 202 {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(CloudStorageError::Upload(format!(
                    "Chunk upload failed: {status}: {body}"
                )));
            }
        }

        debug!("Resumable upload completed for {remote_path}");
        Ok(())
    }
}

#[async_trait]
impl CloudStorage for OneDriveClient {
    /// Upload a file to OneDrive. Automatically uses simple or resumable upload based on file size.
    async fn upload_file(&self, local_path: &Path) -> Result<(), CloudStorageError> {
        let remote_path = self.build_remote_path(local_path);
        let metadata = tokio::fs::metadata(local_path).await?;
        let file_size = metadata.len();

        info!(
            "Uploading {} ({file_size} bytes) to {remote_path}",
            local_path.display(),
        );

        if file_size < SIMPLE_UPLOAD_LIMIT {
            self.simple_upload(local_path, &remote_path).await
        } else {
            self.resumable_upload(local_path, &remote_path, file_size)
                .await
        }
    }
}
