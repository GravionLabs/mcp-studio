//! Anthropic as a model provider: the Messages API with tool use, streaming, and usage.

use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use mcp_studio_core::{
    llm::{
        Completion, CompletionRequest, ContentBlock, LlmError, LlmErrorKind, LlmProvider, Message,
        Role, StopReason, StreamEvent, Usage,
    },
    model::JsonValue,
    secrets::SecretStore,
    tokens::ANTHROPIC_KEY_NAME,
};
use serde_json::{json, Value};

use crate::{sse::SseParser, DEFAULT_BASE_URL};

const API_VERSION: &str = "2023-06-01";

/// Talks to Anthropic with the user's own API key (bring your own key).
pub struct AnthropicProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                // Streaming answers can take long; this only bounds connecting and silence.
                .connect_timeout(Duration::from_secs(15))
                .read_timeout(Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: api_key.into(),
        }
    }

    /// Creates the provider with the API key from the OS keyring.
    pub fn from_store(store: &dyn SecretStore) -> Result<Self, LlmError> {
        let key = store
            .get(ANTHROPIC_KEY_NAME)
            .map_err(|e| LlmError::new(LlmErrorKind::Other, e.to_string()))?
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| {
                LlmError::new(
                    LlmErrorKind::Auth,
                    "Add an Anthropic API key to use Anthropic models",
                )
            })?;
        Ok(Self::new(key))
    }

    fn hide_key(&self, text: &str) -> String {
        if self.api_key.is_empty() {
            text.to_owned()
        } else {
            text.replace(&self.api_key, "••••••••")
        }
    }

    fn body(&self, request: &CompletionRequest, stream: bool) -> Value {
        let mut body = json!({
            "model": request.model,
            "max_tokens": request.max_tokens,
            "messages": request.messages.iter().map(message_json).collect::<Vec<_>>(),
        });
        if stream {
            body["stream"] = json!(true);
        }
        if let Some(system) = &request.system {
            body["system"] = json!(system);
        }
        if let Some(temperature) = request.temperature {
            body["temperature"] = json!(temperature);
        }
        if !request.tools.is_empty() {
            body["tools"] = request
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema.0,
                    })
                })
                .collect();
        }
        body
    }

    async fn send(&self, body: &Value) -> Result<reqwest::Response, LlmError> {
        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| {
                LlmError::new(
                    LlmErrorKind::Network,
                    self.hide_key(&format!("could not reach Anthropic: {e}")),
                )
            })?;
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let text = response.text().await.unwrap_or_default();
        Err(self.http_error(status, retry_after.as_deref(), &text))
    }

    fn http_error(&self, status: u16, retry_after: Option<&str>, body: &str) -> LlmError {
        let detail = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or_default();
        let (kind, message) = match status {
            401 | 403 => (
                LlmErrorKind::Auth,
                "Anthropic rejected the API key".to_owned(),
            ),
            429 => (
                LlmErrorKind::RateLimited,
                match retry_after {
                    Some(seconds) => {
                        format!("Anthropic is rate limiting requests; retry after {seconds} s")
                    }
                    None => "Anthropic is rate limiting requests; try again in a moment".to_owned(),
                },
            ),
            500..=599 => (
                LlmErrorKind::Unavailable,
                format!("Anthropic is unavailable ({status}): {detail}"),
            ),
            400 | 404 | 413 | 422 => (
                LlmErrorKind::BadRequest,
                format!("Anthropic did not accept the request: {detail}"),
            ),
            _ => (
                LlmErrorKind::Other,
                format!("Anthropic answered {status}: {detail}"),
            ),
        };
        LlmError::new(kind, self.hide_key(&message))
    }
}

fn message_json(message: &Message) -> Value {
    json!({
        "role": match message.role { Role::User => "user", Role::Assistant => "assistant" },
        "content": message.content.iter().map(block_json).collect::<Vec<_>>(),
    })
}

fn block_json(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text { text } => json!({ "type": "text", "text": text }),
        ContentBlock::ToolUse { id, name, input } => {
            json!({ "type": "tool_use", "id": id, "name": name, "input": input.0 })
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content,
            "is_error": is_error,
        }),
    }
}

fn stop_reason(value: Option<&str>) -> StopReason {
    match value {
        Some("end_turn") | Some("stop_sequence") | None => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some(other) => StopReason::Other(other.to_owned()),
    }
}

