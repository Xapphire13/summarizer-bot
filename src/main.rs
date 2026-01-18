use ::tracing::error;
use anyhow::{Context, Result};
use serenity::prelude::*;

use crate::config::Config;
use crate::handler::Handler;
use crate::llm::SummaryGenerator;

mod config;
mod handler;
mod llm;
mod tracing;

#[tokio::main]
async fn main() -> Result<()> {
    tracing::init();

    #[cfg(debug_assertions)]
    dotenvy::dotenv()?;

    let config = Config::from_env()?;

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::DIRECT_MESSAGES;

    let summary_generator = SummaryGenerator::new(&config);
    let handler = Handler::new(summary_generator, &config);

    let mut client = Client::builder(&config.discord_token, intents)
        .event_handler(handler)
        .await
        .context("Error creating client")?;

    if let Err(why) = client.start().await {
        error!("Client error: {:?}", why);
    }

    Ok(())
}
