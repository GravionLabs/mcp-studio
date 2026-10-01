//! Spans: every session and every tool call is recorded as a span so a session can be shown as a
//! waterfall and exported to OpenTelemetry.
//!
//! A session is one trace (its `trace_id` is the session id). The session span is the root; each
//! `tools/call` is a child span of kind `tool` that starts at the request and ends at the matching
//! response. Every recorded message points at the span it belongs to (`messages.span_id`): call
//! messages at their call span, everything else at the session span.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqliteConnection;

use crate::{
    db::{new_id, Db, DbResult},
    model::JsonValue,
    recording::Direction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum SpanKind {
    Session,
    Flow,
    Step,
    Llm,
    Tool,
}

impl SpanKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SpanKind::Session => "session",
            SpanKind::Flow => "flow",
            SpanKind::Step => "step",
            SpanKind::Llm => "llm",
            SpanKind::Tool => "tool",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "flow" => SpanKind::Flow,
            "step" => SpanKind::Step,
            "llm" => SpanKind::Llm,
            "tool" => SpanKind::Tool,
            _ => SpanKind::Session,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum SpanStatus {
    Ok,
    Error,
    /// The span was still open when its session ended (for example a cancelled call).
    Cancelled,
}

impl SpanStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SpanStatus::Ok => "ok",
            SpanStatus::Error => "error",
            SpanStatus::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "error" => SpanStatus::Error,
            "cancelled" => SpanStatus::Cancelled,
            _ => SpanStatus::Ok,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    pub id: String,
    pub trace_id: String,
    pub parent_id: Option<String>,
    pub kind: SpanKind,
    pub name: String,
    /// Unix milliseconds.
    #[specta(type = u32)]
    pub started_at: i64,
    /// Unix milliseconds; `None` while the span is still open.
    #[specta(type = Option<u32>)]
    pub ended_at: Option<i64>,
    pub status: SpanStatus,
    pub attributes: JsonValue,
    /// Estimated tokens of the messages in this span (a session span counts the whole session).
    #[specta(type = Option<u32>)]
    pub tokens: Option<i64>,
}

/// Filters for [`query_spans`]. Fields are combined with AND.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct SpanFilter {
    /// Spans of one trace (for sessions: the session id).
    pub trace_id: Option<String>,
    /// Spans of the sessions of one server.
    pub server_id: Option<String>,
    /// Maximum number of rows (default 1000, max 10000), newest first.
    pub limit: Option<u32>,
}

type SpanRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    i64,
    Option<i64>,
    String,
    String,
    Option<i64>,
);

/// Reads spans, ordered by start time (a parent comes before its children).
pub async fn query_spans(db: &Db, filter: &SpanFilter) -> DbResult<Vec<Span>> {
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT s.id, s.trace_id, s.parent_id, s.kind, s.name, s.started_at, s.ended_at, s.status, s.attributes, \
         CASE WHEN s.kind = 'session' \
           THEN (SELECT SUM(m.tokens) FROM messages m WHERE m.session_id = s.trace_id) \
           ELSE (SELECT SUM(m.tokens) FROM messages m WHERE m.span_id = s.id) END AS tokens \
         FROM spans s WHERE 1 = 1",
    );
    if let Some(trace) = &filter.trace_id {
        query.push(" AND s.trace_id = ").push_bind(trace.clone());
    }
    if let Some(server) = &filter.server_id {
        query
            .push(" AND s.trace_id IN (SELECT id FROM sessions WHERE server_id = ")
            .push_bind(server.clone())
            .push(")");
    }
    query
        .push(" ORDER BY s.started_at DESC, s.rowid DESC LIMIT ")
        .push_bind(i64::from(filter.limit.unwrap_or(1000).clamp(1, 10_000)));
    let rows: Vec<SpanRow> = query.build_query_as().fetch_all(db.pool()).await?;
    let mut spans: Vec<Span> = rows
        .into_iter()
        .map(
            |(
                id,
                trace_id,
                parent_id,
                kind,
                name,
                started_at,
                ended_at,
                status,
                attributes,
                tokens,
            )| {
                Span {
                    id,
                    trace_id,
                    parent_id,
                    kind: SpanKind::parse(&kind),
                    name,
                    started_at,
                    ended_at,
                    status: SpanStatus::parse(&status),
                    attributes: JsonValue(serde_json::from_str(&attributes).unwrap_or_default()),
                    tokens,
                }
            },
        )
        .collect();
    spans.reverse();
    Ok(spans)
}

