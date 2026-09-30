//! Proxy mode end to end: a real MCP client talks to the reference server through MCP Studio.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use mcp_studio_core::{
    db::Db,
    environments::Environments,
    events::CollectingSink,
    proxy::{discovery_path, Discovery, ProxyService},
    registry::{Registry, ServerInput, TransportKind},
    secrets::{MemoryStore, SecretStore},
};
use rmcp::{
    model::{CallToolRequestParams, ClientConfig},
    transport::{ConfigureCommandExt, TokioChildProcess},
    ServiceExt,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    process::Command,
};

fn binary(name: &str, package: &str) -> PathBuf {
    static BUILD: std::sync::Once = std::sync::Once::new();
    BUILD.call_once(|| {
        let status = std::process::Command::new(env!("CARGO"))
            .args([
                "build",
                "-p",
                "mcp-studio-testserver",
                "-p",
                "mcp-studio-proxy",
            ])
            .status()
            .unwrap();
        assert!(status.success());
    });
    let _ = package;
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug")
        .join(exe)
        .canonicalize()
        .unwrap()
}

struct Harness {
    db: Db,
    proxy: ProxyService,
    discovery: PathBuf,
    server_name: String,
    _dir: tempfile::TempDir,
}

async fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_in_memory().await.unwrap();
    let registry = Registry::new(db.clone());
    let server_name = "Reference".to_owned();
    registry
        .create(ServerInput {
            name: server_name.clone(),
            transport: TransportKind::Stdio,
            command: Some(
                binary("mcp-studio-testserver", "mcp-studio-testserver")
                    .to_string_lossy()
                    .into_owned(),
            ),
            args: vec![],
            env: BTreeMap::new(),
            cwd: None,
            url: None,
            headers: BTreeMap::new(),
            tags: vec![],
            oauth: false,
        })
        .await
        .unwrap();
    let discovery = discovery_path(dir.path());
    let proxy = ProxyService::start(
        db.clone(),
        registry,
        Environments::new(db.clone()),
        Arc::new(MemoryStore::default()) as Arc<dyn SecretStore>,
        Arc::new(CollectingSink::default()),
        &discovery,
    )
    .await
    .unwrap();
    Harness {
        db,
        proxy,
        discovery,
        server_name,
        _dir: dir,
    }
}

fn read_discovery(path: &PathBuf) -> Discovery {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

async fn handshake(
    port: u16,
    token: &str,
    server: &str,
) -> (
    BufReader<tokio::net::tcp::OwnedReadHalf>,
    tokio::net::tcp::OwnedWriteHalf,
    String,
) {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let (read, mut write) = stream.into_split();
    let hello = serde_json::json!({"token": token, "server": server}).to_string() + "\n";
    write.write_all(hello.as_bytes()).await.unwrap();
    let mut reader = BufReader::new(read);
    let mut reply = String::new();
    reader.read_line(&mut reply).await.unwrap();
    (reader, write, reply)
}

#[tokio::test]
async fn rejects_wrong_tokens_and_unknown_servers() {
    let h = harness().await;
    let info = read_discovery(&h.discovery);
    assert_eq!(info.port, h.proxy.port());

    let (_, _, reply) = handshake(info.port, "wrong", &h.server_name).await;
    assert!(reply.contains("invalid token"), "{reply}");

    let (_, _, reply) = handshake(info.port, &info.token, "No such server").await;
    assert!(reply.contains("no server named"), "{reply}");
}

#[tokio::test]
async fn a_client_can_use_a_server_through_the_proxy_and_everything_is_recorded() {
    let h = harness().await;
    let info = read_discovery(&h.discovery);
    let (reader, writer, reply) = handshake(info.port, &info.token, &h.server_name).await;
    assert_eq!(reply.trim(), r#"{"ok":true}"#);

    let client = ClientConfig::default()
        .serve((reader, writer))
        .await
        .expect("initialize through proxy");
    let tools = client.list_tools(None).await.unwrap();
    assert!(tools.tools.iter().any(|t| t.name == "echo"));
    let result = client
        .call_tool(
            CallToolRequestParams::new("add").with_arguments(
                serde_json::json!({"a": 20, "b": 22})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&result).unwrap()["content"][0]["text"],
        "42"
    );
    client.cancel().await.ok();

    tokio::time::sleep(Duration::from_millis(500)).await;
    let origin: String = sqlx::query_scalar("SELECT origin FROM sessions")
        .fetch_one(h.db.pool())
        .await
        .unwrap();
    assert_eq!(origin, "proxy");
    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT direction, method FROM messages ORDER BY id")
            .fetch_all(h.db.pool())
            .await
            .unwrap();
    let methods = |direction: &str| -> Vec<String> {
        rows.iter()
            .filter(|(d, _)| d == direction)
            .filter_map(|(_, m)| m.clone())
            .collect()
    };
    assert!(methods("out").contains(&"initialize".to_owned()));
    assert!(methods("out").contains(&"tools/call".to_owned()));
    assert!(
        rows.iter().any(|(d, m)| d == "in" && m.is_none()),
        "responses recorded"
    );
    let ended: Option<i64> = sqlx::query_scalar("SELECT ended_at FROM sessions")
        .fetch_one(h.db.pool())
        .await
        .unwrap();
    assert!(ended.is_some(), "session is closed when the client leaves");
}

#[tokio::test]
async fn the_proxy_program_bridges_stdio_to_the_app() {
    let h = harness().await;
    let proxy_bin = binary("mcp-studio-proxy", "mcp-studio-proxy");
    let transport = TokioChildProcess::new(Command::new(proxy_bin).configure(|cmd| {
        cmd.args(["--server", "reference", "--discovery"])
            .arg(&h.discovery);
    }))
    .unwrap();
    let client = ClientConfig::default()
        .serve(transport)
        .await
        .expect("initialize via proxy program");
    let tools = client.list_tools(None).await.unwrap();
    assert!(tools.tools.iter().any(|t| t.name == "fail"));
    client.cancel().await.ok();

    tokio::time::sleep(Duration::from_millis(500)).await;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE method = 'tools/list'")
            .fetch_one(h.db.pool())
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn the_proxy_program_explains_when_the_app_is_not_running() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(binary("mcp-studio-proxy", "mcp-studio-proxy"))
        .args(["--server", "x", "--discovery"])
        .arg(dir.path().join("proxy.json"))
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not seem to be running"), "{stderr}");
    let mut sink = Vec::new();
    let _ = tokio::io::empty().read_to_end(&mut sink).await;
}
