//! Compatibility tests against the official `@modelcontextprotocol/server-everything` reference
//! server. They need Node.js and network access to npm, so they only run when
//! `MCP_STUDIO_EVERYTHING=1` is set (CI does that).

use std::{collections::BTreeMap, process::Stdio, sync::Arc, time::Duration};

use mcp_studio_core::{
    db::Db,
    environments::Environments,
    events::{CollectingSink, ConnectionState},
    explorer,
    model::JsonValue,
    registry::{Registry, ServerInput, TransportKind},
    secrets::{MemoryStore, SecretStore},
    session::{SessionManager, ToolCallRequest},
};
use tokio::process::Command;

fn enabled() -> bool {
    std::env::var_os("MCP_STUDIO_EVERYTHING").is_some()
}

struct Harness {
    db: Db,
    registry: Registry,
    manager: Arc<SessionManager>,
    sink: Arc<CollectingSink>,
}

async fn harness() -> Harness {
    let db = Db::open_in_memory().await.unwrap();
    let registry = Registry::new(db.clone());
    let sink = Arc::new(CollectingSink::default());
    let manager = SessionManager::new(
        db.clone(),
        registry.clone(),
        Environments::new(db.clone()),
        Arc::new(MemoryStore::default()) as Arc<dyn SecretStore>,
        sink.clone(),
    );
    // npx may have to download the package on first use.
    manager.set_connect_timeout(Duration::from_secs(120));
    Harness {
        db,
        registry,
        manager,
        sink,
    }
}

fn call(server_id: &str, tool: &str, arguments: serde_json::Value) -> ToolCallRequest {
    ToolCallRequest {
        server_id: server_id.to_owned(),
        tool_name: tool.to_owned(),
        arguments: JsonValue(arguments),
        environment_id: None,
        call_id: mcp_studio_core::db::new_id(),
    }
}

async fn exercise(h: &Harness, server_id: &str) {
    let session = h
        .manager
        .connect(server_id, None)
        .await
        .expect("connect to server-everything");
    let details = explorer::details(&session.peer).unwrap();
    assert!(details.has_tools);

    let tools = explorer::list_tools(&session.peer).await.unwrap();
    assert!(
        tools.len() >= 5,
        "the reference server offers many tools: {}",
        tools.len()
    );
    let echo = tools.iter().find(|t| t.name == "echo").expect("echo tool");
    assert_eq!(
        echo.input_schema.0["properties"]["message"]["type"],
        "string"
    );

    let echoed = h
        .manager
        .call_tool(call(
            server_id,
            "echo",
            serde_json::json!({"message": "hi from MCP Studio"}),
        ))
        .await
        .unwrap();
    assert!(!echoed.is_error);
    assert!(echoed.result.0["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("hi from MCP Studio"));

    if details.has_resources {
        assert!(!explorer::list_resources(&session.peer)
            .await
            .unwrap()
            .is_empty());
    }
    if details.has_prompts {
        assert!(!explorer::list_prompts(&session.peer)
            .await
            .unwrap()
            .is_empty());
    }

    h.manager.disconnect(server_id).await.unwrap();
    assert_eq!(
        h.sink.statuses.lock().unwrap().last().map(|s| s.state),
        Some(ConnectionState::Disconnected)
    );

    tokio::time::sleep(Duration::from_millis(500)).await;
    let recorded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_id = ?")
        .bind(&session.session_id)
        .fetch_one(h.db.pool())
        .await
        .unwrap();
    assert!(recorded >= 6, "messages recorded: {recorded}");
}

#[tokio::test]
async fn server_everything_over_stdio() {
    if !enabled() {
        eprintln!("skipped: set MCP_STUDIO_EVERYTHING=1 to run");
        return;
    }
    let h = harness().await;
    let server = h
        .registry
        .create(ServerInput {
            name: "everything-stdio".into(),
            transport: TransportKind::Stdio,
            command: Some("npx".into()),
            args: vec![
                "-y".into(),
                "@modelcontextprotocol/server-everything".into(),
                "stdio".into(),
            ],
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
        })
        .await
        .unwrap();
    exercise(&h, &server.id).await;
}

#[tokio::test]
async fn server_everything_over_streamable_http() {
    if !enabled() {
        eprintln!("skipped: set MCP_STUDIO_EVERYTHING=1 to run");
        return;
    }
    // Find a free port and hand it to the server.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let _child = Command::new("npx")
        .args([
            "-y",
            "@modelcontextprotocol/server-everything",
            "streamableHttp",
        ])
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("npx");
    // Wait until the port accepts connections (the first start may download the package).
    tokio::time::timeout(Duration::from_secs(120), async {
        while tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
        {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .expect("server-everything starts in time");

    let h = harness().await;
    let server = h
        .registry
        .create(ServerInput {
            name: "everything-http".into(),
            transport: TransportKind::Http,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            cwd: None,
            url: Some(format!("http://127.0.0.1:{port}/mcp")),
            headers: BTreeMap::new(),
            tags: vec![],
            oauth: false,
            oauth_client_id: None,
            oauth_scopes: None,
            oauth_callback_port: None,
            azure_credentials: false,
        })
        .await
        .unwrap();
    exercise(&h, &server.id).await;
}
