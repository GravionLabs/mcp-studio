//! HTTP proxy mode: a local Streamable HTTP endpoint per server that forwards to the real server
//! (with its configured headers) and records every JSON-RPC message in both directions.
//!
//! A client is configured with `http://127.0.0.1:<port>/mcp/<server name or id>` and needs no
//! credentials; MCP Studio adds the server's headers, so secrets stay in the OS keyring.
//!
//! Because the endpoint adds credentials, it only answers requests that were addressed to it as a
//! loopback host and that do not come from a web page of another site: a page that points its own
//! domain at 127.0.0.1 (DNS rebinding) would otherwise use the server with the user's credentials.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    body::{Body, Bytes},
    extract::{Path, Request, State},
    http::{header, uri::Authority, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    response::Response,
    routing::any,
    Router,
};
use futures_util::StreamExt;
use tokio::net::TcpListener;

use crate::{
    db::{new_id, now_ms, Db, DbError, DbResult},
    environments::Environments,
    events::EventSink,
    message_store::MessageWriter,
    recording::{Direction, RecordedMessage, Recorder},
    registry::{Registry, ServerDefinition, TransportKind},
    secrets::SecretStore,
    session::{prepare, Prepared},
};

/// Default port; if taken, an ephemeral one is used.
pub const DEFAULT_PORT: u16 = 38465;
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
const SESSION_IDLE: Duration = Duration::from_secs(15 * 60);
const SESSION_ID_HEADER: &str = "mcp-session-id";

/// Connection headers that must not be forwarded.
const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
];

struct ProxySession {
    session_id: String,
    writer: Option<MessageWriter>,
    recorder: Arc<dyn Recorder>,
    last_used: std::time::Instant,
}

struct Inner {
    db: Db,
    registry: Registry,
    environments: Environments,
    secrets: Arc<dyn SecretStore>,
    sink: Arc<dyn EventSink>,
    client: reqwest::Client,
    /// The port the endpoint listens on; a request's `Host` must name it.
    port: u16,
    environment: Mutex<Option<String>>,
    /// (server id, upstream `Mcp-Session-Id` or empty) -> recording session.
    sessions: Mutex<HashMap<(String, String), ProxySession>>,
}

/// The local HTTP endpoint. Cheap to clone.
#[derive(Clone)]
pub struct HttpProxy {
    inner: Arc<Inner>,
    addr: SocketAddr,
}

impl HttpProxy {
    /// Starts the endpoint on loopback, preferring `preferred_port`.
    pub async fn start(
        db: Db,
        registry: Registry,
        environments: Environments,
        secrets: Arc<dyn SecretStore>,
        sink: Arc<dyn EventSink>,
        preferred_port: u16,
    ) -> DbResult<Self> {
        let listener = match TcpListener::bind(("127.0.0.1", preferred_port)).await {
            Ok(listener) => listener,
            Err(_) => TcpListener::bind(("127.0.0.1", 0)).await?,
        };
        let addr = listener.local_addr()?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| DbError::Connection(e.to_string()))?;
        let inner = Arc::new(Inner {
            db,
            registry,
            environments,
            secrets,
            sink,
            client,
            port: addr.port(),
            environment: Mutex::new(None),
            sessions: Mutex::default(),
        });
        let router = Router::new()
            .route("/mcp/{server}", any(forward))
            .with_state(inner.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let proxy = Self { inner, addr };
        proxy.spawn_sweeper();
        Ok(proxy)
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// URL a client should use for a server.
    pub fn url_for(&self, server_name: &str) -> String {
        format!(
            "http://127.0.0.1:{}/mcp/{}",
            self.port(),
            urlencode(server_name)
        )
    }

    pub fn set_environment(&self, id: Option<String>) {
        *self.inner.environment.lock().unwrap() = id;
    }

    /// Ends all sessions (app shutdown).
    pub async fn shutdown(&self) {
        let sessions: Vec<ProxySession> = self
            .inner
            .sessions
            .lock()
            .unwrap()
            .drain()
            .map(|(_, s)| s)
            .collect();
        for session in sessions {
            end(&self.inner.db, session).await;
        }
    }

    fn spawn_sweeper(&self) {
        let inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                let Some(inner) = inner.upgrade() else { break };
                let expired: Vec<ProxySession> = {
                    let mut all = inner.sessions.lock().unwrap();
                    let keys: Vec<_> = all
                        .iter()
                        .filter(|(_, s)| s.last_used.elapsed() > SESSION_IDLE)
                        .map(|(k, _)| k.clone())
                        .collect();
                    keys.into_iter().filter_map(|k| all.remove(&k)).collect()
                };
                for session in expired {
                    end(&inner.db, session).await;
                }
            }
        });
    }
}

