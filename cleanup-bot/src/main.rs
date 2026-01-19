use ::tracing::error;
use anyhow::{Context, Result};
use serenity::{Client, all::GatewayIntents};

#[tokio::main]
async fn main() -> Result<()> {
    shared::init_tracing!()?;

    let config = shared::load_bot_config!()?;

    let intents = GatewayIntents::MESSAGE_CONTENT | GatewayIntents::GUILD_MESSAGES;

    let mut client = Client::builder(&config.discord_token, intents)
        // .event_handler(handler)
        .await
        .context("Error creating client")?;

    if let Err(why) = client.start().await {
        error!("Client error: {:?}", why);
    }

    Ok(())
}
