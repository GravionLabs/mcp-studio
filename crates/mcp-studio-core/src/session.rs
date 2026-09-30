//! Live connections to MCP servers: one `rmcp` client per connected server, recorded end to end.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    future::Future,
    pin::Pin,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[allow(deprecated)]
use rmcp::model::{LoggingLevel, LoggingMessageNotificationParam};
use rmcp::{
    model::{ClientConfig, Implementation},
    service::{
        MaybeSendFuture, NotificationContext, Peer, QuitReason, RunningService,
        RunningServiceCancellationToken,
    },
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
        TokioChildProcess,
    },
    ClientHandler, RoleClient, ServiceExt,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::watch,
};

use crate::{
    db::{new_id, now_ms, Db, DbError, DbResult},
    environments::Environments,
    events::{
        ConnectionState, EventSink, ListChangedEvent, ListKind, LogEvent, LogSource, StatusEvent,
    },
    message_store::MessageWriter,
    path_env,
    placeholders::{placeholders_in, resolve_str},
    recording::RecordingTransport,
    registry::{Registry, ServerDefinition, TransportKind},
    secrets::{resolve_values, Redactor, SecretStore},
};

/// Log lines kept per server for the log view.
const LOG_CAPACITY: usize = 1000;
/// How long a server may take to answer `initialize` before the attempt is abandoned.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Delays before automatic reconnect attempts after an unexpected disconnect.
const RECONNECT_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(8),
];

/// Everything needed to open a connection, with placeholders and secrets resolved.
#[derive(Debug)]
pub struct Prepared {
    pub transport: TransportKind,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub redactor: Redactor,
}

/// Applies environment variables and keyring secrets to a server definition.
pub fn prepare(
    server: &ServerDefinition,
    variables: &BTreeMap<String, String>,
    secrets: &dyn SecretStore,
) -> DbResult<Prepared> {
    let (variables, mut secret_values) = resolve_values(secrets, variables)?;
    check_defined(server, &variables)?;
    let text = |value: &str| resolve_str(value, &variables);
    let map = |source: &BTreeMap<String, String>| -> DbResult<BTreeMap<String, String>> {
        source
            .iter()
            .map(|(k, v)| Ok((k.clone(), text(v)?)))
            .collect()
    };
    let input = &server.input;
    let (env, env_secrets) = resolve_values(secrets, &map(&input.env)?)?;
    let (headers, header_secrets) = resolve_values(secrets, &map(&input.headers)?)?;
    secret_values.extend(env_secrets);
    secret_values.extend(header_secrets);
    Ok(Prepared {
        transport: input.transport,
        command: text(input.command.as_deref().unwrap_or_default())?,
        args: input
            .args
            .iter()
            .map(|a| text(a))
            .collect::<DbResult<_>>()?,
        cwd: input.cwd.as_deref().map(text).transpose()?,
        env,
        url: text(input.url.as_deref().unwrap_or_default())?,
        headers,
        redactor: Redactor::new(secret_values),
    })
}

