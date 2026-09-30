//! Proxy mode: record what a real MCP client (Claude Code, Claude Desktop, an IDE) does.
//!
//! The client is configured to start `mcp-studio-proxy --server <name>` instead of the real server.
//! That small program connects to the [`ProxyService`] inside the running app over a loopback TCP
//! connection secured with a token from `proxy.json`. The app starts the real server, records every
//! JSON-RPC message that passes in either direction, and forwards the bytes unchanged.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};

use crate::{
    db::{new_id, now_ms, Db, DbError, DbResult},
    environments::Environments,
    events::{EventSink, LogEvent, LogSource},
    message_store::MessageWriter,
    recording::{Direction, RecordedMessage, Recorder},
    registry::{Registry, ServerDefinition, TransportKind},
    secrets::SecretStore,
    session::{build_command, prepare, Prepared},
};

/// Longest single JSON-RPC line we accept (images can be large).
const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;

/// Contents of `proxy.json`, read by the `mcp-studio-proxy` program.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Discovery {
    pub port: u16,
    pub token: String,
    pub version: String,
}

/// First line the proxy program sends.
#[derive(Debug, Serialize, Deserialize)]
pub struct Hello {
    pub token: String,
    /// Server id or name.
    pub server: String,
}

/// Answer to [`Hello`].
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

struct Inner {
    db: Db,
    registry: Registry,
    environments: Environments,
    secrets: Arc<dyn SecretStore>,
    sink: Arc<dyn EventSink>,
    token: String,
    environment: Mutex<Option<String>>,
}

/// The app side of proxy mode.
#[derive(Clone)]
pub struct ProxyService {
    inner: Arc<Inner>,
    port: u16,
}