/// The parts of a recorded message the tracker needs.
pub struct MessageInfo<'a> {
    pub direction: Direction,
    pub ts: i64,
    pub jsonrpc_id: Option<&'a str>,
    pub method: Option<&'a str>,
    /// Name of the tool for `tools/call` requests.
    pub tool_name: Option<&'a str>,
    pub is_error: bool,
}

struct OpenCall {
    span_id: String,
    /// Direction of the request; the response travels the other way.
    direction: Direction,
}

/// Creates and closes the spans of one session while its messages are written.
pub struct SpanTracker {
    session_id: String,
    session_span: String,
    open: HashMap<String, OpenCall>,
    /// False when the session span could not be stored; messages then carry no span.
    enabled: bool,
}

impl SpanTracker {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_owned(),
            session_span: new_id(),
            open: HashMap::new(),
            enabled: false,
        }
    }

    /// Stores the session span. On failure the tracker stays disabled.
    pub async fn open_session(
        &mut self,
        conn: &mut SqliteConnection,
        name: &str,
        server_id: &str,
        ts: i64,
    ) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO spans (id, trace_id, parent_id, kind, name, started_at, status, attributes) \
             VALUES (?, ?, NULL, 'session', ?, ?, 'ok', ?)",
        )
        .bind(&self.session_span)
        .bind(&self.session_id)
        .bind(name)
        .bind(ts)
        .bind(serde_json::json!({ "serverId": server_id, "sessionId": self.session_id }).to_string())
        .execute(conn)
        .await?;
        self.enabled = true;
        Ok(())
    }

    /// Returns the span a message belongs to, starting a call span for a `tools/call` request and
    /// ending it at the matching response.
    pub async fn on_message(
        &mut self,
        conn: &mut SqliteConnection,
        message: &MessageInfo<'_>,
    ) -> DbResult<Option<String>> {
        if !self.enabled {
            return Ok(None);
        }
        let Some(id) = message.jsonrpc_id else {
            return Ok(Some(self.session_span.clone()));
        };
        if message.method == Some("tools/call") {
            let span_id = new_id();
            let tool = message.tool_name.unwrap_or("tools/call");
            sqlx::query(
                "INSERT INTO spans (id, trace_id, parent_id, kind, name, started_at, status, attributes) \
                 VALUES (?, ?, ?, 'tool', ?, ?, 'ok', ?)",
            )
            .bind(&span_id)
            .bind(&self.session_id)
            .bind(&self.session_span)
            .bind(tool)
            .bind(message.ts)
            .bind(serde_json::json!({ "tool": tool, "jsonrpcId": id }).to_string())
            .execute(conn)
            .await?;
            self.open.insert(
                id.to_owned(),
                OpenCall {
                    span_id: span_id.clone(),
                    direction: message.direction,
                },
            );
            return Ok(Some(span_id));
        }
        if message.method.is_none() {
            if let Some(call) = self.open.get(id) {
                if call.direction != message.direction {
                    let span_id = call.span_id.clone();
                    self.open.remove(id);
                    let status = if message.is_error {
                        SpanStatus::Error
                    } else {
                        SpanStatus::Ok
                    };
                    sqlx::query("UPDATE spans SET ended_at = ?, status = ? WHERE id = ?")
                        .bind(message.ts)
                        .bind(status.as_str())
                        .bind(&span_id)
                        .execute(conn)
                        .await?;
                    return Ok(Some(span_id));
                }
            }
        }
        Ok(Some(self.session_span.clone()))
    }

    /// Ends calls that never got a response (as cancelled) and the session span.
    pub async fn close(&mut self, conn: &mut SqliteConnection, ts: i64) -> DbResult<()> {
        if !self.enabled {
            return Ok(());
        }
        for (_, call) in self.open.drain() {
            sqlx::query("UPDATE spans SET ended_at = ?, status = 'cancelled' WHERE id = ?")
                .bind(ts)
                .bind(call.span_id)
                .execute(&mut *conn)
                .await?;
        }
        sqlx::query("UPDATE spans SET ended_at = ? WHERE id = ?")
            .bind(ts)
            .bind(&self.session_span)
            .execute(conn)
            .await?;
        Ok(())
    }
}

/// Removes old traces: root spans past `cutoff` whose session no longer has messages. Child spans
/// go with their parent.
pub async fn cleanup(db: &Db, cutoff: i64) -> DbResult<u64> {
    Ok(sqlx::query(
        "DELETE FROM spans WHERE parent_id IS NULL AND started_at < ? \
         AND NOT EXISTS (SELECT 1 FROM messages WHERE messages.session_id = spans.trace_id)",
    )
    .bind(cutoff)
    .execute(db.pool())
    .await?
    .rows_affected())
}
