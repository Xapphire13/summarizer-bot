use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use poise::samples::register_in_guild;
use serenity::{Client, all::GatewayIntents};
use tokio::sync::Mutex as TokioMutex;
use tracing::{error, info};

use crate::{
    backup::BackupQueue,
    cancellation::CancellationRegistry,
    cleanup::spawn_worker,
    cloud_storage::{CloudStorage, OneDriveClient, ProtonDriveClient, TokenStore},
    command::{CommandData, cleanup},
    config::{CloudStorageBackendConfig, Config, ConfigStore},
};

mod backup;
mod cancellation;
mod cleanup;
mod cloud_storage;
mod command;
mod config;
mod media;

#[tokio::main]
async fn main() -> Result<()> {
    shared::init_tracing!()?;
    let bot_config = shared::load_bot_config!()?;
    let config = Config::load()?;
    let backup_worker_config = config.media_backup.worker.clone();
    let cloud_backup_config = config.cloud_backup.clone();
    let config_store = ConfigStore::new(config);
    let backup_queue = Arc::new(Mutex::new(BackupQueue::load()?));
    let cancellation = Arc::new(Mutex::new(CancellationRegistry::new()));
    let intents = GatewayIntents::MESSAGE_CONTENT | GatewayIntents::GUILD_MESSAGES;

    // Initialize the cloud storage client if a backend is configured
    let cloud_storage: Option<Arc<dyn CloudStorage>> = match cloud_backup_config {
        Some(cloud_config) => match cloud_config.backend {
            CloudStorageBackendConfig::OneDrive { client_id } => {
                let token_store = Arc::new(TokioMutex::new(TokenStore::new(client_id)));

                // Check if we need to authenticate
                if !token_store.lock().await.has_tokens() {
                    info!("OneDrive tokens not found, starting device code flow...");
                    token_store.lock().await.device_code_flow().await?;
                }

                Some(Arc::new(OneDriveClient::new(
                    token_store,
                    cloud_config.upload_folder,
                )))
            }
            CloudStorageBackendConfig::ProtonDrive => {
                Some(Arc::new(ProtonDriveClient::new(cloud_config.upload_folder)))
            }
        },
        None => {
            info!("Cloud storage not configured, backups will be stored locally only");
            None
        }
    };

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![cleanup()],
            ..Default::default()
        })
        .setup({
            let config_store = config_store.clone();
            let cancellation = Arc::clone(&cancellation);

            move |ctx, ready, framework| {
                let http = Arc::clone(&ctx.http);

                Box::pin(async move {
                    info!("Connected!");

                    for guild_id in &ready.guilds {
                        register_in_guild(ctx, &framework.options().commands, guild_id.id).await?;
                    }

                    // Spawn the backup worker (only if we have somewhere to back up to)
                    if let Some(cloud_storage) = cloud_storage {
                        backup::spawn_worker(
                            Arc::clone(&backup_queue),
                            backup_worker_config,
                            cloud_storage,
                        );
                    }

                    // Spawn the cleanup scheduler
                    spawn_worker(
                        Arc::clone(&http),
                        config_store.clone(),
                        backup_queue,
                        Arc::clone(&cancellation),
                    );

                    Ok(CommandData {
                        config: config_store,
                        cancellation,
                    })
                })
            }
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
