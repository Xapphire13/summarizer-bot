use std::time::Duration;

use anyhow::{Context, Result};
use ollama_rs::{Ollama, generation::completion::request::GenerationRequest};
use tokio::time::timeout;
use tracing::instrument;

use crate::config::Config;

const LLM_TIMEOUT: Duration = Duration::from_mins(10);

#[derive(Debug)]
pub struct SummaryGenerator {
    ollama_client: Ollama,
    llm_model: String,
}

impl SummaryGenerator {
    pub fn new(config: &Config) -> Self {
        Self {
            llm_model: config.llm_model.clone(),
            ollama_client: Ollama::new(&config.llm_host, config.llm_port),
        }
    }

    #[instrument(level = "trace", skip_all)]
    pub async fn generate_summary(&self, author: &str, content: &str) -> Result<String> {
        let result = timeout(
            LLM_TIMEOUT,
            self.ollama_client.generate(
                GenerationRequest::new(
                    self.llm_model.clone(),
                    format!("Author: {author}\nMessage: {content}"),
                )
                .system(include_str!("../system_prompt.txt")),
            ),
        )
        .await
        .context("LLM request timed out")?
        .context("LLM generation failed")?;

        Ok(result.response)
    }
}
