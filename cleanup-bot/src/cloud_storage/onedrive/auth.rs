use std::fs;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::cloud_storage::CloudStorageError;

const TOKENS_PATH: &str = "./onedrive_tokens.toml";
const AUTH_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0";
const SCOPES: &str = "Files.ReadWrite offline_access";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StoredTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
    error_description: Option<String>,
}

pub struct TokenStore {
    client_id: String,
    http: Client,
    tokens: Option<StoredTokens>,
}

impl TokenStore {
    pub fn new(client_id: String) -> Self {
        Self {
            client_id,
            http: Client::new(),
            tokens: Self::load_tokens(),
        }
    }

    fn load_tokens() -> Option<StoredTokens> {
        let path = Path::new(TOKENS_PATH);

        match fs::read_to_string(path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(tokens) => Some(tokens),
                Err(e) => {
                    warn!("Failed to parse tokens file: {e}");
                    None
                }
            },
            Err(e) => {
                warn!("Failed to read tokens file: {e}");
                None
            }
        }
    }

    fn save_tokens(&self) -> Result<(), CloudStorageError> {
        let Some(tokens) = &self.tokens else {
            return Ok(());
        };

        let content = toml::to_string_pretty(tokens)
            .map_err(|e| CloudStorageError::TokenStorage(e.to_string()))?;

        fs::write(TOKENS_PATH, content)?;
        Ok(())
    }

    pub fn has_tokens(&self) -> bool {
        self.tokens.is_some()
    }

    /// Get a valid access token, refreshing if necessary.
    pub async fn get_valid_token(&mut self) -> Result<String, CloudStorageError> {
        let Some(tokens) = &self.tokens else {
            return Err(CloudStorageError::Auth("No tokens available".to_string()));
        };

        // Check if token is expired (with 5 minute buffer)
        let now = Utc::now();
        let buffer = chrono::Duration::minutes(5);

        if tokens.expires_at - buffer <= now {
            debug!("Access token expired, refreshing...");
            self.refresh_token().await?;
        }

        Ok(self.tokens.as_ref().unwrap().access_token.clone())
    }

    /// Perform device code flow for initial authentication.
    pub async fn device_code_flow(&mut self) -> Result<(), CloudStorageError> {
        // Request device code
        let resp = self
            .http
            .post(format!("{AUTH_URL}/devicecode"))
            .form(&[
                ("client_id", &self.client_id),
                ("scope", &SCOPES.to_string()),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let error: ErrorResponse = resp.json().await?;
            return Err(CloudStorageError::Auth(
                error.error_description.unwrap_or(error.error),
            ));
        }

        let device_code: DeviceCodeResponse = resp.json().await?;

        info!(
            "To authenticate OneDrive, visit {} and enter code: {}",
            device_code.verification_uri, device_code.user_code
        );

        // Poll for token
        let poll_interval = Duration::from_secs(device_code.interval);
        let deadline = std::time::Instant::now() + Duration::from_secs(device_code.expires_in);

        loop {
            if std::time::Instant::now() > deadline {
                return Err(CloudStorageError::Auth("Device code expired".to_string()));
            }

            tokio::time::sleep(poll_interval).await;

            let resp = self
                .http
                .post(format!("{AUTH_URL}/token"))
                .form(&[
                    ("client_id", self.client_id.as_str()),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", &device_code.device_code),
                ])
                .send()
                .await?;

            if resp.status().is_success() {
                let token_resp: TokenResponse = resp.json().await?;
                self.tokens = Some(StoredTokens {
                    access_token: token_resp.access_token,
                    refresh_token: token_resp.refresh_token,
                    expires_at: Utc::now() + chrono::Duration::seconds(token_resp.expires_in),
                });
                self.save_tokens()?;
                info!("OneDrive authentication successful");
                return Ok(());
            }

            let error: ErrorResponse = resp.json().await?;
            match error.error.as_str() {
                "authorization_pending" => {
                    debug!("Waiting for user authorization...");
                    continue;
                }
                "slow_down" => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
                _ => {
                    return Err(CloudStorageError::Auth(
                        error.error_description.unwrap_or(error.error),
                    ));
                }
            }
        }
    }

    /// Refresh the access token using the refresh token.
    async fn refresh_token(&mut self) -> Result<(), CloudStorageError> {
        let refresh_token = self
            .tokens
            .as_ref()
            .map(|t| t.refresh_token.clone())
            .ok_or_else(|| CloudStorageError::Auth("No refresh token available".to_string()))?;

        let resp = self
            .http
            .post(format!("{AUTH_URL}/token"))
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh_token),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let error: ErrorResponse = resp.json().await?;
            return Err(CloudStorageError::Auth(
                error.error_description.unwrap_or(error.error),
            ));
        }

        let token_resp: TokenResponse = resp.json().await?;
        self.tokens = Some(StoredTokens {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token,
            expires_at: Utc::now() + chrono::Duration::seconds(token_resp.expires_in),
        });
        self.save_tokens()?;
        debug!("Access token refreshed successfully");

        Ok(())
    }
}