/// Fails with one message naming every undefined variable used anywhere in the definition.
fn check_defined(server: &ServerDefinition, variables: &BTreeMap<String, String>) -> DbResult<()> {
    let input = &server.input;
    let templates = input
        .command
        .iter()
        .chain(input.cwd.iter())
        .chain(input.url.iter())
        .chain(input.args.iter())
        .chain(input.env.values())
        .chain(input.headers.values());
    let mut missing = Vec::new();
    for template in templates {
        for name in placeholders_in(template) {
            if !variables.contains_key(&name) && !missing.contains(&name) {
                missing.push(name);
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        missing.sort();
        Err(DbError::Invalid(format!(
            "undefined variable(s): {}",
            missing.join(", ")
        )))
    }
}

/// Per-server ring buffers of log lines.
#[derive(Default)]
struct LogBuffer {
    lines: Mutex<HashMap<String, VecDeque<LogEvent>>>,
}

impl LogBuffer {
    fn push(&self, event: &LogEvent) {
        let mut all = self.lines.lock().unwrap();
        let lines = all.entry(event.server_id.clone()).or_default();
        if lines.len() == LOG_CAPACITY {
            lines.pop_front();
        }
        lines.push_back(event.clone());
    }

    fn get(&self, server_id: &str) -> Vec<LogEvent> {
        self.lines
            .lock()
            .unwrap()
            .get(server_id)
            .map(|l| l.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// Receives notifications from a connected server.
///
/// MCP logging notifications are deprecated in the spec but still widely sent; we keep showing them.
#[allow(deprecated)]
#[derive(Clone)]
struct StudioClient {
    server_id: String,
    logs: Arc<Logger>,
}

impl StudioClient {
    fn changed(&self, kind: ListKind) {
        self.logs.sink.list_changed(ListChangedEvent {
            server_id: self.server_id.clone(),
            kind,
        });
    }
}

#[allow(deprecated)]
impl ClientHandler for StudioClient {
    fn get_info(&self) -> ClientConfig {
        let mut config = ClientConfig::default();
        config.client_info = Implementation::new("mcp-studio", env!("CARGO_PKG_VERSION"));
        config
    }

    fn on_tool_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.changed(ListKind::Tools);
        std::future::ready(())
    }

    fn on_resource_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.changed(ListKind::Resources);
        std::future::ready(())
    }

    fn on_prompt_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.changed(ListKind::Prompts);
        std::future::ready(())
    }

    fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        let level = match params.level {
            LoggingLevel::Debug => "debug",
            LoggingLevel::Info | LoggingLevel::Notice => "info",
            LoggingLevel::Warning => "warning",
            _ => "error",
        };
        let line = match &params.data {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        self.logs
            .log(&self.server_id, LogSource::Server, level, &line);
        std::future::ready(())
    }
}

/// Redacts, buffers, and forwards log lines.
struct Logger {
    sink: Arc<dyn EventSink>,
    buffer: LogBuffer,
    redactor: Mutex<HashMap<String, Redactor>>,
}

impl Logger {
    fn set_redactor(&self, server_id: &str, redactor: Redactor) {
        self.redactor
            .lock()
            .unwrap()
            .insert(server_id.to_owned(), redactor);
    }

    fn log(&self, server_id: &str, source: LogSource, level: &str, line: &str) {
        let line = match self.redactor.lock().unwrap().get(server_id) {
            Some(redactor) => redactor.redact(line),
            None => line.to_owned(),
        };
        let event = LogEvent {
            server_id: server_id.to_owned(),
            source,
            level: level.to_owned(),
            line,
            ts: now_ms(),
        };
        self.buffer.push(&event);
        self.sink.log(event);
    }
}

/// A connected server.
pub struct LiveSession {
    pub session_id: String,
    pub server_id: String,
    pub peer: Peer<RoleClient>,
    cancel: Mutex<Option<RunningServiceCancellationToken>>,
    user_requested: AtomicBool,
    closed: watch::Receiver<bool>,
}

/// Owns all connections. Cheap to share through `Arc`.
pub struct SessionManager {
    db: Db,
    registry: Registry,
    environments: Environments,
    secrets: Arc<dyn SecretStore>,
    sink: Arc<dyn EventSink>,
    logger: Arc<Logger>,
    live: Mutex<HashMap<String, Arc<LiveSession>>>,
    connect_timeout: Mutex<Duration>,
}

impl SessionManager {
    pub fn new(
        db: Db,
        registry: Registry,
        environments: Environments,
        secrets: Arc<dyn SecretStore>,
        sink: Arc<dyn EventSink>,
    ) -> Arc<Self> {
        let logger = Arc::new(Logger {
            sink: sink.clone(),
            buffer: LogBuffer::default(),
            redactor: Mutex::default(),
        });
        Arc::new(Self {
            db,
            registry,
            environments,
            secrets,
            sink,
            logger,
            live: Mutex::default(),
            connect_timeout: Mutex::new(DEFAULT_CONNECT_TIMEOUT),
        })
    }

    /// Changes how long connecting may take (default 30 seconds).
    pub fn set_connect_timeout(&self, timeout: Duration) {
        *self.connect_timeout.lock().unwrap() = timeout;
    }

    /// The connected session of a server, if any.
    pub fn session(&self, server_id: &str) -> Option<Arc<LiveSession>> {
        self.live.lock().unwrap().get(server_id).cloned()
    }

    /// The MCP peer of a connected server.
    pub fn peer(&self, server_id: &str) -> DbResult<Peer<RoleClient>> {
        self.session(server_id)
            .map(|s| s.peer.clone())
            .ok_or_else(|| DbError::Connection("the server is not connected".into()))
    }

    pub fn logs(&self, server_id: &str) -> Vec<LogEvent> {
        self.logger.buffer.get(server_id)
    }

    fn status(
        &self,
        server_id: &str,
        session_id: Option<&str>,
        state: ConnectionState,
        message: Option<String>,
    ) {
        self.sink.status(StatusEvent {
            server_id: server_id.to_owned(),
            session_id: session_id.map(str::to_owned),
            state,
            message,
        });
    }

    /// Connects to a server (no-op if already connected).
    ///
    /// Returns a boxed future because reconnecting makes this function (indirectly) recursive.
    pub fn connect<'a>(
        self: &'a Arc<Self>,
        server_id: &'a str,
        environment_id: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = DbResult<Arc<LiveSession>>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(existing) = self.session(server_id) {
                return Ok(existing);
            }
            self.status(server_id, None, ConnectionState::Connecting, None);
            match self.open(server_id, environment_id).await {
                Ok(session) => Ok(session),
                Err(error) => {
                    self.status(
                        server_id,
                        None,
                        ConnectionState::Error,
                        Some(error.to_string()),
                    );
                    Err(error)
                }
            }
        })
    }

    async fn open(
        self: &Arc<Self>,
        server_id: &str,
        environment_id: Option<&str>,
    ) -> DbResult<Arc<LiveSession>> {
        let server = self.registry.get(server_id).await?;
        let variables = match environment_id {
            Some(id) => self.environments.get(id).await?.input.variables,
            None => BTreeMap::new(),
        };
        let prepared = prepare(&server, &variables, self.secrets.as_ref())?;
        self.logger
            .set_redactor(server_id, prepared.redactor.clone());

        let session_id = new_id();
        sqlx::query(
            "INSERT INTO sessions (id, server_id, origin, started_at) VALUES (?, ?, 'studio', ?)",
        )
        .bind(&session_id)
        .bind(server_id)
        .bind(now_ms())
        .execute(self.db.pool())
        .await?;

        let writer = MessageWriter::spawn(
            self.db.clone(),
            session_id.clone(),
            prepared.redactor.clone(),
            self.sink.clone(),
        );
        let handler = StudioClient {
            server_id: server_id.to_owned(),
            logs: self.logger.clone(),
        };

        let timeout = *self.connect_timeout.lock().unwrap();
        let connected: DbResult<RunningService<RoleClient, StudioClient>> = match prepared.transport
        {
            TransportKind::Stdio => {
                let (process, stderr) = spawn_stdio(&prepared)?;
                if let Some(stderr) = stderr {
                    let logger = self.logger.clone();
                    let id = server_id.to_owned();
                    tokio::spawn(async move {
                        let mut lines = BufReader::new(stderr).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            logger.log(&id, LogSource::Stderr, "info", &line);
                        }
                    });
                }
                let transport =
                    RecordingTransport::<_, RoleClient>::new(process, writer.recorder());
                initialize(handler, transport, timeout).await
            }
            TransportKind::Http => {
                let http = StreamableHttpClientTransport::from_config(http_config(&prepared)?);
                let transport = RecordingTransport::<_, RoleClient>::new(http, writer.recorder());
                initialize(handler, transport, timeout).await
            }
        };

        let running = match connected {
            Ok(running) => running,
            Err(error) => {
                self.end_session(&session_id).await;
                let _ = tokio::time::timeout(Duration::from_secs(2), writer.finish()).await;
                return Err(error);
            }
        };

        let peer = running.peer().clone();
        if let Some(info) = peer.peer_info() {
            let json = |v: &dyn erased_json::ToJson| v.to_json();
            let _ = sqlx::query("UPDATE sessions SET protocol_version = ?, server_info = ?, capabilities = ? WHERE id = ?")
                .bind(json(&info.protocol_version).trim_matches('"').to_owned())
                .bind(json(&info.server_info))
                .bind(json(&info.capabilities))
                .bind(&session_id)
                .execute(self.db.pool())
                .await;
        }

        let (closed_tx, closed_rx) = watch::channel(false);
        let live = Arc::new(LiveSession {
            session_id: session_id.clone(),
            server_id: server_id.to_owned(),
            peer,
            cancel: Mutex::new(Some(running.cancellation_token())),
            user_requested: AtomicBool::new(false),
            closed: closed_rx,
        });
        self.live
            .lock()
            .unwrap()
            .insert(server_id.to_owned(), live.clone());
        self.status(
            server_id,
            Some(&session_id),
            ConnectionState::Connected,
            None,
        );

        let manager = self.clone();
        let watched = live.clone();
        let env_id = environment_id.map(str::to_owned);
        tokio::spawn(async move {
            let reason: Result<QuitReason, tokio::task::JoinError> = running.waiting().await;
            let _ = tokio::time::timeout(Duration::from_secs(5), writer.finish()).await;
            manager.on_closed(&watched, reason.ok(), env_id).await;
            let _ = closed_tx.send(true);
        });
        Ok(live)
    }

    async fn end_session(&self, session_id: &str) {
        let _ = sqlx::query("UPDATE sessions SET ended_at = ? WHERE id = ?")
            .bind(now_ms())
            .bind(session_id)
            .execute(self.db.pool())
            .await;
    }

    async fn on_closed(
        self: &Arc<Self>,
        session: &Arc<LiveSession>,
        reason: Option<QuitReason>,
        environment_id: Option<String>,
    ) {
        self.live.lock().unwrap().remove(&session.server_id);
        self.end_session(&session.session_id).await;
        let user_requested = session.user_requested.load(Ordering::SeqCst);
        if user_requested || matches!(reason, Some(QuitReason::Cancelled)) {
            self.status(
                &session.server_id,
                Some(&session.session_id),
                ConnectionState::Disconnected,
                None,
            );
            return;
        }
        self.status(
            &session.server_id,
            Some(&session.session_id),
            ConnectionState::Error,
            Some("connection lost".into()),
        );
        self.logger.log(
            &session.server_id,
            LogSource::Studio,
            "warning",
            "connection lost, reconnecting",
        );
        let manager = self.clone();
        let server_id = session.server_id.clone();
        tokio::spawn(async move {
            for delay in RECONNECT_DELAYS {
                tokio::time::sleep(delay).await;
                if manager.session(&server_id).is_some() {
                    return;
                }
                if manager
                    .connect(&server_id, environment_id.as_deref())
                    .await
                    .is_ok()
                {
                    return;
                }
            }
            manager.logger.log(
                &server_id,
                LogSource::Studio,
                "error",
                "giving up reconnecting",
            );
        });
    }

    /// Closes the connection and waits until it is gone.
    pub async fn disconnect(&self, server_id: &str) -> DbResult<()> {
        let Some(session) = self.session(server_id) else {
            return Ok(());
        };
        session.user_requested.store(true, Ordering::SeqCst);
        if let Some(token) = session.cancel.lock().unwrap().take() {
            token.cancel();
        }
        let mut closed = session.closed.clone();
        let _ = tokio::time::timeout(Duration::from_secs(5), closed.wait_for(|done| *done)).await;
        Ok(())
    }

    /// Disconnects everything (app shutdown).
    pub async fn disconnect_all(&self) {
        let ids: Vec<String> = self.live.lock().unwrap().keys().cloned().collect();
        for id in ids {
            let _ = self.disconnect(&id).await;
        }
    }
}

