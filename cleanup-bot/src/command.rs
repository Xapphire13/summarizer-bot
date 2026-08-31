use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Error, Result};
use indoc::formatdoc;
use serenity::all::{
    ButtonStyle, ComponentInteractionCollector, CreateActionRow, CreateButton,
    CreateInteractionResponse, CreateInteractionResponseMessage, EditInteractionResponse,
    Mentionable, UserId, parse_user_mention,
};

use crate::cancellation::CancellationRegistry;
use crate::cleanup::purge::find_messages_by_user;
use crate::cleanup::queue::DeleteJob;
use crate::cleanup::task::delete_messages;
use crate::config::{ChannelConfig, ConfigStore};

const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(60);

pub struct CommandData {
    pub config: ConfigStore,
    pub cancellation: Arc<Mutex<CancellationRegistry>>,
}

type Context<'a> = poise::Context<'a, CommandData, Error>;

/// Parse a user ID from either a raw numeric ID or a `<@id>`/`<@!id>` mention,
/// so users who've left the server (and thus can't be picked via Discord's
/// user-option autocomplete) can still be targeted.
fn parse_user_id(input: &str) -> Option<UserId> {
    let trimmed = input.trim();
    parse_user_mention(trimmed).or_else(|| trimmed.parse().ok())
}

#[poise::command(slash_command, subcommands("enable", "disable", "purge_user"))]
pub async fn cleanup(_ctx: Context<'_>) -> Result<()> {
    Ok(())
}

#[poise::command(slash_command)]
pub async fn enable(
    ctx: Context<'_>,
    #[description = "How many days should messages be retained"]
    #[min = 1]
    policy_days: Option<NonZeroU32>,
) -> Result<()> {
    let channel_config = ChannelConfig {
        name: ctx.channel_id().name(&ctx.http()).await?,
        policy_days,
        pagination_cursor: None,
    };

    let policy_days = ctx
        .data()
        .config
        .add_channel(ctx.channel_id(), channel_config)?;

    ctx.say(formatdoc! {"
        Enabled cleanup for {channel}
        Retention policy: **{policy_days} {day_suffix}**
        ",
        channel = ctx.channel_id().mention(),
        day_suffix = if policy_days.get() == 1 {"day"}  else {"days"}
    })
    .await?;
    Ok(())
}

#[poise::command(slash_command)]
pub async fn disable(ctx: Context<'_>) -> Result<()> {
    ctx.data().config.remove_channel(ctx.channel_id())?;

    // Cancel any running cleanup task for the channel
    let was_running = ctx
        .data()
        .cancellation
        .lock()
        .unwrap()
        .cancel(ctx.channel_id());

    let mut message = format!(
        "Disabled cleanup for {channel}",
        channel = ctx.channel_id().mention()
    );

    if was_running {
        message.push_str("\n_Cancelled running cleanup task._");
    }

    ctx.say(message).await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "purge-user",
    required_permissions = "MANAGE_MESSAGES"
)]
pub async fn purge_user(
    ctx: Context<'_>,
    #[description = "User ID or @mention to purge (works even if they've left the server)"]
    user: String,
) -> Result<()> {
    let Some(user_id) = parse_user_id(&user) else {
        ctx.send(
            poise::CreateReply::default()
                .ephemeral(true)
                .content("That doesn't look like a valid user ID or mention."),
        )
        .await?;
        return Ok(());
    };

    ctx.defer_ephemeral().await?;

    let channel_id = ctx.channel_id();
    let message_ids = find_messages_by_user(ctx.http(), channel_id, user_id).await?;

    if message_ids.is_empty() {
        ctx.send(
            poise::CreateReply::default()
                .ephemeral(true)
                .content(format!(
                    "No messages found from {} in this channel.",
                    user_id.mention()
                )),
        )
        .await?;
        return Ok(());
    }

    let ctx_id = ctx.id();
    let confirm_id = format!("purge-user-confirm-{ctx_id}");
    let cancel_id = format!("purge-user-cancel-{ctx_id}");

    let reply = ctx
        .send(
            poise::CreateReply::default()
                .ephemeral(true)
                .content(formatdoc! {"
                    Found **{count}** message(s) from {user} in {channel}.
                    Delete them? This cannot be undone.
                    ",
                    count = message_ids.len(),
                    user = user_id.mention(),
                    channel = channel_id.mention(),
                })
                .components(vec![CreateActionRow::Buttons(vec![
                    CreateButton::new(&confirm_id)
                        .label("Confirm")
                        .style(ButtonStyle::Danger),
                    CreateButton::new(&cancel_id)
                        .label("Cancel")
                        .style(ButtonStyle::Secondary),
                ])]),
        )
        .await?;

    let message_id = reply.message().await?.id;

    let press = ComponentInteractionCollector::new(ctx.serenity_context())
        .author_id(ctx.author().id)
        .channel_id(channel_id)
        .message_id(message_id)
        .timeout(CONFIRMATION_TIMEOUT)
        .await;

    let Some(press) = press else {
        reply
            .edit(
                ctx,
                poise::CreateReply::default()
                    .content("Timed out, no messages were deleted.")
                    .components(vec![]),
            )
            .await?;
        return Ok(());
    };

    if press.data.custom_id == cancel_id {
        press
            .create_response(
                ctx.http(),
                CreateInteractionResponse::UpdateMessage(
                    CreateInteractionResponseMessage::new()
                        .content("Cancelled, no messages were deleted.")
                        .components(vec![]),
                ),
            )
            .await?;
        return Ok(());
    }

    press
        .create_response(
            ctx.http(),
            CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::new()
                    .content(format!("Deleting {} message(s)...", message_ids.len()))
                    .components(vec![]),
            ),
        )
        .await?;

    // Avoid racing a scheduled cleanup task for the same channel
    let cancel_token = {
        let mut registry = ctx.data().cancellation.lock().unwrap();
        if registry.is_running(channel_id) {
            None
        } else {
            Some(registry.register(channel_id))
        }
    };

    let Some(cancel_token) = cancel_token else {
        press
            .edit_response(
                ctx.http(),
                EditInteractionResponse::new().content(
                    "A cleanup task is already running for this channel, try again shortly.",
                ),
            )
            .await?;
        return Ok(());
    };

    let jobs: Vec<DeleteJob> = message_ids
        .into_iter()
        .map(|message_id| DeleteJob { message_id })
        .collect();
    let delete_result = delete_messages(ctx.http(), channel_id, &jobs, &cancel_token).await;

    ctx.data()
        .cancellation
        .lock()
        .unwrap()
        .deregister(channel_id);

    delete_result?;

    press
        .edit_response(
            ctx.http(),
            EditInteractionResponse::new().content(format!(
                "Deleted {} message(s) from {}.",
                jobs.len(),
                user_id.mention()
            )),
        )
        .await?;

    Ok(())
}
