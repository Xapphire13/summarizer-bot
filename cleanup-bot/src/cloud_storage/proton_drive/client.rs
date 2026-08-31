use std::path::Path;
use std::process::Stdio;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;
use tracing::{debug, info};

use crate::cloud_storage::{CloudStorage, CloudStorageError, build_relative_remote_path};

const CLI_BIN: &str = "proton-drive";
/// Proton Drive's fixed top-level section for user files. All destination
/// paths live under this, and it isn't configurable — the config's
/// `upload_folder` stays backend-independent, so this prefix is applied here.
const DRIVE_ROOT: &str = "/my-files";

#[derive(Deserialize)]
struct Node {
    name: NodeName,
    #[serde(rename = "type")]
    node_type: Option<String>,
}

#[derive(Deserialize)]
struct NodeName {
    value: Option<String>,
}

pub struct ProtonDriveClient {
    upload_folder: String,
}

impl ProtonDriveClient {
    pub fn new(upload_folder: String) -> Self {
        Self { upload_folder }
    }

    async fn run(&self, args: &[&str]) -> Result<String, CloudStorageError> {
        let output = Command::new(CLI_BIN)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CloudStorageError::Upload(format!(
                "`proton-drive {}` failed: {stderr}",
                args.join(" "),
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Whether a folder named `name` directly exists under `parent_path`.
    async fn folder_exists(
        &self,
        parent_path: &str,
        name: &str,
    ) -> Result<bool, CloudStorageError> {
        let stdout = self
            .run(&["filesystem", "list", "-t", "folder", parent_path, "-j"])
            .await?;

        let nodes: Vec<Node> = serde_json::from_str(&stdout).map_err(|e| {
            CloudStorageError::Upload(format!("Failed to parse folder listing: {e}"))
        })?;

        Ok(nodes.iter().any(|n| {
            n.node_type.as_deref() == Some("folder") && n.name.value.as_deref() == Some(name)
        }))
    }

    async fn create_folder(&self, parent_path: &str, name: &str) -> Result<(), CloudStorageError> {
        self.run(&["filesystem", "create-folder", parent_path, name])
            .await?;
        Ok(())
    }

    /// Ensure the full remote folder path exists, creating any missing segments.
    /// The first path segment (e.g. `my-files`) is a fixed top-level section and is
    /// assumed to already exist rather than something we can create.
    async fn ensure_folder_path(&self, remote_folder: &str) -> Result<(), CloudStorageError> {
        let mut segments = remote_folder
            .trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty());

        let Some(section) = segments.next() else {
            return Ok(());
        };
        let mut current = format!("/{section}");

        for segment in segments {
            if !self.folder_exists(&current, segment).await? {
                debug!("Creating remote folder {segment} under {current}");
                self.create_folder(&current, segment).await?;
            }

            current = format!("{current}/{segment}");
        }

        Ok(())
    }
}

#[async_trait]
impl CloudStorage for ProtonDriveClient {
    async fn upload_file(&self, local_path: &Path) -> Result<(), CloudStorageError> {
        let relative_path = build_relative_remote_path(local_path);
        let (date_folder, _file_name) = relative_path
            .rsplit_once('/')
            .expect("relative remote path always contains a date-based folder segment");
        let remote_folder = format!(
            "{DRIVE_ROOT}/{}/{date_folder}",
            self.upload_folder.trim_matches('/'),
        );

        info!("Uploading {} to {remote_folder}", local_path.display());

        self.ensure_folder_path(&remote_folder).await?;

        let local_path_str = local_path.to_str().ok_or_else(|| {
            CloudStorageError::Upload("Local path is not valid UTF-8".to_string())
        })?;

        self.run(&[
            "filesystem",
            "upload",
            "-f",
            "replace",
            "-d",
            "merge",
            local_path_str,
            &remote_folder,
        ])
        .await?;

        Ok(())
    }
}