async fn end(db: &Db, session: ProxySession) {
    let _ = sqlx::query("UPDATE sessions SET ended_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(&session.session_id)
        .execute(db.pool())
        .await;
    drop(session.recorder);
    if let Some(writer) = session.writer {
        writer.finish().await;
    }
}

fn urlencode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(message.into()))
        .expect("static response")
}

fn is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Refuses requests that were not addressed to this endpoint as a loopback host, and requests a
/// browser sends for a page of another site. Clients that are not browsers send no `Origin`.
fn check_local(headers: &HeaderMap, uri: &Uri, port: u16) -> Result<(), String> {
    let authority = match headers.get(header::HOST) {
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|text| text.parse::<Authority>().ok()),
        // HTTP/2 carries the host in the request target instead.
        None => uri.authority().cloned(),
    };
    let addressed_here = authority
        .as_ref()
        .is_some_and(|a| is_loopback(a.host()) && a.port_u16() == Some(port));
    if !addressed_here {
        return Err(format!(
            "this endpoint only answers requests to 127.0.0.1:{port} or localhost:{port}"
        ));
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let local_page = origin
            .to_str()
            .ok()
            .and_then(|text| text.parse::<Uri>().ok())
            .is_some_and(|uri| uri.host().is_some_and(is_loopback));
        if !local_page {
            return Err("requests from web pages of other sites are not allowed".to_owned());
        }
    }
    Ok(())
}

async fn forward(
    State(inner): State<Arc<Inner>>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    if let Err(message) = check_local(request.headers(), request.uri(), inner.port) {
        return error(StatusCode::FORBIDDEN, message);
    }
    match handle(&inner, &key, request).await {
        Ok(response) => response,
        Err((status, message)) => error(status, message),
    }
}

async fn handle(
    inner: &Arc<Inner>,
    key: &str,
    request: Request,
) -> Result<Response, (StatusCode, String)> {
    let server = find_server(inner, key)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if server.input.transport != TransportKind::Http {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "\"{}\" is a stdio server; use the mcp-studio-proxy program for it",
                server.input.name
            ),
        ));
    }
    let prepared = prepare_server(inner, &server)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|_| {
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                "request body too large".to_owned(),
            )
        })?;

    let client_session = header_text(&parts.headers, SESSION_ID_HEADER);
    let recording_key = (
        server.id.clone(),
        client_session.clone().unwrap_or_default(),
    );
    let is_initialize = json_messages(&body)
        .iter()
        .any(|m| m["method"] == "initialize");
    let recorder =
        session_recorder(inner, &recording_key, &server, &prepared, is_initialize).await?;

    if parts.method == Method::POST {
        for message in json_messages(&body) {
            recorder.record(recorded(Direction::Out, message));
        }
    }

    let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes()).map_err(|_| {
        (
            StatusCode::METHOD_NOT_ALLOWED,
            "unsupported method".to_owned(),
        )
    })?;
    let mut upstream = inner.client.request(method, &prepared.url);
    for (name, value) in &parts.headers {
        if !is_hop_by_hop(name) && name != header::CONTENT_LENGTH {
            upstream = upstream.header(name.as_str(), value.as_bytes());
        }
    }
    // The server's own headers (e.g. Authorization from the keyring) win over the client's.
    for (name, value) in &prepared.headers {
        upstream = upstream.header(name.as_str(), value.as_str());
    }
    if !body.is_empty() {
        upstream = upstream.body(body.to_vec());
    }
    let response = upstream
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream error: {e}")))?;

    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut headers = HeaderMap::new();
    for (name, value) in response.headers() {
        if is_hop_by_hop(name) || name == header::CONTENT_LENGTH {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_str().as_bytes()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            headers.append(name, value);
        }
    }

    // The upstream tells us the session id in the answer to `initialize`: re-key the recording.
    let upstream_session = header_text(&headers, SESSION_ID_HEADER);
    if let Some(id) = &upstream_session {
        rekey(inner, &recording_key, (server.id.clone(), id.clone()));
    }
    if parts.method == Method::DELETE {
        let key = (
            server.id.clone(),
            upstream_session.or(client_session).unwrap_or_default(),
        );
        let session = inner.sessions.lock().unwrap().remove(&key);
        if let Some(session) = session {
            end(&inner.db, session).await;
        }
    }

    let is_sse =
        header_text(&headers, "content-type").is_some_and(|t| t.starts_with("text/event-stream"));
    let tap = Tap::new(recorder, is_sse);
    let stream = response.bytes_stream().map(move |chunk| {
        chunk
            .inspect(|bytes| tap.push(bytes))
            .map_err(std::io::Error::other)
    });
    let mut out = Response::builder().status(status);
    *out.headers_mut().expect("builder") = headers;
    out.body(Body::from_stream(stream))
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn is_hop_by_hop(name: &reqwest::header::HeaderName) -> bool {
    HOP_BY_HOP.contains(&name.as_str())
}

