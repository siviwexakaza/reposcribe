use std::env;

use reposcribe_core::AiProvider;
use reqwest::{Client, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use thiserror::Error;

const OPENAI_API_BASE: &str = "https://api.openai.com";
const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderModel {
    pub id: String,
    pub display_name: String,
}

impl std::fmt::Display for ProviderModel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.display_name == self.id {
            formatter.write_str(&self.id)
        } else {
            write!(formatter, "{} ({})", self.display_name, self.id)
        }
    }
}

#[derive(Clone)]
pub struct AiClient {
    provider: AiProvider,
    api_key: SecretString,
    client: Client,
    base_url: String,
}

impl AiClient {
    pub fn from_environment(provider: AiProvider) -> Result<Self, AiError> {
        let variable = provider.api_key_environment_variable();
        let value = env::var(variable).map_err(|_| AiError::MissingApiKey {
            provider,
            environment_variable: variable,
        })?;
        if value.trim().is_empty() {
            return Err(AiError::MissingApiKey {
                provider,
                environment_variable: variable,
            });
        }
        Ok(Self::new(provider, SecretString::from(value)))
    }

    pub fn new(provider: AiProvider, api_key: SecretString) -> Self {
        let base_url = match provider {
            AiProvider::Anthropic => ANTHROPIC_API_BASE,
            AiProvider::OpenAi => OPENAI_API_BASE,
        };
        Self {
            provider,
            api_key,
            client: Client::new(),
            base_url: base_url.to_owned(),
        }
    }

    /// Override the endpoint for deterministic tests and compatible test servers.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
        self
    }

    pub async fn list_models(&self) -> Result<Vec<ProviderModel>, AiError> {
        match self.provider {
            AiProvider::Anthropic => self.list_anthropic_models().await,
            AiProvider::OpenAi => self.list_openai_models().await,
        }
    }

    async fn list_openai_models(&self) -> Result<Vec<ProviderModel>, AiError> {
        let response = self
            .client
            .get(format!("{}/v1/models", self.base_url))
            .bearer_auth(self.api_key.expose_secret())
            .send()
            .await?;
        let response = ensure_success(AiProvider::OpenAi, response).await?;
        let body: OpenAiModelsResponse = response.json().await?;
        let models = body
            .data
            .into_iter()
            .filter(|model| is_openai_text_model(&model.id))
            .map(|model| ProviderModel {
                display_name: model.id.clone(),
                id: model.id,
            })
            .collect();
        normalize_models(models, AiProvider::OpenAi)
    }

    async fn list_anthropic_models(&self) -> Result<Vec<ProviderModel>, AiError> {
        let response = self
            .client
            .get(format!("{}/v1/models?limit=1000", self.base_url))
            .header("x-api-key", self.api_key.expose_secret())
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .send()
            .await?;
        let response = ensure_success(AiProvider::Anthropic, response).await?;
        let body: AnthropicModelsResponse = response.json().await?;
        let models = body
            .data
            .into_iter()
            .map(|model| ProviderModel {
                id: model.id.clone(),
                display_name: model.display_name.unwrap_or(model.id),
            })
            .collect();
        normalize_models(models, AiProvider::Anthropic)
    }
}

fn normalize_models(
    mut models: Vec<ProviderModel>,
    provider: AiProvider,
) -> Result<Vec<ProviderModel>, AiError> {
    models.sort_by(|left, right| left.id.cmp(&right.id));
    models.dedup_by(|left, right| left.id == right.id);
    if models.is_empty() {
        return Err(AiError::NoCompatibleModels(provider));
    }
    Ok(models)
}

fn is_openai_text_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    let likely_text = id.starts_with("gpt-")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4");
    let excluded = [
        "audio",
        "embedding",
        "image",
        "moderation",
        "realtime",
        "search",
        "transcribe",
        "tts",
    ];
    likely_text && !excluded.iter().any(|value| id.contains(value))
}

async fn ensure_success(
    provider: AiProvider,
    response: reqwest::Response,
) -> Result<reqwest::Response, AiError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let message = response
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(500)
        .collect();
    Err(AiError::ProviderResponse {
        provider,
        status,
        message,
    })
}

#[derive(Debug, Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicModelsResponse {
    data: Vec<AnthropicModel>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModel {
    id: String,
    display_name: Option<String>,
}

#[derive(Debug, Error)]
pub enum AiError {
    #[error("{provider} API key was not found in {environment_variable}")]
    MissingApiKey {
        provider: AiProvider,
        environment_variable: &'static str,
    },
    #[error("could not reach the AI provider: {0}")]
    Request(#[from] reqwest::Error),
    #[error("{provider} returned HTTP {status}: {message}")]
    ProviderResponse {
        provider: AiProvider,
        status: StatusCode,
        message: String,
    },
    #[error("{0} did not return any compatible text-generation models")]
    NoCompatibleModels(AiProvider),
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

    use super::*;

    #[tokio::test]
    async fn lists_and_filters_openai_models() {
        let server = MockServer::start().await;
        Mock::given(matchers::method("GET"))
            .and(matchers::path("/v1/models"))
            .and(matchers::header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "text-embedding-3-small"},
                    {"id": "gpt-5-example"},
                    {"id": "gpt-audio-example"},
                    {"id": "o3-example"}
                ]
            })))
            .mount(&server)
            .await;

        let client = AiClient::new(
            AiProvider::OpenAi,
            SecretString::from("test-key".to_owned()),
        )
        .with_base_url(server.uri());
        let models = client.list_models().await.unwrap();

        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["gpt-5-example", "o3-example"]
        );
    }

    #[tokio::test]
    async fn lists_anthropic_models_with_required_headers() {
        let server = MockServer::start().await;
        Mock::given(matchers::method("GET"))
            .and(matchers::path("/v1/models"))
            .and(matchers::query_param("limit", "1000"))
            .and(matchers::header("x-api-key", "test-key"))
            .and(matchers::header("anthropic-version", ANTHROPIC_API_VERSION))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "claude-example", "display_name": "Claude Example"}
                ],
                "has_more": false,
                "first_id": "claude-example",
                "last_id": "claude-example"
            })))
            .mount(&server)
            .await;

        let client = AiClient::new(
            AiProvider::Anthropic,
            SecretString::from("test-key".to_owned()),
        )
        .with_base_url(server.uri());
        let models = client.list_models().await.unwrap();

        assert_eq!(models[0].id, "claude-example");
        assert_eq!(models[0].display_name, "Claude Example");
    }
}
