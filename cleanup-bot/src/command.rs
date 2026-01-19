use std::sync::{Arc, Mutex};

use anyhow::{Error, Result};
use indoc::formatdoc;
use serenity::all::Mentionable;

use crate::config::{ChannelConfig, Config};

pub struct UserData {
    pub config: Arc<Mutex<Config>>,
}

type Context<'a> = poise::Context<'a, UserData, Error>;

#[poise::command(slash_command, subcommands("enable", "disable"))]
pub async fn cleanup(_ctx: Context<'_>) -> Result<()> {
    Ok(())
}

#[poise::command(slash_command)]
pub async fn enable(
    ctx: Context<'_>,
    #[description = "How many days should messages be retained"] policy_days: Option<u32>,
) -> Result<()> {
    let channel_config = ChannelConfig {
        name: ctx.channel_id().name(&ctx.http()).await?,
        policy_days,
    };

    let policy_days = {
        let mut config = ctx.data().config.lock().unwrap();
        let policy_days = channel_config.resolve_policy_days(&config);
        config.add_channel_config(ctx.channel_id(), channel_config)?;

        policy_days
    };

    ctx.say(formatdoc! {"
        Enabled cleanup for {channel}
        Retention policy: **{policy_days} {day_suffix}**
        ",
        channel = ctx.channel_id().mention(),
        day_suffix = if policy_days == 1 {"day"}  else {"days"}
    })
    .await?;
    Ok(())
}

#[poise::command(slash_command)]
pub async fn disable(ctx: Context<'_>) -> Result<()> {
    {
        let mut config = ctx.data().config.lock().unwrap();
        config.remove_channel(ctx.channel_id())?;
    };

    ctx.say(format!(
        "Disabled cleanup for {channel}",
        channel = ctx.channel_id().mention()
    ))
    .await?;
    Ok(())
}
