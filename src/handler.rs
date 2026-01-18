use serenity::{
    all::{EditMessage, EventHandler, Mentionable, Message, Ready},
    async_trait,
};
use tracing::{error, info};

use crate::{config::Config, llm::SummaryGenerator};

#[derive(Debug)]
pub struct Handler {
    summary_generator: SummaryGenerator,
    // Messages at least this long are summarized
    message_length_min: usize,
    // Messages longer than this are not summarized
    message_length_max: usize,
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: serenity::client::Context, msg: Message) {
        // Ignore bot messages to prevent loops
        if msg.author.bot {
            return;
        }

        let is_dm = msg.guild_id.is_none();

        if (msg.content.len() >= self.message_length_min
            && msg.content.len() <= self.message_length_max)
            || is_dm
        {
            if is_dm {
                info!(
                    "Summarizing direct message from {}",
                    msg.author.display_name()
                )
            } else {
                info!(
                    "Summarizing message in {} from {}",
                    msg.channel_id
                        .name(&ctx.http)
                        .await
                        .unwrap_or("unknown channel".to_string()),
                    msg.author.display_name()
                )
            }

            let mut response = match msg
                .channel_id
                .say(
                    &ctx.http,
                    format!(
                        ":hourglass: Summarizing message from {}",
                        msg.author.mention()
                    ),
                )
                .await
            {
                Ok(msg) => msg,
                Err(why) => {
                    error!("Error sending initial message: {why:?}");
                    return;
                }
            };

            let summary = match self
                .summary_generator
                .generate_summary(msg.author.display_name(), &msg.content)
                .await
            {
                Ok(summary) => summary,
                Err(why) => {
                    error!("Error summarizing message: {why:?}");

                    if let Err(why) = response.delete(&ctx.http).await {
                        error!("Error deleting initial message: {:?}", why);
                    }

                    return;
                }
            };

            if let Err(why) = response
                .edit(&ctx.http, EditMessage::new().content(summary))
                .await
            {
                error!("Error sending message: {:?}", why);
            }
        }
    }

    async fn ready(&self, _: serenity::client::Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);
    }
}

impl Handler {
    pub fn new(summary_generator: SummaryGenerator, config: &Config) -> Self {
        Handler {
            summary_generator,
            message_length_min: config.message_length_min,
            message_length_max: config.message_length_max,
        }
    }
}
