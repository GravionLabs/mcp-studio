//! Persists recorded messages: batched writes, live events, and retention.

use std::{sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    db::{now_ms, Db, DbResult},
    events::{EventSink, MessageRecord},
    recording::{Direction, RecordedMessage, Recorder},
    secrets::Redactor,
    tokens::{message_tokens, TokenSource},
    trace::{self, MessageInfo, SpanTracker},
};

/// Flush when this many messages are waiting...
const BATCH_SIZE: usize = 200;
/// ...or after this long, whichever comes first.
const BATCH_DELAY: Duration = Duration::from_millis(50);

struct Pending {
    direction: Direction,
    ts: i64,
    jsonrpc_id: Option<String>,
    method: Option<String>,
    payload: String,
    bytes: i64,
    is_error: bool,
    tokens: i64,
    /// Tool name of a `tools/call` request.
    tool_name: Option<String>,
}

/// Sends messages of one session to a background task that writes them in batches.
pub struct MessageWriter {
    recorder: Arc<ChannelRecorder>,
    handle: JoinHandle<()>,
}

struct ChannelRecorder {
    tx: mpsc::UnboundedSender<Pending>,
    redactor: Redactor,
}

impl Recorder for ChannelRecorder {
    fn record(&self, message: RecordedMessage) {
        let pending = Pending::from_message(message, &self.redactor);
        // The writer only stops after `finish`, so a closed channel means the session is over.
        let _ = self.tx.send(pending);
    }
}

impl Pending {
    fn from_message(message: RecordedMessage, redactor: &Redactor) -> Self {
        let payload = &message.payload;
        let jsonrpc_id = payload.get("id").map(|id| match id {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        });
        let method = payload
            .get("method")
            .and_then(|m| m.as_str())
            .map(str::to_owned);
        let is_error = payload.get("error").is_some()
            || payload["result"]["isError"].as_bool().unwrap_or(false);
        let tool_name = (method.as_deref() == Some("tools/call"))
            .then(|| payload["params"]["name"].as_str().map(str::to_owned))
            .flatten();
        let text = redactor.redact(&payload.to_string());
        Self {
            direction: message.direction,
            ts: message.ts,
            jsonrpc_id,
            method,
            bytes: text.len() as i64,
            payload: text,
            is_error,
            tokens: i64::from(message_tokens(payload)),
            tool_name,
        }
    }
}

impl MessageWriter {
    pub fn spawn(
        db: Db,
        session_id: String,
        server_id: String,
        redactor: Redactor,
        sink: Arc<dyn EventSink>,
    ) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Pending>();
        let handle = tokio::spawn(async move {
            let mut tracker = SpanTracker::new(&session_id);
            if let Err(error) = open_session_span(&db, &mut tracker, &session_id, &server_id).await
            {
                tracing_error(&error.to_string());
            }
            let mut batch = Vec::with_capacity(BATCH_SIZE);
            loop {
                // Wait for the first message, then collect more for a short moment.
                let Some(first) = rx.recv().await else { break };
                batch.push(first);
                let deadline = tokio::time::sleep(BATCH_DELAY);
                tokio::pin!(deadline);
                let mut closed = false;
                while batch.len() < BATCH_SIZE {
                    tokio::select! {
                        message = rx.recv() => match message {
                            Some(message) => batch.push(message),
                            None => { closed = true; break }
                        },
                        () = &mut deadline => break,
                    }
                }
                if let Err(error) = write_batch(
                    &db,
                    &session_id,
                    &server_id,
                    &batch,
                    &mut tracker,
                    sink.as_ref(),
                )
                .await
                {
                    tracing_error(&error.to_string());
                }
                batch.clear();
                if closed {
                    break;
                }
            }
            if let Err(error) = close_spans(&db, &mut tracker).await {
                tracing_error(&error.to_string());
            }
        });
        Self {
            recorder: Arc::new(ChannelRecorder { tx, redactor }),
            handle,
        }
    }

    pub fn recorder(&self) -> Arc<dyn Recorder> {
        self.recorder.clone()
    }

    /// Flushes everything still queued and stops the writer. Recorders handed out earlier must be
    /// dropped by the caller first (closing a session drops its transport).
    pub async fn finish(self) {
        drop(self.recorder);
        let _ = self.handle.await;
    }
}

