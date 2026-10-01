//! Provider-neutral types for talking to a language model: the request, the answer, tool use,
//! usage, and streaming. Provider implementations live in `mcp-studio-llm`; the flow engine only
//! sees [`LlmProvider`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use thiserror::Error;

use crate::{db::DbError, model::JsonValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// One piece of a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    /// The model asks to call a tool.
    ToolUse {
        id: String,
        name: String,
        input: JsonValue,
    },
    /// The result of a tool call, sent back to the model.
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text { text: text.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::text(text)],
        }
    }
}

/// A tool the model may call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments.
    pub input_schema: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CompletionRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: Option<f64>,
}

impl CompletionRequest {
    pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            system: None,
            messages,
            tools: Vec::new(),
            max_tokens: 1024,
            temperature: None,
        }
    }
}

/// Why the model stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The model finished its answer.
    EndTurn,
    /// The model wants tools called; the answer contains `ToolUse` blocks.
    ToolUse,
    /// The answer was cut off at `max_tokens`.
    MaxTokens,
    Other(String),
}

/// Tokens the provider reported for a request. Always exact, unlike the estimates of the inspector.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: Option<u32>,
    pub cache_write_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    pub model: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

impl Completion {
    /// The text blocks of the answer, joined.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The tool calls the model asked for.
    pub fn tool_uses(&self) -> Vec<(&str, &str, &Value)> {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, name, input } => {
                    Some((id.as_str(), name.as_str(), &input.0))
                }
                _ => None,
            })
            .collect()
    }
}

/// Progress of a streamed answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    TextDelta {
        text: String,
    },
    /// The model starts a tool call; its arguments follow as they are generated.
    ToolUseStart {
        id: String,
        name: String,
    },
    /// A fragment of the JSON arguments of the tool call that is being generated.
    ToolInputDelta {
        partial_json: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmErrorKind {
    /// The API key is missing or was rejected.
    Auth,
    RateLimited,
    /// The provider is overloaded or failing; trying again later may work.
    Unavailable,
    /// The request was not accepted (unknown model, invalid parameters).
    BadRequest,
    /// The provider could not be reached.
    Network,
    Other,
}

/// A failure of a model call. The message never contains the API key.
#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct LlmError {
    pub kind: LlmErrorKind,
    pub message: String,
}

impl LlmError {
    pub fn new(kind: LlmErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Whether the same request may succeed when repeated later.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            LlmErrorKind::RateLimited | LlmErrorKind::Unavailable | LlmErrorKind::Network
        )
    }
}

impl From<LlmError> for DbError {
    fn from(error: LlmError) -> Self {
        DbError::Connection(error.message)
    }
}

/// A model provider (Anthropic, an OpenAI-compatible endpoint, Ollama, ...).
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Short name of the provider, for example `anthropic`.
    fn name(&self) -> &str;

    /// Runs a request and returns the whole answer.
    async fn complete(&self, request: &CompletionRequest) -> Result<Completion, LlmError>;

    /// Runs a request and reports progress while the answer is generated. Returns the same
    /// [`Completion`] that [`complete`](Self::complete) would.
    async fn stream(
        &self,
        request: &CompletionRequest,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<Completion, LlmError>;
}

/// Name of the keyring entry that holds the API key of an OpenAI-compatible endpoint.
pub const OPENAI_KEY_NAME: &str = "openai-api-key";

/// Where the providers other than Anthropic are reached. Keys are not part of this: they live in
/// the OS keyring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettings {
    /// Base address of Ollama, without `/v1`.
    pub ollama_url: String,
    /// Base address of an OpenAI-compatible API, without `/v1` (OpenAI, LM Studio, vLLM, ...).
    pub openai_url: String,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            ollama_url: "http://localhost:11434".into(),
            openai_url: "https://api.openai.com".into(),
        }
    }
}

/// Which providers are set up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub settings: ProviderSettings,
    /// An Anthropic API key is stored in the keyring.
    pub anthropic_key: bool,
    /// An API key for the OpenAI-compatible endpoint is stored in the keyring (local endpoints
    /// often need none).
    pub openai_key: bool,
}

/// Result of checking that a provider and its key work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestResult {
    pub model: String,
    pub reply: String,
    pub usage: Usage,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn completion_helpers_read_text_and_tool_calls() {
        let completion = Completion {
            model: "m".into(),
            content: vec![
                ContentBlock::text("Hello "),
                ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "echo".into(),
                    input: JsonValue(json!({"message": "hi"})),
                },
                ContentBlock::text("world"),
            ],
            stop_reason: StopReason::ToolUse,
            usage: Usage::default(),
        };
        assert_eq!(completion.text(), "Hello world");
        let calls = completion.tool_uses();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "echo");
        assert_eq!(calls[0].2["message"], "hi");
    }

    #[test]
    fn content_blocks_serialize_with_a_type_tag() {
        let value = serde_json::to_value(ContentBlock::ToolResult {
            tool_use_id: "t1".into(),
            content: "ok".into(),
            is_error: false,
        })
        .unwrap();
        assert_eq!(value["type"], "tool_result");
        assert_eq!(value["tool_use_id"], "t1");
    }

    #[test]
    fn only_transient_errors_are_retryable() {
        for (kind, retry) in [
            (LlmErrorKind::RateLimited, true),
            (LlmErrorKind::Unavailable, true),
            (LlmErrorKind::Network, true),
            (LlmErrorKind::Auth, false),
            (LlmErrorKind::BadRequest, false),
            (LlmErrorKind::Other, false),
        ] {
            assert_eq!(LlmError::new(kind, "x").is_retryable(), retry);
        }
    }

    #[test]
    fn requests_have_sensible_defaults() {
        let request = CompletionRequest::new("m", vec![Message::user("hi")]);
        assert_eq!(request.max_tokens, 1024);
        assert!(request.tools.is_empty() && request.system.is_none());
    }
}
