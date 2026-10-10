//! Live connections to MCP servers: one `rmcp` client per connected server, recorded end to end.

// Sampling, roots and logging are deprecated in the MCP spec (SEP-2577), but servers still use them.
#![allow(deprecated)]

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
use azure_core::credentials::TokenCredential;
// Server logging is deprecated in the MCP spec (SEP-2577) but servers still send it.
#[allow(deprecated)]
use rmcp::model::{LoggingLevel, LoggingMessageNotificationParam};
use rmcp::{
    model::{
        CallToolRequest, CallToolRequestParams, CancelledNotificationParam, ClientCapabilities,
        ClientConfig, ClientRequest, CreateMessageRequestParams, CreateMessageResult,
        ElicitRequestParams, ElicitResult, ElicitationAction, ElicitationCapability, ErrorCode,
        FormElicitationCapability, Implementation, ListRootsResult, ProgressNotificationParam,
        ResourceUpdatedNotificationParam, Root, RootsCapabilities, SamplingCapability,
        SamplingMessage, ServerResult,
    },
    service::{
        MaybeSendFuture, NotificationContext, Peer, PeerRequestOptions, QuitReason, RequestContext,
        RunningService, RunningServiceCancellationToken,
    },
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
        TokioChildProcess,
    },
    ClientHandler, ErrorData as McpError, RoleClient, ServiceExt,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::watch,
};
use tokio_util::sync::CancellationToken;

use crate::{
    azure_auth::{self, AzureAuthClient, TokenCache},
    client_requests::{ClientAnswer, ClientRequestKind, ClientRequests},
    db::{new_id, now_ms, Db, DbError, DbResult},
    environments::Environments,
    events::{
        ConnectionState, EventSink, ListChangedEvent, ListKind, LogEvent, LogSource, ProgressEvent,
        ResourceUpdatedEvent, StatusEvent,
    },
    history::{History, NewEntry},
    message_store::MessageWriter,
    model::JsonValue,
    oauth::{self, KeyringCredentialStore, UrlOpener},
    path_env,
    placeholders::{placeholders_in, resolve_json, resolve_str},
    recording::RecordingTransport,
    registry::{root_uri, Registry, ServerDefinition, TransportKind},
    secrets::{resolve_values, Redactor, SecretStore},
};

/// Log lines kept per server for the log view.
const LOG_CAPACITY: usize = 1000;
/// A tool call to run.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRequest {
    pub server_id: String,
    pub tool_name: String,
    /// Arguments; `{{variables}}` are resolved from the environment.
    pub arguments: JsonValue,
    pub environment_id: Option<String>,
    /// Chosen by the caller; identifies the call for progress events and cancellation.
    pub call_id: String,
}

/// The outcome of a tool call.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResult {
    pub call_id: String,
    /// The MCP `CallToolResult` (content, structuredContent, isError, ...). Null if cancelled.
    pub result: JsonValue,
    /// The tool reported an error (`isError`).
    pub is_error: bool,
    pub cancelled: bool,
    #[specta(type = u32)]
    pub duration_ms: i64,
}

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
    /// rmcp progress token (as text) -> caller-chosen call id.
    progress: Arc<Mutex<HashMap<String, String>>>,
    /// Questions of the server that wait for the user.
    requests: Arc<ClientRequests>,
    /// What `roots/list` answers; replaced when the roots of the server are edited.
    roots: Arc<Mutex<Vec<Root>>>,
}

/// The roots of a server as MCP sends them.
fn roots_of(server: &ServerDefinition) -> Vec<Root> {
    server
        .input
        .roots
        .iter()
        .filter_map(|root| root_uri(root))
        .map(|(uri, name)| Root::new(uri).with_name(name))
        .collect()
}

