use std::{
    collections::HashMap,
    fs,
    num::NonZeroU32,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serenity::all::ChannelId;

const CONFIG_PATH: &str = "./config.toml";
const CONFIG_TEMP_PATH: &str = "./config.toml.tmp";

fn default_upload_folder() -> String {
    "/discord-backups".to_string()
}

/// Cloud storage backend selection and any auth params it needs.
/// Settings that apply regardless of backend (e.g. `upload_folder`) live on
/// `CloudBackupConfig` instead.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum CloudStorageBackendConfig {
    OneDrive { client_id: String },
    ProtonDrive,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CloudBackupConfig {
    #[serde(default = "default_upload_folder")]
    pub upload_folder: String,
    #[serde(flatten)]
    pub backend: CloudStorageBackendConfig,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ChannelConfig {
    pub name: String,
    /// Override for the global retention policy
    pub policy_days: Option<NonZeroU32>,
    /// Pagination cursor: oldest message ID seen, next run fetches BEFORE this
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination_cursor: Option<u64>,
}

impl ChannelConfig {
    pub fn resolve_policy_days(&self, config: &Config) -> NonZeroU32 {
        self.policy_days
            .unwrap_or(config.retention.default_policy_days)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RetentionConfig {
    pub default_policy_days: NonZeroU32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BackupWorkerConfig {
    #[serde(default = "default_check_interval")]
    pub check_interval_seconds: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_check_interval() -> u64 {
    60
}

fn default_max_retries() -> u32 {
    5
}

impl Default for BackupWorkerConfig {
    fn default() -> Self {
        Self {
            check_interval_seconds: default_check_interval(),
            max_retries: default_max_retries(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MediaBackupConfig {
    pub download_dir: PathBuf,
    #[serde(default)]
    pub worker: BackupWorkerConfig,
}

impl Default for MediaBackupConfig {
    fn default() -> Self {
        Self {
            download_dir: PathBuf::from("./media_backups"),
            worker: BackupWorkerConfig::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub schedule_interval_seconds: NonZeroU32,
    pub retention: RetentionConfig,
    pub media_backup: MediaBackupConfig,
    #[serde(default)]
    pub cloud_backup: Option<CloudBackupConfig>,
    #[serde(default)]
    channels: HashMap<ChannelId, ChannelConfig>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let bytes = fs::read(CONFIG_PATH).context(format!("Error reading {CONFIG_PATH}"))?;
        let config = toml::from_slice(bytes.as_slice())?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let content = toml::to_string_pretty(&self)?;
        fs::write(CONFIG_TEMP_PATH, &content).context("saving temp config file")?;
        fs::rename(CONFIG_TEMP_PATH, CONFIG_PATH).context("updating config file")?;
        Ok(())
    }

    pub fn add_channel_config(
        &mut self,
        channel_id: ChannelId,
        config: ChannelConfig,
    ) -> Result<NonZeroU32> {
        let new_days = config
            .policy_days
            .unwrap_or(self.retention.default_policy_days);

        // Check if policy is becoming stricter (fewer days) - if so, clear pagination cursor
        if let Some(existing) = self.channels.get(&channel_id) {
            let old_days = existing.resolve_policy_days(self);
            if new_days < old_days {
                // Policy is stricter, start fresh from newest messages
                let mut config = config;
                config.pagination_cursor = None;
                self.channels.insert(channel_id, config);
                self.save()?;
                return Ok(new_days);
            }
        }

        self.channels.insert(channel_id, config);
        self.save()?;
        Ok(new_days)
    }

    pub fn get_pagination_cursor(&self, channel_id: ChannelId) -> Option<u64> {
        self.channels
            .get(&channel_id)
            .and_then(|c| c.pagination_cursor)
    }

    pub fn set_pagination_cursor(
        &mut self,
        channel_id: ChannelId,
        cursor: Option<u64>,
    ) -> Result<()> {
        if let Some(config) = self.channels.get_mut(&channel_id) {
            config.pagination_cursor = cursor;
            self.save()?;
        }
        Ok(())
    }

    pub fn remove_channel(&mut self, channel_id: ChannelId) -> Result<()> {
        self.channels.remove(&channel_id);
        self.save()
    }

    /// Returns a list of all enabled channels with their resolved retention policies.
    pub fn enabled_channels(&self) -> Vec<(ChannelId, NonZeroU32)> {
        self.channels
            .iter()
            .map(|(id, config)| (*id, config.resolve_policy_days(self)))
            .collect()
    }
}

/// Thread-safe wrapper around Config for clean state management.
#[derive(Clone)]
pub struct ConfigStore {
    inner: Arc<Mutex<Config>>,
}

impl ConfigStore {
    pub fn new(config: Config) -> Self {
        Self {
            inner: Arc::new(Mutex::new(config)),
        }
    }

    /// Returns the schedule interval in seconds.
    pub fn schedule_interval_seconds(&self) -> NonZeroU32 {
        self.inner.lock().unwrap().schedule_interval_seconds
    }

    /// Returns a list of all enabled channels with their resolved retention policies.
    pub fn enabled_channels(&self) -> Vec<(ChannelId, NonZeroU32)> {
        self.inner.lock().unwrap().enabled_channels()
    }

    /// Returns the media backup configuration.
    pub fn media_backup_config(&self) -> MediaBackupConfig {
        self.inner.lock().unwrap().media_backup.clone()
    }

    /// Adds or updates a channel configuration.
    /// Returns the resolved policy days for the channel.
    pub fn add_channel(&self, channel_id: ChannelId, config: ChannelConfig) -> Result<NonZeroU32> {
        self.inner
            .lock()
            .unwrap()
            .add_channel_config(channel_id, config)
    }

    /// Removes a channel from the configuration.
    pub fn remove_channel(&self, channel_id: ChannelId) -> Result<()> {
        self.inner.lock().unwrap().remove_channel(channel_id)
    }

    /// Gets the pagination cursor for a channel.
    pub fn get_pagination_cursor(&self, channel_id: ChannelId) -> Option<u64> {
        self.inner.lock().unwrap().get_pagination_cursor(channel_id)
    }

    /// Sets the pagination cursor for a channel.
    pub fn set_pagination_cursor(&self, channel_id: ChannelId, cursor: Option<u64>) -> Result<()> {
        self.inner
            .lock()
            .unwrap()
            .set_pagination_cursor(channel_id, cursor)
    }
}