impl ProxyService {
    /// Starts listening on a loopback port and writes the discovery file.
    pub async fn start(
        db: Db,
        registry: Registry,
        environments: Environments,
        secrets: Arc<dyn SecretStore>,
        sink: Arc<dyn EventSink>,
        discovery_file: &Path,
    ) -> DbResult<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let token = format!("{}{}", new_id().replace('-', ""), new_id().replace('-', ""));
        write_discovery(
            discovery_file,
            &Discovery {
                port,
                token: token.clone(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
        )?;
        let service = Self {
            inner: Arc::new(Inner {
                db,
                registry,
                environments,
                secrets,
                sink,
                token,
                environment: Mutex::new(None),
            }),
            port,
        };
        let acceptor = service.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let handler = acceptor.clone();
                tokio::spawn(async move {
                    let _ = handler.handle(stream).await;
                });
            }
        });
        Ok(service)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The environment whose variables new proxy sessions use.
    pub fn set_environment(&self, id: Option<String>) {
        *self.inner.environment.lock().unwrap() = id;
    }

    async fn handle(&self, stream: TcpStream) -> DbResult<()> {
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        // The hello is small; refuse anything absurd before authenticating.
        let read = (&mut reader).take(64 * 1024).read_line(&mut line).await?;
        if read == 0 {
            return Ok(());
        }
        let hello: Hello = match serde_json::from_str(line.trim()) {
            Ok(hello) => hello,
            Err(_) => return reject(&mut write_half, "malformed hello").await,
        };
        if !constant_time_eq(hello.token.as_bytes(), self.inner.token.as_bytes()) {
            return reject(&mut write_half, "invalid token").await;
        }
        let server = match self.find_server(&hello.server).await {
            Ok(server) => server,
            Err(error) => return reject(&mut write_half, &error.to_string()).await,
        };
        if server.input.transport != TransportKind::Stdio {
            return reject(
                &mut write_half,
                &format!(
                    "\"{}\" is an HTTP server; point the client at its HTTP proxy URL instead",
                    server.input.name
                ),
            )
            .await;
        }
        let prepared = match self.prepare(&server).await {
            Ok(prepared) => prepared,
            Err(error) => return reject(&mut write_half, &error.to_string()).await,
        };
        let mut child = match build_command(&prepared)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                return reject(
                    &mut write_half,
                    &format!("could not start \"{}\": {error}", prepared.command),
                )
                .await
            }
        };
        write_half.write_all(b"{\"ok\":true}\n").await?;

        let session_id = new_id();
        sqlx::query(
            "INSERT INTO sessions (id, server_id, origin, started_at) VALUES (?, ?, 'proxy', ?)",
        )
        .bind(&session_id)
        .bind(&server.id)
        .bind(now_ms())
        .execute(self.inner.db.pool())
        .await?;
        let writer = MessageWriter::spawn(
            self.inner.db.clone(),
            session_id.clone(),
            server.id.clone(),
            prepared.redactor.clone(),
            self.inner.sink.clone(),
        );
        let recorder = writer.recorder();

        let mut child_stdin = child.stdin.take().expect("piped");
        let child_stdout = child.stdout.take().expect("piped");
        let child_stderr = child.stderr.take().expect("piped");

        let log_sink = self.inner.sink.clone();
        let server_id = server.id.clone();
        let redactor = prepared.redactor.clone();
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(child_stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                log_sink.log(LogEvent {
                    server_id: server_id.clone(),
                    source: LogSource::Stderr,
                    level: "info".into(),
                    line: redactor.redact(&line),
                    ts: now_ms(),
                });
            }
        });

        // Client -> server (recorded as "out": the client is the party that sends).
        let out_recorder = recorder.clone();
        let to_server = tokio::spawn(async move {
            let _ = pump(reader, &mut child_stdin, out_recorder, Direction::Out).await;
            let _ = child_stdin.shutdown().await;
        });
        // Server -> client.
        let in_recorder = recorder.clone();
        let mut client_out = write_half;
        let to_client = tokio::spawn(async move {
            let _ = pump(
                BufReader::new(child_stdout),
                &mut client_out,
                in_recorder,
                Direction::In,
            )
            .await;
            let _ = client_out.shutdown().await;
        });

        // Either side ending ends the session.
        tokio::select! {
            _ = to_client => {}
            _ = child.wait() => {}
        }
        to_server.abort();
        let _ = child.kill().await;
        let _ = stderr_task.await;
        drop(recorder);
        writer.finish().await;
        sqlx::query("UPDATE sessions SET ended_at = ? WHERE id = ?")
            .bind(now_ms())
            .bind(&session_id)
            .execute(self.inner.db.pool())
            .await?;
        Ok(())
    }

    async fn find_server(&self, key: &str) -> DbResult<ServerDefinition> {
        if let Ok(server) = self.inner.registry.get(key).await {
            return Ok(server);
        }
        self.inner
            .registry
            .list()
            .await?
            .into_iter()
            .find(|s| s.input.name.eq_ignore_ascii_case(key))
            .ok_or_else(|| DbError::NotFound(format!("no server named \"{key}\" in MCP Studio")))
    }

    async fn prepare(&self, server: &ServerDefinition) -> DbResult<Prepared> {
        let environment = self.inner.environment.lock().unwrap().clone();
        let variables: BTreeMap<String, String> = match environment {
            Some(id) => self.inner.environments.get(&id).await?.input.variables,
            None => BTreeMap::new(),
        };
        prepare(server, &variables, self.inner.secrets.as_ref())
    }
}