async fn open_session_span(
    db: &Db,
    tracker: &mut SpanTracker,
    session_id: &str,
    server_id: &str,
) -> DbResult<()> {
    let name: Option<String> = sqlx::query_scalar("SELECT name FROM servers WHERE id = ?")
        .bind(server_id)
        .fetch_optional(db.pool())
        .await?;
    let mut conn = db.pool().acquire().await?;
    tracker
        .open_session(
            &mut conn,
            name.as_deref().unwrap_or(session_id),
            server_id,
            now_ms(),
        )
        .await
}

async fn close_spans(db: &Db, tracker: &mut SpanTracker) -> DbResult<()> {
    let mut conn = db.pool().acquire().await?;
    tracker.close(&mut conn, now_ms()).await
}

fn tracing_error(message: &str) {
    eprintln!("mcp-studio: failed to store messages: {message}");
}

async fn write_batch(
    db: &Db,
    session_id: &str,
    server_id: &str,
    batch: &[Pending],
    tracker: &mut SpanTracker,
    sink: &dyn EventSink,
) -> DbResult<()> {
    let mut tx = db.pool().begin().await?;
    let mut records = Vec::with_capacity(batch.len());
    for message in batch {
        let direction = match message.direction {
            Direction::Out => "out",
            Direction::In => "in",
        };
        let span_id = tracker
            .on_message(
                &mut tx,
                &MessageInfo {
                    direction: message.direction,
                    ts: message.ts,
                    jsonrpc_id: message.jsonrpc_id.as_deref(),
                    method: message.method.as_deref(),
                    tool_name: message.tool_name.as_deref(),
                    is_error: message.is_error,
                },
            )
            .await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO messages (session_id, span_id, direction, jsonrpc_id, method, payload, bytes, is_error, tokens, token_source, ts) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(session_id)
        .bind(&span_id)
        .bind(direction)
        .bind(&message.jsonrpc_id)
        .bind(&message.method)
        .bind(&message.payload)
        .bind(message.bytes)
        .bind(message.is_error)
        .bind(message.tokens)
        .bind(TokenSource::Estimate.as_str())
        .bind(message.ts)
        .fetch_one(&mut *tx)
        .await?;
        records.push(MessageRecord {
            id,
            session_id: session_id.to_owned(),
            server_id: server_id.to_owned(),
            direction: message.direction,
            jsonrpc_id: message.jsonrpc_id.clone(),
            method: message.method.clone(),
            payload: message.payload.clone(),
            bytes: message.bytes,
            is_error: message.is_error,
            ts: message.ts,
            duration_ms: None,
            tokens: Some(message.tokens),
            token_source: Some(TokenSource::Estimate),
            span_id,
        });
    }
    tx.commit().await?;
    for record in records {
        sink.message(record);
    }
    Ok(())
}

/// Filters for [`query_messages`]. All fields are optional and combined with AND.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
#[serde(rename_all = "camelCase", default)]
pub struct MessageFilter {
    pub session_id: Option<String>,
    pub server_id: Option<String>,
    /// Only messages with a larger id (used to catch up after a live subscription started).
    #[specta(type = u32)]
    pub after_id: Option<i64>,
    /// Only messages with a smaller id (paging backwards).
    #[specta(type = u32)]
    pub before_id: Option<i64>,
    /// Exact JSON-RPC method, e.g. `tools/call`. Responses match through their request.
    pub method: Option<String>,
    pub direction: Option<Direction>,
    /// Only messages of one span (a tool call or a session).
    pub span_id: Option<String>,
    pub errors_only: bool,
    /// Case-insensitive text search over the raw payload.
    pub search: Option<String>,
    /// Maximum number of rows (default 500, max 5000). The newest matching rows are returned,
    /// oldest first.
    pub limit: Option<u32>,
}