/// A request that the user refused or that cannot be answered.
fn refused(message: &'static str) -> McpError {
    McpError::new(ErrorCode(-1), message, None)
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
        let mut capabilities = ClientCapabilities::default();
        capabilities.sampling = Some(SamplingCapability::default());
        capabilities.elicitation =
            Some(ElicitationCapability::new().with_form(FormElicitationCapability::new()));
        let mut roots = RootsCapabilities::default();
        roots.list_changed = Some(true);
        capabilities.roots = Some(roots);
        config.capabilities = capabilities;
        config
    }

    fn list_roots(
        &self,
        _context: RequestContext<RoleClient>,
    ) -> impl Future<Output = Result<ListRootsResult, McpError>> + MaybeSendFuture + '_ {
        let roots = self.roots.lock().unwrap().clone();
        std::future::ready(Ok(ListRootsResult::new(roots)))
    }

    async fn create_message(
        &self,
        params: CreateMessageRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<CreateMessageResult, McpError> {
        let params = serde_json::to_value(&params)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let answer = self
            .requests
            .ask(
                &self.server_id,
                ClientRequestKind::Sampling,
                params,
                &context.ct,
            )
            .await;
        match answer {
            Some(ClientAnswer::Respond { text, model }) => {
                let model = model
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| "mcp-studio-manual".to_owned());
                let mut result =
                    CreateMessageResult::new(SamplingMessage::assistant_text(text), model);
                result.stop_reason = Some(CreateMessageResult::STOP_REASON_END_TURN.to_owned());
                Ok(result)
            }
            Some(_) => Err(refused("User rejected sampling request")),
            None => Err(refused("The sampling request was cancelled")),
        }
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, McpError> {
        // Only forms are advertised; a server that sends a URL request anyway gets "decline".
        if matches!(request, ElicitRequestParams::UrlElicitationParams { .. }) {
            return Ok(ElicitResult::new(ElicitationAction::Decline));
        }
        let params = serde_json::to_value(&request)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let answer = self
            .requests
            .ask(
                &self.server_id,
                ClientRequestKind::Elicitation,
                params,
                &context.ct,
            )
            .await;
        match answer {
            Some(ClientAnswer::Accept { content }) => {
                Ok(ElicitResult::new(ElicitationAction::Accept).with_content(content.0))
            }
            Some(ClientAnswer::Decline) => Ok(ElicitResult::new(ElicitationAction::Decline)),
            _ => Ok(ElicitResult::new(ElicitationAction::Cancel)),
        }
    }

    fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        let token = token_text(&params.progress_token);
        let call_id = self.progress.lock().unwrap().get(&token).cloned();
        if let Some(call_id) = call_id {
            self.logs.sink.progress(ProgressEvent {
                server_id: self.server_id.clone(),
                call_id,
                progress: params.progress,
                total: params.total,
                message: params.message.clone(),
            });
        }
        std::future::ready(())
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

    fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.logs.sink.resource_updated(ResourceUpdatedEvent {
            server_id: self.server_id.clone(),
            uri: params.uri,
        });
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

fn token_text(token: &rmcp::model::ProgressToken) -> String {
    match serde_json::to_value(token) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
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

    /// Teaches the redactor about more secret values (e.g. from environment variables used in a call).
    fn add_secrets(&self, server_id: &str, secrets: Vec<String>) {
        if secrets.is_empty() {
            return;
        }
        let mut all = self.redactor.lock().unwrap();
        let known = all
            .remove(server_id)
            .map(|r| r.secrets().to_vec())
            .unwrap_or_default();
        all.insert(
            server_id.to_owned(),
            Redactor::new(known.into_iter().chain(secrets)),
        );
    }

    fn redact(&self, server_id: &str, text: &str) -> String {
        match self.redactor.lock().unwrap().get(server_id) {
            Some(redactor) => redactor.redact(text),
            None => text.to_owned(),
        }
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
    roots: Arc<Mutex<Vec<Root>>>,
    cancel: Mutex<Option<RunningServiceCancellationToken>>,
    user_requested: AtomicBool,
    closed: watch::Receiver<bool>,
}

/// What was set up in a live session. It ends with the session: a new connection starts clean.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    /// The resources that are being watched.
    pub subscriptions: Vec<String>,
    /// The level sent with `logging/setLevel`, if any.
    pub log_level: Option<String>,
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
    /// Replaces the Azure developer tools as the token source (tests).
    azure_credential: Mutex<Option<Arc<dyn TokenCredential>>>,
    history: History,
    progress: Arc<Mutex<HashMap<String, String>>>,
    calls: Mutex<HashMap<String, CancellationToken>>,
    requests: Arc<ClientRequests>,
    state: Mutex<HashMap<String, SessionState>>,
}

impl SessionManager {
    pub fn new(
        db: Db,
        registry: Registry,
        environments: Environments,
        secrets: Arc<dyn SecretStore>,
        sink: Arc<dyn EventSink>,
    ) -> Arc<Self> {
        let db_for_history = db.clone();
        let requests = Arc::new(ClientRequests::new(sink.clone()));
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
            azure_credential: Mutex::new(None),
            history: History::new(db_for_history),
            progress: Arc::default(),
            calls: Mutex::default(),
            requests,
            state: Mutex::default(),
        })
    }

    /// Changes how long connecting may take (default 30 seconds).
    /// Uses `credential` instead of the Azure CLI for servers that sign in with the Azure login.
    pub fn set_azure_credential(&self, credential: Arc<dyn TokenCredential>) {
        *self.azure_credential.lock().unwrap() = Some(credential);
    }

    pub fn set_connect_timeout(&self, timeout: Duration) {
        *self.connect_timeout.lock().unwrap() = timeout;
    }

    /// The questions servers ask the user (sampling, elicitation).
    pub fn requests(&self) -> &Arc<ClientRequests> {
        &self.requests
    }

    /// What is set up in the live session of a server (empty when not connected).
    pub fn session_state(&self, server_id: &str) -> SessionState {
        self.state
            .lock()
            .unwrap()
            .get(server_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Starts watching a resource of a connected server.
    pub async fn subscribe_resource(&self, server_id: &str, uri: &str) -> DbResult<()> {
        crate::explorer::subscribe(&self.peer(server_id)?, uri).await?;
        let mut all = self.state.lock().unwrap();
        let subscriptions = &mut all.entry(server_id.to_owned()).or_default().subscriptions;
        if !subscriptions.iter().any(|u| u == uri) {
            subscriptions.push(uri.to_owned());
        }
        Ok(())
    }

    /// Stops watching a resource.
    pub async fn unsubscribe_resource(&self, server_id: &str, uri: &str) -> DbResult<()> {
        crate::explorer::unsubscribe(&self.peer(server_id)?, uri).await?;
        if let Some(state) = self.state.lock().unwrap().get_mut(server_id) {
            state.subscriptions.retain(|u| u != uri);
        }
        Ok(())
    }

    /// Sets the log level of a connected server.
    pub async fn set_log_level(&self, server_id: &str, level: &str) -> DbResult<()> {
        crate::explorer::set_log_level(&self.peer(server_id)?, level).await?;
        self.state
            .lock()
            .unwrap()
            .entry(server_id.to_owned())
            .or_default()
            .log_level = Some(level.to_owned());
        Ok(())
    }

    /// Tells a connected server that its roots changed, after the definition was edited.
    pub async fn roots_changed(&self, server_id: &str) -> DbResult<()> {
        let Some(session) = self.session(server_id) else {
            return Ok(());
        };
        let roots = roots_of(&self.registry.get(server_id).await?);
        if *session.roots.lock().unwrap() == roots {
            return Ok(());
        }
        *session.roots.lock().unwrap() = roots;
        // Servers that did not ask for roots cannot be told; the failure is not the user's.
        let _ = session.peer.notify_roots_list_changed().await;
        Ok(())
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

        let roots = Arc::new(Mutex::new(roots_of(&server)));
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
            server_id.to_owned(),
            prepared.redactor.clone(),
            self.sink.clone(),
        );
        let handler = StudioClient {
            server_id: server_id.to_owned(),
            logs: self.logger.clone(),
            progress: self.progress.clone(),
            requests: self.requests.clone(),
            roots: roots.clone(),
        };

        let timeout = *self.connect_timeout.lock().unwrap();
        // Everything that can fail while opening the transport happens in this block, so that the
        // session row is closed and the writer stopped on every error path.
        let connected: DbResult<RunningService<RoleClient, StudioClient>> = async {
            match prepared.transport {
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
                TransportKind::Http if server.input.oauth => {
                    let store = KeyringCredentialStore::new(self.secrets.clone(), server_id);
                    let client = oauth::auth_client(&prepared.url, store).await?;
                    let http =
                        StreamableHttpClientTransport::with_client(client, http_config(&prepared)?);
                    let transport =
                        RecordingTransport::<_, RoleClient>::new(http, writer.recorder());
                    initialize(handler, transport, timeout).await
                }
                TransportKind::Http if server.input.azure_credentials => {
                    let credential = self.azure_credential.lock().unwrap().clone();
                    let client = azure_client(
                        &prepared.url,
                        server.input.oauth_scopes.as_deref(),
                        credential,
                    )
                    .await?;
                    let http =
                        StreamableHttpClientTransport::with_client(client, http_config(&prepared)?);
                    let transport =
                        RecordingTransport::<_, RoleClient>::new(http, writer.recorder());
                    initialize(handler, transport, timeout).await
                }
                TransportKind::Http => {
                    let http = StreamableHttpClientTransport::from_config(http_config(&prepared)?);
                    let transport =
                        RecordingTransport::<_, RoleClient>::new(http, writer.recorder());
                    initialize(handler, transport, timeout).await
                }
            }
        }
        .await;

        let running = match connected {
            Ok(running) => running,
            Err(error) => {
                let error = self.explain_entra(&server, &prepared.url, error).await;
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
            roots,
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
        self.requests.drop_server(&session.server_id);
        self.state.lock().unwrap().remove(&session.server_id);
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

    /// Calls a tool. Protocol errors (unknown tool, invalid params) are `Err`; a tool that ran and
    /// failed is `Ok` with `is_error` set.
    pub async fn call_tool(&self, request: ToolCallRequest) -> DbResult<ToolCallResult> {
        let outcome = self.call_tool_inner(&request).await;
        let entry = match &outcome {
            Ok(done) => NewEntry {
                server_id: request.server_id.clone(),
                method: "tools/call".into(),
                target: request.tool_name.clone(),
                arguments: request.arguments.0.clone(),
                is_error: done.is_error,
                cancelled: done.cancelled,
                duration_ms: Some(done.duration_ms),
                result: (!done.cancelled)
                    .then(|| self.redact_json(&request.server_id, &done.result.0)),
                error: None,
            },
            Err(error) => NewEntry {
                server_id: request.server_id.clone(),
                method: "tools/call".into(),
                target: request.tool_name.clone(),
                arguments: request.arguments.0.clone(),
                is_error: true,
                cancelled: false,
                duration_ms: None,
                result: None,
                error: Some(self.logger.redact(&request.server_id, &error.to_string())),
            },
        };
        self.record(entry).await;
        outcome
    }

    /// Reads a resource and records the request in the history.
    pub async fn read_resource(&self, server_id: &str, uri: &str) -> DbResult<JsonValue> {
        let peer = self.peer(server_id)?;
        let started = std::time::Instant::now();
        let outcome = crate::explorer::read_resource(&peer, uri).await;
        self.record_simple(
            server_id,
            "resources/read",
            uri,
            serde_json::json!({}),
            &outcome,
            started,
        )
        .await;
        outcome
    }

    /// Gets a prompt and records the request in the history.
    pub async fn get_prompt(
        &self,
        server_id: &str,
        name: &str,
        arguments: &BTreeMap<String, String>,
    ) -> DbResult<JsonValue> {
        let peer = self.peer(server_id)?;
        let started = std::time::Instant::now();
        let outcome = crate::explorer::get_prompt(&peer, name, arguments).await;
        let args = serde_json::to_value(arguments).unwrap_or_default();
        self.record_simple(server_id, "prompts/get", name, args, &outcome, started)
            .await;
        outcome
    }

    /// The request history.
    pub fn history(&self) -> &History {
        &self.history
    }

    fn redact_json(&self, server_id: &str, value: &serde_json::Value) -> serde_json::Value {
        let text = value.to_string();
        let redacted = self.logger.redact(server_id, &text);
        if redacted == text {
            value.clone()
        } else {
            serde_json::from_str(&redacted).unwrap_or(serde_json::Value::Null)
        }
    }

    async fn record(&self, entry: NewEntry) {
        // History is a convenience; failing to store it must never fail the request itself.
        let _ = self.history.record(entry).await;
    }

    async fn record_simple(
        &self,
        server_id: &str,
        method: &str,
        target: &str,
        arguments: serde_json::Value,
        outcome: &DbResult<JsonValue>,
        started: std::time::Instant,
    ) {
        let (result, error) = match outcome {
            Ok(value) => (Some(self.redact_json(server_id, &value.0)), None),
            Err(error) => (
                None,
                Some(self.logger.redact(server_id, &error.to_string())),
            ),
        };
        self.record(NewEntry {
            server_id: server_id.to_owned(),
            method: method.into(),
            target: target.into(),
            arguments,
            is_error: outcome.is_err(),
            cancelled: false,
            duration_ms: Some(started.elapsed().as_millis() as i64),
            result,
            error,
        })
        .await;
    }

    async fn call_tool_inner(&self, request: &ToolCallRequest) -> DbResult<ToolCallResult> {
        let request = request.clone();
        let peer = self.peer(&request.server_id)?;
        let variables = match request.environment_id.as_deref() {
            Some(id) => {
                let env = self.environments.get(id).await?;
                let (values, secrets) =
                    resolve_values(self.secrets.as_ref(), &env.input.variables)?;
                self.logger.add_secrets(&request.server_id, secrets);
                values
            }
            None => BTreeMap::new(),
        };
        let arguments = match resolve_json(&request.arguments.0, &variables)? {
            serde_json::Value::Null => None,
            serde_json::Value::Object(map) => Some(map),
            _ => {
                return Err(DbError::Invalid(
                    "tool arguments must be a JSON object".into(),
                ))
            }
        };
        let mut params = CallToolRequestParams::new(request.tool_name.clone());
        params.arguments = arguments;

        let started = std::time::Instant::now();
        let handle = peer
            .send_cancellable_request(
                ClientRequest::CallToolRequest(CallToolRequest::new(params)),
                PeerRequestOptions::no_options(),
            )
            .await
            .map_err(|e| DbError::Connection(e.to_string()))?;

        let token = token_text(&handle.progress_token);
        let cancel = CancellationToken::new();
        self.progress
            .lock()
            .unwrap()
            .insert(token.clone(), request.call_id.clone());
        self.calls
            .lock()
            .unwrap()
            .insert(request.call_id.clone(), cancel.clone());

        let request_id = handle.id.clone();
        let outcome = tokio::select! {
            response = handle.await_response() => Some(response),
            () = cancel.cancelled() => None,
        };
        self.progress.lock().unwrap().remove(&token);
        self.calls.lock().unwrap().remove(&request.call_id);
        let duration_ms = started.elapsed().as_millis() as i64;

        match outcome {
            None => {
                let _ = peer
                    .notify_cancelled(CancelledNotificationParam::new(
                        Some(request_id),
                        Some("cancelled by the user".into()),
                    ))
                    .await;
                Ok(ToolCallResult {
                    call_id: request.call_id,
                    result: JsonValue::default(),
                    is_error: false,
                    cancelled: true,
                    duration_ms,
                })
            }
            Some(Err(error)) => Err(DbError::Connection(error.to_string())),
            Some(Ok(ServerResult::CallToolResult(result))) => Ok(ToolCallResult {
                call_id: request.call_id,
                is_error: result.is_error.unwrap_or(false),
                result: JsonValue(serde_json::to_value(&result).unwrap_or_default()),
                cancelled: false,
                duration_ms,
            }),
            Some(Ok(_)) => Err(DbError::Connection(
                "the server answered with an unexpected result type".into(),
            )),
        }
    }

    /// Cancels a running tool call. Returns false if the call is unknown or already finished.
    pub fn cancel_call(&self, call_id: &str) -> bool {
        match self.calls.lock().unwrap().get(call_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Signs in to an OAuth-protected server in the user's browser and stores the tokens.
    pub async fn sign_in(&self, server_id: &str, opener: &dyn UrlOpener) -> DbResult<()> {
        let server = self.registry.get(server_id).await?;
        if server.input.transport != TransportKind::Http || !server.input.oauth {
            return Err(DbError::Invalid(format!(
                "\"{}\" is not configured for OAuth sign-in",
                server.input.name
            )));
        }
        let url = server.input.url.clone().unwrap_or_default();
        // Sign-in only needs the URL; placeholders in it are resolved like for connecting.
        let variables = std::collections::BTreeMap::new();
        let url = resolve_str(&url, &variables).unwrap_or(url);
        let store = KeyringCredentialStore::new(self.secrets.clone(), server_id);
        let settings = oauth::SignInSettings {
            client_id: server.input.oauth_client_id.clone(),
            scopes: server.input.oauth_scopes.clone(),
            callback_port: server.input.oauth_callback_port,
        };
        match oauth::sign_in(&url, &settings, store, opener, oauth::SIGN_IN_TIMEOUT).await {
            Err(error) if azure_auth::uses_entra(&url).await => Err(entra_error(error, &url, true)),
            result => result,
        }
    }

    /// Adds a hint to a connection error when it comes from a server that uses Microsoft Entra ID
    /// without being set up for it.
    async fn explain_entra(&self, server: &ServerDefinition, url: &str, error: DbError) -> DbError {
        let input = &server.input;
        let rejected = error.to_string().contains("Auth required");
        if input.transport == TransportKind::Http
            && !input.oauth
            && !input.azure_credentials
            && rejected
            && azure_auth::uses_entra(url).await
        {
            return entra_error(error, url, false);
        }
        error
    }

    /// Forgets the stored OAuth credentials of a server and disconnects it.
    pub async fn sign_out(&self, server_id: &str) -> DbResult<()> {
        self.disconnect(server_id).await?;
        oauth::sign_out(&KeyringCredentialStore::new(
            self.secrets.clone(),
            server_id,
        ))
        .await
    }

    /// Whether OAuth credentials are stored for the server.
    pub async fn is_signed_in(&self, server_id: &str) -> bool {
        oauth::is_signed_in(&KeyringCredentialStore::new(
            self.secrets.clone(),
            server_id,
        ))
        .await
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

/// The command line of a stdio server, with environment, working directory, and the login-shell
/// `PATH` applied.
pub(crate) fn build_command(prepared: &Prepared) -> tokio::process::Command {
    // Search path for the child: the server's own PATH, else the login shell's merged with ours.
    let search_path = prepared.env.get("PATH").cloned().or_else(|| {
        path_env::login_shell_path()
            .map(|login| path_env::merge_paths(login, &std::env::var("PATH").unwrap_or_default()))
    });
    let program = resolve_program(
        &prepared.command,
        search_path.as_deref(),
        prepared.cwd.as_deref(),
    );
    let mut command = tokio::process::Command::new(program);
    command.args(&prepared.args);
    command.envs(&prepared.env);
    if let Some(cwd) = &prepared.cwd {
        command.current_dir(cwd);
    }
    if let Some(path) = search_path {
        command.env("PATH", path);
    }
    command
}

/// Finds the executable the way a shell would (including `PATHEXT`, so `npx` finds `npx.cmd` on
/// Windows, which `CreateProcess` cannot do by itself). Falls back to the plain name so that the
/// spawn error names what the user typed.
fn resolve_program(
    command: &str,
    search_path: Option<&str>,
    cwd: Option<&str>,
) -> std::path::PathBuf {
    let cwd = cwd
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let found = match search_path {
        Some(path) => which::which_in(command, Some(path), &cwd),
        None => which::which_in(command, std::env::var_os("PATH"), &cwd),
    };
    found.unwrap_or_else(|_| std::path::PathBuf::from(command))
}

fn spawn_stdio(
    prepared: &Prepared,
) -> DbResult<(TokioChildProcess, Option<tokio::process::ChildStderr>)> {
    TokioChildProcess::builder(build_command(prepared))
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DbError::Connection(format!("could not start \"{}\": {e}", prepared.command)))
}

/// `error` with the advice for a server behind Microsoft Entra ID. `oauth`: the server is already
/// set up for OAuth, which fails without a client ID registered by hand.
fn entra_error(error: DbError, url: &str, oauth: bool) -> DbError {
    let advice = if oauth {
        "This server uses Microsoft Entra ID, which has no dynamic client registration. Turn on \"Uses the Azure login\" in the server settings, or enter an OAuth client ID."
    } else if azure_auth::is_azure_devops(url) {
        "This server uses Microsoft Entra ID. Use the Azure DevOps preset in the server settings (it turns on \"Uses the Azure login\"), then use \"Sign in with Azure\" on the server page."
    } else {
        "This server uses Microsoft Entra ID. Turn on \"Uses the Azure login\" in the server settings, then use \"Sign in with Azure\" on the server page."
    };
    DbError::Connection(format!("{error}. {advice}"))
}

/// Builds the HTTP client for a server that signs in with the Azure login. The first token is
/// fetched here, so that a missing or expired login is reported before the connection is opened.
async fn azure_client(
    url: &str,
    scopes: Option<&str>,
    credential: Option<Arc<dyn TokenCredential>>,
) -> DbResult<AzureAuthClient> {
    let http = reqwest::Client::new();
    let scopes = azure_auth::resolve_scopes(&http, url, scopes).await?;
    let credential = match credential {
        Some(credential) => credential,
        None => azure_auth::developer_tools()?,
    };
    let tokens = Arc::new(TokenCache::new(credential, scopes));
    tokens.token().await?;
    Ok(AzureAuthClient::new(http, tokens))
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
            oauth: false,
            oauth_client_id: None,
            oauth_scopes: None,
            oauth_callback_port: None,
            azure_credentials: false,
            roots: vec![],
        }
    }

    fn variables() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("bin".into(), "node".into()),
            ("port".into(), "3000".into()),
            ("host".into(), "example.com".into()),
        ])
    }

    #[cfg(unix)]
    #[test]
    fn programs_are_found_on_the_given_search_path() {
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("my-mcp-tool");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = dir.path().to_string_lossy().into_owned();
        assert_eq!(resolve_program("my-mcp-tool", Some(&path), None), tool);
        // Unknown programs keep their name so the spawn error is understandable.
        assert_eq!(
            resolve_program("nope-nope", Some(&path), None),
            std::path::PathBuf::from("nope-nope")
        );
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
