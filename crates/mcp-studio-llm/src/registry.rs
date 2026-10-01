//! Picks the provider for a model name and keeps the provider settings.
//!
//! The `model` of a flow's LLM step names the provider with a prefix:
//!
//! | model                      | provider                                  |
//! | -------------------------- | ----------------------------------------- |
//! | `claude-sonnet-5-5`        | Anthropic (no prefix means Anthropic)     |
//! | `anthropic:claude-opus-5-5`| Anthropic                                 |
//! | `openai:gpt-4o`            | the OpenAI-compatible endpoint            |
//! | `ollama:llama3.1:8b`       | Ollama (the part after `ollama:` is kept) |

use std::sync::Arc;

use mcp_studio_core::{
    db::{DbError, DbResult},
    llm::{LlmError, LlmErrorKind, LlmProvider, ProviderSettings, OPENAI_KEY_NAME},
    secrets::SecretStore,
    settings::Settings,
};
use serde_json::Value;
use url::Url;

use crate::{AnthropicProvider, OpenAiCompatProvider};

const SETTINGS_KEY: &str = "providers";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Ollama,
}

/// Splits `provider:model` into the provider and the model it is asked for.
pub fn split_model(model: &str) -> (ProviderKind, &str) {
    let model = model.trim();
    match model.split_once(':') {
        Some(("ollama", rest)) => (ProviderKind::Ollama, rest),
        Some(("openai", rest)) => (ProviderKind::OpenAi, rest),
        Some(("anthropic", rest)) => (ProviderKind::Anthropic, rest),
        // Other colons belong to the model name (Ollama tags look like `llama3.1:8b`).
        _ => (ProviderKind::Anthropic, model),
    }
}

/// A provider and the model name to send to it.
pub struct ResolvedModel {
    pub provider: Arc<dyn LlmProvider>,
    pub model: String,
}

/// Creates the provider for `model`. Keys come from the OS keyring.
pub fn resolve(
    settings: &ProviderSettings,
    secrets: &dyn SecretStore,
    model: &str,
) -> Result<ResolvedModel, LlmError> {
    let (kind, name) = split_model(model);
    if name.is_empty() {
        return Err(LlmError::new(
            LlmErrorKind::BadRequest,
            "the model name is empty",
        ));
    }
    let provider: Arc<dyn LlmProvider> = match kind {
        ProviderKind::Anthropic => Arc::new(AnthropicProvider::from_store(secrets)?),
        ProviderKind::OpenAi => {
            let key = secrets
                .get(OPENAI_KEY_NAME)
                .map_err(|e| LlmError::new(LlmErrorKind::Other, e.to_string()))?;
            Arc::new(OpenAiCompatProvider::new(settings.openai_url.clone(), key))
        }
        ProviderKind::Ollama => Arc::new(OpenAiCompatProvider::ollama(settings.ollama_url.clone())),
    };
    Ok(ResolvedModel {
        provider,
        model: name.to_owned(),
    })
}

fn normalize_url(label: &str, value: &str, default: &str) -> DbResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(default.to_owned());
    }
    let url = Url::parse(value)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
        .ok_or_else(|| {
            DbError::Invalid(format!(
                "the {label} address must be a URL such as {default}"
            ))
        })?;
    Ok(url
        .as_str()
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_owned())
}

/// Trims, validates, and fills in defaults for empty addresses.
pub fn normalize_settings(settings: ProviderSettings) -> DbResult<ProviderSettings> {
    let defaults = ProviderSettings::default();
    Ok(ProviderSettings {
        ollama_url: normalize_url("Ollama", &settings.ollama_url, &defaults.ollama_url)?,
        openai_url: normalize_url(
            "OpenAI-compatible",
            &settings.openai_url,
            &defaults.openai_url,
        )?,
    })
}

pub async fn load_settings(store: &Settings) -> DbResult<ProviderSettings> {
    Ok(match store.get(SETTINGS_KEY).await? {
        Some(text) => serde_json::from_str(&text).unwrap_or_default(),
        None => ProviderSettings::default(),
    })
}

pub async fn save_settings(
    store: &Settings,
    settings: ProviderSettings,
) -> DbResult<ProviderSettings> {
    let settings = normalize_settings(settings)?;
    let text = serde_json::to_string(&settings)
        .map_err(|e| DbError::Invalid(format!("could not save the settings: {e}")))?;
    store.set(SETTINGS_KEY, &text).await?;
    Ok(settings)
}

/// The models that are installed in a local Ollama.
pub async fn ollama_models(base_url: &str) -> Result<Vec<String>, LlmError> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let unreachable = |detail: String| {
        LlmError::new(
            LlmErrorKind::Network,
            format!("could not reach Ollama at {base_url}: {detail}. Is Ollama running?"),
        )
    };
    let response = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default()
        .get(url)
        .send()
        .await
        .map_err(|e| unreachable(e.to_string()))?;
    if !response.status().is_success() {
        return Err(LlmError::new(
            LlmErrorKind::Other,
            format!("Ollama at {base_url} answered {}", response.status()),
        ));
    }
    let value: Value = response.json_value().await?;
    let mut names: Vec<String> = value["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["name"].as_str().map(str::to_owned))
        .collect();
    names.sort();
    Ok(names)
}

/// Reads a response body as JSON without needing reqwest's `json` feature.
trait JsonBody {
    async fn json_value(self) -> Result<Value, LlmError>;
}