fn token(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|n| u32::try_from(n).ok())
}

/// Reads the `usage` object of a response or stream event into `usage`; fields that are missing
/// keep their value.
fn read_usage(value: &Value, usage: &mut Usage) {
    if let Some(n) = token(&value["input_tokens"]) {
        usage.input_tokens = n;
    }
    if let Some(n) = token(&value["output_tokens"]) {
        usage.output_tokens = n;
    }
    if let Some(n) = token(&value["cache_read_input_tokens"]) {
        usage.cache_read_tokens = Some(n);
    }
    if let Some(n) = token(&value["cache_creation_input_tokens"]) {
        usage.cache_write_tokens = Some(n);
    }
}

fn malformed(what: &str) -> LlmError {
    LlmError::new(
        LlmErrorKind::Other,
        format!("Anthropic's answer was not understood: {what}"),
    )
}

fn parse_completion(value: &Value) -> Result<Completion, LlmError> {
    let blocks = value["content"]
        .as_array()
        .ok_or_else(|| malformed("no content"))?;
    let mut content = Vec::new();
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => content.push(ContentBlock::text(block["text"].as_str().unwrap_or(""))),
            Some("tool_use") => content.push(ContentBlock::ToolUse {
                id: block["id"].as_str().unwrap_or("").to_owned(),
                name: block["name"].as_str().unwrap_or("").to_owned(),
                input: JsonValue(block["input"].clone()),
            }),
            // Other block types (for example thinking) are not used by flows.
            _ => {}
        }
    }
    let mut usage = Usage::default();
    read_usage(&value["usage"], &mut usage);
    Ok(Completion {
        model: value["model"].as_str().unwrap_or("").to_owned(),
        content,
        stop_reason: stop_reason(value["stop_reason"].as_str()),
        usage,
    })
}

enum Partial {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        json: String,
    },
    Ignored,
}

/// Builds the final answer from stream events.
#[derive(Default)]
struct StreamState {
    model: String,
    blocks: Vec<Partial>,
    stop_reason: Option<String>,
    usage: Usage,
    finished: bool,
}

impl StreamState {
    fn handle(
        &mut self,
        event: &Value,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), LlmError> {
        match event["type"].as_str() {
            Some("message_start") => {
                self.model = event["message"]["model"].as_str().unwrap_or("").to_owned();
                read_usage(&event["message"]["usage"], &mut self.usage);
            }
            Some("content_block_start") => {
                let index = event["index"].as_u64().unwrap_or(0) as usize;
                let block = &event["content_block"];
                let partial = match block["type"].as_str() {
                    Some("text") => Partial::Text(block["text"].as_str().unwrap_or("").to_owned()),
                    Some("tool_use") => {
                        let id = block["id"].as_str().unwrap_or("").to_owned();
                        let name = block["name"].as_str().unwrap_or("").to_owned();
                        on_event(StreamEvent::ToolUseStart {
                            id: id.clone(),
                            name: name.clone(),
                        });
                        Partial::ToolUse {
                            id,
                            name,
                            json: String::new(),
                        }
                    }
                    _ => Partial::Ignored,
                };
                while self.blocks.len() <= index {
                    self.blocks.push(Partial::Ignored);
                }
                self.blocks[index] = partial;
            }
            Some("content_block_delta") => {
                let index = event["index"].as_u64().unwrap_or(0) as usize;
                let delta = &event["delta"];
                match (self.blocks.get_mut(index), delta["type"].as_str()) {
                    (Some(Partial::Text(text)), Some("text_delta")) => {
                        let piece = delta["text"].as_str().unwrap_or("");
                        text.push_str(piece);
                        on_event(StreamEvent::TextDelta {
                            text: piece.to_owned(),
                        });
                    }
                    (Some(Partial::ToolUse { json, .. }), Some("input_json_delta")) => {
                        let piece = delta["partial_json"].as_str().unwrap_or("");
                        json.push_str(piece);
                        on_event(StreamEvent::ToolInputDelta {
                            partial_json: piece.to_owned(),
                        });
                    }
                    _ => {}
                }
            }
            Some("message_delta") => {
                if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_owned());
                }
                read_usage(&event["usage"], &mut self.usage);
            }
            Some("message_stop") => self.finished = true,
            Some("error") => {
                let kind = match event["error"]["type"].as_str() {
                    Some("overloaded_error") | Some("api_error") => LlmErrorKind::Unavailable,
                    Some("rate_limit_error") => LlmErrorKind::RateLimited,
                    Some("authentication_error") | Some("permission_error") => LlmErrorKind::Auth,
                    Some("invalid_request_error") => LlmErrorKind::BadRequest,
                    _ => LlmErrorKind::Other,
                };
                return Err(LlmError::new(
                    kind,
                    format!(
                        "Anthropic stopped the answer: {}",
                        event["error"]["message"]
                            .as_str()
                            .unwrap_or("unknown error")
                    ),
                ));
            }
            // `ping` and event types added later.
            _ => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<Completion, LlmError> {
        if !self.finished {
            return Err(LlmError::new(
                LlmErrorKind::Network,
                "the answer from Anthropic ended before it was complete",
            ));
        }
        let mut content = Vec::new();
        for block in self.blocks {
            match block {
                Partial::Text(text) => content.push(ContentBlock::Text { text }),
                Partial::ToolUse { id, name, json } => {
                    let input = if json.trim().is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(&json)
                            .map_err(|_| malformed("the arguments of a tool call are not JSON"))?
                    };
                    content.push(ContentBlock::ToolUse {
                        id,
                        name,
                        input: JsonValue(input),
                    });
                }
                Partial::Ignored => {}
            }
        }
        Ok(Completion {
            model: self.model,
            content,
            stop_reason: stop_reason(self.stop_reason.as_deref()),
            usage: self.usage,
        })
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<Completion, LlmError> {
        let response = self.send(&self.body(request, false)).await?;
        let text = response.text().await.map_err(|e| {
            LlmError::new(
                LlmErrorKind::Network,
                format!("could not read Anthropic's answer: {e}"),
            )
        })?;
        let value: Value = serde_json::from_str(&text).map_err(|_| malformed("not JSON"))?;
        parse_completion(&value)
    }

