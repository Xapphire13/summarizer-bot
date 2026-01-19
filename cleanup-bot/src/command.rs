use std::sync::Arc;

use anyhow::{Error, Result};
use indoc::formatdoc;
use serenity::all::Mentionable;

use crate::config::{ChannelConfig, Config};

pub struct UserData {
    pub config: Arc<Config>,
}

type Context<'a> = poise::Context<'a, UserData, Error>;

#[poise::command(slash_command, subcommands("enable", "disable"))]
pub async fn cleanup(_ctx: Context<'_>) -> Result<()> {
    Ok(())
}

#[poise::command(slash_command)]
pub async fn enable(ctx: Context<'_>) -> Result<(), Error> {
    let channel_config = ChannelConfig {
        name: ctx.channel_id().name(&ctx.http()).await?,
        policy_days: None,
    };

    ctx.say(formatdoc! {"
        Enabled cleanup for {channel}
        Retention policy: **{policy} days**
        ",
        channel = ctx.channel_id().mention(),
        policy = channel_config.resolve_policy_days(&ctx.data().config)
    })
    .await?;
    Ok(())
}

#[poise::command(slash_command)]
pub async fn disable(ctx: Context<'_>) -> Result<(), Error> {
    ctx.say(format!(
        "Disable command for {}",
        ctx.channel_id().mention()
    ))
    .await?;
    Ok(())
}
