use anyhow::Result;
use serde::{Deserialize, Serialize};
use serenity::all::ChannelId;
use std::{collections::HashMap, fs};

const CONFIG_PATH: &'static str = "./config.toml";

#[derive(Serialize, Deserialize, Debug)]
pub struct ChannelConfig {
    pub name: String,
    /// Override for the global retention policy
    pub policy_days: Option<u32>,
}

impl ChannelConfig {
    pub fn resolve_policy_days(&self, config: &Config) -> u32 {
        self.policy_days
            .unwrap_or(config.retention.default_policy_days)
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct RetentionConfig {
    default_policy_days: u32,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Config {
    retention: RetentionConfig,
    #[serde(default)]
    channels: HashMap<ChannelId, ChannelConfig>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let config = if let Ok(bytes) = fs::read(CONFIG_PATH) {
            toml::from_slice(bytes.as_slice())?
        } else {
            Self::default()
        };

        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        fs::write(CONFIG_PATH, toml::to_string_pretty(&self)?)?;

        Ok(())
    }

    pub fn add_channel_config(
        &mut self,
        channel_id: ChannelId,
        config: ChannelConfig,
    ) -> Result<()> {
        self.channels.insert(channel_id, config);
        self.save()
    }

    pub fn remove_channel(&mut self, channel_id: ChannelId) -> Result<()> {
        self.channels.remove(&channel_id);
        self.save()
    }
}