type MessageRow = (
    i64,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    i64,
    bool,
    i64,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<String>,
);

/// Reads recorded messages. Responses carry `duration_ms`, measured from their request.
pub async fn query_messages(db: &Db, filter: &MessageFilter) -> DbResult<Vec<MessageRecord>> {
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT m.id, m.session_id, (SELECT server_id FROM sessions WHERE id = m.session_id), m.direction, m.jsonrpc_id, m.method, m.payload, m.bytes, m.is_error, m.ts, \
         (SELECT m.ts - r.ts FROM messages r WHERE r.session_id = m.session_id AND r.jsonrpc_id = m.jsonrpc_id \
            AND r.method IS NOT NULL AND r.direction != m.direction AND r.id < m.id \
            ORDER BY r.id DESC LIMIT 1) AS duration, m.tokens, m.token_source, m.span_id \
         FROM messages m WHERE 1 = 1",
    );
    if let Some(session) = &filter.session_id {
        query
            .push(" AND m.session_id = ")
            .push_bind(session.clone());
    }
    if let Some(server) = &filter.server_id {
        query
            .push(" AND m.session_id IN (SELECT id FROM sessions WHERE server_id = ")
            .push_bind(server.clone())
            .push(")");
    }
    if let Some(id) = filter.after_id {
        query.push(" AND m.id > ").push_bind(id);
    }
    if let Some(id) = filter.before_id {
        query.push(" AND m.id < ").push_bind(id);
    }
    if let Some(method) = &filter.method {
        query
            .push(" AND (m.method = ")
            .push_bind(method.clone())
            .push(" OR (m.method IS NULL AND EXISTS (SELECT 1 FROM messages q WHERE q.session_id = m.session_id AND q.jsonrpc_id = m.jsonrpc_id AND q.method = ")
            .push_bind(method.clone())
            .push(")))");
    }
    if let Some(span) = &filter.span_id {
        query.push(" AND m.span_id = ").push_bind(span.clone());
    }
    if let Some(direction) = filter.direction {
        query
            .push(" AND m.direction = ")
            .push_bind(match direction {
                Direction::Out => "out",
                Direction::In => "in",
            });
    }
    if filter.errors_only {
        query.push(" AND m.is_error = 1");
    }
    if let Some(text) = filter.search.as_ref().filter(|t| !t.is_empty()) {
        let escaped = text
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        query
            .push(" AND m.payload LIKE ")
            .push_bind(format!("%{escaped}%"))
            .push(" ESCAPE '\\'");
    }
    query
        .push(" ORDER BY m.id DESC LIMIT ")
        .push_bind(i64::from(filter.limit.unwrap_or(500).clamp(1, 5000)));

    let rows: Vec<MessageRow> = query.build_query_as().fetch_all(db.pool()).await?;
    let mut records: Vec<MessageRecord> = rows
        .into_iter()
        .map(
            |(
                id,
                session_id,
                server_id,
                direction,
                jsonrpc_id,
                method,
                payload,
                bytes,
                is_error,
                ts,
                duration,
                tokens,
                token_source,
                span_id,
            )| {
                MessageRecord {
                    id,
                    session_id,
                    server_id,
                    direction: if direction == "in" {
                        Direction::In
                    } else {
                        Direction::Out
                    },
                    jsonrpc_id,
                    method,
                    payload,
                    bytes,
                    is_error,
                    ts,
                    duration_ms: duration,
                    tokens,
                    token_source: token_source.as_deref().and_then(TokenSource::parse),
                    span_id,
                }
            },
        )
        .collect();
    records.reverse();
    Ok(records)
}

