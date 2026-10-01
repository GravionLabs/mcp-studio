//! What a server and a session cost in tokens and money.
//!
//! - **Context cost** of a server: the tokens its tool definitions take in a model's context. A
//!   client sends them with every request, so the money shown is the cost of *one* request.
//! - **Session usage**: the tool calls of one session. Arguments are tokens the model writes
//!   (output price); results and tool definitions are tokens it reads (input price).
//!
//! All counts are offline estimates (see [`crate::tokens`]) and are labeled as such.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;

use crate::{
    db::{Db, DbResult},
    explorer::ToolInfo,
    prices::{Cost, Price},
    tokens::{estimate_value, TokenSource},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolCost {
    pub name: String,
    pub tokens: u32,
}

/// The context a server's tool definitions take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContextCost {
    pub tokens: u32,
    /// Largest definitions first.
    pub tools: Vec<ToolCost>,
    pub token_source: TokenSource,
    /// Cost of sending the definitions once, when a price was chosen.
    pub cost: Option<Cost>,
}

/// Tokens of the tool definitions as a client places them into the context window: name,
/// description, and input schema.
pub fn context_cost(tools: &[ToolInfo], price: Option<&Price>) -> ContextCost {
    let mut per_tool: Vec<ToolCost> = tools
        .iter()
        .map(|tool| ToolCost {
            name: tool.name.clone(),
            tokens: estimate_value(&json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.input_schema.0,
            })),
        })
        .collect();
    per_tool.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.name.cmp(&b.name)));
    let tokens = per_tool.iter().map(|t| t.tokens).sum();
    ContextCost {
        tokens,
        tools: per_tool,
        token_source: TokenSource::Estimate,
        cost: price.map(|p| Cost {
            amount: p.input_cost(u64::from(tokens)),
            currency: p.currency.clone(),
        }),
    }
}

/// Tokens and cost of one session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsage {
    pub calls: u32,
    /// Tokens of tool call arguments.
    pub argument_tokens: u32,
    /// Tokens of tool call results.
    pub result_tokens: u32,
    /// Tokens of tool lists received (counted once per `tools/list` response).
    pub definition_tokens: u32,
    pub token_source: TokenSource,
    pub cost: Option<Cost>,
}

impl SessionUsage {
    pub fn total_tokens(&self) -> u32 {
        self.argument_tokens + self.result_tokens + self.definition_tokens
    }

    fn priced(mut self, price: Option<&Price>) -> Self {
        self.cost = price.map(|p| Cost {
            amount: p.output_cost(u64::from(self.argument_tokens))
                + p.input_cost(u64::from(self.result_tokens) + u64::from(self.definition_tokens)),
            currency: p.currency.clone(),
        });
        self
    }
}

type Row = (Option<String>, Option<i64>, Option<String>);