async fn reject<W: AsyncWrite + Unpin>(writer: &mut W, message: &str) -> DbResult<()> {
    let reply = serde_json::to_string(&Reply {
        ok: false,
        error: Some(message.to_owned()),
    })
    .unwrap_or_default();
    writer.write_all(reply.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    Ok(())
}

/// Copies newline-delimited messages, recording each one. Bytes are forwarded exactly as read.
async fn pump<R, W>(
    mut reader: BufReader<R>,
    writer: &mut W,
    recorder: Arc<dyn Recorder>,
    direction: Direction,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = (&mut reader)
            .take(MAX_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
            .await?;
        if read == 0 {
            return Ok(());
        }
        if line.len() > MAX_LINE_BYTES {
            return Err(std::io::Error::other("message too large"));
        }
        writer.write_all(&line).await?;
        writer.flush().await?;
        let trimmed = line.trim_ascii();
        if trimmed.is_empty() {
            continue;
        }
        let payload = serde_json::from_slice(trimmed).unwrap_or_else(|_| {
            serde_json::Value::String(String::from_utf8_lossy(trimmed).into_owned())
        });
        recorder.record(RecordedMessage {
            direction,
            ts: now_ms(),
            bytes: trimmed.len(),
            payload,
        });
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Writes `proxy.json` (readable only by the current user on Unix).
pub fn write_discovery(path: &Path, discovery: &Discovery) -> DbResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json =
        serde_json::to_string_pretty(discovery).map_err(|e| DbError::Invalid(e.to_string()))?;
    std::fs::write(path, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// What the UI needs to help the user configure a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProxyInfo {
    /// Loopback port the stdio proxy program connects to.
    pub control_port: u16,
    /// Absolute path of the `mcp-studio-proxy` program, if it was found.
    pub proxy_binary: Option<String>,
    pub discovery_file: String,
}

/// Finds the `mcp-studio-proxy` program: `MCP_STUDIO_PROXY_BIN`, then next to the running
/// executable (where bundled sidecars are installed, and where `cargo build` puts it in development).
pub fn locate_proxy_binary() -> Option<PathBuf> {
    locate_proxy_binary_with(
        std::env::var_os("MCP_STUDIO_PROXY_BIN").map(PathBuf::from),
        std::env::current_exe().ok(),
    )
}

fn locate_proxy_binary_with(
    override_path: Option<PathBuf>,
    current_exe: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(path) = override_path.filter(|p| p.is_file()) {
        return Some(path);
    }
    let dir = current_exe?.parent()?.to_path_buf();
    let name = if cfg!(windows) {
        "mcp-studio-proxy.exe"
    } else {
        "mcp-studio-proxy"
    };
    // Test binaries live in `deps/`; the real program is one level up.
    [dir.join(name), dir.join("..").join(name)]
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| p.canonicalize().unwrap_or(p))
}

impl ProxyService {
    pub fn info(&self, discovery_file: &Path) -> ProxyInfo {
        ProxyInfo {
            control_port: self.port,
            proxy_binary: locate_proxy_binary().map(|p| p.to_string_lossy().into_owned()),
            discovery_file: discovery_file.to_string_lossy().into_owned(),
        }
    }
}

/// Default location of `proxy.json` for a given app data directory.
pub fn discovery_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("proxy.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_compare_matches_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn discovery_file_roundtrips_and_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = discovery_path(&dir.path().join("nested"));
        let info = Discovery {
            port: 4242,
            token: "t".into(),
            version: "1".into(),
        };
        write_discovery(&path, &info).unwrap();
        let read: Discovery =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read, info);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn locates_the_proxy_next_to_the_executable_or_via_override() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "mcp-studio-proxy.exe"
        } else {
            "mcp-studio-proxy"
        };
        let program = dir.path().join(name);
        std::fs::write(&program, "").unwrap();
        let app = dir.path().join("mcp-studio");
        assert_eq!(
            locate_proxy_binary_with(None, Some(app.clone()))
                .map(|p| p.file_name().unwrap().to_owned()),
            Some(name.into())
        );
        let nested = dir.path().join("deps").join("test-binary");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        assert!(
            locate_proxy_binary_with(None, Some(nested)).is_some(),
            "found one level up"
        );
        assert!(locate_proxy_binary_with(
            Some(dir.path().join("missing")),
            Some(dir.path().join("x/y"))
        )
        .is_none());
        assert_eq!(
            locate_proxy_binary_with(Some(program.clone()), None),
            Some(program)
        );
    }

    #[test]
    fn reply_omits_the_error_when_ok() {
        assert_eq!(
            serde_json::to_string(&Reply {
                ok: true,
                error: None
            })
            .unwrap(),
            r#"{"ok":true}"#
        );
    }
}
