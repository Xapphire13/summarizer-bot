use anyhow::{Context, Result};
use serenity::all::{ChannelId, GetMessages, Http, MessageId, UserId};
use tracing::debug;

const MAX_MESSAGES_PER_FETCH: u8 = 100;
// Manual, on-demand scan: bound the number of API calls so a very old channel
// can't page forever, but scan much deeper than the scheduled cleanup does.
const MAX_SCAN_ROUNDS: usize = 500;

/// Scan a channel's message history for messages authored by the given user.
pub async fn find_messages_by_user(
    http: &Http,
    channel_id: ChannelId,
    user_id: UserId,
) -> Result<Vec<MessageId>> {
    let mut cursor: Option<MessageId> = None;
    let mut matches = Vec::new();

    for _ in 0..MAX_SCAN_ROUNDS {
        let request = match cursor {
            Some(before_id) => GetMessages::new()
                .limit(MAX_MESSAGES_PER_FETCH)
                .before(before_id),
            None => GetMessages::new().limit(MAX_MESSAGES_PER_FETCH),
        };

        let messages = channel_id
            .messages(http, request)
            .await
            .context("Failed to fetch messages")?;

        if messages.is_empty() {
            break;
        }

        let batch_size = messages.len();
        if let Some(oldest) = messages.last() {
            cursor = Some(oldest.id);
        }

        matches.extend(
            messages
                .into_iter()
                .filter(|m| m.author.id == user_id)
                .map(|m| m.id),
        );

        if batch_size < MAX_MESSAGES_PER_FETCH as usize {
            break;
        }
    }

    debug!(
        "Found {} messages from user {user_id} in channel {channel_id}",
        matches.len()
    );

    Ok(matches)
}