impl JsonBody for reqwest::Response {
    async fn json_value(self) -> Result<Value, LlmError> {
        let text = self
            .text()
            .await
            .map_err(|e| LlmError::new(LlmErrorKind::Network, e.to_string()))?;
        serde_json::from_str(&text)
            .map_err(|_| LlmError::new(LlmErrorKind::Other, "Ollama's answer was not understood"))
    }
}

#[cfg(test)]
mod tests {
    use axum::{http::StatusCode, routing::get, Router};
    use mcp_studio_core::{db::Db, secrets::MemoryStore, tokens::ANTHROPIC_KEY_NAME};

    use super::*;

    #[test]
    fn the_prefix_picks_the_provider_and_other_colons_stay_in_the_model() {
        assert_eq!(
            split_model("claude-sonnet-5-5"),
            (ProviderKind::Anthropic, "claude-sonnet-5-5")
        );
        assert_eq!(
            split_model("anthropic:claude-opus-5-5"),
            (ProviderKind::Anthropic, "claude-opus-5-5")
        );
        assert_eq!(
            split_model("openai:gpt-4o"),
            (ProviderKind::OpenAi, "gpt-4o")
        );
        assert_eq!(
            split_model("ollama:llama3.1:8b"),
            (ProviderKind::Ollama, "llama3.1:8b")
        );
        assert_eq!(
            split_model("  ollama:qwen3  "),
            (ProviderKind::Ollama, "qwen3")
        );
        // An unknown prefix is part of the model name.
        assert_eq!(
            split_model("llama3.1:8b"),
            (ProviderKind::Anthropic, "llama3.1:8b")
        );
    }

    #[test]
    fn resolving_checks_keys_only_where_they_are_needed() {
        let settings = ProviderSettings::default();
        let secrets = MemoryStore::default();

        let error = resolve(&settings, &secrets, "claude-x").err().unwrap();
        assert_eq!(error.kind, LlmErrorKind::Auth);
        secrets.set(ANTHROPIC_KEY_NAME, "k").unwrap();
        let resolved = resolve(&settings, &secrets, "claude-x").unwrap();
        assert_eq!(
            (resolved.provider.name(), resolved.model.as_str()),
            ("anthropic", "claude-x")
        );

        // OpenAI-compatible endpoints may run without a key.
        let resolved = resolve(&settings, &secrets, "openai:gpt-4o").unwrap();
        assert_eq!(
            (resolved.provider.name(), resolved.model.as_str()),
            ("openai", "gpt-4o")
        );

        let resolved = resolve(&settings, &secrets, "ollama:llama3.1:8b").unwrap();
        assert_eq!(
            (resolved.provider.name(), resolved.model.as_str()),
            ("ollama", "llama3.1:8b")
        );

        assert_eq!(
            resolve(&settings, &secrets, "ollama:").err().unwrap().kind,
            LlmErrorKind::BadRequest
        );
    }

    #[test]
    fn settings_are_validated_and_normalized() {
        let normalized = normalize_settings(ProviderSettings {
            ollama_url: " http://gpu-box:11434/ ".into(),
            openai_url: "http://localhost:1234/v1".into(),
        })
        .unwrap();
        assert_eq!(normalized.ollama_url, "http://gpu-box:11434");
        assert_eq!(normalized.openai_url, "http://localhost:1234");

        let defaults = normalize_settings(ProviderSettings {
            ollama_url: "".into(),
            openai_url: " ".into(),
        })
        .unwrap();
        assert_eq!(defaults, ProviderSettings::default());

        for bad in ["localhost:11434", "ftp://x", "not a url", "http://"] {
            let result = normalize_settings(ProviderSettings {
                ollama_url: bad.into(),
                openai_url: String::new(),
            });
            assert!(result.is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn settings_are_saved_and_default_until_then() {
        let store = Settings::new(Db::open_in_memory().await.unwrap());
        assert_eq!(
            load_settings(&store).await.unwrap(),
            ProviderSettings::default()
        );
        let saved = save_settings(
            &store,
            ProviderSettings {
                ollama_url: "http://gpu-box:11434/".into(),
                openai_url: String::new(),
            },
        )
        .await
        .unwrap();
        assert_eq!(load_settings(&store).await.unwrap(), saved);
        assert_eq!(saved.ollama_url, "http://gpu-box:11434");
        assert!(save_settings(
            &store,
            ProviderSettings {
                ollama_url: "nope".into(),
                openai_url: String::new()
            }
        )
        .await
        .is_err());
        assert_eq!(load_settings(&store).await.unwrap(), saved);
    }

    async fn tags_server(status: StatusCode, body: &'static str) -> String {
        let app = Router::new().route("/api/tags", get(move || async move { (status, body) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn lists_the_installed_ollama_models_sorted() {
        let url = tags_server(
            StatusCode::OK,
            r#"{"models":[{"name":"qwen3:8b"},{"name":"llama3.1:8b"},{"model":"no-name"}]}"#,
        )
        .await;
        assert_eq!(
            ollama_models(&url).await.unwrap(),
            ["llama3.1:8b", "qwen3:8b"]
        );
        let empty = tags_server(StatusCode::OK, r#"{"models":[]}"#).await;
        assert!(ollama_models(&empty).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn ollama_problems_are_explained() {
        let error = ollama_models("http://127.0.0.1:9").await.unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::Network);
        assert!(error.message.contains("Is Ollama running?"), "{error}");
        let url = tags_server(StatusCode::INTERNAL_SERVER_ERROR, "").await;
        assert!(ollama_models(&url)
            .await
            .unwrap_err()
            .message
            .contains("500"));
        let url = tags_server(StatusCode::OK, "not json").await;
        assert!(ollama_models(&url)
            .await
            .unwrap_err()
            .message
            .contains("not understood"));
    }
}
