//! OpenAI-compatible chat completions: OpenAI itself, LM Studio, vLLM, and Ollama (which serves the
//! same API under `/v1`). One implementation, with tool calls, streaming, and usage.

use std::{collections::BTreeMap, time::Duration};

use async_trait::async_trait;
use futures_util::StreamExt;
use mcp_studio_core::{
    llm::{
        Completion, CompletionRequest, ContentBlock, LlmError, LlmErrorKind, LlmProvider, Message,
        Role, StopReason, StreamEvent, Usage,
    },
    model::JsonValue,
};
use serde_json::{json, Value};

use crate::sse::SseParser;

/// Talks to an endpoint that implements `POST <root>/chat/completions` (root is usually `/v1`).
pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    name: &'static str,
    /// What to call the endpoint in error messages, for example `Ollama`.
    label: String,
    /// The address as configured, without a trailing slash.
    base_url: String,
    /// `None` for local endpoints that need no key.
    api_key: Option<String>,
}

/// Where chat completions are posted. A bare host (`http://localhost:1234`) gets the usual `/v1`;
/// an address with a path (`.../v1`, `https://models.github.ai/inference`) already names the API
/// root and only gets `/chat/completions`.
pub fn chat_completions_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let bare_host = reqwest::Url::parse(base).is_ok_and(|u| u.path() == "/");
    if bare_host {
        format!("{base}/v1/chat/completions")
    } else {
        format!("{base}/chat/completions")
    }
}

impl OpenAiCompatProvider {
    /// An OpenAI-compatible endpoint at `base_url` (see [`chat_completions_url`]).
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self::build(
            "openai",
            "the OpenAI-compatible endpoint",
            base_url.into(),
            api_key,
        )
    }

    /// A local Ollama server.
    pub fn ollama(base_url: impl Into<String>) -> Self {
        Self::build("ollama", "Ollama", base_url.into(), None)
    }

    fn build(name: &'static str, label: &str, base_url: String, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                // Local models can think for a long time before the first token.
                .read_timeout(Duration::from_secs(300))
                .build()
                .unwrap_or_default(),
            name,
            label: label.to_owned(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key: api_key.filter(|k| !k.trim().is_empty()),
        }
    }

    fn hide_key(&self, text: &str) -> String {
        match &self.api_key {
            Some(key) => text.replace(key, "••••••••"),
            None => text.to_owned(),
        }
    }

    fn body(&self, request: &CompletionRequest, stream: bool) -> Value {
        let mut messages = Vec::new();
        if let Some(system) = &request.system {
            messages.push(json!({ "role": "system", "content": system }));
        }
        for message in &request.messages {
            messages.extend(message_json(message));
        }
        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "max_tokens": request.max_tokens,
        });
        if stream {
            body["stream"] = json!(true);
            body["stream_options"] = json!({ "include_usage": true });
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
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.input_schema.0,
                        },
                    })
                })
                .collect();
        }
        body
    }

    async fn send(&self, body: &Value) -> Result<reqwest::Response, LlmError> {
        let mut request = self
            .client
            .post(chat_completions_url(&self.base_url))
            .header("content-type", "application/json")
            .body(body.to_string());
        if let Some(key) = &self.api_key {
            request = request.header("authorization", format!("Bearer {key}"));
        }
        let response = request.send().await.map_err(|e| {
            LlmError::new(
                LlmErrorKind::Network,
                self.hide_key(&format!(
                    "could not reach {} at {}: {e}",
                    self.label, self.base_url
                )),
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
        let json = serde_json::from_str::<Value>(body).ok();
        // OpenAI and Ollama both answer {"error": {"message": ...}}; some servers use a plain string.
        let detail = json
            .as_ref()
            .and_then(|v| {
                v["error"]["message"]
                    .as_str()
                    .or_else(|| v["error"].as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let label = &self.label;
        let (kind, message) = match status {
            401 | 403 => (LlmErrorKind::Auth, format!("{label} rejected the API key")),
            429 => (
                LlmErrorKind::RateLimited,
                match retry_after {
                    Some(seconds) => {
                        format!("{label} is rate limiting requests; retry after {seconds} s")
                    }
                    None => format!("{label} is rate limiting requests; try again in a moment"),
                },
            ),
            500..=599 => (
                LlmErrorKind::Unavailable,
                format!("{label} is unavailable ({status}): {detail}"),
            ),
            400 | 404 | 413 | 422 => (
                LlmErrorKind::BadRequest,
                format!("{label} did not accept the request: {detail}"),
            ),
            _ => (
                LlmErrorKind::Other,
                format!("{label} answered {status}: {detail}"),
            ),
        };
        LlmError::new(kind, self.hide_key(&message))
    }
}

/// One neutral message as one or more chat messages: tool results become `tool` messages.
fn message_json(message: &Message) -> Vec<Value> {
    let text: String = message
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    match message.role {
        Role::User => {
            let mut out: Vec<Value> = message
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolResult { tool_use_id, content, is_error } => Some(json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": if *is_error { format!("Error: {content}") } else { content.clone() },
                    })),
                    _ => None,
                })
                .collect();
            if !text.is_empty() || out.is_empty() {
                out.push(json!({ "role": "user", "content": text }));
            }
            out
        }
        Role::Assistant => {
            let calls: Vec<Value> = message
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, name, input } => Some(json!({
                        "id": id,
                        "type": "function",
                        "function": { "name": name, "arguments": input.0.to_string() },
                    })),
                    _ => None,
                })
                .collect();
            let mut value = json!({
                "role": "assistant",
                "content": if text.is_empty() { Value::Null } else { json!(text) },
            });
            if !calls.is_empty() {
                value["tool_calls"] = Value::Array(calls);
            }
            vec![value]
        }
    }
}

