//! Optional export of spans to an OpenTelemetry collector (OTLP over HTTP, JSON encoding).
//!
//! Off by default. Once enabled it sends only spans recorded afterwards, in batches, and only
//! metadata: names, ids, timing, status, and estimated token counts. Message payloads, tool
//! arguments, and results never leave the app. Header values that are secrets (an API key for a
//! hosted backend) are `keyring:` references and are resolved only when a request is sent.
//!
//! Span names and attributes follow the OpenTelemetry semantic conventions for MCP and GenAI tool
//! calls (`mcp.method.name`, `gen_ai.operation.name = execute_tool`, `gen_ai.tool.name`,
//! `mcp.session.id`); MCP Studio specific values use the `mcpstudio.` prefix.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use url::Url;

use crate::{
    db::{now_ms, Db, DbError, DbResult},
    secrets::{resolve_values, Redactor, SecretStore},
    settings::Settings,
    trace::{self, Span, SpanKind, SpanStatus},
};

const SETTINGS_KEY: &str = "trace_export";
const BATCH_SIZE: u32 = 500;
/// Batches sent per export run, so one run cannot go on forever.
const MAX_BATCHES: usize = 20;
const INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportConfig {
    pub enabled: bool,
    /// Collector address, for example `http://localhost:4318`; `/v1/traces` is added when missing.
    pub endpoint: String,
    /// Extra HTTP headers. Values are plain text or `keyring:` references.
    pub headers: BTreeMap<String, String>,
}

impl ExportConfig {
    pub fn normalized(mut self) -> DbResult<Self> {
        self.endpoint = self.endpoint.trim().to_owned();
        if self.enabled || !self.endpoint.is_empty() {
            traces_url(&self.endpoint)?;
        }
        let mut headers = BTreeMap::new();
        for (name, value) in self.headers {
            let name = name.trim().to_owned();
            if name.is_empty() {
                continue;
            }
            http::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| DbError::Invalid(format!("\"{name}\" is not a valid header name")))?;
            headers.insert(name, value);
        }
        self.headers = headers;
        Ok(self)
    }
}

/// The URL spans are posted to.
pub fn traces_url(endpoint: &str) -> DbResult<Url> {
    let mut url = Url::parse(endpoint.trim()).map_err(|_| {
        DbError::Invalid("the endpoint must be a URL such as http://localhost:4318".into())
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(DbError::Invalid(
            "the endpoint must start with http:// or https://".into(),
        ));
    }
    let path = url.path().trim_end_matches('/').to_owned();
    if !path.ends_with("/v1/traces") {
        url.set_path(&format!("{path}/v1/traces"));
    }
    Ok(url)
}

/// How the exporter is doing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportStatus {
    #[specta(type = Option<u32>)]
    pub last_attempt_at: Option<i64>,
    #[specta(type = Option<u32>)]
    pub last_success_at: Option<i64>,
    pub last_error: Option<String>,
    /// Spans sent since the app started.
    pub exported: u32,
}

/// 32 hex characters for a trace id (session ids are UUIDs; anything else is hashed).
pub fn trace_id_hex(id: &str) -> String {
    match uuid::Uuid::parse_str(id) {
        Ok(uuid) => uuid.simple().to_string(),
        Err(_) => format!(
            "{:016x}{:016x}",
            fnv(id, 0xcbf2_9ce4_8422_2325),
            fnv(id, 0x1000_0000_01b3)
        ),
    }
}

/// 16 hex characters for a span id.
pub fn span_id_hex(id: &str) -> String {
    match uuid::Uuid::parse_str(id) {
        Ok(uuid) => uuid.simple().to_string()[..16].to_owned(),
        Err(_) => format!("{:016x}", fnv(id, 0xcbf2_9ce4_8422_2325)),
    }
}

