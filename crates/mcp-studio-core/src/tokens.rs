//! Offline token estimates for MCP messages.
//!
//! Real tokenizers differ per model and need large vocabularies, so this module only approximates:
//! it is meant to show *where* context goes (which tool definition or result is large), not to bill.
//! Everything computed here is stored with [`TokenSource::Estimate`] and labeled as an estimate in the
//! UI. Exact counts come from the provider's token-counting endpoint (a later PBI).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

/// Where a token count comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum TokenSource {
    /// Approximated offline; see [`estimate_text`].
    Estimate,
    /// Reported by the model provider.
    Exact,
}

impl TokenSource {
    /// The value stored in `messages.token_source`.
    pub fn as_str(self) -> &'static str {
        match self {
            TokenSource::Estimate => "estimate",
            TokenSource::Exact => "exact",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "estimate" => Some(TokenSource::Estimate),
            "exact" => Some(TokenSource::Exact),
            _ => None,
        }
    }
}

/// Estimates the tokens of a text.
///
/// Runs of ASCII letters and digits cost one token per four characters (rounded up), every other
/// visible character (punctuation, symbols, non-ASCII letters) costs one token, and whitespace is
/// free because tokenizers merge it into neighbouring tokens. This tracks common BPE tokenizers
/// within roughly 20 % for English text and JSON, and is deterministic.
pub fn estimate_text(text: &str) -> u32 {
    let mut tokens: u64 = 0;
    let mut run: u64 = 0;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            run += 1;
            continue;
        }
        tokens += run.div_ceil(4);
        run = 0;
        if !c.is_whitespace() {
            tokens += 1;
        }
    }
    tokens += run.div_ceil(4);
    u32::try_from(tokens).unwrap_or(u32::MAX)
}

/// Estimates the tokens of a JSON value as a client would put it into the context window
/// (compact serialization).
pub fn estimate_value(value: &Value) -> u32 {
    estimate_text(&value.to_string())
}

/// Estimates the tokens a JSON-RPC message adds to a model's context, ignoring the envelope
/// (`jsonrpc`, `id`):
///
/// - `tools/call` request: the arguments
/// - `tools/list` response: the tool definitions
/// - other responses: the `result`; error responses: the `error`
/// - other requests and notifications: the `params`
pub fn message_tokens(payload: &Value) -> u32 {
    let content = if payload.get("method").and_then(Value::as_str) == Some("tools/call") {
        payload["params"].get("arguments")
    } else if let Some(result) = payload.get("result") {
        Some(result.get("tools").unwrap_or(result))
    } else if let Some(error) = payload.get("error") {
        Some(error)
    } else {
        payload.get("params")
    };
    content.map_or(0, estimate_value)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn empty_and_whitespace_texts_are_free() {
        assert_eq!(estimate_text(""), 0);
        assert_eq!(estimate_text(" \n\t "), 0);
    }

    #[test]
    fn words_cost_one_token_per_four_characters() {
        assert_eq!(estimate_text("hi"), 1);
        assert_eq!(estimate_text("abcd"), 1);
        assert_eq!(estimate_text("abcde"), 2);
        assert_eq!(estimate_text("hello world"), 4);
    }

    #[test]
    fn punctuation_and_non_ascii_cost_one_token_each() {
        assert_eq!(estimate_text("{}"), 2);
        assert_eq!(estimate_text("a,b"), 3);
        assert_eq!(estimate_text("日本語"), 3);
    }

    #[test]
    fn longer_text_costs_more() {
        let short = estimate_text("list the open issues");
        let long = estimate_text("list the open issues of this repository and summarize them");
        assert!(long > short);
    }

    #[test]
    fn json_values_are_measured_compactly() {
        // {"a":1} -> { " a " : 1 } = 7 punctuation/quote characters + two runs
        assert_eq!(estimate_value(&json!({ "a": 1 })), 7);
    }

    #[test]
    fn tools_call_requests_count_only_the_arguments() {
        let request = json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": "echo", "arguments": { "message": "hello" } }
        });
        assert_eq!(
            message_tokens(&request),
            estimate_value(&json!({ "message": "hello" }))
        );
    }

    #[test]
    fn tools_list_responses_count_the_tool_definitions() {
        let tools = json!([{ "name": "echo", "description": "Echo the message back" }]);
        let response = json!({ "jsonrpc": "2.0", "id": 2, "result": { "tools": tools } });
        assert_eq!(message_tokens(&response), estimate_value(&tools));
    }

    #[test]
    fn call_results_errors_and_notifications() {
        let result = json!({ "content": [{ "type": "text", "text": "done" }] });
        let response = json!({ "jsonrpc": "2.0", "id": 3, "result": result });
        assert_eq!(message_tokens(&response), estimate_value(&result));

        let error = json!({ "code": -32601, "message": "Method not found" });
        let failed = json!({ "jsonrpc": "2.0", "id": 4, "error": error });
        assert_eq!(message_tokens(&failed), estimate_value(&error));

        let notification = json!({
            "jsonrpc": "2.0", "method": "notifications/progress", "params": { "progress": 1 }
        });
        assert_eq!(
            message_tokens(&notification),
            estimate_value(&json!({ "progress": 1 }))
        );
    }

    #[test]
    fn messages_without_content_cost_nothing() {
        assert_eq!(
            message_tokens(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" })),
            0
        );
    }

    #[test]
    fn token_source_round_trips_through_its_stored_value() {
        for source in [TokenSource::Estimate, TokenSource::Exact] {
            assert_eq!(TokenSource::parse(source.as_str()), Some(source));
        }
        assert_eq!(TokenSource::parse("other"), None);
    }
}
