//! SessionManager against the reference server (stdio and Streamable HTTP).

use std::{
    collections::BTreeMap,
    process::Stdio,
    sync::{Arc, Once},
    time::Duration,
};

use mcp_studio_core::{
    db::Db,
    environments::{EnvironmentInput, Environments},
    events::{CollectingSink, ConnectionState},
    registry::{Registry, ServerInput, TransportKind},
    secrets::{MemoryStore, SecretStore},
    session::SessionManager,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

fn server_binary() -> String {
    static BUILD: Once = Once::new();
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
        .to_string_lossy()
        .into_owned()
}

struct Harness {
    db: Db,
    registry: Registry,
    environments: Environments,
    secrets: Arc<MemoryStore>,
    sink: Arc<CollectingSink>,
    manager: Arc<SessionManager>,
}

async fn harness() -> Harness {
    let db = Db::open_in_memory().await.unwrap();
    let registry = Registry::new(db.clone());
    let environments = Environments::new(db.clone());
    let secrets = Arc::new(MemoryStore::default());
    let sink = Arc::new(CollectingSink::default());
    let manager = SessionManager::new(
        db.clone(),
        registry.clone(),
        environments.clone(),
        secrets.clone() as Arc<dyn SecretStore>,
        sink.clone(),
    );
    Harness {
        db,
        registry,
        environments,
        secrets,
        sink,
        manager,
    }
}

fn stdio_server(command: &str, args: &[&str]) -> ServerInput {
    ServerInput {
        name: format!("srv-{command}-{}", args.join("-")),
        transport: TransportKind::Stdio,
        command: Some(command.into()),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        env: BTreeMap::new(),
        cwd: None,
        url: None,
        headers: BTreeMap::new(),
        tags: vec![],
    }
}

fn states(sink: &CollectingSink) -> Vec<ConnectionState> {
    sink.statuses
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.state)
        .collect()
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test]
async fn connects_lists_tools_records_and_disconnects() {
    let h = harness().await;
    let server = h
        .registry
        .create(stdio_server(&server_binary(), &[]))
        .await
        .unwrap();

    let session = h.manager.connect(&server.id, None).await.expect("connect");
    let tools = session.peer.list_all_tools().await.unwrap();
    assert!(tools.iter().any(|t| t.name == "echo"));
    assert_eq!(
        states(&h.sink),
        [ConnectionState::Connecting, ConnectionState::Connected]
    );

    // Connecting twice returns the same session.
    let again = h.manager.connect(&server.id, None).await.unwrap();
    assert_eq!(again.session_id, session.session_id);

    h.manager.disconnect(&server.id).await.unwrap();
    assert!(h.manager.session(&server.id).is_none());
    assert_eq!(states(&h.sink).last(), Some(&ConnectionState::Disconnected));

    settle().await;
    let (messages, ended, protocol): (i64, Option<i64>, Option<String>) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM messages WHERE session_id = s.id), s.ended_at, s.protocol_version \
         FROM sessions s WHERE s.id = ?",
    )
    .bind(&session.session_id)
    .fetch_one(h.db.pool())
    .await
    .unwrap();
    assert!(messages >= 4, "messages: {messages}");
    assert!(ended.is_some());
    assert!(protocol.is_some());
    assert!(!h.sink.messages.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unknown_command_reports_an_error() {
    let h = harness().await;
    let server = h
        .registry
        .create(stdio_server("/definitely/not/a/binary", &[]))
        .await
        .unwrap();
    let error = h
        .manager
        .connect(&server.id, None)
        .await
        .err()
        .expect("must fail");
    assert!(error.to_string().contains("could not start"), "{error}");
    assert_eq!(states(&h.sink).last(), Some(&ConnectionState::Error));
    assert!(h.manager.session(&server.id).is_none());
}

#[tokio::test]
async fn stderr_of_a_failing_server_is_captured_and_secrets_are_masked() {
    let h = harness().await;
    h.secrets.set("tok", "sk-topsecret").unwrap();
    let mut input = stdio_server("sh", &["-c", "echo booting with $TOKEN >&2; echo oops >&2"]);
    input.env.insert("TOKEN".into(), "keyring:tok".into());
    let server = h.registry.create(input).await.unwrap();

    assert!(h.manager.connect(&server.id, None).await.is_err());
    settle().await;
    let logs = h.manager.logs(&server.id);
    let text: Vec<_> = logs.iter().map(|l| l.line.clone()).collect();
    assert!(text.iter().any(|l| l.contains("oops")), "{text:?}");
    assert!(text.iter().all(|l| !l.contains("sk-topsecret")), "{text:?}");
    assert!(
        text.iter()
            .any(|l| l.contains(mcp_studio_core::secrets::MASK)),
        "{text:?}"
    );
}

#[tokio::test]
async fn environment_variables_fill_placeholders_in_arguments() {
    let h = harness().await;
    let environment = h
        .environments
        .create(EnvironmentInput {
            name: "dev".into(),
            variables: BTreeMap::from([("mode".into(), "--http".into())]),
        })
        .await
        .unwrap();
    // `{{mode}}` becomes `--http`, so this starts the HTTP flavor of the server, which speaks
    // HTTP instead of stdio and therefore fails to initialize; the point is that resolution ran.
    let server = h
        .registry
        .create(stdio_server(&server_binary(), &["{{mode}}", "0"]))
        .await
        .unwrap();
    h.manager.set_connect_timeout(Duration::from_secs(2));
    let error = h
        .manager
        .connect(&server.id, Some(&environment.id))
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("did not answer"), "{error}");
    // Without the environment the placeholder is reported as undefined.
    let error = h.manager.connect(&server.id, None).await.err().unwrap();
    assert!(error.to_string().contains("mode"), "{error}");
}