fn fnv(text: &str, seed: u64) -> u64 {
    text.bytes().fold(seed, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn string(key: &str, value: &str) -> Value {
    json!({ "key": key, "value": { "stringValue": value } })
}

fn int(key: &str, value: i64) -> Value {
    json!({ "key": key, "value": { "intValue": value.to_string() } })
}

fn nanos(ms: i64) -> String {
    (i128::from(ms) * 1_000_000).to_string()
}

fn span_to_otlp(span: &Span, session_id: &str) -> Option<Value> {
    let ended = span.ended_at?;
    let mut attributes = vec![string("mcp.session.id", session_id)];
    let (name, kind) = match span.kind {
        SpanKind::Tool => {
            attributes.push(string("mcp.method.name", "tools/call"));
            attributes.push(string("gen_ai.operation.name", "execute_tool"));
            attributes.push(string("gen_ai.tool.name", &span.name));
            if let Some(id) = span.attributes.0.get("jsonrpcId").and_then(Value::as_str) {
                attributes.push(string("jsonrpc.request.id", id));
            }
            if span.status == SpanStatus::Error {
                attributes.push(string("error.type", "tool_error"));
            }
            (format!("tools/call {}", span.name), 3)
        }
        _ => {
            if let Some(server) = span.attributes.0.get("serverId").and_then(Value::as_str) {
                attributes.push(string("mcpstudio.server.id", server));
            }
            (format!("session {}", span.name), 1)
        }
    };
    if let Some(tokens) = span.tokens {
        attributes.push(int("mcpstudio.tokens.estimated", tokens));
    }
    let status = match span.status {
        SpanStatus::Ok => json!({ "code": 1 }),
        SpanStatus::Error => json!({ "code": 2, "message": "tool call failed" }),
        SpanStatus::Cancelled => json!({ "code": 2, "message": "cancelled" }),
    };
    let mut value = json!({
        "traceId": trace_id_hex(&span.trace_id),
        "spanId": span_id_hex(&span.id),
        "name": name,
        "kind": kind,
        "startTimeUnixNano": nanos(span.started_at),
        "endTimeUnixNano": nanos(ended),
        "attributes": attributes,
        "status": status,
    });
    if let Some(parent) = &span.parent_id {
        value["parentSpanId"] = json!(span_id_hex(parent));
    }
    Some(value)
}

/// Builds the OTLP/JSON request body for finished spans. Open spans are skipped.
pub fn to_otlp(spans: &[Span]) -> Value {
    let converted: Vec<Value> = spans
        .iter()
        .filter_map(|span| span_to_otlp(span, &span.trace_id))
        .collect();
    json!({
        "resourceSpans": [{
            "resource": { "attributes": [
                string("service.name", "mcp-studio"),
                string("service.version", crate::version()),
            ] },
            "scopeSpans": [{
                "scope": { "name": "mcp-studio", "version": crate::version() },
                "spans": converted,
            }],
        }]
    })
}

/// Sends spans to the configured collector. One instance runs for the lifetime of the app.
pub struct TraceExporter {
    db: Db,
    settings: Settings,
    secrets: Arc<dyn SecretStore>,
    client: reqwest::Client,
    status: Mutex<ExportStatus>,
    /// Only one run at a time, so a manual export cannot overlap the background one.
    running: tokio::sync::Mutex<()>,
}

impl TraceExporter {
    pub fn new(db: Db, secrets: Arc<dyn SecretStore>) -> Arc<Self> {
        Arc::new(Self {
            settings: Settings::new(db.clone()),
            db,
            secrets,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            status: Mutex::new(ExportStatus::default()),
            running: tokio::sync::Mutex::new(()),
        })
    }

    /// Exports every few seconds while enabled. Runs forever; spawn it on the app's runtime.
    pub async fn run(self: Arc<Self>) {
        loop {
            tokio::time::sleep(INTERVAL).await;
            let _ = self.export_now().await;
        }
    }

    pub async fn config(&self) -> DbResult<ExportConfig> {
        Ok(match self.settings.get(SETTINGS_KEY).await? {
            Some(text) => serde_json::from_str(&text).unwrap_or_default(),
            None => ExportConfig::default(),
        })
    }

    /// Saves the configuration. Turning the exporter on skips what was recorded before.
    pub async fn set_config(&self, config: ExportConfig) -> DbResult<ExportConfig> {
        let config = config.normalized()?;
        let previous = self.config().await?;
        if config.enabled && !previous.enabled {
            trace::mark_all_exported(&self.db).await?;
        }
        let text = serde_json::to_string(&config)
            .map_err(|e| DbError::Invalid(format!("could not save the configuration: {e}")))?;
        self.settings.set(SETTINGS_KEY, &text).await?;
        Ok(config)
    }

    pub fn status(&self) -> ExportStatus {
        self.status.lock().unwrap().clone()
    }

    /// Sends everything that is waiting. Does nothing while the exporter is disabled.
    pub async fn export_now(&self) -> ExportStatus {
        let _guard = self.running.lock().await;
        let config = match self.config().await {
            Ok(config) if config.enabled => config,
            _ => return self.status(),
        };
        let result = self.send_pending(&config).await;
        let mut status = self.status.lock().unwrap();
        match result {
            Ok(sent) => {
                if sent > 0 {
                    status.last_attempt_at = Some(now_ms());
                    status.last_success_at = Some(now_ms());
                    status.last_error = None;
                    status.exported = status.exported.saturating_add(sent);
                }
            }
            Err(message) => {
                status.last_attempt_at = Some(now_ms());
                status.last_error = Some(message);
            }
        }
        status.clone()
    }

    async fn send_pending(&self, config: &ExportConfig) -> Result<u32, String> {
        let url = traces_url(&config.endpoint).map_err(|e| e.to_string())?;
        let (headers, secrets) =
            resolve_values(self.secrets.as_ref(), &config.headers).map_err(|e| e.to_string())?;
        let redactor = Redactor::new(secrets);
        let mut sent = 0u32;
        for _ in 0..MAX_BATCHES {
            let spans = trace::unexported_spans(&self.db, BATCH_SIZE)
                .await
                .map_err(|e| e.to_string())?;
            if spans.is_empty() {
                break;
            }
            let body = serde_json::to_vec(&to_otlp(&spans)).map_err(|e| e.to_string())?;
            let mut request = self
                .client
                .post(url.clone())
                .header("content-type", "application/json")
                .body(body);
            for (name, value) in &headers {
                request = request.header(name, value);
            }
            let response = request
                .send()
                .await
                .map_err(|e| redactor.redact(&format!("could not reach the collector: {e}")))?;
            if !response.status().is_success() {
                return Err(
                    redactor.redact(&format!("the collector answered {}", response.status()))
                );
            }
            let ids: Vec<String> = spans.iter().map(|s| s.id.clone()).collect();
            trace::mark_exported(&self.db, &ids)
                .await
                .map_err(|e| e.to_string())?;
            sent += u32::try_from(ids.len()).unwrap_or(u32::MAX);
            if spans.len() < BATCH_SIZE as usize {
                break;
            }
        }
        Ok(sent)
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Bytes, http::HeaderMap, http::StatusCode, routing::post, Router};

    use super::*;
    use crate::{
        model::JsonValue,
        secrets::{self, MemoryStore},
    };

    fn span(id: &str, kind: SpanKind, parent: Option<&str>) -> Span {
        Span {
            id: id.into(),
            trace_id: "6f1c1c9e-6a1b-4f8e-9d38-0a8a52a0f001".into(),
            parent_id: parent.map(str::to_owned),
            kind,
            name: if kind == SpanKind::Tool {
                "echo"
            } else {
                "My server"
            }
            .into(),
            started_at: 1_700_000_000_000,
            ended_at: Some(1_700_000_000_250),
            status: SpanStatus::Ok,
            attributes: JsonValue(json!({ "jsonrpcId": "7", "serverId": "srv" })),
            tokens: Some(42),
        }
    }

    #[test]
    fn endpoint_gets_the_traces_path() {
        let url = |e: &str| traces_url(e).unwrap().to_string();
        assert_eq!(
            url("http://localhost:4318"),
            "http://localhost:4318/v1/traces"
        );
        assert_eq!(
            url("http://localhost:4318/"),
            "http://localhost:4318/v1/traces"
        );
        assert_eq!(
            url("https://h.example/otlp"),
            "https://h.example/otlp/v1/traces"
        );
        assert_eq!(
            url("https://h.example/v1/traces"),
            "https://h.example/v1/traces"
        );
        assert!(traces_url("localhost:4318").is_err());
        assert!(traces_url("ftp://h").is_err());
        assert!(traces_url("").is_err());
    }

    #[test]
    fn config_validation() {
        let ok = ExportConfig {
            enabled: true,
            endpoint: " http://localhost:4318 ".into(),
            headers: BTreeMap::from([
                (" x-key ".into(), "v".into()),
                (" ".into(), "dropped".into()),
            ]),
        };
        let ok = ok.normalized().unwrap();
        assert_eq!(ok.endpoint, "http://localhost:4318");
        assert_eq!(ok.headers.keys().collect::<Vec<_>>(), ["x-key"]);

        let missing = ExportConfig {
            enabled: true,
            ..Default::default()
        };
        assert!(missing.normalized().is_err());
        // A disabled exporter may have no endpoint yet.
        assert!(ExportConfig::default().normalized().is_ok());
        let bad_header = ExportConfig {
            headers: BTreeMap::from([("bad name".into(), "v".into())]),
            ..Default::default()
        };
        assert!(bad_header.normalized().is_err());
    }

    #[test]
    fn ids_are_hex_of_the_right_length_and_stable() {
        let trace = trace_id_hex("6f1c1c9e-6a1b-4f8e-9d38-0a8a52a0f001");
        assert_eq!(trace, "6f1c1c9e6a1b4f8e9d380a8a52a0f001");
        assert_eq!(
            span_id_hex("6f1c1c9e-6a1b-4f8e-9d38-0a8a52a0f001"),
            "6f1c1c9e6a1b4f8e"
        );
        let other = trace_id_hex("not-a-uuid");
        assert_eq!(other.len(), 32);
        assert!(other.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(other, trace_id_hex("not-a-uuid"));
        assert_eq!(span_id_hex("not-a-uuid").len(), 16);
    }

    #[test]
    fn tool_spans_follow_the_genai_and_mcp_conventions() {
        let root = span(
            "11111111-1111-4111-8111-111111111111",
            SpanKind::Session,
            None,
        );
        let call = span(
            "22222222-2222-4222-8222-222222222222",
            SpanKind::Tool,
            Some("11111111-1111-4111-8111-111111111111"),
        );
        let body = to_otlp(&[root, call]);
        let spans = &body["resourceSpans"][0]["scopeSpans"][0]["spans"];
        assert_eq!(spans.as_array().unwrap().len(), 2);
        let tool = &spans[1];
        assert_eq!(tool["name"], "tools/call echo");
        assert_eq!(tool["kind"], 3);
        assert_eq!(tool["parentSpanId"], "1111111111114111");
        assert_eq!(tool["startTimeUnixNano"], "1700000000000000000");
        assert_eq!(tool["endTimeUnixNano"], "1700000000250000000");
        assert_eq!(tool["status"]["code"], 1);
        let attributes = tool["attributes"].as_array().unwrap();
        let get = |key: &str| {
            attributes
                .iter()
                .find(|a| a["key"] == key)
                .map(|a| a["value"].clone())
        };
        assert_eq!(
            get("gen_ai.operation.name").unwrap()["stringValue"],
            "execute_tool"
        );
        assert_eq!(get("gen_ai.tool.name").unwrap()["stringValue"], "echo");
        assert_eq!(get("mcp.method.name").unwrap()["stringValue"], "tools/call");
        assert_eq!(get("jsonrpc.request.id").unwrap()["stringValue"], "7");
        assert_eq!(get("mcpstudio.tokens.estimated").unwrap()["intValue"], "42");
        assert!(get("error.type").is_none());
        // The root has no parent and is an internal span.
        assert!(spans[0].get("parentSpanId").is_none());
        assert_eq!(spans[0]["kind"], 1);
        assert_eq!(spans[0]["name"], "session My server");
        let resource = &body["resourceSpans"][0]["resource"]["attributes"];
        assert_eq!(resource[0]["value"]["stringValue"], "mcp-studio");
    }

    #[test]
    fn failed_and_cancelled_spans_are_errors_and_open_spans_are_skipped() {
        let mut failed = span("33333333-3333-4333-8333-333333333333", SpanKind::Tool, None);
        failed.status = SpanStatus::Error;
        let mut cancelled = span("44444444-4444-4444-8444-444444444444", SpanKind::Tool, None);
        cancelled.status = SpanStatus::Cancelled;
        let mut open = span("55555555-5555-4555-8555-555555555555", SpanKind::Tool, None);
        open.ended_at = None;
        let body = to_otlp(&[failed, cancelled, open]);
        let spans = body["resourceSpans"][0]["scopeSpans"][0]["spans"]
            .as_array()
            .unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0]["status"]["code"], 2);
        assert!(spans[0]["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["key"] == "error.type"));
        assert_eq!(spans[1]["status"]["message"], "cancelled");
    }

    #[test]
    fn exported_data_has_no_payloads() {
        let body = to_otlp(&[span(
            "22222222-2222-4222-8222-222222222222",
            SpanKind::Tool,
            None,
        )])
        .to_string();
        for forbidden in ["arguments", "result", "payload", "content"] {
            assert!(!body.contains(forbidden), "{forbidden} leaked: {body}");
        }
    }

    type Captured = Arc<Mutex<Vec<(HeaderMap, Vec<u8>)>>>;

    async fn collector(status: StatusCode) -> (String, Captured) {
        let captured: Captured = Arc::default();
        let sink = captured.clone();
        let app = Router::new().route(
            "/v1/traces",
            post(move |headers: HeaderMap, body: Bytes| {
                let sink = sink.clone();
                async move {
                    sink.lock().unwrap().push((headers, body.to_vec()));
                    status
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}"), captured)
    }

    async fn exporter_with_spans() -> (Arc<TraceExporter>, Db, Arc<MemoryStore>) {
        let db = Db::open_in_memory().await.unwrap();
        let secrets = Arc::new(MemoryStore::default());
        let now = now_ms();
        for (id, parent, kind) in [
            ("11111111-1111-4111-8111-111111111111", None, "session"),
            (
                "22222222-2222-4222-8222-222222222222",
                Some("11111111-1111-4111-8111-111111111111"),
                "tool",
            ),
        ] {
            sqlx::query("INSERT INTO spans (id, trace_id, parent_id, kind, name, started_at, ended_at, status, attributes) VALUES (?, '6f1c1c9e-6a1b-4f8e-9d38-0a8a52a0f001', ?, ?, 'n', ?, ?, 'ok', '{}')")
                .bind(id).bind(parent).bind(kind).bind(now).bind(now + 5)
                .execute(db.pool()).await.unwrap();
        }
        (TraceExporter::new(db.clone(), secrets.clone()), db, secrets)
    }

    async fn unexported(db: &Db) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM spans WHERE exported = 0")
            .fetch_one(db.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn does_nothing_while_disabled() {
        let (exporter, db, _) = exporter_with_spans().await;
        assert_eq!(exporter.export_now().await, ExportStatus::default());
        assert_eq!(unexported(&db).await, 2);
    }

    #[tokio::test]
    async fn enabling_skips_spans_recorded_before_and_sends_new_ones() {
        let (url, captured) = collector(StatusCode::OK).await;
        let (exporter, db, _) = exporter_with_spans().await;
        exporter
            .set_config(ExportConfig {
                enabled: true,
                endpoint: url,
                headers: BTreeMap::new(),
            })
            .await
            .unwrap();
        // Existing spans were marked as exported: nothing is sent.
        exporter.export_now().await;
        assert!(captured.lock().unwrap().is_empty());

        sqlx::query("INSERT INTO spans (id, trace_id, parent_id, kind, name, started_at, ended_at, status, attributes) VALUES ('99999999-9999-4999-8999-999999999999', 'x', NULL, 'tool', 'late', 1, 2, 'ok', '{}')")
            .execute(db.pool()).await.unwrap();
        let status = exporter.export_now().await;
        assert_eq!(status.exported, 1);
        assert_eq!(status.last_error, None);
        assert!(status.last_success_at.is_some());
        assert_eq!(unexported(&db).await, 0);
        let requests = captured.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0["content-type"], "application/json");
        let body: Value = serde_json::from_slice(&requests[0].1).unwrap();
        assert_eq!(
            body["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["name"],
            "tools/call late"
        );
    }

    #[tokio::test]
    async fn keeps_spans_and_reports_the_error_when_the_collector_fails() {
        let (url, captured) = collector(StatusCode::INTERNAL_SERVER_ERROR).await;
        let (exporter, db, _) = exporter_with_spans().await;
        exporter
            .set_config(ExportConfig {
                enabled: true,
                endpoint: url,
                headers: BTreeMap::new(),
            })
            .await
            .unwrap();
        sqlx::query("UPDATE spans SET exported = 0")
            .execute(db.pool())
            .await
            .unwrap();
        let status = exporter.export_now().await;
        assert!(
            status.last_error.as_deref().unwrap().contains("500"),
            "{status:?}"
        );
        assert_eq!(status.exported, 0);
        assert_eq!(unexported(&db).await, 2);
        assert_eq!(captured.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn sends_headers_resolving_keyring_references_and_hides_them_in_errors() {
        let (url, captured) = collector(StatusCode::UNAUTHORIZED).await;
        let (exporter, db, secrets) = exporter_with_spans().await;
        secrets.set("otlp-key", "s3cret-token").unwrap();
        let reference = secrets::reference("otlp-key");
        exporter
            .set_config(ExportConfig {
                enabled: true,
                endpoint: url,
                headers: BTreeMap::from([("authorization".into(), reference.clone())]),
            })
            .await
            .unwrap();
        sqlx::query("UPDATE spans SET exported = 0")
            .execute(db.pool())
            .await
            .unwrap();
        let status = exporter.export_now().await;
        {
            let requests = captured.lock().unwrap();
            assert_eq!(requests[0].0["authorization"], "s3cret-token");
        }
        assert!(!status.last_error.unwrap().contains("s3cret-token"));
        // Only the reference is stored, never the value.
        let stored = exporter.config().await.unwrap();
        assert_eq!(stored.headers["authorization"], reference);
    }

    #[tokio::test]
    async fn an_unreachable_collector_is_an_error_not_a_crash() {
        let (exporter, db, _) = exporter_with_spans().await;
        exporter
            .set_config(ExportConfig {
                enabled: true,
                endpoint: "http://127.0.0.1:9".into(),
                headers: BTreeMap::new(),
            })
            .await
            .unwrap();
        sqlx::query("UPDATE spans SET exported = 0")
            .execute(db.pool())
            .await
            .unwrap();
        let status = exporter.export_now().await;
        assert!(status.last_error.unwrap().contains("could not reach"));
        assert_eq!(unexported(&db).await, 2);
    }
}