/// Adds up the stored token estimates of a session.
pub async fn session_usage(
    db: &Db,
    session_id: &str,
    price: Option<&Price>,
) -> DbResult<SessionUsage> {
    // A response carries no method; it belongs to the closest earlier request with its id.
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT m.method, m.tokens, \
           (SELECT r.method FROM messages r WHERE r.session_id = m.session_id \
              AND r.jsonrpc_id = m.jsonrpc_id AND r.method IS NOT NULL \
              AND r.direction != m.direction AND r.id < m.id ORDER BY r.id DESC LIMIT 1) \
         FROM messages m WHERE m.session_id = ? AND m.tokens IS NOT NULL",
    )
    .bind(session_id)
    .fetch_all(db.pool())
    .await?;

    let mut by_kind: HashMap<&str, u64> = HashMap::new();
    let mut calls = 0u32;
    for (method, tokens, request_method) in &rows {
        let tokens = u64::try_from(tokens.unwrap_or(0)).unwrap_or(0);
        match (method.as_deref(), request_method.as_deref()) {
            (Some("tools/call"), _) => {
                calls += 1;
                *by_kind.entry("arguments").or_default() += tokens;
            }
            (None, Some("tools/call")) => *by_kind.entry("results").or_default() += tokens,
            (None, Some("tools/list")) => *by_kind.entry("definitions").or_default() += tokens,
            _ => {}
        }
    }
    let get =
        |kind: &str| u32::try_from(by_kind.get(kind).copied().unwrap_or(0)).unwrap_or(u32::MAX);
    Ok(SessionUsage {
        calls,
        argument_tokens: get("arguments"),
        result_tokens: get("results"),
        definition_tokens: get("definitions"),
        token_source: TokenSource::Estimate,
        cost: None,
    }
    .priced(price))
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;
    use crate::{
        db::now_ms,
        model::JsonValue,
        tokens::{estimate_value, message_tokens},
    };

    fn tool(name: &str, description: &str, schema: Value) -> ToolInfo {
        ToolInfo {
            name: name.into(),
            title: None,
            description: Some(description.into()),
            input_schema: JsonValue(schema),
            output_schema: None,
            annotations: None,
        }
    }

    fn price() -> Price {
        Price {
            model: "m".into(),
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: None,
            cache_write_per_mtok: None,
            currency: "USD".into(),
        }
    }

    #[test]
    fn context_cost_sums_definitions_and_lists_largest_first() {
        let small = tool("a", "Short", json!({"type": "object"}));
        let large = tool(
            "b",
            "A much longer description that explains in detail what this tool does",
            json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        );
        let cost = context_cost(&[small.clone(), large.clone()], None);
        assert_eq!(cost.tools[0].name, "b");
        assert!(cost.tools[0].tokens > cost.tools[1].tokens);
        assert_eq!(
            cost.tokens,
            cost.tools.iter().map(|t| t.tokens).sum::<u32>()
        );
        assert_eq!(cost.token_source, TokenSource::Estimate);
        assert_eq!(cost.cost, None);
    }

    #[test]
    fn context_cost_uses_the_input_price() {
        let tools = [tool("a", "Echo", json!({"type": "object"}))];
        let cost = context_cost(&tools, Some(&price()));
        let expected = f64::from(cost.tokens) * 3.0 / 1_000_000.0;
        assert!((cost.cost.unwrap().amount - expected).abs() < 1e-12);
    }

    #[test]
    fn a_server_without_tools_costs_nothing() {
        let cost = context_cost(&[], Some(&price()));
        assert_eq!(cost.tokens, 0);
        assert_eq!(cost.cost.unwrap().amount, 0.0);
    }

    async fn seed(db: &Db, messages: &[(&str, Value)]) {
        let now = now_ms();
        sqlx::query("INSERT INTO servers (id, name, transport, command, created_at, updated_at) VALUES ('srv', 'S', 'stdio', 'x', ?, ?)")
            .bind(now).bind(now).execute(db.pool()).await.unwrap();
        sqlx::query("INSERT INTO sessions (id, server_id, origin, started_at) VALUES ('sess', 'srv', 'studio', ?)")
            .bind(now).execute(db.pool()).await.unwrap();
        for (direction, payload) in messages {
            let method = payload.get("method").and_then(Value::as_str);
            let id = payload.get("id").map(Value::to_string);
            sqlx::query("INSERT INTO messages (session_id, direction, jsonrpc_id, method, payload, bytes, tokens, token_source, ts) VALUES ('sess', ?, ?, ?, ?, 1, ?, 'estimate', ?)")
                .bind(direction).bind(id).bind(method).bind(payload.to_string())
                .bind(i64::from(message_tokens(payload))).bind(now)
                .execute(db.pool()).await.unwrap();
        }
    }

    #[tokio::test]
    async fn session_usage_separates_arguments_results_and_definitions() {
        let db = Db::open_in_memory().await.unwrap();
        let args = json!({"message": "hello there"});
        let result = json!({"content": [{"type": "text", "text": "hello there"}]});
        let tools = json!([{"name": "echo", "description": "Echo it"}]);
        seed(
            &db,
            &[
                ("out", json!({"id": 1, "method": "tools/list"})),
                ("in", json!({"id": 1, "result": {"tools": tools}})),
                (
                    "out",
                    json!({"id": 2, "method": "tools/call", "params": {"name": "echo", "arguments": args}}),
                ),
                ("in", json!({"id": 2, "result": result})),
                ("out", json!({"id": 3, "method": "ping"})),
            ],
        )
        .await;

        let usage = session_usage(&db, "sess", Some(&price())).await.unwrap();
        assert_eq!(usage.calls, 1);
        assert_eq!(usage.argument_tokens, estimate_value(&args));
        assert_eq!(usage.result_tokens, estimate_value(&result));
        assert_eq!(usage.definition_tokens, estimate_value(&tools));
        let expected = f64::from(usage.argument_tokens) * 15.0 / 1e6
            + f64::from(usage.result_tokens + usage.definition_tokens) * 3.0 / 1e6;
        assert!((usage.cost.clone().unwrap().amount - expected).abs() < 1e-12);
        assert_eq!(
            usage.total_tokens(),
            usage.argument_tokens + usage.result_tokens + usage.definition_tokens
        );
    }

    #[tokio::test]
    async fn session_usage_is_empty_without_calls_and_unpriced_without_a_price() {
        let db = Db::open_in_memory().await.unwrap();
        seed(&db, &[("out", json!({"id": 1, "method": "ping"}))]).await;
        let usage = session_usage(&db, "sess", None).await.unwrap();
        assert_eq!(usage.calls, 0);
        assert_eq!(usage.total_tokens(), 0);
        assert_eq!(usage.cost, None);
        let other = session_usage(&db, "missing", None).await.unwrap();
        assert_eq!(other.calls, 0);
    }
}