mod erased_json {
    use serde::Serialize;

    pub trait ToJson {
        fn to_json(&self) -> String;
    }

    impl<T: Serialize> ToJson for T {
        fn to_json(&self) -> String {
            serde_json::to_string(self).unwrap_or_default()
        }
    }
}

async fn initialize<T, E, A>(
    handler: StudioClient,
    transport: T,
    timeout: Duration,
) -> DbResult<RunningService<RoleClient, StudioClient>>
where
    T: rmcp::transport::IntoTransport<RoleClient, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    match tokio::time::timeout(timeout, handler.serve(transport)).await {
        Ok(Ok(running)) => Ok(running),
        Ok(Err(error)) => Err(DbError::Connection(format!("initialize failed: {error}"))),
        Err(_) => Err(DbError::Connection(format!(
            "the server did not answer initialize within {} seconds",
            timeout.as_secs().max(1)
        ))),
    }
}

fn spawn_stdio(
    prepared: &Prepared,
) -> DbResult<(TokioChildProcess, Option<tokio::process::ChildStderr>)> {
    let mut command = tokio::process::Command::new(&prepared.command);
    command.args(&prepared.args);
    command.envs(&prepared.env);
    if let Some(cwd) = &prepared.cwd {
        command.current_dir(cwd);
    }
    if !prepared.env.contains_key("PATH") {
        if let Some(login) = path_env::login_shell_path() {
            let current = std::env::var("PATH").unwrap_or_default();
            command.env("PATH", path_env::merge_paths(login, &current));
        }
    }
    TokioChildProcess::builder(command)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DbError::Connection(format!("could not start \"{}\": {e}", prepared.command)))
}