#[tokio::test]
async fn connects_over_streamable_http_with_headers() {
    let h = harness().await;
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

    h.secrets.set("h", "Bearer abc").unwrap();
    let server = h
        .registry
        .create(ServerInput {
            name: "remote".into(),
            transport: TransportKind::Http,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            cwd: None,
            url: Some(url),
            headers: BTreeMap::from([("Authorization".into(), "keyring:h".into())]),
            tags: vec![],
        })
        .await
        .unwrap();

    let session = h
        .manager
        .connect(&server.id, None)
        .await
        .expect("connect over http");
    assert!(session
        .peer
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .any(|t| t.name == "add"));
    h.manager.disconnect(&server.id).await.unwrap();
    assert_eq!(states(&h.sink).last(), Some(&ConnectionState::Disconnected));
}

#[tokio::test]
async fn unexpected_exit_is_reported_as_error_and_reconnects() {
    let h = harness().await;
    // The unique marker argument lets us find exactly this server process (the server ignores it).
    let marker = format!("--marker-{}", mcp_studio_core::db::new_id());
    let server = h
        .registry
        .create(stdio_server(&server_binary(), &[&marker]))
        .await
        .unwrap();
    let session = h.manager.connect(&server.id, None).await.unwrap();
    let first_session = session.session_id.clone();

    // Kill the child behind the manager's back.
    let pid = std::process::Command::new("pgrep")
        .args(["-n", "-f", &marker])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    if pid.is_empty() {
        return; // pgrep unavailable on this machine
    }
    let _ = std::process::Command::new("kill").arg(&pid).status();

    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(now) = h.manager.session(&server.id) {
                if now.session_id != first_session {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("reconnected with a new session");
    assert!(states(&h.sink).contains(&ConnectionState::Error));
    h.manager.disconnect_all().await;
}

#[tokio::test]
async fn explorer_lists_tools_resources_prompts_and_details() {
    use mcp_studio_core::explorer;

    let h = harness().await;
    let server = h
        .registry
        .create(stdio_server(&server_binary(), &[]))
        .await
        .unwrap();
    let session = h.manager.connect(&server.id, None).await.unwrap();

    let details = explorer::details(&session.peer).unwrap();
    assert!(details.has_tools && details.has_resources && details.has_prompts);
    assert!(!details.protocol_version.is_empty());
    assert_eq!(
        details.instructions.as_deref(),
        Some("MCP Studio reference server")
    );

    let tools = explorer::list_tools(&session.peer).await.unwrap();
    let add = tools.iter().find(|t| t.name == "add").expect("add tool");
    assert_eq!(add.description.as_deref(), Some("Add two integers"));
    assert_eq!(add.input_schema.0["properties"]["a"]["type"], "integer");

    let resources = explorer::list_resources(&session.peer).await.unwrap();
    assert_eq!(resources[0].uri, "test://greeting");
    assert_eq!(resources[0].mime_type.as_deref(), Some("text/plain"));

    let prompts = explorer::list_prompts(&session.peer).await.unwrap();
    assert_eq!(prompts[0].name, "greet");
    assert!(prompts[0].arguments[0].required);

    assert!(explorer::list_resource_templates(&session.peer)
        .await
        .unwrap()
        .is_empty());
    h.manager.disconnect_all().await;
}

fn call(
    server_id: &str,
    tool: &str,
    arguments: serde_json::Value,
) -> mcp_studio_core::session::ToolCallRequest {
    mcp_studio_core::session::ToolCallRequest {
        server_id: server_id.to_owned(),
        tool_name: tool.to_owned(),
        arguments: mcp_studio_core::model::JsonValue(arguments),
        environment_id: None,
        call_id: mcp_studio_core::db::new_id(),
    }
}

#[tokio::test]
async fn calls_tools_and_reports_results_and_tool_errors() {
    let h = harness().await;
    let server = h
        .registry
        .create(stdio_server(&server_binary(), &[]))
        .await
        .unwrap();
    h.manager.connect(&server.id, None).await.unwrap();

    let sum = h
        .manager
        .call_tool(call(&server.id, "add", serde_json::json!({"a": 2, "b": 3})))
        .await
        .unwrap();
    assert!(!sum.is_error && !sum.cancelled);
    assert_eq!(sum.result.0["content"][0]["text"], "5");

    let failed = h
        .manager
        .call_tool(call(&server.id, "fail", serde_json::json!(null)))
        .await
        .unwrap();
    assert!(failed.is_error);

    // Wrong argument types are a protocol error, not a tool result.
    let invalid = h
        .manager
        .call_tool(call(&server.id, "add", serde_json::json!({"a": "x"})))
        .await;
    assert!(invalid.is_err() || invalid.unwrap().is_error);
    let unknown = h
        .manager
        .call_tool(call(&server.id, "nope", serde_json::json!({})))
        .await;
    assert!(unknown.is_err() || unknown.unwrap().is_error);

    let not_object = h
        .manager
        .call_tool(call(&server.id, "add", serde_json::json!([1, 2])))
        .await;
    assert!(not_object.unwrap_err().to_string().contains("JSON object"));
    h.manager.disconnect_all().await;
}

#[tokio::test]
async fn call_requires_a_connection() {
    let h = harness().await;
    let error = h
        .manager
        .call_tool(call("missing", "add", serde_json::json!({})))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not connected"), "{error}");
}

#[tokio::test]
async fn tool_arguments_use_environment_variables() {
    let h = harness().await;
    let environment = h
        .environments
        .create(EnvironmentInput {
            name: "dev".into(),
            variables: BTreeMap::from([("greeting".into(), "hello there".into())]),
        })
        .await
        .unwrap();
    let server = h
        .registry
        .create(stdio_server(&server_binary(), &[]))
        .await
        .unwrap();
    h.manager.connect(&server.id, None).await.unwrap();

    let mut request = call(
        &server.id,
        "echo",
        serde_json::json!({"message": "{{greeting}}!"}),
    );
    request.environment_id = Some(environment.id.clone());
    let echoed = h.manager.call_tool(request).await.unwrap();
    assert_eq!(echoed.result.0["content"][0]["text"], "hello there!");

    let missing = h
        .manager
        .call_tool(call(
            &server.id,
            "echo",
            serde_json::json!({"message": "{{nope}}"}),
        ))
        .await;
    assert!(missing.unwrap_err().to_string().contains("nope"));
    h.manager.disconnect_all().await;
}

#[tokio::test]
async fn running_calls_can_be_cancelled() {
    let h = harness().await;
    let server = h
        .registry
        .create(stdio_server(&server_binary(), &[]))
        .await
        .unwrap();
    h.manager.connect(&server.id, None).await.unwrap();

    let request = call(&server.id, "sleep", serde_json::json!({"ms": 20_000}));
    let call_id = request.call_id.clone();
    let manager = h.manager.clone();
    let running = tokio::spawn(async move { manager.call_tool(request).await });

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(h.manager.cancel_call(&call_id));
    let outcome = tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .expect("returns quickly")
        .unwrap()
        .unwrap();
    assert!(outcome.cancelled);
    assert!(outcome.duration_ms < 5_000);
    assert!(
        !h.manager.cancel_call(&call_id),
        "finished calls are unknown"
    );

    // The connection is still usable, and the cancellation was sent to the server.
    let sum = h
        .manager
        .call_tool(call(&server.id, "add", serde_json::json!({"a": 1, "b": 1})))
        .await
        .unwrap();
    assert_eq!(sum.result.0["content"][0]["text"], "2");
    settle().await;
    let sent: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE method = 'notifications/cancelled'",
    )
    .fetch_one(h.db.pool())
    .await
    .unwrap();
    assert_eq!(sent, 1);
    h.manager.disconnect_all().await;
}