    async fn stream(
        &self,
        request: &CompletionRequest,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<Completion, LlmError> {
        let response = self.send(&self.body(request, true)).await?;
        let mut bytes = response.bytes_stream();
        let mut parser = SseParser::default();
        let mut state = StreamState::default();
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(|e| {
                LlmError::new(
                    LlmErrorKind::Network,
                    self.hide_key(&format!("the answer from Anthropic was interrupted: {e}")),
                )
            })?;
            for event in parser.feed(&chunk) {
                if let Ok(value) = serde_json::from_str::<Value>(&event.data) {
                    state.handle(&value, on_event)?;
                }
            }
        }
        state.finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{
        body::Bytes,
        http::{header, HeaderMap, StatusCode},
        routing::post,
        Router,
    };
    use mcp_studio_core::{llm::ToolDefinition, secrets::MemoryStore};

    use super::*;

    type Calls = Arc<Mutex<Vec<(HeaderMap, Value)>>>;

    async fn server(
        status: StatusCode,
        content_type: &'static str,
        body: String,
    ) -> (String, Calls) {
        let calls: Calls = Arc::default();
        let sink = calls.clone();
        let app = Router::new().route(
            "/v1/messages",
            post(move |headers: HeaderMap, request: Bytes| {
                let sink = sink.clone();
                let body = body.clone();
                async move {
                    sink.lock()
                        .unwrap()
                        .push((headers, serde_json::from_slice(&request).unwrap()));
                    (status, [(header::CONTENT_TYPE, content_type)], body)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}"), calls)
    }

    fn request() -> CompletionRequest {
        let mut request =
            CompletionRequest::new("claude-test", vec![Message::user("List my issues")]);
        request.system = Some("Be brief.".into());
        request.temperature = Some(0.2);
        request.max_tokens = 300;
        request.tools = vec![ToolDefinition {
            name: "list_issues".into(),
            description: "List issues".into(),
            input_schema: JsonValue(
                json!({"type": "object", "properties": {"repo": {"type": "string"}}}),
            ),
        }];
        request
    }

    const ANSWER: &str = r#"{
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-test",
        "content": [
            {"type": "text", "text": "I will look."},
            {"type": "tool_use", "id": "toolu_1", "name": "list_issues", "input": {"repo": "a/b"}}
        ],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 120, "output_tokens": 45, "cache_read_input_tokens": 100, "cache_creation_input_tokens": 0}
    }"#;

    #[tokio::test]
    async fn complete_sends_the_request_and_reads_text_tool_use_and_usage() {
        let (url, calls) = server(StatusCode::OK, "application/json", ANSWER.into()).await;
        let provider = AnthropicProvider::with_base_url("key-1", url);
        let completion = provider.complete(&request()).await.unwrap();

        assert_eq!(completion.text(), "I will look.");
        assert_eq!(completion.stop_reason, StopReason::ToolUse);
        let calls_made = completion.tool_uses();
        assert_eq!(
            (calls_made[0].0, calls_made[0].1),
            ("toolu_1", "list_issues")
        );
        assert_eq!(calls_made[0].2["repo"], "a/b");
        assert_eq!(
            completion.usage,
            Usage {
                input_tokens: 120,
                output_tokens: 45,
                cache_read_tokens: Some(100),
                cache_write_tokens: Some(0)
            }
        );

        let calls = calls.lock().unwrap();
        let (headers, body) = &calls[0];
        assert_eq!(headers["x-api-key"], "key-1");
        assert_eq!(headers["anthropic-version"], "2023-06-01");
        assert_eq!(body["model"], "claude-test");
        assert_eq!(body["max_tokens"], 300);
        assert_eq!(body["system"], "Be brief.");
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(
            body["messages"][0]["content"][0],
            json!({"type": "text", "text": "List my issues"})
        );
        assert_eq!(body["tools"][0]["name"], "list_issues");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert!(body.get("stream").is_none());
    }

    #[tokio::test]
    async fn tool_results_and_earlier_tool_calls_are_sent_back() {
        let (url, calls) = server(
            StatusCode::OK,
            "application/json",
            r#"{"model":"m","content":[{"type":"text","text":"done"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#.into(),
        )
        .await;
        let provider = AnthropicProvider::with_base_url("k", url);
        let mut request = CompletionRequest::new("m", vec![Message::user("go")]);
        request.messages.push(Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "echo".into(),
                input: JsonValue(json!({"message": "hi"})),
            }],
        });
        request.messages.push(Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: "hi".into(),
                is_error: false,
            }],
        });
        let completion = provider.complete(&request).await.unwrap();
        assert_eq!(completion.stop_reason, StopReason::EndTurn);
        let calls = calls.lock().unwrap();
        let messages = &calls[0].1["messages"];
        assert_eq!(messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(messages[1]["content"][0]["input"]["message"], "hi");
        assert_eq!(
            messages[2]["content"][0],
            json!({"type": "tool_result", "tool_use_id": "t1", "content": "hi", "is_error": false})
        );
    }

    const STREAM: &str = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-test\",\"usage\":{\"input_tokens\":120,\"output_tokens\":1,\"cache_read_input_tokens\":100}}}\n\n",
        "event: ping\ndata: {\"type\":\"ping\"}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"I will \"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"look.\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"list_issues\",\"input\":{}}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"repo\\\": \"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"a/b\\\"}\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":45}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );

    #[tokio::test]
    async fn streaming_reports_progress_and_builds_the_same_answer() {
        let (url, calls) = server(StatusCode::OK, "text/event-stream", STREAM.into()).await;
        let provider = AnthropicProvider::with_base_url("k", url);
        let mut events = Vec::new();
        let completion = provider
            .stream(&request(), &mut |e| events.push(e))
            .await
            .unwrap();

        assert_eq!(calls.lock().unwrap()[0].1["stream"], true);
        assert_eq!(
            events,
            vec![
                StreamEvent::TextDelta {
                    text: "I will ".into()
                },
                StreamEvent::TextDelta {
                    text: "look.".into()
                },
                StreamEvent::ToolUseStart {
                    id: "toolu_1".into(),
                    name: "list_issues".into()
                },
                StreamEvent::ToolInputDelta {
                    partial_json: "{\"repo\": ".into()
                },
                StreamEvent::ToolInputDelta {
                    partial_json: "\"a/b\"}".into()
                },
            ]
        );
        assert_eq!(completion.model, "claude-test");
        assert_eq!(completion.text(), "I will look.");
        assert_eq!(completion.stop_reason, StopReason::ToolUse);
        assert_eq!(completion.tool_uses()[0].2["repo"], "a/b");
        // Input tokens come from message_start, output tokens from the last message_delta.
        assert_eq!(completion.usage.input_tokens, 120);
        assert_eq!(completion.usage.output_tokens, 45);
        assert_eq!(completion.usage.cache_read_tokens, Some(100));
    }

    #[tokio::test]
    async fn a_tool_call_without_arguments_gets_an_empty_object() {
        let stream = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":1}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t\",\"name\":\"now\",\"input\":{}}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":3}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let (url, _) = server(StatusCode::OK, "text/event-stream", stream.into()).await;
        let provider = AnthropicProvider::with_base_url("k", url);
        let completion = provider.stream(&request(), &mut |_| {}).await.unwrap();
        assert_eq!(completion.tool_uses()[0].2, &json!({}));
    }

    #[tokio::test]
    async fn an_error_event_or_a_cut_off_stream_is_an_error() {
        let overloaded = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
        let (url, _) = server(StatusCode::OK, "text/event-stream", overloaded.into()).await;
        let error = AnthropicProvider::with_base_url("k", url)
            .stream(&request(), &mut |_| {})
            .await
            .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::Unavailable);
        assert!(error.message.contains("Overloaded"), "{error}");

        let cut =
            "data: {\"type\":\"message_start\",\"message\":{\"model\":\"m\",\"usage\":{}}}\n\n";
        let (url, _) = server(StatusCode::OK, "text/event-stream", cut.into()).await;
        let error = AnthropicProvider::with_base_url("k", url)
            .stream(&request(), &mut |_| {})
            .await
            .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::Network);
        assert!(error.is_retryable());
    }

    #[tokio::test]
    async fn http_errors_are_classified_and_never_contain_the_key() {
        for (status, kind, expected) in [
            (
                StatusCode::UNAUTHORIZED,
                LlmErrorKind::Auth,
                "rejected the API key",
            ),
            (
                StatusCode::TOO_MANY_REQUESTS,
                LlmErrorKind::RateLimited,
                "rate limiting",
            ),
            (
                StatusCode::from_u16(529).unwrap(),
                LlmErrorKind::Unavailable,
                "unavailable",
            ),
            (
                StatusCode::BAD_REQUEST,
                LlmErrorKind::BadRequest,
                "model: nope sk-secret",
            ),
            (StatusCode::IM_A_TEAPOT, LlmErrorKind::Other, "418"),
        ] {
            let body = r#"{"error":{"type":"x","message":"model: nope sk-secret"}}"#;
            let (url, _) = server(status, "application/json", body.into()).await;
            let error = AnthropicProvider::with_base_url("sk-secret", url)
                .complete(&request())
                .await
                .unwrap_err();
            assert_eq!(error.kind, kind, "{status}");
            assert!(
                error
                    .message
                    .contains(&expected.replace("sk-secret", "••••••••")),
                "{status}: {error}"
            );
            assert!(!error.message.contains("sk-secret"), "{error}");
        }
    }

    #[tokio::test]
    async fn an_unreachable_provider_is_a_retryable_network_error() {
        let error = AnthropicProvider::with_base_url("k", "http://127.0.0.1:9")
            .complete(&request())
            .await
            .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::Network);
        assert!(error.is_retryable());
    }

    #[test]
    fn the_key_comes_from_the_keyring() {
        let store = MemoryStore::default();
        let missing = AnthropicProvider::from_store(&store).err().unwrap();
        assert_eq!(missing.kind, LlmErrorKind::Auth);
        store.set(ANTHROPIC_KEY_NAME, "  ").unwrap();
        assert!(AnthropicProvider::from_store(&store).is_err());
        store.set(ANTHROPIC_KEY_NAME, "sk-real").unwrap();
        let provider = AnthropicProvider::from_store(&store).unwrap();
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn stop_reasons_map_to_the_neutral_ones() {
        assert_eq!(stop_reason(Some("end_turn")), StopReason::EndTurn);
        assert_eq!(stop_reason(Some("stop_sequence")), StopReason::EndTurn);
        assert_eq!(stop_reason(Some("max_tokens")), StopReason::MaxTokens);
        assert_eq!(
            stop_reason(Some("refusal")),
            StopReason::Other("refusal".into())
        );
    }
}