fn http_config(prepared: &Prepared) -> DbResult<StreamableHttpClientTransportConfig> {
    let mut headers = HashMap::new();
    for (name, value) in &prepared.headers {
        let name = http::HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| DbError::Invalid(format!("invalid header name \"{name}\": {e}")))?;
        let value = http::HeaderValue::from_str(value)
            .map_err(|e| DbError::Invalid(format!("invalid value for header \"{name}\": {e}")))?;
        headers.insert(name, value);
    }
    Ok(StreamableHttpClientTransportConfig::with_uri(prepared.url.clone()).custom_headers(headers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{registry::ServerInput, secrets::MemoryStore};

    fn server(input: ServerInput) -> ServerDefinition {
        ServerDefinition {
            id: "id".into(),
            input,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn stdio_input() -> ServerInput {
        ServerInput {
            name: "n".into(),
            transport: TransportKind::Stdio,
            command: Some("{{bin}}".into()),
            args: vec!["--port".into(), "{{port}}".into()],
            env: BTreeMap::from([
                ("TOKEN".into(), "keyring:tok".into()),
                ("HOST".into(), "{{host}}".into()),
            ]),
            cwd: Some("/work/{{host}}".into()),
            url: None,
            headers: BTreeMap::new(),
            tags: vec![],
        }
    }

    fn variables() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("bin".into(), "node".into()),
            ("port".into(), "3000".into()),
            ("host".into(), "example.com".into()),
        ])
    }

    #[test]
    fn prepare_applies_variables_and_secrets() {
        let store = MemoryStore::default();
        store.set("tok", "sk-123").unwrap();
        let prepared = prepare(&server(stdio_input()), &variables(), &store).unwrap();
        assert_eq!(prepared.command, "node");
        assert_eq!(prepared.args, ["--port", "3000"]);
        assert_eq!(prepared.cwd.as_deref(), Some("/work/example.com"));
        assert_eq!(prepared.env["TOKEN"], "sk-123");
        assert_eq!(prepared.env["HOST"], "example.com");
        assert_eq!(
            prepared.redactor.redact("key sk-123"),
            format!("key {}", crate::secrets::MASK)
        );
    }

    #[test]
    fn prepare_reports_undefined_variables() {
        let store = MemoryStore::default();
        store.set("tok", "x").unwrap();
        let error = prepare(&server(stdio_input()), &BTreeMap::new(), &store)
            .unwrap_err()
            .to_string();
        assert!(error.contains("bin") && error.contains("port"), "{error}");
    }

    #[test]
    fn prepare_reports_missing_secrets() {
        let error = prepare(
            &server(stdio_input()),
            &variables(),
            &MemoryStore::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("tok"), "{error}");
    }

    #[test]
    fn secret_variables_are_resolved_and_redacted() {
        let store = MemoryStore::default();
        store.set("tok", "a").unwrap();
        store.set("varsecret", "hunter2").unwrap();
        let mut vars = variables();
        vars.insert("host".into(), "keyring:varsecret".into());
        let prepared = prepare(&server(stdio_input()), &vars, &store).unwrap();
        assert_eq!(prepared.env["HOST"], "hunter2");
        assert!(!prepared.redactor.redact("hunter2").contains("hunter2"));
    }
}