fn stop_reason(value: Option<&str>) -> StopReason {
    match value {
        Some("stop") | None => StopReason::EndTurn,
        Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
        Some("length") => StopReason::MaxTokens,
        Some(other) => StopReason::Other(other.to_owned()),
    }
}

fn token(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|n| u32::try_from(n).ok())
}

fn read_usage(value: &Value) -> Usage {
    Usage {
        input_tokens: token(&value["prompt_tokens"]).unwrap_or(0),
        output_tokens: token(&value["completion_tokens"]).unwrap_or(0),
        cache_read_tokens: token(&value["prompt_tokens_details"]["cached_tokens"]),
        cache_write_tokens: None,
    }
}

fn malformed(what: &str) -> LlmError {
    LlmError::new(
        LlmErrorKind::Other,
        format!("the answer was not understood: {what}"),
    )
}

fn parse_arguments(arguments: &str) -> Result<Value, LlmError> {
    if arguments.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(arguments)
        .map_err(|_| malformed("the arguments of a tool call are not JSON"))
}

fn parse_completion(value: &Value) -> Result<Completion, LlmError> {
    let choice = value["choices"]
        .get(0)
        .ok_or_else(|| malformed("no choices"))?;
    let message = &choice["message"];
    let mut content = Vec::new();
    if let Some(text) = message["content"].as_str().filter(|t| !t.is_empty()) {
        content.push(ContentBlock::text(text));
    }
    for call in message["tool_calls"].as_array().into_iter().flatten() {
        content.push(ContentBlock::ToolUse {
            id: call["id"].as_str().unwrap_or("").to_owned(),
            name: call["function"]["name"].as_str().unwrap_or("").to_owned(),
            input: JsonValue(parse_arguments(
                call["function"]["arguments"].as_str().unwrap_or(""),
            )?),
        });
    }
    Ok(Completion {
        model: value["model"].as_str().unwrap_or("").to_owned(),
        content,
        stop_reason: stop_reason(choice["finish_reason"].as_str()),
        usage: read_usage(&value["usage"]),
    })
}

#[derive(Default)]
struct PendingCall {
    id: String,
    name: String,
    arguments: String,
    started: bool,
}

#[derive(Default)]
struct StreamState {
    model: String,
    text: String,
    calls: BTreeMap<usize, PendingCall>,
    finish_reason: Option<String>,
    usage: Usage,
    done: bool,
}

