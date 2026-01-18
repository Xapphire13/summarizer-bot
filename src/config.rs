use std::env;

use anyhow::{Context, Result, anyhow};

pub struct Config {
    pub discord_token: String,
    pub llm_model: String,
    pub llm_host: String,
    pub llm_port: u16,
    pub message_length_min: usize,
    pub message_length_max: usize,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let config = Self {
            discord_token: env::var("DISCORD_TOKEN")
                .context("Expected DISCORD_TOKEN in environment")?,
            llm_model: env::var("LLM_MODEL").context("Expected LLM_MODEL in environment")?,
            llm_host: env::var("LLM_HOST").context("Expected LLM_HOST in environment")?,
            llm_port: env::var("LLM_PORT")
                .context("Expected LLM_PORT in environment")?
                .parse()
                .context("LLM_PORT must be a valid port number")?,
            message_length_min: env::var("MESSAGE_LENGTH_MIN")
                .context("Expected MESSAGE_LENGTH_MIN in environment")?
                .parse()
                .context("MESSAGE_LENGTH_MIN must be a valid number")?,
            message_length_max: env::var("MESSAGE_LENGTH_MAX")
                .context("Expected MESSAGE_LENGTH_MAX in environment")?
                .parse()
                .context("MESSAGE_LENGTH_MAX must be a valid number")?,
        };

        if config.message_length_min > config.message_length_max {
            return Err(anyhow!("MESSAGE_LENGTH_MIN must be <= MESSAGE_LENGTH_MAX"));
        }

        Ok(config)
    }
}
