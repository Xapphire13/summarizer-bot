use std::sync::Arc;

use ::tracing::error;
use anyhow::{Context, Result};
use poise::samples::register_in_guild;
use serenity::{Client, all::GatewayIntents};
use tracing::info;

use crate::{
    command::{UserData, cleanup},
    config::Config,
};

mod command;
mod config;

#[tokio::main]
async fn main() -> Result<()> {
    shared::init_tracing!()?;
    let bot_config = shared::load_bot_config!()?;
    let config = Arc::from(Config::load()?);
    let intents = GatewayIntents::MESSAGE_CONTENT | GatewayIntents::GUILD_MESSAGES;

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![cleanup()],
            ..Default::default()
        })
        .setup(|ctx, ready, framework| {
            Box::pin(async move {
                info!("Connected!");

                for guild_id in &ready.guilds {
                    register_in_guild(ctx, &framework.options().commands, guild_id.id).await?;
                }

                Ok(UserData { config })
            })
        })
        .build();

    let mut client = Client::builder(&bot_config.discord_token, intents)
        .framework(framework)
        .await
        .context("Error creating client")?;

    if let Err(why) = client.start().await {
        error!("Client error: {:?}", why);
    }

    Ok(())
}
