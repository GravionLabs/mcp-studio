//! Events the core emits towards the UI. The Tauri layer forwards them; tests collect them.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{recording::Direction, tokens::TokenSource};

/// Connection state of one server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

/// `mcp://status`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StatusEvent {
    pub server_id: String,
    pub session_id: Option<String>,
    pub state: ConnectionState,
    pub message: Option<String>,
}

/// A recorded JSON-RPC message as stored, sent to the UI live and returned by queries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MessageRecord {
    #[specta(type = u32)]
    pub id: i64,
    pub session_id: String,
    pub server_id: String,
    pub direction: Direction,
    pub jsonrpc_id: Option<String>,
    pub method: Option<String>,
    /// The JSON-RPC message as text.
    pub payload: String,
    #[specta(type = u32)]
    pub bytes: i64,
    pub is_error: bool,
    /// Unix milliseconds.
    #[specta(type = u32)]
    pub ts: i64,
    /// For responses: milliseconds since the matching request (filled by queries).
    #[specta(type = Option<u32>)]
    pub duration_ms: Option<i64>,
    /// Tokens this message adds to a model's context (arguments, result, or tool definitions).
    #[specta(type = Option<u32>)]
    pub tokens: Option<i64>,
    /// Whether `tokens` is an offline estimate or an exact count.
    pub token_source: Option<TokenSource>,
    /// The span this message belongs to: its tool call, or the session.
    pub span_id: Option<String>,
}

/// Where a log line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum LogSource {
    /// The server's stderr (stdio servers).
    Stderr,
    /// An MCP `notifications/message` log notification.
    Server,
    /// MCP Studio itself (spawn, reconnect, ...).
    Studio,
}

/// `mcp://log`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LogEvent {
    pub server_id: String,
    pub source: LogSource,
    /// `debug` | `info` | `warning` | `error` (MCP levels are mapped onto these).
    pub level: String,
    pub line: String,
    #[specta(type = u32)]
    pub ts: i64,
}

/// Which list of a server changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ListKind {
    Tools,
    Resources,
    Prompts,
}

/// `mcp://list-changed`: the server sent `notifications/*/list_changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListChangedEvent {
    pub server_id: String,
    pub kind: ListKind,
}

/// `mcp://progress`: progress of a running tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub server_id: String,
    pub call_id: String,
    pub progress: f64,
    pub total: Option<f64>,
    pub message: Option<String>,
}

/// Receiver of core events.
pub trait EventSink: Send + Sync + 'static {
    fn status(&self, event: StatusEvent);
    fn message(&self, event: MessageRecord);
    fn log(&self, event: LogEvent);
    fn list_changed(&self, event: ListChangedEvent);
    fn progress(&self, event: ProgressEvent);
}

/// Ignores everything.
pub struct NullSink;

impl EventSink for NullSink {
    fn status(&self, _: StatusEvent) {}
    fn message(&self, _: MessageRecord) {}
    fn log(&self, _: LogEvent) {}
    fn list_changed(&self, _: ListChangedEvent) {}
    fn progress(&self, _: ProgressEvent) {}
}

/// Collects events in memory (tests).
#[derive(Default)]
pub struct CollectingSink {
    pub statuses: std::sync::Mutex<Vec<StatusEvent>>,
    pub messages: std::sync::Mutex<Vec<MessageRecord>>,
    pub logs: std::sync::Mutex<Vec<LogEvent>>,
    pub list_changes: std::sync::Mutex<Vec<ListChangedEvent>>,
    pub progress: std::sync::Mutex<Vec<ProgressEvent>>,
}

impl EventSink for CollectingSink {
    fn status(&self, event: StatusEvent) {
        self.statuses.lock().unwrap().push(event);
    }
    fn message(&self, event: MessageRecord) {
        self.messages.lock().unwrap().push(event);
    }
    fn log(&self, event: LogEvent) {
        self.logs.lock().unwrap().push(event);
    }
    fn list_changed(&self, event: ListChangedEvent) {
        self.list_changes.lock().unwrap().push(event);
    }
    fn progress(&self, event: ProgressEvent) {
        self.progress.lock().unwrap().push(event);
    }
}