fn recorded(direction: Direction, payload: serde_json::Value) -> RecordedMessage {
    RecordedMessage {
        direction,
        ts: now_ms(),
        bytes: payload.to_string().len(),
        payload,
    }
}

/// JSON-RPC messages in a request body (a single message or a batch).
fn json_messages(body: &[u8]) -> Vec<serde_json::Value> {
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(serde_json::Value::Array(items)) => items,
        Ok(value @ serde_json::Value::Object(_)) => vec![value],
        _ => vec![],
    }
}

async fn find_server(inner: &Inner, key: &str) -> DbResult<ServerDefinition> {
    if let Ok(server) = inner.registry.get(key).await {
        return Ok(server);
    }
    inner
        .registry
        .list()
        .await?
        .into_iter()
        .find(|s| s.input.name.eq_ignore_ascii_case(key))
        .ok_or_else(|| DbError::NotFound(format!("no server named \"{key}\" in MCP Studio")))
}

async fn prepare_server(inner: &Inner, server: &ServerDefinition) -> DbResult<Prepared> {
    let environment = inner.environment.lock().unwrap().clone();
    let variables = match environment {
        Some(id) => inner.environments.get(&id).await?.input.variables,
        None => Default::default(),
    };
    prepare(server, &variables, inner.secrets.as_ref())
}

/// The recorder for this request's session, creating the session when needed.
async fn session_recorder(
    inner: &Arc<Inner>,
    key: &(String, String),
    server: &ServerDefinition,
    prepared: &Prepared,
    is_initialize: bool,
) -> Result<Arc<dyn Recorder>, (StatusCode, String)> {
    {
        let mut all = inner.sessions.lock().unwrap();
        // A new `initialize` always starts a new recording, even without a session header.
        if !is_initialize {
            if let Some(existing) = all.get_mut(key) {
                existing.last_used = std::time::Instant::now();
                return Ok(existing.recorder.clone());
            }
        }
    }
    let session_id = new_id();
    sqlx::query(
        "INSERT INTO sessions (id, server_id, origin, started_at) VALUES (?, ?, 'proxy', ?)",
    )
    .bind(&session_id)
    .bind(&server.id)
    .bind(now_ms())
    .execute(inner.db.pool())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let writer = MessageWriter::spawn(
        inner.db.clone(),
        session_id.clone(),
        server.id.clone(),
        prepared.redactor.clone(),
        inner.sink.clone(),
    );
    let recorder = writer.recorder();
    let previous = inner.sessions.lock().unwrap().insert(
        key.clone(),
        ProxySession {
            session_id,
            writer: Some(writer),
            recorder: recorder.clone(),
            last_used: std::time::Instant::now(),
        },
    );
    if let Some(previous) = previous {
        end(&inner.db, previous).await;
    }
    Ok(recorder)
}

fn rekey(inner: &Inner, from: &(String, String), to: (String, String)) {
    if *from == to {
        return;
    }
    let mut all = inner.sessions.lock().unwrap();
    if let Some(session) = all.remove(from) {
        all.insert(to, session);
    }
}

/// Watches response bytes and records the JSON-RPC messages inside.
struct Tap {
    recorder: Arc<dyn Recorder>,
    sse: bool,
    buffer: Mutex<Vec<u8>>,
}

impl Tap {
    fn new(recorder: Arc<dyn Recorder>, sse: bool) -> Arc<Self> {
        Arc::new(Self {
            recorder,
            sse,
            buffer: Mutex::new(Vec::new()),
        })
    }

    fn push(&self, chunk: &Bytes) {
        let mut buffer = self.buffer.lock().unwrap();
        buffer.extend_from_slice(chunk);
        if self.sse {
            self.drain_events(&mut buffer);
        }
    }

