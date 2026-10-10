//! HTTP proxy mode end to end: a real MCP client uses a remote server through MCP Studio.

use std::{collections::BTreeMap, process::Stdio, sync::Arc, time::Duration};

use mcp_studio_core::{
    db::Db,
    environments::Environments,
    events::CollectingSink,
    http_proxy::HttpProxy,
    registry::{Registry, ServerInput, TransportKind},
    secrets::{MemoryStore, SecretStore},
};
use rmcp::{
    model::{CallToolRequestParams, ClientConfig},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
};

fn server_binary() -> std::path::PathBuf {
    static BUILD: std::sync::Once = std::sync::Once::new();
    BUILD.call_once(|| {
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "-p", "mcp-studio-testserver"])
            .status()
            .unwrap();
        assert!(status.success());
    });
    let exe = if cfg!(windows) {
        "mcp-studio-testserver.exe"
    } else {
        "mcp-studio-testserver"
    };
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug")
        .join(exe)
        .canonicalize()
        .unwrap()
}

/// Starts the reference server in HTTP mode; returns the process and its URL.
async fn upstream() -> (Child, String) {
    let mut child = Command::new(server_binary())
        .args(["--http", "0"])
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let url = lines
        .next_line()
        .await
        .unwrap()
        .unwrap()
        .trim_start_matches("listening on ")
        .to_owned();
    (child, url)
}

struct Harness {
    db: Db,
    proxy: HttpProxy,
    registry: Registry,
    secrets: Arc<MemoryStore>,
    _upstream: Child,
    upstream_url: String,
}

async fn harness() -> Harness {
    let db = Db::open_in_memory().await.unwrap();
    let registry = Registry::new(db.clone());
    let secrets = Arc::new(MemoryStore::default());
    let (child, upstream_url) = upstream().await;
    let proxy = HttpProxy::start(
        db.clone(),
        registry.clone(),
        Environments::new(db.clone()),
        secrets.clone() as Arc<dyn SecretStore>,
        Arc::new(CollectingSink::default()),
        0,
    )
    .await
    .unwrap();
    Harness {
        db,
        proxy,
        registry,
        secrets,
        _upstream: child,
        upstream_url,
    }
}

async fn add_http_server(h: &Harness, name: &str, headers: BTreeMap<String, String>) {
    h.registry
        .create(ServerInput {
            name: name.into(),
            transport: TransportKind::Http,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            cwd: None,
            url: Some(h.upstream_url.clone()),
            headers,
            tags: vec![],
            oauth: false,
            oauth_client_id: None,
            oauth_scopes: None,
            oauth_callback_port: None,
            azure_credentials: false,
            roots: vec![],
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_client_uses_a_remote_server_through_the_proxy_and_traffic_is_recorded() {
    let h = harness().await;
    add_http_server(&h, "Remote", BTreeMap::new()).await;

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(h.proxy.url_for("remote")),
    );
    let client = ClientConfig::default()
        .serve(transport)
        .await
        .expect("initialize through the proxy");
    let tools = client.list_tools(None).await.unwrap();
    assert!(tools.tools.iter().any(|t| t.name == "echo"));
    let result = client
        .call_tool(
            CallToolRequestParams::new("add").with_arguments(
                serde_json::json!({"a": 40, "b": 2})
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
    h.proxy.shutdown().await;

    tokio::time::sleep(Duration::from_millis(300)).await;
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
    let sent: Vec<_> = rows
        .iter()
        .filter(|(d, _)| d == "out")
        .filter_map(|(_, m)| m.clone())
        .collect();
    assert!(sent.contains(&"initialize".to_owned()), "{sent:?}");
    assert!(sent.contains(&"tools/call".to_owned()), "{sent:?}");
    assert!(
        rows.iter()
            .filter(|(d, m)| d == "in" && m.is_none())
            .count()
            >= 3,
        "responses (initialize, tools/list, tools/call) recorded: {rows:?}"
    );
    let ended: Option<i64> = sqlx::query_scalar("SELECT ended_at FROM sessions")
        .fetch_one(h.db.pool())
        .await
        .unwrap();
    assert!(ended.is_some());
}

#[tokio::test]
async fn the_servers_headers_are_added_and_secrets_stay_masked_in_the_recording() {
    let h = harness().await;
    h.secrets.set("token", "Bearer sk-live-123").unwrap();
    add_http_server(
        &h,
        "Secured",
        BTreeMap::from([("Authorization".into(), "keyring:token".into())]),
    )
    .await;

    // A plain HTTP client sees the upstream answer; the point is that the proxy accepted the request
    // without the client sending credentials and that nothing secret ended up in the database.
    let http = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}
    });
    let response = http
        .post(h.proxy.url_for("Secured"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    let _ = response.text().await.unwrap();
    h.proxy.shutdown().await;

    tokio::time::sleep(Duration::from_millis(300)).await;
    let payloads: Vec<String> = sqlx::query_scalar("SELECT payload FROM messages")
        .fetch_all(h.db.pool())
        .await
        .unwrap();
    assert!(!payloads.is_empty());
    assert!(payloads.iter().all(|p| !p.contains("sk-live-123")));
}

#[tokio::test]
async fn unknown_and_stdio_servers_are_rejected_with_a_clear_message() {
    let h = harness().await;
    let http = reqwest::Client::new();

    let missing = http
        .post(h.proxy.url_for("nope"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
    assert!(missing.text().await.unwrap().contains("no server named"));

    h.registry
        .create(ServerInput {
            name: "Local".into(),
            transport: TransportKind::Stdio,
            command: Some("x".into()),
            args: vec![],
            env: BTreeMap::new(),
            cwd: None,
            url: None,
            headers: BTreeMap::new(),
            tags: vec![],
            oauth: false,
            oauth_client_id: None,
            oauth_scopes: None,
            oauth_callback_port: None,
            azure_credentials: false,
            roots: vec![],
        })
        .await
        .unwrap();
    let stdio = http
        .post(h.proxy.url_for("Local"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(stdio.status(), 400);
    assert!(stdio.text().await.unwrap().contains("mcp-studio-proxy"));
}

#[tokio::test]
async fn requests_for_another_host_or_from_another_site_are_refused_before_forwarding() {
    let h = harness().await;
    add_http_server(&h, "demo", BTreeMap::new()).await;
    let url = h.proxy.url_for("demo");
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    let post = || {
        reqwest::Client::new()
            .post(&url)
            .header("content-type", "application/json")
            .body(body)
    };

    // A rebinding page reaches 127.0.0.1 but its requests still name the attacker's host.
    let rebound = post().header("host", "evil.example").send().await.unwrap();
    assert_eq!(rebound.status(), 403);

    let foreign_page = post()
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(foreign_page.status(), 403);

    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(h.db.pool())
        .await
        .unwrap();
    assert_eq!(sessions, 0, "a refused request must not reach the server");

    let local_page = post()
        .header("origin", "http://localhost:6274")
        .send()
        .await
        .unwrap();
    assert_ne!(local_page.status(), 403);
}

#[tokio::test]
async fn the_proxy_url_encodes_server_names() {
    let h = harness().await;
    assert!(h.proxy.url_for("My Server").ends_with("/mcp/My%20Server"));
}
