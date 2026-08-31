use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Days;
use serenity::all::{ChannelId, GetMessages, Http, Timestamp};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::backup::{BackupQueue, BackupStatus, PendingBackup};
use crate::cancellation::{CancellationRegistry, CancellationToken};
use crate::cleanup::queue::{BackupJob, DeleteJob, classify_messages, filter_expired_messages};
use crate::config::ConfigStore;
use crate::media::MediaDownloader;

// Note: Discord requires messages to be < 14 days old for bulk delete
// see (https://discord.com/developers/docs/resources/message#bulk-delete-messages).
const BULK_DELETE_THRESHOLD: Days = Days::new(14);
const BULK_DELETE_MIN: usize = 2;
const BULK_DELETE_MAX: usize = 100;
const SINGLE_DELETE_DELAY: Duration = Duration::from_millis(200);
const BULK_DELETE_DELAY: Duration = Duration::from_secs(1);
const MAX_MESSAGES_PER_FETCH: u8 = 100;
const TARGET_EXPIRED_MESSAGES: usize = 100;
const MAX_PAGINATION_ROUNDS: usize = 10;

/// Run cleanup for a single channel.
pub async fn cleanup_channel(
    http: Arc<Http>,
    config: ConfigStore,
    backup_queue: Arc<Mutex<BackupQueue>>,
    cancellation: Arc<Mutex<CancellationRegistry>>,
    channel_id: ChannelId,
    retention_days: NonZeroU32,
    cancel_token: CancellationToken,
) {
    let result = run_cleanup(
        http,
        config,
        backup_queue,
        channel_id,
        retention_days,
        cancel_token,
    )
    .await;

    // Deregister cancellation token
    cancellation.lock().unwrap().deregister(channel_id);

    if let Err(e) = result {
        error!("Cleanup failed for channel {channel_id}: {e:?}");
    }
}

async fn run_cleanup(
    http: Arc<Http>,
    config: ConfigStore,
    backup_queue: Arc<Mutex<BackupQueue>>,
    channel_id: ChannelId,
    retention_days: NonZeroU32,
    cancel_token: CancellationToken,
) -> Result<()> {
    use serenity::all::{Message, MessageId};

    info!("Starting cleanup for channel {channel_id} (retention: {retention_days} days)");

    // Load pagination cursor from config
    let mut cursor: Option<MessageId> =
        config.get_pagination_cursor(channel_id).map(MessageId::new);

    let mut expired_messages: Vec<Message> = Vec::new();
    let mut reached_end = false;

    // Pagination loop
    for round in 0..MAX_PAGINATION_ROUNDS {
        if cancel_token.is_cancelled() {
            info!("Cleanup cancelled for channel {channel_id}");
            return Ok(());
        }

        // Build request with pagination
        let request = match cursor {
            Some(before_id) => GetMessages::new()
                .limit(MAX_MESSAGES_PER_FETCH)
                .before(before_id),
            None => GetMessages::new().limit(MAX_MESSAGES_PER_FETCH),
        };

        debug!(
            "Pagination round {}: fetching messages before {}",
            round + 1,
            cursor.unwrap_or_default()
        );

        // Fetch messages
        let messages = channel_id
            .messages(&http, request)
            .await
            .context("Failed to fetch messages")?;

        if messages.is_empty() {
            debug!("No more messages in channel {channel_id}");
            reached_end = true;
            break;
        }

        debug!(
            "Fetched {} messages from channel {channel_id}",
            messages.len()
        );

        // Update cursor to oldest message in batch (last element, since messages are newest-first)
        if let Some(oldest) = messages.last() {
            cursor = Some(oldest.id);
        }

        // Check if we got a partial batch (indicates end of channel history)
        let batch_size = messages.len();
        if batch_size < MAX_MESSAGES_PER_FETCH as usize {
            reached_end = true;
        }

        // Filter expired messages and add to collection
        let batch_expired = filter_expired_messages(messages, retention_days);
        debug!("Found {} expired messages in batch", batch_expired.len());
        expired_messages.extend(batch_expired);

        // Check if we've collected enough
        if expired_messages.len() >= TARGET_EXPIRED_MESSAGES {
            expired_messages.truncate(TARGET_EXPIRED_MESSAGES);
            debug!(
                "Reached target of {} expired messages",
                TARGET_EXPIRED_MESSAGES
            );

            // Update cursor to oldest message in truncated batch
            if let Some(oldest) = expired_messages.last() {
                cursor = Some(oldest.id);
            }

            break;
        }

        if reached_end {
            break;
        }
    }

    if expired_messages.is_empty() {
        info!("No expired messages in channel {channel_id}");
    } else {
        info!(
            "Found {} expired messages in channel {channel_id}",
            expired_messages.len()
        );

        // Classify into delete vs backup jobs
        let classified = classify_messages(expired_messages);
        info!(
            "Classified: {} delete jobs, {} backup jobs",
            classified.delete_jobs.len(),
            classified.backup_jobs.len()
        );

        if cancel_token.is_cancelled() {
            info!("Cleanup cancelled for channel {channel_id}");
            return Ok(());
        }

        // Process delete jobs (non-media messages)
        if !classified.delete_jobs.is_empty() {
            delete_messages(&http, channel_id, &classified.delete_jobs, &cancel_token).await?;
        }

        if cancel_token.is_cancelled() {
            info!("Cleanup cancelled for channel {channel_id}");
            return Ok(());
        }

        // Process backup jobs (media messages)
        if !classified.backup_jobs.is_empty() {
            let download_dir = config.media_backup_config().download_dir;

            process_backup_jobs(
                &http,
                channel_id,
                download_dir,
                &backup_queue,
                &classified.backup_jobs,
                &cancel_token,
            )
            .await?;
        }
    }

    if reached_end {
        debug!("Reached end of channel history, clearing pagination cursor");
        config.set_pagination_cursor(channel_id, None)?;
    } else {
        debug!("Saving pagination cursor: {:?}", cursor);
        config.set_pagination_cursor(channel_id, cursor.map(|c| c.get()))?;
    }

    info!("Cleanup completed for channel {channel_id}");

    Ok(())
}

