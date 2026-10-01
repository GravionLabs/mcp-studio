//! Anthropic: exact token counts through the Messages API token-counting endpoint.

use std::time::Duration;

use async_trait::async_trait;
use mcp_studio_core::{
    db::{DbError, DbResult},
    tokens::TokenCounter,
};
use serde_json::{json, Value};
use tokio::sync::OnceCell;

/// Model used for counting until the user chooses another.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
const API_VERSION: &str = "2023-06-01";

/// Counts tokens with `POST /v1/messages/count_tokens`.
///
/// The endpoint counts a whole request, so the text is sent as one user message and the fixed
/// overhead of a message is subtracted. The overhead is measured once with a one-token text.
pub struct AnthropicCounter {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    overhead: OnceCell<u32>,
}

impl AnthropicCounter {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::with_base_url(api_key, model, crate::DEFAULT_BASE_URL)
    }

    pub fn with_base_url(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: api_key.into(),
            model: model.into(),
            overhead: OnceCell::new(),
        }
    }

    /// Tokens of a request that has `text` as its only user message, including message overhead.
    async fn request(&self, text: &str) -> DbResult<u32> {
        let body = json!({
            "model": self.model,
            "messages": [{ "role": "user", "content": text }],
        });
        let response = self
            .client
            .post(format!("{}/v1/messages/count_tokens", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| {
                connection(format!(
                    "could not reach Anthropic: {}",
                    self.hide_key(&e.to_string())
                ))
            })?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| connection(format!("could not read Anthropic's answer: {e}")))?;
        if !status.is_success() {
            return Err(connection(match status.as_u16() {
                401 | 403 => "Anthropic rejected the API key".to_owned(),
                429 => "Anthropic is rate limiting token counts; try again in a moment".to_owned(),
                code => {
                    let detail = serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
                        .unwrap_or_default();
                    self.hide_key(&format!("Anthropic answered {code}: {detail}"))
                }
            }));
        }
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| connection("Anthropic's answer was not understood".into()))?;
        value["input_tokens"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| connection("Anthropic's answer has no token count".into()))
    }

    fn hide_key(&self, text: &str) -> String {
        if self.api_key.is_empty() {
            text.to_owned()
        } else {
            text.replace(&self.api_key, "••••••••")
        }
    }
}

fn connection(message: String) -> DbError {
    DbError::Connection(message)
}

#[async_trait]
impl TokenCounter for AnthropicCounter {
    async fn count_text(&self, text: &str) -> DbResult<u32> {
        let total = self.request(text).await?;
        let overhead = self
            .overhead
            .get_or_try_init(|| async {
                Ok::<_, DbError>(self.request(".").await?.saturating_sub(1))
            })
            .await?;
        Ok(total.saturating_sub(*overhead))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{body::Bytes, http::HeaderMap, http::StatusCode, routing::post, Router};

    use super::*;

    type Calls = Arc<Mutex<Vec<(HeaderMap, Value)>>>;

    /// Answers like the real endpoint: 10 tokens of overhead plus one token per 4 characters.
    async fn server(status: StatusCode, error_body: &'static str) -> (String, Calls) {
        let calls: Calls = Arc::default();
        let sink = calls.clone();
        let app = Router::new().route(
            "/v1/messages/count_tokens",
            post(move |headers: HeaderMap, body: Bytes| {
                let sink = sink.clone();
                async move {
                    let value: Value = serde_json::from_slice(&body).unwrap();
                    let text = value["messages"][0]["content"].as_str().unwrap_or("").len();
                    sink.lock().unwrap().push((headers, value));
                    if status.is_success() {
                        (
                            status,
                            format!(r#"{{"input_tokens":{}}}"#, 10 + text.div_ceil(4)),
                        )
                    } else {
                        (status, error_body.to_owned())
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}"), calls)
    }

    #[tokio::test]
    async fn counts_text_without_the_message_overhead() {
        let (url, calls) = server(StatusCode::OK, "").await;
        let counter = AnthropicCounter::with_base_url("key-1", "some-model", url);
        // 8 characters = 2 tokens; the 10 token overhead is measured and subtracted.
        assert_eq!(counter.count_text("abcdefgh").await.unwrap(), 2);
        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].0["x-api-key"], "key-1");
        assert_eq!(calls[0].0["anthropic-version"], "2023-06-01");
        assert_eq!(calls[0].1["model"], "some-model");
        assert_eq!(calls[0].1["messages"][0]["role"], "user");
        assert_eq!(calls[0].1["messages"][0]["content"], "abcdefgh");
    }

    #[tokio::test]
    async fn measures_the_overhead_only_once() {
        let (url, calls) = server(StatusCode::OK, "").await;
        let counter = AnthropicCounter::with_base_url("k", "m", url);
        counter.count_text("one").await.unwrap();
        counter.count_text("two").await.unwrap();
        // First text + overhead probe, then only the second text.
        assert_eq!(calls.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn explains_rejected_keys_and_rate_limits() {
        let (url, _) = server(StatusCode::UNAUTHORIZED, "{}").await;
        let counter = AnthropicCounter::with_base_url("bad", "m", url);
        let error = counter.count_text("x").await.unwrap_err().to_string();
        assert!(error.contains("rejected the API key"), "{error}");

        let (url, _) = server(StatusCode::TOO_MANY_REQUESTS, "{}").await;
        let counter = AnthropicCounter::with_base_url("k", "m", url);
        let error = counter.count_text("x").await.unwrap_err().to_string();
        assert!(error.contains("rate limiting"), "{error}");
    }

    #[tokio::test]
    async fn shows_the_providers_error_message_without_the_key() {
        let (url, _) = server(
            StatusCode::BAD_REQUEST,
            r#"{"error":{"type":"invalid_request_error","message":"model: unknown model sk-secret-key"}}"#,
        )
        .await;
        let counter = AnthropicCounter::with_base_url("sk-secret-key", "nope", url);
        let error = counter.count_text("x").await.unwrap_err().to_string();
        assert!(
            error.contains("answered 400") && error.contains("unknown model"),
            "{error}"
        );
        assert!(!error.contains("sk-secret-key"), "{error}");
    }

    #[tokio::test]
    async fn an_unreachable_server_is_an_error() {
        let counter = AnthropicCounter::with_base_url("k", "m", "http://127.0.0.1:9");
        let error = counter.count_text("x").await.unwrap_err().to_string();
        assert!(error.contains("could not reach Anthropic"), "{error}");
    }
}