impl StreamState {
    fn handle(&mut self, chunk: &Value, on_event: &mut (dyn FnMut(StreamEvent) + Send)) {
        if let Some(model) = chunk["model"].as_str() {
            self.model = model.to_owned();
        }
        if chunk["usage"].is_object() {
            self.usage = read_usage(&chunk["usage"]);
        }
        let Some(choice) = chunk["choices"].get(0) else {
            return;
        };
        let delta = &choice["delta"];
        if let Some(text) = delta["content"].as_str().filter(|t| !t.is_empty()) {
            self.text.push_str(text);
            on_event(StreamEvent::TextDelta {
                text: text.to_owned(),
            });
        }
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = call["index"].as_u64().unwrap_or(0) as usize;
            let pending = self.calls.entry(index).or_default();
            if let Some(id) = call["id"].as_str().filter(|i| !i.is_empty()) {
                pending.id = id.to_owned();
            }
            if let Some(name) = call["function"]["name"].as_str().filter(|n| !n.is_empty()) {
                pending.name.push_str(name);
            }
            if !pending.started && !pending.name.is_empty() {
                pending.started = true;
                on_event(StreamEvent::ToolUseStart {
                    id: pending.id.clone(),
                    name: pending.name.clone(),
                });
            }
            if let Some(fragment) = call["function"]["arguments"]
                .as_str()
                .filter(|a| !a.is_empty())
            {
                pending.arguments.push_str(fragment);
                on_event(StreamEvent::ToolInputDelta {
                    partial_json: fragment.to_owned(),
                });
            }
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.finish_reason = Some(reason.to_owned());
        }
    }

    fn finish(self) -> Result<Completion, LlmError> {
        if !self.done && self.finish_reason.is_none() {
            return Err(LlmError::new(
                LlmErrorKind::Network,
                "the answer ended before it was complete",
            ));
        }
        let mut content = Vec::new();
        if !self.text.is_empty() {
            content.push(ContentBlock::Text { text: self.text });
        }
        for (_, call) in self.calls {
            content.push(ContentBlock::ToolUse {
                id: call.id,
                name: call.name,
                input: JsonValue(parse_arguments(&call.arguments)?),
            });
        }
        Ok(Completion {
            model: self.model,
            content,
            stop_reason: stop_reason(self.finish_reason.as_deref()),
            usage: self.usage,
        })
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatProvider {
    fn name(&self) -> &str {
        self.name
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<Completion, LlmError> {
        let response = self.send(&self.body(request, false)).await?;
        let text = response.text().await.map_err(|e| {
            LlmError::new(
                LlmErrorKind::Network,
                format!("could not read the answer: {e}"),
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
                    self.hide_key(&format!("the answer was interrupted: {e}")),
                )
            })?;
            for event in parser.feed(&chunk) {
                if event.data.trim() == "[DONE]" {
                    state.done = true;
                } else if let Ok(value) = serde_json::from_str::<Value>(&event.data) {
                    state.handle(&value, on_event);
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
    use mcp_studio_core::llm::ToolDefinition;

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
            "/v1/chat/completions",
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
        let mut request = CompletionRequest::new("llama3.1", vec![Message::user("List my issues")]);
        request.system = Some("Be brief.".into());
        request.max_tokens = 200;
        request.temperature = Some(0.1);
        request.tools = vec![ToolDefinition {
            name: "list_issues".into(),
            description: "List issues".into(),
            input_schema: JsonValue(json!({"type": "object"})),
        }];
        request
    }

    const ANSWER: &str = r#"{
        "model": "llama3.1",
        "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
            "role": "assistant", "content": null,
            "tool_calls": [{"id": "call_1", "type": "function",
                "function": {"name": "list_issues", "arguments": "{\"repo\":\"a/b\"}"}}]
        }}],
        "usage": {"prompt_tokens": 50, "completion_tokens": 12, "prompt_tokens_details": {"cached_tokens": 40}}
    }"#;

    #[tokio::test]
    async fn complete_sends_a_chat_request_and_reads_tool_calls_and_usage() {
        let (url, calls) = server(StatusCode::OK, "application/json", ANSWER.into()).await;
        let provider = OpenAiCompatProvider::new(url, Some("sk-test".into()));
        let completion = provider.complete(&request()).await.unwrap();

        assert_eq!(completion.stop_reason, StopReason::ToolUse);
        assert_eq!(completion.text(), "");
        let tool_calls = completion.tool_uses();
        assert_eq!(
            (tool_calls[0].0, tool_calls[0].1),
            ("call_1", "list_issues")
        );
        assert_eq!(tool_calls[0].2["repo"], "a/b");
        assert_eq!(
            completion.usage,
            Usage {
                input_tokens: 50,
                output_tokens: 12,
                cache_read_tokens: Some(40),
                cache_write_tokens: None
            }
        );

        let calls = calls.lock().unwrap();
        let (headers, body) = &calls[0];
        assert_eq!(headers["authorization"], "Bearer sk-test");
        assert_eq!(body["model"], "llama3.1");
        assert_eq!(body["max_tokens"], 200);
        assert_eq!(body["temperature"], 0.1);
        assert_eq!(
            body["messages"][0],
            json!({"role": "system", "content": "Be brief."})
        );
        assert_eq!(
            body["messages"][1],
            json!({"role": "user", "content": "List my issues"})
        );
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "list_issues");
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
        assert!(body.get("stream").is_none());
    }

    #[tokio::test]
    async fn local_endpoints_need_no_key_and_trailing_v1_is_accepted() {
        let (url, calls) = server(
            StatusCode::OK,
            "application/json",
            r#"{"model":"m","choices":[{"message":{"content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#.into(),
        )
        .await;
        let provider = OpenAiCompatProvider::ollama(format!("{url}/v1/"));
        let completion = provider.complete(&request()).await.unwrap();
        assert_eq!(completion.text(), "hi");
        assert_eq!(completion.stop_reason, StopReason::EndTurn);
        assert_eq!(provider.name(), "ollama");
        assert!(calls.lock().unwrap()[0].0.get("authorization").is_none());
    }

    #[tokio::test]
    async fn tool_calls_and_results_are_converted_to_chat_messages() {
        let (url, calls) = server(
            StatusCode::OK,
            "application/json",
            r#"{"model":"m","choices":[{"message":{"content":"done"},"finish_reason":"stop"}]}"#
                .into(),
        )
        .await;
        let provider = OpenAiCompatProvider::new(url, None);
        let mut request = CompletionRequest::new("m", vec![Message::user("go")]);
        request.messages.push(Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::text("Calling"),
                ContentBlock::ToolUse {
                    id: "c1".into(),
                    name: "echo".into(),
                    input: JsonValue(json!({"message": "hi"})),
                },
            ],
        });
        request.messages.push(Message {
            role: Role::User,
            content: vec![
                ContentBlock::ToolResult {
                    tool_use_id: "c1".into(),
                    content: "hi".into(),
                    is_error: false,
                },
                ContentBlock::ToolResult {
                    tool_use_id: "c2".into(),
                    content: "boom".into(),
                    is_error: true,
                },
            ],
        });
        provider.complete(&request).await.unwrap();
        let calls = calls.lock().unwrap();
        let messages = calls[0].1["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "Calling");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "c1");
        assert_eq!(
            messages[1]["tool_calls"][0]["function"]["arguments"],
            "{\"message\":\"hi\"}"
        );
        assert_eq!(
            messages[2],
            json!({"role": "tool", "tool_call_id": "c1", "content": "hi"})
        );
        assert_eq!(
            messages[3],
            json!({"role": "tool", "tool_call_id": "c2", "content": "Error: boom"})
        );
    }

    const STREAM: &str = concat!(
        "data: {\"model\":\"llama3.1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"I will \"}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"look.\"}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"list_issues\",\"arguments\":\"\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"repo\\\": \"}}]}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"a/b\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":50,\"completion_tokens\":12}}\n\n",
        "data: [DONE]\n\n",
    );

    #[tokio::test]
    async fn streaming_reports_progress_and_builds_the_same_answer() {
        let (url, calls) = server(StatusCode::OK, "text/event-stream", STREAM.into()).await;
        let provider = OpenAiCompatProvider::ollama(url);
        let mut events = Vec::new();
        let completion = provider
            .stream(&request(), &mut |e| events.push(e))
            .await
            .unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].1["stream"], true);
        assert_eq!(calls[0].1["stream_options"]["include_usage"], true);
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
                    id: "call_1".into(),
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
        assert_eq!(completion.model, "llama3.1");
        assert_eq!(completion.text(), "I will look.");
        assert_eq!(completion.stop_reason, StopReason::ToolUse);
        assert_eq!(completion.tool_uses()[0].2["repo"], "a/b");
        assert_eq!(completion.usage.input_tokens, 50);
        assert_eq!(completion.usage.output_tokens, 12);
    }

    #[tokio::test]
    async fn a_stream_without_done_is_fine_when_it_finished_and_an_error_when_cut_off() {
        let finished =
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n";
        let (url, _) = server(StatusCode::OK, "text/event-stream", finished.into()).await;
        let completion = OpenAiCompatProvider::new(url, None)
            .stream(&request(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(completion.text(), "ok");

        let cut = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
        let (url, _) = server(StatusCode::OK, "text/event-stream", cut.into()).await;
        let error = OpenAiCompatProvider::new(url, None)
            .stream(&request(), &mut |_| {})
            .await
            .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::Network);
    }

    #[tokio::test]
    async fn invalid_tool_arguments_are_reported() {
        let answer = r#"{"model":"m","choices":[{"finish_reason":"tool_calls","message":{"content":null,"tool_calls":[{"id":"c","type":"function","function":{"name":"x","arguments":"{not json"}}]}}]}"#;
        let (url, _) = server(StatusCode::OK, "application/json", answer.into()).await;
        let error = OpenAiCompatProvider::new(url, None)
            .complete(&request())
            .await
            .unwrap_err();
        assert!(error.message.contains("not JSON"), "{error}");
    }

    #[tokio::test]
    async fn http_errors_are_classified_and_hide_the_key() {
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
                StatusCode::BAD_GATEWAY,
                LlmErrorKind::Unavailable,
                "unavailable",
            ),
            (
                StatusCode::NOT_FOUND,
                LlmErrorKind::BadRequest,
                "model 'nope' not found",
            ),
        ] {
            let body = r#"{"error":{"message":"model 'nope' not found sk-secret"}}"#;
            let (url, _) = server(status, "application/json", body.into()).await;
            let error = OpenAiCompatProvider::new(url, Some("sk-secret".into()))
                .complete(&request())
                .await
                .unwrap_err();
            assert_eq!(error.kind, kind, "{status}");
            assert!(error.message.contains(expected), "{status}: {error}");
            assert!(!error.message.contains("sk-secret"), "{error}");
        }
    }

    #[tokio::test]
    async fn a_plain_string_error_is_shown_and_an_unreachable_server_names_the_endpoint() {
        let (url, _) = server(
            StatusCode::BAD_REQUEST,
            "application/json",
            r#"{"error":"bad model"}"#.into(),
        )
        .await;
        let error = OpenAiCompatProvider::ollama(url)
            .complete(&request())
            .await
            .unwrap_err();
        assert!(
            error.message.contains("Ollama did not accept") && error.message.contains("bad model"),
            "{error}"
        );

        let error = OpenAiCompatProvider::ollama("http://127.0.0.1:9")
            .complete(&request())
            .await
            .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::Network);
        assert!(
            error.message.contains("Ollama") && error.message.contains("127.0.0.1:9"),
            "{error}"
        );
        assert!(error.is_retryable());
    }

    #[test]
    fn finish_reasons_map_to_the_neutral_ones() {
        assert_eq!(stop_reason(Some("stop")), StopReason::EndTurn);
        assert_eq!(stop_reason(Some("length")), StopReason::MaxTokens);
        assert_eq!(stop_reason(Some("tool_calls")), StopReason::ToolUse);
        assert_eq!(
            stop_reason(Some("content_filter")),
            StopReason::Other("content_filter".into())
        );
    }

    #[test]
    fn the_endpoint_follows_the_address() {
        assert_eq!(
            chat_completions_url("http://localhost:1234"),
            "http://localhost:1234/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("http://localhost:1234/"),
            "http://localhost:1234/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://openrouter.ai/api/v1"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://models.github.ai/inference/"),
            "https://models.github.ai/inference/chat/completions"
        );
    }
}
