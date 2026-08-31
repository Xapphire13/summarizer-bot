use std::ops::Deref;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

use super::queue::BackupQueue;
use crate::cloud_storage::CloudStorage;
use crate::config::BackupWorkerConfig;

/// Spawn the background backup worker.
pub fn spawn_worker(
    queue: Arc<Mutex<BackupQueue>>,
    config: BackupWorkerConfig,
    cloud_storage: Arc<dyn CloudStorage>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_worker(queue, config, cloud_storage).await;
    })
}

async fn run_worker(
    queue: Arc<Mutex<BackupQueue>>,
    config: BackupWorkerConfig,
    cloud_storage: Arc<dyn CloudStorage>,
) {
    let check_interval = Duration::from_secs(config.check_interval_seconds);
    let mut interval = interval(check_interval);

    info!(
        "Backup worker started (check interval: {}s, max retries: {})",
        config.check_interval_seconds, config.max_retries
    );

    loop {
        interval.tick().await;

        let pending: Vec<_> = {
            let queue = queue.lock().unwrap();
            queue
                .get_pending()
                .into_iter()
                .map(|b| b.local_path.clone())
                .collect()
        };

        if pending.is_empty() {
            debug!("No pending backups to process");
            continue;
        }

        info!("Processing {} pending backups", pending.len());

        for local_path in pending {
            // Check if file still exists
            if !local_path.exists() {
                warn!("Backup file missing: {}", local_path.display());
                let mut queue = queue.lock().unwrap();
                if let Err(e) = queue.mark_failed(&local_path, "file missing".to_string()) {
                    error!("Failed to mark backup as failed: {e:?}");
                }
                continue;
            }

            // Get backup info and check retry count
            let (retry_count, should_skip) = {
                let queue = queue.lock().unwrap();
                if let Some(backup) = queue.get(&local_path) {
                    (backup.retry_count, backup.retry_count >= config.max_retries)
                } else {
                    continue;
                }
            };

            if should_skip {
                debug!(
                    "Skipping {} - max retries ({}) exceeded",
                    local_path.display(),
                    config.max_retries
                );
                continue;
            }

            // Mark as in progress
            {
                let mut queue = queue.lock().unwrap();
                if let Err(e) = queue.mark_in_progress(&local_path) {
                    error!("Failed to mark backup as in progress: {e:?}");
                    continue;
                }
            }

            // Attempt upload
            match upload_to_cloud(&local_path, cloud_storage.deref()).await {
                Ok(()) => {
                    info!("Successfully uploaded {}", local_path.display());

                    // Remove from queue
                    {
                        let mut queue = queue.lock().unwrap();
                        if let Err(e) = queue.remove(&local_path) {
                            error!("Failed to remove backup from queue: {e:?}");
                        }
                    }

                    // Delete local file
                    if let Err(e) = tokio::fs::remove_file(&local_path).await {
                        error!(
                            "Failed to delete local file {}: {e:?}",
                            local_path.display()
                        );
                    } else {
                        debug!("Deleted local file {}", local_path.display());

                        // remove_dir only removes empty directories — safe to call unconditionally
                        if let Some(parent) = local_path.parent() {
                            let _ = tokio::fs::remove_dir(parent).await;
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to upload {} (attempt {}): {e}",
                        local_path.display(),
                        retry_count + 1
                    );

                    // Mark as failed (will be retried on next cycle after delay)
                    let mut queue = queue.lock().unwrap();
                    if let Err(e) = queue.mark_failed(&local_path, e.to_string()) {
                        error!("Failed to mark backup as failed: {e:?}");
                    }
                }
            }
        }

        // Reset failed backups to pending for retry
        reset_failed_for_retry(&queue, &config);
    }
}

/// Upload file to cloud storage.
async fn upload_to_cloud(local_path: &Path, client: &dyn CloudStorage) -> Result<(), String> {
    client
        .upload_file(local_path)
        .await
        .map_err(|e| e.to_string())
}

/// Reset failed backups to pending status for retry.
fn reset_failed_for_retry(queue: &Arc<Mutex<BackupQueue>>, config: &BackupWorkerConfig) {
    let failed_paths: Vec<_> = {
        let queue = queue.lock().unwrap();
        queue
            .get_failed(config.max_retries)
            .into_iter()
            .map(|b| b.local_path.clone())
            .collect()
    };

    if !failed_paths.is_empty() {
        let mut queue = queue.lock().unwrap();
        for path in failed_paths {
            if let Err(e) = queue.reset_to_pending(&path) {
                error!("Failed to reset backup to pending: {e:?}");
            }
        }
    }
}