    /// Records every complete SSE event (`data:` lines up to a blank line).
    fn drain_events(&self, buffer: &mut Vec<u8>) {
        loop {
            let text = String::from_utf8_lossy(buffer).into_owned();
            let Some(end) = text.find("\n\n").or_else(|| text.find("\r\n\r\n")) else {
                return;
            };
            let event = &text[..end];
            let data: Vec<&str> = event
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|d| d.strip_prefix(' ').unwrap_or(d))
                .collect();
            if !data.is_empty() {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&data.join("\n")) {
                    self.recorder.record(recorded(Direction::In, value));
                }
            }
            let consumed = text[..end].len()
                + if text[end..].starts_with("\r\n\r\n") {
                    4
                } else {
                    2
                };
            buffer.drain(..consumed.min(buffer.len()));
        }
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        // Plain JSON answers are recorded once the whole body has passed through.
        if !self.sse {
            let buffer = self.buffer.lock().unwrap();
            if !buffer.is_empty() {
                for message in json_messages(&buffer) {
                    self.recorder.record(recorded(Direction::In, message));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    struct Collect(StdMutex<Vec<RecordedMessage>>);
    impl Recorder for Collect {
        fn record(&self, message: RecordedMessage) {
            self.0.lock().unwrap().push(message);
        }
    }

    fn local(host: Option<&str>, origin: Option<&str>) -> Result<(), String> {
        let mut headers = HeaderMap::new();
        if let Some(host) = host {
            headers.insert(header::HOST, host.parse().unwrap());
        }
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, origin.parse().unwrap());
        }
        check_local(&headers, &Uri::from_static("/mcp/demo"), 38465)
    }

    #[test]
    fn requests_to_a_loopback_host_and_this_port_are_accepted() {
        assert!(local(Some("127.0.0.1:38465"), None).is_ok());
        assert!(local(Some("localhost:38465"), None).is_ok());
        assert!(local(Some("LOCALHOST:38465"), None).is_ok());
        assert!(local(Some("[::1]:38465"), None).is_ok());
    }

    #[test]
    fn requests_to_another_host_or_port_are_refused() {
        // DNS rebinding: the browser connects to 127.0.0.1 but still names the attacker's host.
        assert!(local(Some("evil.example:38465"), None).is_err());
        assert!(local(Some("127.0.0.1.evil.example:38465"), None).is_err());
        assert!(local(Some("127.0.0.1:1"), None).is_err());
        assert!(local(Some("127.0.0.1"), None).is_err());
        assert!(local(None, None).is_err());
    }

    #[test]
    fn the_host_of_an_http2_request_comes_from_the_request_target() {
        let uri = Uri::from_static("http://127.0.0.1:38465/mcp/demo");
        assert!(check_local(&HeaderMap::new(), &uri, 38465).is_ok());
        let uri = Uri::from_static("http://evil.example:38465/mcp/demo");
        assert!(check_local(&HeaderMap::new(), &uri, 38465).is_err());
    }

    #[test]
    fn only_local_pages_may_call_from_a_browser() {
        let host = Some("127.0.0.1:38465");
        assert!(local(host, Some("http://localhost:6274")).is_ok());
        assert!(local(host, Some("http://127.0.0.1:5173")).is_ok());
        assert!(local(host, Some("http://[::1]:5173")).is_ok());
        assert!(local(host, Some("https://evil.example")).is_err());
        assert!(local(host, Some("http://localhost.evil.example")).is_err());
        assert!(local(host, Some("null")).is_err());
    }

    #[test]
    fn parses_single_and_batch_messages() {
        assert_eq!(json_messages(br#"{"id":1}"#).len(), 1);
        assert_eq!(json_messages(br#"[{"id":1},{"id":2}]"#).len(), 2);
        assert!(json_messages(b"garbage").is_empty());
        assert!(json_messages(b"").is_empty());
        assert!(json_messages(b"42").is_empty());
    }

    #[test]
    fn sse_events_are_recorded_even_when_split_across_chunks() {
        let collect = Arc::new(Collect(StdMutex::default()));
        let tap = Tap::new(collect.clone(), true);
        tap.push(&Bytes::from_static(
            b"event: message\ndata: {\"id\":1,\"res",
        ));
        assert!(collect.0.lock().unwrap().is_empty());
        tap.push(&Bytes::from_static(
            b"ult\":{}}\n\ndata: {\"id\":2}\r\n\r\n: keepalive\n\n",
        ));
        let recorded = collect.0.lock().unwrap();
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].payload["id"], 1);
        assert_eq!(recorded[1].payload["id"], 2);
        assert!(recorded.iter().all(|m| m.direction == Direction::In));
    }

    #[test]
    fn json_bodies_are_recorded_when_the_stream_ends() {
        let collect = Arc::new(Collect(StdMutex::default()));
        let tap = Tap::new(collect.clone(), false);
        tap.push(&Bytes::from_static(b"{\"id\":7,"));
        tap.push(&Bytes::from_static(b"\"result\":{}}"));
        assert!(collect.0.lock().unwrap().is_empty());
        drop(tap);
        assert_eq!(collect.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn url_encoding_keeps_unreserved_characters() {
        assert_eq!(urlencode("My Server/1"), "My%20Server%2F1");
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn hop_by_hop_headers_are_recognized() {
        assert!(is_hop_by_hop(&reqwest::header::HeaderName::from_static(
            "connection"
        )));
        assert!(!is_hop_by_hop(&reqwest::header::HeaderName::from_static(
            "mcp-session-id"
        )));
    }
}