/// Delete non-media messages with rate limiting.
pub(crate) async fn delete_messages(
    http: &Http,
    channel_id: ChannelId,
    jobs: &[DeleteJob],
    cancel_token: &CancellationToken,
) -> Result<()> {
    let bulk_delete_cutoff: Timestamp = Timestamp::now()
        .checked_sub_days(BULK_DELETE_THRESHOLD)
        .context("can't compute bulk delete cutoff")?
        .into();
    let (mut bulk_jobs, mut individual_jobs): (Vec<_>, Vec<_>) = jobs.iter().partition(|j|
        // We can bulk delete messages newer than the cutoff
        j.message_id.created_at() > bulk_delete_cutoff);

    if bulk_jobs.len() < BULK_DELETE_MIN {
        individual_jobs.append(&mut bulk_jobs);
    }

    if !bulk_jobs.is_empty() {
        let chunks: Vec<_> = bulk_jobs.chunks(BULK_DELETE_MAX).collect();

        for chunk in chunks {
            if cancel_token.is_cancelled() {
                return Ok(());
            }

            if let Err(e) = channel_id
                .delete_messages(http, chunk.iter().map(|f| f.message_id))
                .await
            {
                warn!("Bulk delete failed: {e:?}",);
            } else {
                info!(
                    "Bulk deleted {} messages from channel {channel_id}",
                    chunk.len(),
                );
            }

            sleep(BULK_DELETE_DELAY).await;
        }
    }

    if !individual_jobs.is_empty() {
        for job in jobs {
            if cancel_token.is_cancelled() {
                return Ok(());
            }

            if let Err(e) = channel_id.delete_message(http, job.message_id).await {
                error!("Failed to delete message {}: {e:?}", job.message_id);
            } else {
                debug!("Deleted message {}", job.message_id);
            }

            sleep(SINGLE_DELETE_DELAY).await;
        }
    }

    Ok(())
}

/// Process backup jobs: download media locally, add to backup queue, then delete Discord message.
async fn process_backup_jobs(
    http: &Http,
    channel_id: ChannelId,
    download_dir: std::path::PathBuf,
    backup_queue: &Mutex<BackupQueue>,
    jobs: &[BackupJob],
    cancel_token: &CancellationToken,
) -> Result<()> {
    let downloader = MediaDownloader::new(download_dir);

    for job in jobs {
        if cancel_token.is_cancelled() {
            return Ok(());
        }

        info!(
            "Processing media backup for message {} ({} attachments)",
            job.message_id,
            job.attachments.len()
        );

        let results = match downloader
            .download_attachments(job.message_id, job.timestamp, &job.attachments)
            .await
        {
            Ok(results) => {
                info!(
                    "Downloaded {} files for message {}",
                    results.len(),
                    job.message_id
                );
                results
            }
            Err(e) => {
                error!(
                    "Failed to download media for message {}: {e:?}",
                    job.message_id
                );
                // Don't delete the message if download failed
                continue;
            }
        };

        {
            let mut queue = backup_queue.lock().unwrap();
            for result in &results {
                let pending = PendingBackup {
                    message_id: job.message_id.get(),
                    channel_id: channel_id.get(),
                    local_path: result.local_path.clone(),
                    original_filename: result.filename.clone(),
                    timestamp: job.timestamp,
                    retry_count: 0,
                    status: BackupStatus::Pending,
                };
                if let Err(e) = queue.add(pending) {
                    error!(
                        "Failed to add backup to queue for {}: {e:?}",
                        result.local_path.display()
                    );
                    // Don't delete the message if we can't track it
                    continue;
                }
            }
        }

        // NOW it's safe to delete Discord message
        if let Err(e) = channel_id.delete_message(http, job.message_id).await {
            error!(
                "Failed to delete message {} after backup: {e:?}",
                job.message_id
            );
            // Message stays in Discord, but files are queued for backup
            // This is acceptable - the message might get re-processed next run
        } else {
            info!("Deleted message {} after successful backup", job.message_id);
        }

        // Rate limit between message deletions
        sleep(SINGLE_DELETE_DELAY).await;
    }

    Ok(())
}