/// How long history is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct RetentionPolicy {
    pub max_age_days: u32,
    pub max_messages: u32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_age_days: 30,
            max_messages: 100_000,
        }
    }
}

/// Deletes messages older than the age limit, then the oldest ones beyond the count limit, then
/// finished sessions without messages. Returns the number of deleted messages.
pub async fn cleanup(db: &Db, policy: RetentionPolicy) -> DbResult<u64> {
    let cutoff = now_ms() - i64::from(policy.max_age_days) * 24 * 60 * 60 * 1000;
    let mut deleted = sqlx::query("DELETE FROM messages WHERE ts < ?")
        .bind(cutoff)
        .execute(db.pool())
        .await?
        .rows_affected();
    deleted += sqlx::query(
        "DELETE FROM messages WHERE id <= (SELECT id FROM messages ORDER BY id DESC LIMIT 1 OFFSET ?)",
    )
    .bind(i64::from(policy.max_messages))
    .execute(db.pool())
    .await?
    .rows_affected();
    sqlx::query(
        "DELETE FROM sessions WHERE ended_at IS NOT NULL AND ended_at < ? \
         AND NOT EXISTS (SELECT 1 FROM messages WHERE messages.session_id = sessions.id)",
    )
    .bind(cutoff)
    .execute(db.pool())
    .await?;
    trace::cleanup(db, cutoff).await?;
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::CollectingSink;
    use serde_json::json;

    async fn setup() -> (Db, String) {
        let db = Db::open_in_memory().await.unwrap();
        let now = now_ms();
        sqlx::query("INSERT INTO servers (id, name, transport, command, created_at, updated_at) VALUES ('srv', 'S', 'stdio', 'x', ?, ?)")
            .bind(now).bind(now).execute(db.pool()).await.unwrap();
        sqlx::query("INSERT INTO sessions (id, server_id, origin, started_at) VALUES ('sess', 'srv', 'studio', ?)")
            .bind(now).execute(db.pool()).await.unwrap();
        (db, "sess".to_owned())
    }

    fn message(direction: Direction, payload: serde_json::Value) -> RecordedMessage {
        RecordedMessage {
            direction,
            ts: now_ms(),
            bytes: payload.to_string().len(),
            payload,
        }
    }

    #[tokio::test]
    async fn stores_messages_in_order_and_emits_events() {
        let (db, session) = setup().await;
        let sink = Arc::new(CollectingSink::default());
        let writer = MessageWriter::spawn(
            db.clone(),
            session,
            "srv".into(),
            Redactor::default(),
            sink.clone(),
        );
        let recorder = writer.recorder();
        recorder.record(message(
            Direction::Out,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        ));
        recorder.record(message(
            Direction::In,
            json!({"jsonrpc":"2.0","id":1,"result":{"tools":[]}}),
        ));
        recorder.record(message(
            Direction::In,
            json!({"jsonrpc":"2.0","id":"abc","error":{"code":-1,"message":"no"}}),
        ));
        drop(recorder);
        writer.finish().await;

        let rows: Vec<(String, Option<String>, Option<String>, bool)> = sqlx::query_as(
            "SELECT direction, jsonrpc_id, method, is_error FROM messages ORDER BY id",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            (
                "out".into(),
                Some("1".into()),
                Some("tools/list".into()),
                false
            )
        );
        assert!(!rows[1].3);
        assert_eq!(rows[2], ("in".into(), Some("abc".into()), None, true));

        let events = sink.messages.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert!(events[0].id < events[1].id);
    }

    #[tokio::test]
    async fn redacts_secrets_before_storing() {
        let (db, session) = setup().await;
        let redactor = Redactor::new(["sk-secret".to_owned()]);
        let writer = MessageWriter::spawn(
            db.clone(),
            session,
            "srv".into(),
            redactor,
            Arc::new(CollectingSink::default()),
        );
        writer.recorder().record(message(
            Direction::Out,
            json!({"id":1,"method":"x","params":{"key":"sk-secret"}}),
        ));
        writer.finish().await;
        let payload: String = sqlx::query_scalar("SELECT payload FROM messages")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert!(!payload.contains("sk-secret"), "{payload}");
        assert!(payload.contains(crate::secrets::MASK));
    }

    #[tokio::test]
    async fn flags_tool_errors_in_results() {
        let (db, session) = setup().await;
        let writer = MessageWriter::spawn(
            db.clone(),
            session,
            "srv".into(),
            Redactor::default(),
            Arc::new(CollectingSink::default()),
        );
        writer.recorder().record(message(
            Direction::In,
            json!({"id":2,"result":{"isError":true,"content":[]}}),
        ));
        writer.finish().await;
        let flagged: bool = sqlx::query_scalar("SELECT is_error FROM messages")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert!(flagged);
    }

    #[tokio::test]
    async fn stores_labeled_token_estimates_for_calls_and_definitions() {
        let (db, session) = setup().await;
        let sink = Arc::new(CollectingSink::default());
        let writer = MessageWriter::spawn(
            db.clone(),
            session,
            "srv".into(),
            Redactor::default(),
            sink.clone(),
        );
        let arguments = json!({"message": "hello there"});
        let tools = json!([{"name": "echo", "description": "Echo the message back"}]);
        let recorder = writer.recorder();
        recorder.record(message(
            Direction::Out,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"echo","arguments":arguments}}),
        ));
        recorder.record(message(
            Direction::In,
            json!({"jsonrpc":"2.0","id":2,"result":{"tools":tools}}),
        ));
        recorder.record(message(
            Direction::Out,
            json!({"jsonrpc":"2.0","id":3,"method":"ping"}),
        ));
        drop(recorder);
        writer.finish().await;

        let records = query_messages(&db, &MessageFilter::default())
            .await
            .unwrap();
        let tokens: Vec<_> = records.iter().map(|r| r.tokens).collect();
        assert_eq!(
            tokens,
            [
                Some(i64::from(crate::tokens::estimate_value(&arguments))),
                Some(i64::from(crate::tokens::estimate_value(&tools))),
                Some(0),
            ]
        );
        assert!(records
            .iter()
            .all(|r| r.token_source == Some(TokenSource::Estimate)));
        // Live events carry the same label.
        let events = sink.messages.lock().unwrap();
        assert_eq!(events[0].tokens, tokens[0]);
        assert_eq!(events[0].token_source, Some(TokenSource::Estimate));
    }

    fn timed(direction: Direction, ts: i64, payload: serde_json::Value) -> RecordedMessage {
        RecordedMessage {
            direction,
            ts,
            bytes: payload.to_string().len(),
            payload,
        }
    }

    async fn record_all(db: &Db, session: &str, messages: Vec<RecordedMessage>) {
        let writer = MessageWriter::spawn(
            db.clone(),
            session.to_owned(),
            "srv".into(),
            Redactor::default(),
            Arc::new(CollectingSink::default()),
        );
        let recorder = writer.recorder();
        for message in messages {
            recorder.record(message);
        }
        drop(recorder);
        writer.finish().await;
    }

    #[tokio::test]
    async fn records_the_session_as_a_root_span_named_after_the_server() {
        let (db, session) = setup().await;
        record_all(
            &db,
            &session,
            vec![message(
                Direction::Out,
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            )],
        )
        .await;
        let spans = trace::query_spans(&db, &trace::SpanFilter::default())
            .await
            .unwrap();
        assert_eq!(spans.len(), 1);
        let root = &spans[0];
        assert_eq!(root.kind, trace::SpanKind::Session);
        assert_eq!(root.name, "S");
        assert_eq!(root.trace_id, "sess");
        assert_eq!(root.parent_id, None);
        assert!(root.ended_at.is_some());
        assert_eq!(root.status, trace::SpanStatus::Ok);
        // Non-call messages belong to the session span.
        let owner: Option<String> = sqlx::query_scalar("SELECT span_id FROM messages")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(owner.as_deref(), Some(root.id.as_str()));
    }

    #[tokio::test]
    async fn a_tool_call_becomes_a_child_span_from_request_to_response() {
        let (db, session) = setup().await;
        record_all(
            &db,
            &session,
            vec![
                timed(
                    Direction::Out,
                    1_000,
                    json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"echo","arguments":{"message":"hi"}}}),
                ),
                timed(
                    Direction::In,
                    1_250,
                    json!({"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","text":"hi"}]}}),
                ),
            ],
        )
        .await;
        let spans = trace::query_spans(&db, &trace::SpanFilter::default())
            .await
            .unwrap();
        let call = spans
            .iter()
            .find(|s| s.kind == trace::SpanKind::Tool)
            .unwrap();
        let root = spans
            .iter()
            .find(|s| s.kind == trace::SpanKind::Session)
            .unwrap();
        assert_eq!(call.name, "echo");
        assert_eq!(call.parent_id.as_deref(), Some(root.id.as_str()));
        assert_eq!((call.started_at, call.ended_at), (1_000, Some(1_250)));
        assert_eq!(call.status, trace::SpanStatus::Ok);
        assert!(call.tokens.unwrap() > 0);
        // The session span counts the tokens of the whole session.
        assert!(root.tokens.unwrap() >= call.tokens.unwrap());
        // Request and response both attach to the call span.
        let owners: Vec<Option<String>> =
            sqlx::query_scalar("SELECT span_id FROM messages ORDER BY id")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(owners, vec![Some(call.id.clone()), Some(call.id.clone())]);
    }

    #[tokio::test]
    async fn failed_calls_get_error_status_and_unanswered_calls_are_cancelled() {
        let (db, session) = setup().await;
        record_all(
            &db,
            &session,
            vec![
                timed(
                    Direction::Out,
                    10,
                    json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fail"}}),
                ),
                timed(
                    Direction::In,
                    20,
                    json!({"jsonrpc":"2.0","id":1,"result":{"isError":true,"content":[]}}),
                ),
                timed(
                    Direction::Out,
                    30,
                    json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"sleep"}}),
                ),
                timed(
                    Direction::Out,
                    35,
                    json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"other"}}),
                ),
                // A response in the same direction as the request is not its answer.
                timed(
                    Direction::Out,
                    36,
                    json!({"jsonrpc":"2.0","id":3,"result":{}}),
                ),
            ],
        )
        .await;
        let spans = trace::query_spans(&db, &trace::SpanFilter::default())
            .await
            .unwrap();
        let status = |name: &str| spans.iter().find(|s| s.name == name).unwrap().status;
        assert_eq!(status("fail"), trace::SpanStatus::Error);
        assert_eq!(status("sleep"), trace::SpanStatus::Cancelled);
        assert_eq!(status("other"), trace::SpanStatus::Cancelled);
        assert!(spans.iter().all(|s| s.ended_at.is_some()));
    }

    #[tokio::test]
    async fn filters_messages_by_span_and_spans_by_trace_and_server() {
        let (db, session) = setup().await;
        record_all(
            &db,
            &session,
            vec![
                message(
                    Direction::Out,
                    json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"echo"}}),
                ),
                message(Direction::In, json!({"jsonrpc":"2.0","id":1,"result":{}})),
                message(
                    Direction::Out,
                    json!({"jsonrpc":"2.0","id":2,"method":"ping"}),
                ),
            ],
        )
        .await;
        let by_trace = trace::query_spans(
            &db,
            &trace::SpanFilter {
                trace_id: Some("sess".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(by_trace.len(), 2);
        let none = trace::query_spans(
            &db,
            &trace::SpanFilter {
                trace_id: Some("other".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(none.is_empty());
        let by_server = trace::query_spans(
            &db,
            &trace::SpanFilter {
                server_id: Some("srv".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(by_server.len(), 2);

        let call = by_trace
            .iter()
            .find(|s| s.kind == trace::SpanKind::Tool)
            .unwrap();
        let in_call = query_messages(
            &db,
            &MessageFilter {
                span_id: Some(call.id.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(in_call.len(), 2);
        assert!(in_call
            .iter()
            .all(|m| m.span_id.as_deref() == Some(call.id.as_str())));
    }

    #[tokio::test]
    async fn lists_only_root_spans_newest_first_when_asked() {
        let (db, session) = setup().await;
        record_all(
            &db,
            &session,
            vec![timed(
                Direction::Out,
                1,
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"echo"}}),
            )],
        )
        .await;
        let now = now_ms();
        sqlx::query("INSERT INTO sessions (id, server_id, origin, started_at) VALUES ('later', 'srv', 'studio', ?)")
            .bind(now).execute(db.pool()).await.unwrap();
        record_all(
            &db,
            "later",
            vec![message(
                Direction::Out,
                json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
            )],
        )
        .await;
        let roots = trace::query_spans(
            &db,
            &trace::SpanFilter {
                roots_only: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(roots.iter().all(|s| s.kind == trace::SpanKind::Session));
        assert_eq!(roots.len(), 2);
        // Oldest first within the returned window, like messages.
        assert_eq!(roots[0].trace_id, "sess");
        assert_eq!(roots[1].trace_id, "later");
    }

    #[tokio::test]
    async fn retention_removes_old_traces_without_messages() {
        let (db, session) = setup().await;
        record_all(
            &db,
            &session,
            vec![timed(
                Direction::Out,
                1,
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"echo"}}),
            )],
        )
        .await;
        // Spans of a trace that still has messages stay, however old they are.
        trace::cleanup(&db, now_ms() + 1_000).await.unwrap();
        assert_eq!(
            trace::query_spans(&db, &trace::SpanFilter::default())
                .await
                .unwrap()
                .len(),
            2
        );
        sqlx::query("DELETE FROM messages")
            .execute(db.pool())
            .await
            .unwrap();
        trace::cleanup(&db, now_ms() + 1_000).await.unwrap();
        assert!(trace::query_spans(&db, &trace::SpanFilter::default())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn writes_large_bursts_in_batches() {
        let (db, session) = setup().await;
        let writer = MessageWriter::spawn(
            db.clone(),
            session,
            "srv".into(),
            Redactor::default(),
            Arc::new(CollectingSink::default()),
        );
        let recorder = writer.recorder();
        for i in 0..1000 {
            recorder.record(message(Direction::Out, json!({"id":i,"method":"ping"})));
        }
        drop(recorder);
        writer.finish().await;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 1000);
    }

    async fn seed_conversation(db: &Db) {
        let base = now_ms();
        let rows = [
            (
                "out",
                Some("1"),
                Some("tools/list"),
                json!({"id":1,"method":"tools/list"}),
                false,
                base,
            ),
            (
                "in",
                Some("1"),
                None,
                json!({"id":1,"result":{"tools":[]}}),
                false,
                base + 25,
            ),
            (
                "out",
                Some("2"),
                Some("tools/call"),
                json!({"id":2,"method":"tools/call","params":{"name":"echo"}}),
                false,
                base + 30,
            ),
            (
                "in",
                Some("2"),
                None,
                json!({"id":2,"error":{"message":"boom"}}),
                true,
                base + 90,
            ),
            (
                "in",
                None,
                Some("notifications/message"),
                json!({"method":"notifications/message"}),
                false,
                base + 100,
            ),
        ];
        for (direction, id, method, payload, is_error, ts) in rows {
            sqlx::query("INSERT INTO messages (session_id, direction, jsonrpc_id, method, payload, bytes, is_error, ts) VALUES ('sess', ?, ?, ?, ?, 1, ?, ?)")
                .bind(direction).bind(id).bind(method).bind(payload.to_string()).bind(is_error).bind(ts)
                .execute(db.pool()).await.unwrap();
        }
    }

    #[tokio::test]
    async fn queries_pair_responses_with_their_request_duration() {
        let (db, _) = setup().await;
        seed_conversation(&db).await;
        let all = query_messages(&db, &MessageFilter::default())
            .await
            .unwrap();
        assert_eq!(all.len(), 5);
        assert!(all.windows(2).all(|w| w[0].id < w[1].id));
        assert_eq!(all[0].duration_ms, None);
        assert_eq!(all[1].duration_ms, Some(25));
        assert_eq!(all[3].duration_ms, Some(60));
    }

    #[tokio::test]
    async fn filters_by_method_including_responses() {
        let (db, _) = setup().await;
        seed_conversation(&db).await;
        let calls = query_messages(
            &db,
            &MessageFilter {
                method: Some("tools/call".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].jsonrpc_id.as_deref(), Some("2"));
    }

    #[tokio::test]
    async fn filters_errors_direction_search_and_paging() {
        let (db, _) = setup().await;
        seed_conversation(&db).await;
        let errors = query_messages(
            &db,
            &MessageFilter {
                errors_only: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(errors.len(), 1);
        let incoming = query_messages(
            &db,
            &MessageFilter {
                direction: Some(Direction::In),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(incoming.len(), 3);
        let found = query_messages(
            &db,
            &MessageFilter {
                search: Some("ECHO".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(found.len(), 1);
        let percent = query_messages(
            &db,
            &MessageFilter {
                search: Some("100%".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(percent.is_empty());
        let newest_two = query_messages(
            &db,
            &MessageFilter {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(newest_two.len(), 2);
        let older = query_messages(
            &db,
            &MessageFilter {
                before_id: Some(newest_two[0].id),
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(older.len(), 3);
        let newer = query_messages(
            &db,
            &MessageFilter {
                after_id: Some(all_ids(&db).await[2]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(newer.len(), 2);
    }

    async fn all_ids(db: &Db) -> Vec<i64> {
        sqlx::query_scalar("SELECT id FROM messages ORDER BY id")
            .fetch_all(db.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn filters_by_server() {
        let (db, _) = setup().await;
        seed_conversation(&db).await;
        let by_server = query_messages(
            &db,
            &MessageFilter {
                server_id: Some("srv".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(by_server.len(), 5);
        let none = query_messages(
            &db,
            &MessageFilter {
                server_id: Some("other".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn retention_removes_old_and_excess_messages() {
        let (db, _) = setup().await;
        let day = 24 * 60 * 60 * 1000;
        for (i, age_days) in [40, 35, 1, 0, 0].into_iter().enumerate() {
            sqlx::query("INSERT INTO messages (session_id, direction, payload, bytes, ts) VALUES ('sess', 'out', ?, 1, ?)")
                .bind(format!("m{i}"))
                .bind(now_ms() - age_days * day)
                .execute(db.pool()).await.unwrap();
        }
        let deleted = cleanup(
            &db,
            RetentionPolicy {
                max_age_days: 30,
                max_messages: 100,
            },
        )
        .await
        .unwrap();
        assert_eq!(deleted, 2);
        let deleted = cleanup(
            &db,
            RetentionPolicy {
                max_age_days: 30,
                max_messages: 2,
            },
        )
        .await
        .unwrap();
        assert_eq!(deleted, 1);
        let left: Vec<String> = sqlx::query_scalar("SELECT payload FROM messages ORDER BY id")
            .fetch_all(db.pool())
            .await
            .unwrap();
        assert_eq!(left, ["m3", "m4"]);
    }
}
